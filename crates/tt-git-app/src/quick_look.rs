//! Previewing a file with Quick Look (macOS): the copies of files as of a
//! commit that the preview reads. The app writes them; the front end shows
//! them (see `App::take_quick_look`) and removes them with [`clean_up`]
//! when it quits.

use std::path::PathBuf;

/// Where this process writes its copies: a folder of its own under the
/// temp folder, so one quitting doesn't take another's preview away.
pub fn dir() -> PathBuf {
    std::env::temp_dir()
        .join("tt-git-quicklook")
        .join(std::process::id().to_string())
}

/// Removes this process's copies, if it wrote any.
pub fn clean_up() {
    let dir = dir();
    if dir.exists()
        && let Err(e) = std::fs::remove_dir_all(&dir)
    {
        tracing::warn!("couldn't remove {}: {e}", dir.display());
    }
    // The shared folder too, unless another process is using it.
    if let Some(parent) = dir.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

/// Previews `file` in a Quick Look window of its own, with `qlmanage -p`,
/// and leaves it open.
#[cfg(target_os = "macos")]
pub fn qlmanage(file: &std::path::Path) -> Result<(), String> {
    use std::process::{Command, Stdio};

    let mut child = Command::new("qlmanage")
        .arg("-p")
        .arg(file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("couldn't start qlmanage: {e}"))?;
    // Reap it when the preview closes.
    let _ = std::thread::Builder::new()
        .name("tt-git-quicklook".into())
        .stack_size(crate::IO_THREAD_STACK)
        .spawn(move || child.wait());
    Ok(())
}
