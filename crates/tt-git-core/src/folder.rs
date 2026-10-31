//! Telling whether a folder can be deleted, on Windows.

use std::path::Path;
use std::time::Duration;

use crate::git::{GitError, Result};

/// How many times a folder is tried before it counts as in use, and the
/// wait between tries.
const TRIES: u32 = 6;
const WAIT: Duration = Duration::from_millis(200);

/// Refuses if something has the folder at `path` open, which makes
/// `git worktree remove` fail halfway, past the point of no return (see
/// [`Repo::finish_worktree_removal`](crate::Repo::finish_worktree_removal)).
/// Windows has no call that says so, but it won't move a folder with
/// anything open under it, so the folder is moved aside and back. A virus
/// scan or a `git status` lets go by itself, so a few tries are made first.
pub fn ensure_free(path: &Path) -> Result<()> {
    let Some(name) = path.file_name().filter(|_| path.exists()) else {
        return Ok(());
    };
    let aside = path.with_file_name(format!("{}.tt-git-{}", name.to_string_lossy(), std::process::id()));
    for attempt in 0..TRIES {
        if attempt > 0 {
            std::thread::sleep(WAIT);
        }
        match std::fs::rename(path, &aside) {
            Ok(()) => return move_back(&aside, path),
            Err(e) => tracing::debug!("{} can't be moved: {e}", path.display()),
        }
    }
    Err(GitError::Refused(format!(
        "{} is in use, so it can't be deleted.\n\
         Close any terminal, editor or program open in it, then try again.\n\
         Nothing was removed.",
        path.display()
    )))
}

/// Puts a folder [`ensure_free`] moved aside back.
fn move_back(aside: &Path, path: &Path) -> Result<()> {
    let mut moved = std::fs::rename(aside, path);
    for _ in 1..TRIES {
        if moved.is_ok() {
            break;
        }
        std::thread::sleep(WAIT);
        moved = std::fs::rename(aside, path);
    }
    moved.map_err(|e| {
        GitError::Refused(format!(
            "couldn't move {} back to {}: {e}\nNothing was removed; rename it back by hand.",
            aside.display(),
            path.display()
        ))
    })
}
