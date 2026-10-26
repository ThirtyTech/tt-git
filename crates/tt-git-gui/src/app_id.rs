//! The app's identity to the OS. On Windows it's the AppUserModelID: the
//! taskbar groups the windows under it, the jump list belongs to it, and
//! notifications are sent as it. The installer gives the Start menu entry
//! the same one.

/// The app's AppUserModelID. The installer (`packaging/windows/tt-git.iss`)
/// has it too: change both together.
pub const APP_ID: &str = "ThirtyTech.tt-git";

/// The name the OS shows notifications under.
pub const APP_NAME: &str = "tt-git";

/// Gives this process its identity: on Windows GPUI makes [`APP_ID`] the
/// process's AppUserModelID (`SetCurrentProcessExplicitAppUserModelID`).
/// Call once, before the first window opens.
pub fn set(cx: &gpui::App) {
    cx.set_app_identity(APP_ID, APP_NAME);
}
