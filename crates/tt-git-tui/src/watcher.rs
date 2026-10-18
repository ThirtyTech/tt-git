//! Watches the working tree and `.git` so the UI refreshes when something
//! outside tt-git (an editor, another terminal) changes the repo.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer};

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

/// Starts watching `root` (recursively, which includes a `.git` inside it)
/// and `git_dir`, emitting `Action::RepoChanged` after changes settle.
pub fn watch(root: &Path, git_dir: &Path, ctx: Ctx) -> notify_debouncer_mini::notify::Result<RepoWatcher> {
    let root_buf: PathBuf = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let git_buf: PathBuf = std::fs::canonicalize(git_dir).unwrap_or_else(|_| git_dir.to_path_buf());
    let (r, g) = (root_buf.clone(), git_buf.clone());
    let mut debouncer = new_debouncer(DEBOUNCE, move |res: DebounceEventResult| {
        let Ok(events) = res else {
            return;
        };
        let mut change = Change::default();
        for ev in &events {
            let path = std::fs::canonicalize(&ev.path).unwrap_or_else(|_| ev.path.clone());
            let c = classify(&path, &r, &g);
            change.refs |= c.refs;
            change.status |= c.status;
        }
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
}
