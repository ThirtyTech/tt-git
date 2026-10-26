//! Quick Look on macOS: the details pane's `quick_look` commands
//! (`App::take_quick_look`) show the file in the app's own Quick Look
//! panel (`tt_git_win::quick_look`), over the window as Finder shows it.
//! (`qlmanage -p`, which the TUI uses, opens a separate app with its own
//! Dock icon and menu bar, and takes the focus.) The copies of files as of
//! a commit are removed when the app quits.

use std::path::Path;

use gpui::App;

/// Removes the copies Quick Look previewed when the app quits. Only `main`
/// installs it, so tests never touch the temp folder.
pub fn install(cx: &mut App) {
    cx.on_app_quit(|_| {
        tt_git_app::quick_look::clean_up();
        async {}
    })
    .detach();
}

/// Previews `file` with Quick Look.
#[cfg(target_os = "macos")]
pub fn preview(file: &Path) -> Result<(), String> {
    if tt_git_win::quick_look::preview(file) {
        Ok(())
    } else {
        Err("couldn't open the Quick Look panel".into())
    }
}

/// Quick Look is macOS's: elsewhere the commands aren't offered.
#[cfg(not(target_os = "macos"))]
pub fn preview(_file: &Path) -> Result<(), String> {
    Ok(())
}
