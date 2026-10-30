//! Watches the working tree and `.git` so the UI refreshes when something
//! outside tt-git (an editor, another terminal) changes the repo.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use notify_debouncer_mini::notify::event::{AccessKind, AccessMode};
use notify_debouncer_mini::notify::{
    self, Event, EventHandler, EventKind, RecommendedWatcher, RecursiveMode, Watcher, WatcherKind,
};
use notify_debouncer_mini::{Config, DebounceEventResult, Debouncer, new_debouncer_opt};

use tt_git_core::{IgnoreRules, Repo};

use crate::action::Action;
use crate::jobs::Ctx;

const DEBOUNCE: Duration = Duration::from_millis(300);

/// Keeps the watch alive; dropping it stops watching.
pub struct RepoWatcher {
    _debouncer: Shared,
}

/// Shared with the handler (weakly, so dropping the [`RepoWatcher`] still
/// stops it), which watches folders as they appear (see
/// [`watches_folders`]).
type Shared = Arc<Mutex<Debouncer<WriteWatcher>>>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// The platform's watcher, minus reads. Linux reports files being opened,
/// so every git command reading the index would look like a change to it,
/// and the reload that follows reads it again.
struct WriteWatcher(RecommendedWatcher);

impl Watcher for WriteWatcher {
    fn new<F: EventHandler>(mut handler: F, config: notify::Config) -> notify::Result<Self> {
        let inner = RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                if !res.as_ref().is_ok_and(|ev| only_reads(&ev.kind)) {
                    handler.handle_event(res);
                }
            },
            config,
        )?;
        Ok(Self(inner))
    }

    fn watch(&mut self, path: &Path, recursive_mode: RecursiveMode) -> notify::Result<()> {
        self.0.watch(path, recursive_mode)
    }

    fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
        self.0.unwatch(path)
    }

    fn configure(&mut self, option: notify::Config) -> notify::Result<bool> {
        self.0.configure(option)
    }

    fn kind() -> WatcherKind {
        RecommendedWatcher::kind()
    }
}

/// Whether `kind` says a file was only read: opened, read, or closed
/// without writing. Closing after a write still counts.
fn only_reads(kind: &EventKind) -> bool {
    matches!(kind, EventKind::Access(access) if *access != AccessKind::Close(AccessMode::Write))
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

const REFS_ONLY: Change = Change {
    refs: true,
    status: false,
};

/// Classifies a changed path. `git_dir` changes to refs/HEAD reload refs and
/// the graph; the index and worktree reload status. Object writes and lock
/// files are noise. `common_dir` is the shared `.git` (the same as `git_dir`
/// outside a linked worktree): its refs count, and so does a worktree's
/// `gitdir`, `locked` or `HEAD` under `worktrees/`, which change what the
/// worktree list shows. A worktree's index and logs change on every stage
/// and commit there and don't count.
pub fn classify(path: &Path, root: &Path, git_dir: &Path, common_dir: &Path) -> Change {
    let own = path.strip_prefix(git_dir).map(|rel| (rel, true));
    if let Ok((rel, own)) = own.or_else(|_| path.strip_prefix(common_dir).map(|rel| (rel, false))) {
        let mut parts = rel.components().map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        });
        let is_lock = path.extension().is_some_and(|e| e == "lock");
        return match parts.next().flatten() {
            _ if is_lock => Change::default(),
            Some("objects") | Some("hooks") | Some("info") | Some("description") => Change::default(),
            // Another worktree's index.
            Some("index") if !own => Change::default(),
            Some("index") => STATUS_ONLY,
            // Not FETCH_HEAD: a fetch rewrites it even when it brings nothing,
            // and refs it moved are reported themselves.
            Some("refs") | Some("reftable") | Some("HEAD") | Some("packed-refs") | Some("logs") | Some("shallow")
            | Some("ORIG_HEAD") | Some("MERGE_HEAD") => Change {
                refs: true,
                status: true,
            },
            Some("worktrees") => match (parts.next(), parts.next().flatten(), parts.next()) {
                (Some(_), Some("gitdir" | "locked" | "HEAD"), None) => REFS_ONLY,
                _ => Change::default(),
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
    common_dir: PathBuf,
    repo: Repo,
    rules: Rules,
    /// The linked worktrees, as their folders under `common_dir/worktrees` say.
    worktrees: Vec<Linked>,
    /// The watcher, when it watches folders one at a time (see
    /// [`watches_folders`]).
    folders: Option<Weak<Mutex<Debouncer<WriteWatcher>>>>,
}

/// inotify (Linux) watches one folder at a time, so a recursive watch is a
/// walk adding a watch to every folder, `node_modules/` and `target/`
/// included: tens of thousands of them, which takes seconds. With inotify,
/// the watcher walks the tree itself instead, leaving out the folders whose
/// changes it would drop anyway, and watches folders as they appear.
/// Elsewhere one watch covers a whole tree.
fn watches_folders() -> bool {
    RecommendedWatcher::kind() == WatcherKind::Inotify
}

impl Watch {
    /// Changes git ignores (and doesn't track) are dropped, so a build
    /// writing into `target/` doesn't reload the status over and over.
    ///
    /// When it [watches folders](watches_folders), it also watches the
    /// folders that appeared, or all of them again when the rules changed.
    fn batch<'a>(&mut self, paths: impl IntoIterator<Item = &'a Path>) -> Change {
        let mut rescan = false;
        if matches!(&self.rules, Rules::Loaded(r) if r.stale()) {
            self.rules = Rules::Unknown;
            rescan = true;
        }
        let mut real_dirs = HashMap::new();
        let mut change = Change::default();
        let mut reload_rules = false;
        let mut worktrees_touched = false;
        let mut appeared = Vec::new();
        for raw in paths {
            let path = self.resolve(raw);
            let c = classify(&path, &self.root, &self.git_dir, &self.common_dir);
            worktrees_touched |= path.starts_with(self.common_dir.join("worktrees"));
            // A linked worktree's folder deleted by hand makes it prunable,
            // though the folder is ignored. (On Windows it's also reported
            // when a file right inside it is written, which doesn't count.)
            if self.worktrees.iter().any(|w| w.root.as_deref() == Some(&*path)) && !path.exists() {
                change.refs = true;
            }
            if c == STATUS_ONLY && !path.starts_with(&self.git_dir) && !path.starts_with(&self.common_dir) {
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
            if self.folders.is_some() && path.is_dir() {
                appeared.push(path.into_owned());
            }
            change.refs |= c.refs;
            change.status |= c.status;
        }
        if reload_rules {
            self.rules = Rules::Unknown;
        }
        // A worktree's folder under `worktrees/` coming or going may be all
        // that's reported: its files can be gone, or written before a new
        // folder is watched. The folder itself counts only if the worktrees
        // changed, since on Windows it's reported whenever a file in it is
        // written.
        if worktrees_touched {
            let now = linked_worktrees(&self.common_dir);
            if now != self.worktrees {
                self.worktrees = now;
                change.refs = true;
                rescan = true;
            }
        }
        if self.folders.is_some() {
            if rescan || reload_rules {
                // Folders the rules ignored may count now, and so may the
                // way to a new worktree.
                appeared.retain(|dir| self.in_git_dirs(dir));
                appeared.push(self.root.clone());
            }
            for dir in appeared {
                self.watch_folders(&dir);
            }
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
            .or_else(|_| raw.strip_prefix(&self.common_dir))
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

    fn in_git_dirs(&self, path: &Path) -> bool {
        path.starts_with(&self.git_dir) || path.starts_with(&self.common_dir)
    }

    /// Watches `top` and the folders under it, each on its own, leaving
    /// out those whose changes are dropped: folders git ignores (with
    /// nothing tracked in them), the git dirs' `objects`, and, under the
    /// root, the git dirs, which are walked from their own top. Ignored
    /// folders a linked worktree is in are watched, though not the
    /// worktree, so deleting it by hand is seen.
    fn watch_folders(&mut self, top: &Path) {
        let Some(debouncer) = self.folders.as_ref().and_then(Weak::upgrade) else {
            return;
        };
        let in_git = self.in_git_dirs(top);
        if !in_git {
            self.rules();
        }
        let rules = match &self.rules {
            Rules::Loaded(rules) if !in_git => Some(&**rules),
            _ => None,
        };
        let objects = [self.git_dir.join("objects"), self.common_dir.join("objects")];
        let mut real_dirs = HashMap::new();
        let mut skip = |path: &Path| {
            if objects.iter().any(|o| path == o) {
                return true;
            }
            if in_git {
                return false;
            }
            let on_the_way = |root: &Path| root != path && root.starts_with(path);
            path == self.git_dir
                || path == self.common_dir
                || rules.is_some_and(|r| {
                    r.hides(path)
                        && real_parent(path, &mut real_dirs).is_some_and(|real| r.hides(&real))
                        && !self.worktrees.iter().any(|w| w.root.as_deref().is_some_and(on_the_way))
                })
        };
        let mut debouncer = lock(&debouncer);
        let watcher = debouncer.watcher();
        let folders = walkdir::WalkDir::new(top)
            .follow_links(true)
            .into_iter()
            .filter_entry(|entry| entry.file_type().is_dir() && !skip(entry.path()));
        for folder in folders.filter_map(Result::ok) {
            match watcher.watch(folder.path(), RecursiveMode::NonRecursive) {
                Ok(()) => {}
                Err(e) if matches!(e.kind, notify::ErrorKind::MaxFilesWatch) => {
                    tracing::warn!("not watching every folder: {e}");
                    return;
                }
                // Gone already.
                Err(_) => {}
            }
        }
    }
}

/// A linked worktree, from its folder under `common_dir/worktrees`.
#[derive(Debug, PartialEq, Eq)]
struct Linked {
    name: OsString,
    /// Where its working tree is, from its `gitdir` file, canonical as
    /// events come.
    root: Option<PathBuf>,
}

/// The linked worktrees, by name.
fn linked_worktrees(common_dir: &Path) -> Vec<Linked> {
    let mut real_dirs = HashMap::new();
    let mut linked: Vec<Linked> = std::fs::read_dir(common_dir.join("worktrees"))
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .map(|entry| {
            // `<root>/.git`, with forward slashes on Windows too.
            let gitdir = std::fs::read_to_string(entry.path().join("gitdir")).ok();
            let root = gitdir
                .as_deref()
                .and_then(|g| Path::new(g.trim_end()).parent())
                .and_then(|root| real_parent(root, &mut real_dirs));
            Linked {
                name: entry.file_name(),
                root,
            }
        })
        .collect();
    linked.sort_by(|a, b| a.name.cmp(&b.name));
    linked
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
/// `.git` inside it), git dir and shared git dir, emitting
/// `Action::RepoChanged` after changes settle. With inotify, the folders
/// are watched one by one, without the ignored ones (see
/// [`watches_folders`]).
pub fn watch(repo: Repo, ctx: Ctx) -> notify_debouncer_mini::notify::Result<RepoWatcher> {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let root_buf = canonical(repo.root());
    let git_buf = canonical(repo.git_dir());
    let common_buf = canonical(repo.common_dir());
    let state = Arc::new(Mutex::new(Watch {
        root: root_buf.clone(),
        git_dir: git_buf.clone(),
        worktrees: linked_worktrees(&common_buf),
        common_dir: common_buf.clone(),
        repo,
        rules: Rules::Unknown,
        folders: None,
    }));
    let handler_state = Arc::clone(&state);
    let config = Config::default().with_timeout(DEBOUNCE);
    let mut debouncer = new_debouncer_opt::<_, WriteWatcher>(config, move |res: DebounceEventResult| {
        let Ok(events) = res else {
            return;
        };
        let change = lock(&handler_state).batch(events.iter().map(|ev| ev.path.as_path()));
        if change.refs || change.status {
            ctx.emit(Action::RepoChanged {
                refs: change.refs,
                status: change.status,
            });
        }
    })?;
    if watches_folders() {
        let debouncer = Arc::new(Mutex::new(debouncer));
        let mut state = lock(&state);
        state.folders = Some(Arc::downgrade(&debouncer));
        // The root's walk leaves out the git dirs. A linked worktree's git
        // dir is inside the shared one.
        state.watch_folders(&root_buf);
        state.watch_folders(&common_buf);
        if !git_buf.starts_with(&common_buf) {
            state.watch_folders(&git_buf);
        }
        drop(state);
        return Ok(RepoWatcher { _debouncer: debouncer });
    }
    // Each folder once: a linked worktree's git dir is inside the shared one.
    let mut dirs = vec![root_buf, git_buf, common_buf];
    dirs.sort_by_key(|d| d.as_os_str().len());
    let mut watched: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        if !watched.iter().any(|w| dir.starts_with(w)) {
            debouncer.watcher().watch(&dir, RecursiveMode::Recursive)?;
            watched.push(dir);
        }
    }
    Ok(RepoWatcher {
        _debouncer: Arc::new(Mutex::new(debouncer)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_paths() {
        let root = Path::new("/r");
        let git = Path::new("/r/.git");
        let c = |p: &str| classify(Path::new(p), root, git, git);
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

        // A fetch writes FETCH_HEAD whatever it brings; deepening a shallow
        // history moves no ref, and a reftable has no ref files to report.
        assert_eq!(c("/r/.git/FETCH_HEAD"), Change::default());
        assert_eq!(c("/r/.git/shallow"), BOTH);
        assert_eq!(c("/r/.git/reftable/tables.list"), BOTH);

        // Adding, removing, moving and locking a linked worktree.
        assert_eq!(c("/r/.git/worktrees/side/gitdir"), REFS_ONLY);
        assert_eq!(c("/r/.git/worktrees/side/locked"), REFS_ONLY);
        assert_eq!(c("/r/.git/worktrees/side/HEAD"), REFS_ONLY);
        assert_eq!(c("/r/.git/worktrees/side/HEAD.lock"), Change::default());
        // Folders count only if the set of them changed; see `Watch::batch`.
        assert_eq!(c("/r/.git/worktrees"), Change::default());
        assert_eq!(c("/r/.git/worktrees/side"), Change::default());
        // Staging and committing in a linked worktree.
        assert_eq!(c("/r/.git/worktrees/side/index"), Change::default());
        assert_eq!(c("/r/.git/worktrees/side/logs/HEAD"), Change::default());
        assert_eq!(c("/r/.git/worktrees/side/ORIG_HEAD"), Change::default());
        assert_eq!(c("/r/.git/worktrees/side/refs/bisect/bad"), Change::default());
    }

    #[test]
    fn classifies_paths_in_a_linked_worktree() {
        let root = Path::new("/r/.worktrees/side");
        let common = Path::new("/r/.git");
        let git = Path::new("/r/.git/worktrees/side");
        let c = |p: &str| classify(Path::new(p), root, git, common);
        assert_eq!(c("/r/.worktrees/side/src/main.rs"), STATUS_ONLY);
        assert_eq!(c("/r/.git/worktrees/side/index"), STATUS_ONLY);
        assert_eq!(c("/r/.git/worktrees/side/HEAD"), BOTH);
        assert_eq!(c("/r/.git/worktrees/side/logs/HEAD"), BOTH);
        // The shared refs.
        assert_eq!(c("/r/.git/refs/heads/main"), BOTH);
        assert_eq!(c("/r/.git/packed-refs"), BOTH);
        // The main worktree and the other linked ones.
        assert_eq!(c("/r/.git/HEAD"), BOTH);
        assert_eq!(c("/r/.git/index"), Change::default());
        assert_eq!(c("/r/.git/worktrees/other/gitdir"), REFS_ONLY);
        assert_eq!(c("/r/.git/worktrees/other/index"), Change::default());
        assert_eq!(c("/r/.git/objects/ab/cdef"), Change::default());
        // The main worktree's files.
        assert_eq!(c("/r/README.md"), Change::default());
    }

    #[test]
    fn reads_are_not_changes() {
        use notify::event::{CreateKind, DataChange, ModifyKind};
        let read = [
            AccessKind::Open(AccessMode::Any),
            AccessKind::Read,
            AccessKind::Close(AccessMode::Read),
            AccessKind::Any,
        ];
        for access in read {
            assert!(only_reads(&EventKind::Access(access)), "{access:?}");
        }
        let written = [
            EventKind::Access(AccessKind::Close(AccessMode::Write)),
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            EventKind::Create(CreateKind::File),
            EventKind::Any,
        ];
        for kind in written {
            assert!(!only_reads(&kind), "{kind:?}");
        }
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
            let git_dir = std::fs::canonicalize(repo.git_dir()).unwrap();
            let watch = Watch {
                worktrees: linked_worktrees(&git_dir),
                common_dir: git_dir.clone(),
                git_dir,
                root: root.clone(),
                repo,
                rules: Rules::Unknown,
                folders: None,
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
    fn a_worktree_folder_counts_when_the_set_of_them_changes() {
        let mut f = Fixture::new();
        std::fs::write(f.root.join(".git/info/exclude"), "/.worktrees/\n").unwrap();
        let none = Change::default();
        assert_eq!(f.batch(&[".git/worktrees"]), none);
        f.sb.git(&["worktree", "add", "-q", ".worktrees/side"]);
        // Only the folder, as when its files were written before it was watched.
        assert_eq!(f.batch(&[".git/worktrees/side"]), REFS_ONLY);
        // A stage in it touches the folder on Windows, but the set is the same.
        assert_eq!(f.batch(&[".git/worktrees/side", ".git/worktrees/side/index"]), none);
        f.sb.git(&["worktree", "remove", ".worktrees/side"]);
        assert_eq!(f.batch(&[".git/worktrees"]), REFS_ONLY);
        assert_eq!(f.batch(&[".git/worktrees"]), none);

        // Moved: the same folder under `worktrees/`, pointing elsewhere.
        f.sb.git(&["worktree", "add", "-q", ".worktrees/a"]);
        assert_eq!(f.batch(&[".git/worktrees/a"]), REFS_ONLY);
        f.sb.git(&["worktree", "move", ".worktrees/a", ".worktrees/b"]);
        assert_eq!(f.batch(&[".git/worktrees/a"]), REFS_ONLY);
    }

    #[test]
    fn a_worktree_deleted_by_hand_counts_though_it_is_ignored() {
        let mut f = Fixture::new();
        std::fs::write(f.root.join(".git/info/exclude"), "/.worktrees/\n").unwrap();
        f.sb.git(&["worktree", "add", "-q", ".worktrees/side"]);
        assert_eq!(f.batch(&[".git/worktrees/side/gitdir"]), REFS_ONLY);
        assert_eq!(f.batch(&[".worktrees/side/file.txt"]), Change::default());
        assert_eq!(f.batch(&[".worktrees/side"]), Change::default());
        std::fs::remove_dir_all(f.root.join(".worktrees/side")).unwrap();
        assert_eq!(f.batch(&[".worktrees/side"]), REFS_ONLY);
    }

    #[test]
    fn deleted_paths_resolve_through_their_parents() {
        let mut f = Fixture::new();
        assert_eq!(f.batch(&["target/gone/deeper/file.o"]), Change::default());
    }
}
