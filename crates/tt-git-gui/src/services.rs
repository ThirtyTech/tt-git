//! Open in tt-git on folders in Finder (right-click, or Finder > Services),
//! the macOS counterpart of the Explorer entry the Windows installer adds.
//!
//! The service is declared in the app bundle's `Info.plist` (`NSServices`,
//! `crates/tt-git-gui/packaging/macos`), so it comes and goes with the app.
//! macOS sends it to the running copy, starting one first if none is, and
//! `tt_git_win::services` hands the folders over; they open as
//! [`windows::open_requested`] does. Only `main` calls [`install`].
//! Elsewhere than macOS it does nothing.

use gpui::App;

#[cfg(target_os = "macos")]
use crate::windows;

/// Answers the service: each folder sent opens in a window.
pub fn install(cx: &mut App) {
    #[cfg(target_os = "macos")]
    {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        if !tt_git_win::services::install(move |folders| {
            let _ = tx.send(folders);
        }) {
            tracing::warn!("couldn't set up the Open in tt-git service");
            return;
        }
        cx.spawn(async move |cx| {
            while let Some(folders) = rx.recv().await {
                tracing::info!("opening {folders:?} for Finder");
                cx.update(|cx| windows::open_requested(&folders, cx));
            }
        })
        .detach();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = cx;
}
