//! Watches the working tree and `.git` so the UI refreshes when something
//! outside tt-git (an editor, another terminal) changes the repo.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer};

use tt_git_core::{IgnoreRules, Repo};

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

const STATUS_ONLY: Change = Change {
    refs: false,
    status: true,
};

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

/// What the watcher knows of git's ignore rules.
enum Rules {
    /// Not loaded since something that may change them did.
    Unknown,
    Loaded(Box<IgnoreRules>),
    /// Loading failed; nothing is dropped until something that may change
    /// them does.
    Unavailable,
}

/// The watcher's view of the repo, which turns a batch of changed paths
/// into a [`Change`].
struct Watch {
    /// Canonical, as the watch was started on them.
    root: PathBuf,
    git_dir: PathBuf,
    repo: Repo,
    rules: Rules,
}

impl Watch {
    /// Changes git ignores (and doesn't track) are dropped, so a build
    /// writing into `target/` doesn't reload the status over and over.
    fn batch<'a>(&mut self, paths: impl IntoIterator<Item = &'a Path>) -> Change {
        if matches!(&self.rules, Rules::Loaded(r) if r.stale()) {
            self.rules = Rules::Unknown;
        }
        let mut real_dirs = HashMap::new();
        let mut change = Change::default();
        let mut reload_rules = false;
        for raw in paths {
            let path = self.resolve(raw);
            let c = classify(&path, &self.root, &self.git_dir);
            if c == STATUS_ONLY && !path.starts_with(&self.git_dir) {
                if let Some(rules) = self.rules() {
                    // Seen by way of a symlinked directory, it can be
                    // somewhere else that counts.
                    if rules.hides(&path) && real_parent(raw, &mut real_dirs).is_some_and(|real| rules.hides(&real)) {
                        continue;
                    }
                    reload_rules |= rules.affected_by(&path);
                }
            } else if path
                .strip_prefix(&self.git_dir)
                .is_ok_and(|rel| rel == Path::new("index"))
            {
                // What's tracked may have changed.
                reload_rules = true;
            }
            change.refs |= c.refs;
            change.status |= c.status;
        }
        if reload_rules {
            self.rules = Rules::Unknown;
        }
        change
    }

    /// The rules, loaded if they aren't.
    fn rules(&mut self) -> Option<&IgnoreRules> {
        if let Rules::Unknown = self.rules {
            self.rules = match IgnoreRules::load(&self.repo, &self.root) {
                Ok(rules) => Rules::Loaded(Box::new(rules)),
                Err(e) => {
                    tracing::warn!("not skipping ignored changes: {e}");
                    Rules::Unavailable
                }
            };
        }
        match &self.rules {
            Rules::Loaded(rules) => Some(rules),
            _ => None,
        }
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
}

/// Where `path` really is: its directory canonicalized, plus its name. A
/// directory that's gone is resolved through its parent. `cache` holds the
/// directories resolved so far.
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

/// Starts watching `repo`'s working tree (recursively, which includes a
/// `.git` inside it) and git dir, emitting `Action::RepoChanged` after
/// changes settle.
pub fn watch(repo: Repo, ctx: Ctx) -> notify_debouncer_mini::notify::Result<RepoWatcher> {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let root_buf = canonical(repo.root());
    let git_buf = canonical(repo.git_dir());
    let mut state = Watch {
        root: root_buf.clone(),
        git_dir: git_buf.clone(),
        repo,
        rules: Rules::Unknown,
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

    const BOTH: Change = Change {
        refs: true,
        status: true,
    };

    /// A repo (canonical, as the watcher has it) that ignores `target/`.
    struct Fixture {
        sb: tt_git_testkit::Sandbox,
        root: PathBuf,
        watch: Watch,
    }

    impl Fixture {
        fn new() -> Self {
            let sb = tt_git_testkit::Sandbox::new();
            sb.write(".gitignore", "target/\n");
            sb.write("src/main.rs", "fn main() {}\n");
            sb.git(&["add", "."]);
            sb.commit_staged("base");
            sb.write("target/debug/a.o", "o");
            let git = tt_git_core::GitCli::new(".").with_env(sb.env());
            let repo = Repo::discover(sb.path(), &git).unwrap();
            let root = std::fs::canonicalize(repo.root()).unwrap();
            let watch = Watch {
                git_dir: std::fs::canonicalize(repo.git_dir()).unwrap(),
                root: root.clone(),
                repo,
                rules: Rules::Unknown,
            };
            Self { sb, root, watch }
        }

        fn batch(&mut self, rels: &[&str]) -> Change {
            let paths: Vec<PathBuf> = rels.iter().map(|r| self.root.join(r)).collect();
            self.watch.batch(paths.iter().map(PathBuf::as_path))
        }

        /// git commands run so far.
        fn git_runs(&self) -> usize {
            self.watch.repo.git().log().entries().len()
        }
    }

    #[test]
    fn drops_changes_git_ignores() {
        let mut f = Fixture::new();
        let build = ["target/debug/a.o", "target/debug/b.o", "target/new-dir"];
        assert_eq!(f.batch(&build), Change::default());
        let loaded = f.git_runs();
        for _ in 0..3 {
            assert_eq!(f.batch(&build), Change::default());
        }
        assert_eq!(f.git_runs(), loaded, "rules loaded once");
        assert_eq!(f.batch(&["src/main.rs"]), STATUS_ONLY);
        assert_eq!(f.batch(&["targetx/y"]), STATUS_ONLY);
        assert_eq!(f.batch(&["target/x", ".git/refs/heads/main"]), BOTH);
        assert_eq!(f.git_runs(), loaded, "nothing there changes the rules");
    }

    #[test]
    fn the_ignored_dir_counts_once_it_is_not_a_directory() {
        let mut f = Fixture::new();
        assert_eq!(f.batch(&["target"]), Change::default());
        std::fs::remove_dir_all(f.root.join("target")).unwrap();
        std::fs::write(f.root.join("target"), "now a file").unwrap();
        assert_eq!(f.batch(&["target"]), STATUS_ONLY);
    }

    #[test]
    fn tracked_files_under_an_ignored_dir_count() {
        let mut f = Fixture::new();
        assert_eq!(f.batch(&["target/keep.txt"]), Change::default());
        f.sb.write("target/keep.txt", "x");
        f.sb.git(&["add", "-f", "target/keep.txt"]);
        // The index changed, so what's tracked is loaded again.
        assert_eq!(f.batch(&[".git/index"]), STATUS_ONLY);
        assert_eq!(f.batch(&["target/keep.txt"]), STATUS_ONLY);
        // Moving the directory moves the tracked file too.
        assert_eq!(f.batch(&["target"]), STATUS_ONLY);
        assert_eq!(f.batch(&["target/debug/a.o"]), Change::default());
    }

    #[test]
    fn a_gitignore_change_reloads_the_rules() {
        let mut f = Fixture::new();
        assert_eq!(f.batch(&["target/x"]), Change::default());
        f.sb.write(".gitignore", "");
        assert_eq!(f.batch(&[".gitignore"]), STATUS_ONLY);
        assert_eq!(f.batch(&["target/x"]), STATUS_ONLY);
        f.sb.write(".gitignore", "target/\n");
        assert_eq!(f.batch(&[".gitignore"]), STATUS_ONLY);
        assert_eq!(f.batch(&["target/x"]), Change::default());
        // A .gitignore inside an ignored directory doesn't count.
        let loaded = f.git_runs();
        assert_eq!(f.batch(&["target/.gitignore"]), Change::default());
        assert_eq!(f.batch(&["target/x"]), Change::default());
        assert_eq!(f.git_runs(), loaded);
    }

    #[test]
    fn a_directory_moved_in_with_a_gitignore_reloads_the_rules() {
        let mut f = Fixture::new();
        f.sb.write(".gitignore", "target/\n*.log\n");
        assert_eq!(f.batch(&[".gitignore"]), STATUS_ONLY);
        assert_eq!(f.batch(&["logs/a.log"]), Change::default());
        // logs/ arrives (all at once, as a rename does) un-ignoring its logs.
        f.sb.write("logs/.gitignore", "!*.log\n");
        f.sb.write("logs/a.log", "a");
        assert_eq!(f.batch(&["logs"]), STATUS_ONLY);
        assert_eq!(f.batch(&["logs/a.log"]), STATUS_ONLY);
        // And going away again.
        std::fs::remove_dir_all(f.root.join("logs")).unwrap();
        assert_eq!(f.batch(&["logs"]), STATUS_ONLY);
        assert_eq!(f.batch(&["logs/a.log"]), Change::default());
    }

    #[test]
    fn an_info_exclude_change_reloads_the_rules() {
        let mut f = Fixture::new();
        assert_eq!(f.batch(&["scratch.txt"]), STATUS_ONLY);
        let exclude = f.watch.repo.common_dir().join("info").join("exclude");
        std::fs::create_dir_all(exclude.parent().unwrap()).unwrap();
        std::fs::write(&exclude, "scratch.txt\n").unwrap();
        assert_eq!(f.batch(&["scratch.txt"]), Change::default());
    }

    #[cfg(unix)]
    #[test]
    fn a_change_by_way_of_a_symlink_counts_where_it_really_is() {
        let mut f = Fixture::new();
        std::os::unix::fs::symlink(f.root.join("src"), f.root.join("target/link")).unwrap();
        assert_eq!(f.batch(&["target/link/main.rs"]), STATUS_ONLY);
    }

    #[test]
    fn deleted_paths_resolve_through_their_parents() {
        let mut f = Fixture::new();
        assert_eq!(f.batch(&["target/gone/deeper/file.o"]), Change::default());
    }
}
