//! The `tt-git` binary plugins get as `TTGIT_BIN_PATH`.
//!
//! Plugins call `$TTGIT_BIN_PATH <subcommand>` (`ui`, `api`, `events`,
//! `status`…), which talk to the window's app through `TTGIT_SOCKET_PATH`.
//! `tt-git-gui` has no subcommands of its own, so it points them at the
//! TUI's `tt-git`: the one installed next to it, else the first on `PATH`.
//! With neither, plugins get no `TTGIT_BIN_PATH`, as before.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The binary's file name on this OS.
pub const NAME: &str = if cfg!(windows) { "tt-git.exe" } else { "tt-git" };

/// `tt-git` next to the running `tt-git-gui`, else on `PATH`.
pub fn find() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok();
    let found = find_in(
        exe.as_deref().and_then(Path::parent),
        std::env::var_os("PATH").as_deref(),
    );
    match &found {
        Some(path) => tracing::debug!("plugins get TTGIT_BIN_PATH={}", path.display()),
        None => tracing::info!("no {NAME} next to tt-git-gui or on PATH: plugins get no TTGIT_BIN_PATH"),
    }
    found
}

/// [`NAME`] in `exe_dir`, else in the first directory of `path` (a `PATH`
/// value) that has it.
pub fn find_in(exe_dir: Option<&Path>, path: Option<&OsStr>) -> Option<PathBuf> {
    let beside = exe_dir.map(|d| d.join(NAME)).filter(|p| p.is_file());
    beside.or_else(|| std::env::split_paths(path?).map(|d| d.join(NAME)).find(|p| p.is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_the_binary_next_to_the_gui_then_path() {
        let dir = tempfile::tempdir().unwrap();
        let (gui, on_path, empty) = (dir.path().join("gui"), dir.path().join("bin"), dir.path().join("empty"));
        for d in [&gui, &on_path, &empty] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(on_path.join(NAME), "").unwrap();
        let path = std::env::join_paths([&empty, &on_path]).unwrap();

        assert_eq!(find_in(Some(&gui), Some(&path)), Some(on_path.join(NAME)));
        std::fs::write(gui.join(NAME), "").unwrap();
        assert_eq!(find_in(Some(&gui), Some(&path)), Some(gui.join(NAME)));
        assert_eq!(find_in(Some(&empty), None), None);
        assert_eq!(find_in(None, Some(&std::env::join_paths([&empty]).unwrap())), None);
    }
}
