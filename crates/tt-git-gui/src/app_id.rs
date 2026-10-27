//! The app's identity to the OS. On Windows it's the AppUserModelID: the
//! taskbar groups the windows under it, the jump list belongs to it, and
//! notifications are sent as it. The installer gives the Start menu entry
//! the same one. On Linux it's the app id, the `.desktop` file's name.

/// The app's AppUserModelID on Windows, as released. The installer
/// (`packaging/windows/tt-git.iss`) has it too.
///
/// On Linux it's the app id: the Wayland app id and the X11 `WM_CLASS` of
/// every window, the launcher badge's `application://ThirtyTech.tt-git.desktop`,
/// and the name the notification portal knows the app by, which needs a dot
/// in it. The compositor finds the launcher, and so the icon, by it, so it's
/// the name of the `.desktop` file the packages install
/// (`packaging/linux/ThirtyTech.tt-git.desktop`).
///
/// Change them all together.
pub const RELEASE_APP_ID: &str = "ThirtyTech.tt-git";

/// The id this build runs as: [`RELEASE_APP_ID`], except that a debug build
/// on Windows is `ThirtyTech.tt-git.dev`. So a `cargo run` window gets a
/// taskbar button of its own instead of stacking with the installed app's,
/// and its toasts, toast icon and jump list don't replace the installed
/// app's. Linux keeps the one id, which has to match the installed
/// `.desktop` file.
pub const APP_ID: &str = if cfg!(all(windows, debug_assertions)) {
    "ThirtyTech.tt-git.dev"
} else {
    RELEASE_APP_ID
};

/// The name the OS shows notifications under.
pub const APP_NAME: &str = "tt-git";

/// Gives this process its identity: on Windows GPUI makes [`APP_ID`] the
/// process's AppUserModelID (`SetCurrentProcessExplicitAppUserModelID`).
/// Call once, before the first window opens.
pub fn set(cx: &gpui::App) {
    cx.set_app_identity(APP_ID, APP_NAME);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `.desktop` file the Linux packages install.
    fn desktop_file() -> String {
        let path = format!(
            "{}/packaging/linux/{RELEASE_APP_ID}.desktop",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    #[test]
    fn the_desktop_file_is_named_for_the_app_id() {
        let file = desktop_file();
        assert!(file.contains(&format!("\nStartupWMClass={RELEASE_APP_ID}\n")), "{file}");
        // The icon keeps the hicolor files' name.
        assert!(file.contains("\nIcon=tt-git\n"), "{file}");
    }

    #[test]
    fn the_desktop_file_opens_folders_and_has_the_jump_lists_tasks() {
        let file = desktop_file();
        assert!(file.contains("\nMimeType=inode/directory;\n"), "{file}");
        for task in crate::jump_list::TASKS {
            let action = format!("\nName={}\nExec=tt-git-gui {}\n", task.title, task.arguments);
            assert!(file.contains(&action), "{action:?} isn't in the desktop file:\n{file}");
        }
    }
}
