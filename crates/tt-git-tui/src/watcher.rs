//! Watches the working tree and `.git` so the UI refreshes when something
//! outside tt-git (an editor, another terminal) changes the repo.

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::Hasher;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer};

use tt_git_core::IgnoreRules;

use crate::action::Action;
use crate::jobs::Ctx;

const DEBOUNCE: Duration = Duration::from_millis(300);

/// Keeps the watch alive; dropping it stops watching.
pub struct RepoWatcher {
    _debouncer: Debouncer<RecommendedWatcher>,
}

/// What a changed path means for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Change {
    pub refs: bool,
    pub status: bool,
}

/// Classifies a changed path. `git_dir` changes to refs/HEAD reload refs and
/// the graph; the index and worktree reload status. Object writes and lock
/// files are noise.
pub fn classify(path: &Path, root: &Path, git_dir: &Path) -> Change {
    if let Ok(rel) = path.strip_prefix(git_dir) {
        let first = rel.components().next().and_then(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        });
        let is_lock = path.extension().is_some_and(|e| e == "lock");
        return match first {
            _ if is_lock => Change::default(),
            Some("objects") | Some("hooks") | Some("info") | Some("description") => Change::default(),
            Some("index") => Change {
                refs: false,
                status: true,
            },
            Some("refs") | Some("HEAD") | Some("packed-refs") | Some("logs") | Some("FETCH_HEAD")
            | Some("ORIG_HEAD") | Some("MERGE_HEAD") => Change {
                refs: true,
                status: true,
            },
            _ => Change::default(),
        };
    }
    if path.starts_with(root) {
        return Change {
            refs: false,
            status: true,
        };
    }
    Change::default()
}

/// Untracked directories git ignores as a whole (`target/` and the like),
/// as of the last status load, so the watcher can drop changes inside them
/// instead of reloading the status for each build step.
///
/// Status loads look for them only once the watcher has seen a change in
/// the working tree, and never while git may use its untracked cache
/// (`--ignored` turns it off).
///
/// Dropping is only safe while nothing that decides what git ignores has
/// changed, so any change the watcher doesn't drop forgets the directories
/// until the next status load (one that started after it) finds them
/// again. Ignore rules can also come from files outside the watch (the
/// global excludes file, config files): their contents are checked before
/// each drop, and a difference forgets the directories too.
#[derive(Debug, Default)]
pub struct IgnoredDirs(Mutex<Ignored>);

#[derive(Debug, Default)]
struct Ignored {
    /// Bumped whenever the directories are forgotten, so a status load that
    /// started before can't put back what it found.
    epoch: u64,
    /// Relative to the root.
    dirs: Option<Arc<HashSet<PathBuf>>>,
    /// The rule files as they were when the status load that found `dirs`
    /// started. Without them nothing is dropped.
    rules: Option<Arc<RuleFiles>>,
    /// The rule files, listed once the directories were wanted.
    listed: Option<Arc<RuleFiles>>,
    /// The watcher saw a working tree change with no directories to check
    /// it against, so status loads find them (listing the rule files first).
    wanted: bool,
    /// They can't all be located, so nothing is ever dropped.
    unlistable: bool,
}

/// Files that decide what git ignores, besides the `.gitignore` files in
/// the tree, with what they held when listed.
#[derive(Debug, PartialEq, Eq)]
struct RuleFiles {
    rules: IgnoreRules,
    stamps: Vec<Stamp>,
}

/// A file's length and a hash of its contents, or why it couldn't be read.
type Stamp = Result<(u64, u64), std::io::ErrorKind>;

fn stamps(paths: &[PathBuf]) -> Vec<Stamp> {
    paths
        .iter()
        .map(|p| {
            let content = std::fs::read(p).map_err(|e| e.kind())?;
            let mut hash = DefaultHasher::new();
            hash.write(&content);
            Ok((content.len() as u64, hash.finish()))
        })
        .collect()
}

/// Taken by a status load before it runs git; see [`IgnoredDirs::finish`].
#[derive(Debug)]
pub struct StatusToken {
    epoch: u64,
    rules: Option<Arc<RuleFiles>>,
}

impl StatusToken {
    /// Whether the load should find the ignored directories (and pass them
    /// to [`IgnoredDirs::finish`]), rather than run the plain status.
    pub fn wants_dirs(&self) -> bool {
        self.rules.as_ref().is_some_and(|r| !r.rules.untracked_cache)
    }
}

impl IgnoredDirs {
    fn lock(&self) -> MutexGuard<'_, Ignored> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Called by a status load before it runs git. `list` gives the rule
    /// files (`None` when they can't all be located), and runs only if they
    /// haven't been listed yet but are wanted, or one of them changed since.
    pub fn begin(&self, list: impl Fn() -> Option<IgnoreRules>) -> StatusToken {
        let (epoch, mut listed, wanted) = {
            let inner = self.lock();
            (inner.epoch, inner.listed.clone(), inner.wanted && !inner.unlistable)
        };
        // A changed file may include others now, so list them again.
        if let Some(files) = &listed
            && stamps(&files.rules.files) != files.stamps
        {
            listed = None;
        }
        if listed.is_none() && wanted {
            // Listing again after stamping catches a config edit made
            // between the two: had it added an include, the lists differ
            // (and the next load tries again).
            let mut unlistable = false;
            listed = match list() {
                Some(rules) => {
                    let stamps = stamps(&rules.files);
                    match list() {
                        Some(again) if again == rules => Some(Arc::new(RuleFiles { rules, stamps })),
                        Some(_) => None,
                        None => {
                            unlistable = true;
                            None
                        }
                    }
                }
                None => {
                    unlistable = true;
                    None
                }
            };
            let mut inner = self.lock();
            inner.listed = listed.clone();
            inner.unlistable |= unlistable;
        }
        StatusToken { epoch, rules: listed }
    }

    /// Records the directories a status load (begun with `token`) found,
    /// unless something forgot them since it began or it wasn't to look.
    pub fn finish(&self, token: StatusToken, dirs: Vec<String>) {
        let mut inner = self.lock();
        if inner.epoch != token.epoch || !token.wants_dirs() {
            return;
        }
        inner.dirs = Some(Arc::new(dirs.into_iter().map(PathBuf::from).collect()));
        inner.rules = token.rules;
    }

    /// Forgets the directories until the next status load.
    fn forget(&self) {
        let mut inner = self.lock();
        inner.epoch += 1;
        inner.dirs = None;
        inner.rules = None;
    }

    fn dirs(&self) -> Option<Arc<HashSet<PathBuf>>> {
        self.lock().dirs.clone()
    }

    /// Has status loads find the directories from now on.
    fn want(&self) {
        self.lock().wanted = true;
    }

    /// Whether no rule file changed since the directories were found. If
    /// one did they're forgotten.
    fn rules_unchanged(&self) -> bool {
        let Some(rules) = self.lock().rules.clone() else {
            return false;
        };
        if stamps(&rules.rules.files) == rules.stamps {
            return true;
        }
        self.forget();
        false
    }
}

/// The watcher's view of the repo, which turns a batch of changed paths
/// into a [`Change`].
struct Watch {
    /// Canonical, as the watch was started on them.
    root: PathBuf,
    git_dir: PathBuf,
    ignored: Arc<IgnoredDirs>,
}

impl Watch {
    fn batch<'a>(&self, paths: impl IntoIterator<Item = &'a Path>) -> Change {
        let dirs = self.ignored.dirs();
        let mut real_dirs = HashMap::new();
        let mut change = Change::default();
        let (mut kept, mut dropped, mut worktree) = (false, false, false);
        for raw in paths {
            let path = self.resolve(raw);
            let c = classify(&path, &self.root, &self.git_dir);
            let worktree_only =
                c == Change {
                    refs: false,
                    status: true,
                } && !path.starts_with(&self.git_dir);
            worktree |= worktree_only;
            if worktree_only
                && let Some(dirs) = &dirs
                && self.in_ignored_dir(&path, dirs)
                && real_parent(raw, &mut real_dirs).is_some_and(|real| self.in_ignored_dir(&real, dirs))
            {
                dropped = true;
                continue;
            }
            kept = true;
            change.refs |= c.refs;
            change.status |= c.status;
            if change.refs && change.status {
                break;
            }
        }
        if worktree && dirs.is_none() {
            self.ignored.want();
        }
        if kept {
            // It might have changed what's ignored. Changes dropped beside
            // it are only safe to drop when nothing else changed, so they
            // count as they always did.
            self.ignored.forget();
            change.status |= dropped;
        } else if dropped && !self.ignored.rules_unchanged() {
            change.status = true;
        }
        change
    }

    /// `raw` as given, if it's plainly inside the root or git dir (which
    /// are canonical, and events come relative to them); otherwise, or if
    /// a part could be an 8.3 short name, canonicalized.
    fn resolve<'p>(&self, raw: &'p Path) -> Cow<'p, Path> {
        let rel = raw
            .strip_prefix(&self.git_dir)
            .or_else(|_| raw.strip_prefix(&self.root));
        let plain = rel.is_ok_and(|rel| {
            rel.components()
                .all(|c| matches!(c, Component::Normal(s) if !s.as_encoded_bytes().contains(&b'~')))
        });
        if plain {
            return Cow::Borrowed(raw);
        }
        match std::fs::canonicalize(raw) {
            Ok(path) => Cow::Owned(path),
            Err(_) => Cow::Borrowed(raw),
        }
    }

    /// Whether `path` is inside one of the ignored `dirs`, or is one that's
    /// still a directory (its modified time changes as files come and go;
    /// replaced by a file or a symlink, git would list it).
    fn in_ignored_dir(&self, path: &Path, dirs: &HashSet<PathBuf>) -> bool {
        let Ok(rel) = path.strip_prefix(&self.root) else {
            return false;
        };
        if dirs.contains(rel) {
            return std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir());
        }
        rel.ancestors()
            .skip(1)
            .take_while(|a| !a.as_os_str().is_empty())
            .any(|a| dirs.contains(a))
    }
}

/// Where `path` really is: its directory canonicalized (an event can come
/// by way of a symlinked directory), plus its name. A directory that's gone
/// is resolved through its parent. `cache` holds the directories resolved
/// so far.
fn real_parent(path: &Path, cache: &mut HashMap<PathBuf, Option<PathBuf>>) -> Option<PathBuf> {
    fn real_dir(dir: &Path, cache: &mut HashMap<PathBuf, Option<PathBuf>>) -> Option<PathBuf> {
        if let Some(real) = cache.get(dir) {
            return real.clone();
        }
        let real = match std::fs::canonicalize(dir) {
            Ok(real) => Some(real),
            Err(_) => match (dir.parent(), dir.file_name()) {
                (Some(parent), Some(name)) => real_dir(parent, cache).map(|p| p.join(name)),
                _ => None,
            },
        };
        cache.insert(dir.to_path_buf(), real.clone());
        real
    }
    let (parent, name) = (path.parent()?, path.file_name()?);
    real_dir(parent, cache).map(|p| p.join(name))
}

/// Starts watching `root` (recursively, which includes a `.git` inside it)
/// and `git_dir`, emitting `Action::RepoChanged` after changes settle.
/// Changes inside the directories in `ignored` don't count while it says
/// they can't matter.
pub fn watch(
    root: &Path,
    git_dir: &Path,
    ignored: Arc<IgnoredDirs>,
    ctx: Ctx,
) -> notify_debouncer_mini::notify::Result<RepoWatcher> {
    let root_buf: PathBuf = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let git_buf: PathBuf = std::fs::canonicalize(git_dir).unwrap_or_else(|_| git_dir.to_path_buf());
    let state = Watch {
        root: root_buf.clone(),
        git_dir: git_buf.clone(),
        ignored,
    };
    let mut debouncer = new_debouncer(DEBOUNCE, move |res: DebounceEventResult| {
        let Ok(events) = res else {
            return;
        };
        let change = state.batch(events.iter().map(|ev| ev.path.as_path()));
        if change.refs || change.status {
            ctx.emit(Action::RepoChanged {
                refs: change.refs,
                status: change.status,
            });
        }
    })?;
    debouncer.watcher().watch(&root_buf, RecursiveMode::Recursive)?;
    if !git_buf.starts_with(&root_buf) {
        debouncer.watcher().watch(&git_buf, RecursiveMode::Recursive)?;
    }
    Ok(RepoWatcher { _debouncer: debouncer })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_paths() {
        let root = Path::new("/r");
        let git = Path::new("/r/.git");
        let c = |p: &str| classify(Path::new(p), root, git);
        assert_eq!(
            c("/r/src/main.rs"),
            Change {
                refs: false,
                status: true
            }
        );
        assert_eq!(
            c("/r/.git/index"),
            Change {
                refs: false,
                status: true
            }
        );
        assert_eq!(
            c("/r/.git/HEAD"),
            Change {
                refs: true,
                status: true
            }
        );
        assert_eq!(
            c("/r/.git/refs/heads/main"),
            Change {
                refs: true,
                status: true
            }
        );
        assert_eq!(c("/r/.git/objects/ab/cdef"), Change::default());
        assert_eq!(c("/r/.git/index.lock"), Change::default());
        assert_eq!(c("/r/.git/refs/heads/main.lock"), Change::default());
        assert_eq!(c("/elsewhere/x"), Change::default());
    }

    const STATUS: Change = Change {
        refs: false,
        status: true,
    };
    const BOTH: Change = Change {
        refs: true,
        status: true,
    };

    /// A real directory (canonical, as the watcher has it) with `target/`
    /// and `src/` in it, and a rule file outside it.
    struct Fixture {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        rule: PathBuf,
        watch: Watch,
    }

    impl Fixture {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let base = std::fs::canonicalize(tmp.path()).unwrap();
            let root = base.join("repo");
            for dir in ["target/debug", "src", ".git/refs/heads"] {
                std::fs::create_dir_all(root.join(dir)).unwrap();
            }
            let rule = base.join("ignore");
            std::fs::write(&rule, "target/\n").unwrap();
            let watch = Watch {
                git_dir: root.join(".git"),
                root: root.clone(),
                ignored: Arc::default(),
            };
            Self {
                _tmp: tmp,
                root,
                rule,
                watch,
            }
        }

        /// The rule files as listed: the one rule file.
        fn rules(&self, untracked_cache: bool) -> IgnoreRules {
            IgnoreRules {
                files: vec![self.rule.clone()],
                untracked_cache,
            }
        }

        /// A status load that would find `dirs` ignored, as `load_status`
        /// does it. Whether it looked for them.
        fn status_load(&self, dirs: &[&str]) -> bool {
            self.status_load_with(dirs, false)
        }

        fn status_load_with(&self, dirs: &[&str], untracked_cache: bool) -> bool {
            let rules = self.rules(untracked_cache);
            let token = self.watch.ignored.begin(move || Some(rules.clone()));
            let looked = token.wants_dirs();
            self.watch
                .ignored
                .finish(token, dirs.iter().map(|d| d.to_string()).collect());
            looked
        }

        fn batch(&self, rels: &[&str]) -> Change {
            let paths: Vec<PathBuf> = rels.iter().map(|r| self.root.join(r)).collect();
            self.watch.batch(paths.iter().map(PathBuf::as_path))
        }
    }

    #[test]
    fn drops_changes_in_ignored_dirs_once_a_worktree_change_wanted_them() {
        let f = Fixture::new();
        let build = ["target/debug/a.o", "target/debug/b.o"];
        // Status loads don't look for them until the watcher sees a change
        // in the working tree.
        let listed = std::cell::Cell::new(0);
        let token = f.watch.ignored.begin(|| {
            listed.set(listed.get() + 1);
            None
        });
        assert!(!token.wants_dirs());
        assert_eq!(listed.get(), 0);
        f.watch.ignored.finish(token, vec!["target".into()]);
        assert_eq!(f.batch(&build), STATUS);
        // Which has the next load list the rule files and look.
        assert!(f.status_load(&["target"]));
        assert_eq!(listed.get(), 0, "with its own listing");
        assert_eq!(f.batch(&build), Change::default());
        assert_eq!(f.batch(&["target/new-dir"]), Change::default());
        assert_eq!(f.batch(&build), Change::default());
    }

    #[test]
    fn a_git_dir_change_does_not_want_the_dirs() {
        let f = Fixture::new();
        assert_eq!(f.batch(&[".git/refs/heads/main"]), BOTH);
        assert!(!f.status_load(&["target"]));
        assert_eq!(f.batch(&["target/x"]), STATUS);
        assert!(f.status_load(&["target"]));
    }

    #[test]
    fn nothing_is_looked_for_or_dropped_while_the_untracked_cache_is_used() {
        let f = Fixture::new();
        assert_eq!(f.batch(&["target/x"]), STATUS);
        for _ in 0..3 {
            assert!(!f.status_load_with(&["target"], true));
            assert_eq!(f.batch(&["target/x"]), STATUS);
        }
        // Once a rule file changes they're listed again, and might say
        // otherwise (here: the cache was turned off).
        std::fs::write(&f.rule, "target/\n# edited\n").unwrap();
        assert!(f.status_load_with(&["target"], false));
        assert_eq!(f.batch(&["target/x"]), Change::default());
    }

    #[test]
    fn the_ignored_dir_itself_and_other_paths_still_count() {
        let f = Fixture::new();
        f.status_load(&["target"]);
        f.batch(&["target/x"]);
        f.status_load(&["target"]);
        // The dir itself counts only once it's no longer a directory.
        assert_eq!(f.batch(&["target"]), Change::default());
        std::fs::remove_dir_all(f.root.join("target")).unwrap();
        assert_eq!(f.batch(&["target"]), STATUS);
        f.status_load(&["target"]);
        std::fs::write(f.root.join("target"), "now a file").unwrap();
        assert_eq!(f.batch(&["target"]), STATUS);
        f.status_load(&["target"]);
        assert_eq!(f.batch(&["targetx/y"]), STATUS);
        f.status_load(&["target"]);
        assert_eq!(f.batch(&["src/main.rs"]), STATUS);
    }

    #[test]
    fn any_other_change_forgets_the_dirs_until_the_next_load() {
        let f = Fixture::new();
        f.status_load(&["target"]);
        f.batch(&["target/x"]);
        f.status_load(&["target"]);
        // Changes beside another one count, as always.
        assert_eq!(f.batch(&["target/x", "src/.gitignore"]), STATUS);
        assert_eq!(f.batch(&["target/x"]), STATUS);
        // Even one that doesn't reload anything, like .git/info/exclude.
        f.status_load(&["target"]);
        assert_eq!(f.batch(&[".git/info/exclude"]), Change::default());
        assert_eq!(f.batch(&["target/x"]), STATUS);
        f.status_load(&["target"]);
        assert_eq!(f.batch(&["target/x", ".git/refs/heads/main"]), BOTH);
        assert_eq!(f.batch(&["target/x"]), STATUS);
        f.status_load(&["target"]);
        assert_eq!(f.batch(&["target/x"]), Change::default());
    }

    #[test]
    fn a_load_that_began_before_a_change_does_not_bring_the_dirs_back() {
        let f = Fixture::new();
        f.status_load(&["target"]);
        f.batch(&["target/x"]);
        let rules = f.rules(false);
        let token = f.watch.ignored.begin(move || Some(rules.clone()));
        assert!(token.wants_dirs());
        assert_eq!(f.batch(&["src/main.rs"]), STATUS);
        f.watch.ignored.finish(token, vec!["target".into()]);
        assert_eq!(f.batch(&["target/x"]), STATUS);
    }

    #[test]
    fn a_rule_file_change_forgets_the_dirs() {
        let f = Fixture::new();
        f.status_load(&["target"]);
        f.batch(&["target/x"]);
        f.status_load(&["target"]);
        assert_eq!(f.batch(&["target/x"]), Change::default());
        std::fs::write(&f.rule, "").unwrap();
        assert_eq!(f.batch(&["target/x"]), STATUS);
        assert_eq!(f.batch(&["target/x"]), STATUS);
        // The next load lists the files again and takes their new contents.
        f.status_load(&["target"]);
        assert_eq!(f.batch(&["target/x"]), Change::default());
    }

    #[test]
    fn rule_files_that_cannot_be_listed_mean_no_drops() {
        let f = Fixture::new();
        for _ in 0..3 {
            let token = f.watch.ignored.begin(|| None);
            f.watch.ignored.finish(token, vec!["target".into()]);
            assert_eq!(f.batch(&["target/x"]), STATUS);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_change_by_way_of_a_symlink_counts_where_it_really_is() {
        let f = Fixture::new();
        std::os::unix::fs::symlink(f.root.join("src"), f.root.join("target/link")).unwrap();
        f.status_load(&["target"]);
        f.batch(&["target/x"]);
        f.status_load(&["target"]);
        assert_eq!(f.batch(&["target/link/main.rs"]), STATUS);
    }

    #[test]
    fn deleted_paths_resolve_through_their_parents() {
        let f = Fixture::new();
        f.status_load(&["target"]);
        f.batch(&["target/x"]);
        f.status_load(&["target"]);
        assert_eq!(f.batch(&["target/gone/deeper/file.o"]), Change::default());
    }
}
