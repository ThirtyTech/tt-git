//! Plugin and agent panes (Phase B8): a pseudo-terminal's screen, split in
//! place of details or floating over the panes. A placeholder until B8
//! paints the `vt100` grid, sizes the PTY from the font's cell size and
//! forwards keys.
//!
//! The app sizes a new pane's PTY from the areas the shell reports, which
//! it still reads as terminal cells: see the `GUI:` comments in
//! `tt-git-app` (`app/panes.rs`, `app/agent.rs`).

use gpui::AnyElement;
use tt_git_app::panes::PaneId;

use crate::dialogs::DialogCx;
use crate::panes::PaneCx;
use crate::placeholder;

/// The plugin pane split in place of details, if there is one.
pub fn split(cx: &mut PaneCx) -> Option<AnyElement> {
    let title = cx.app.split_pane()?.title.clone();
    Some(placeholder::pane(
        cx.ui,
        PaneId::Details,
        cx.focused,
        title,
        "B8 Terminal panes",
        "the plugin's terminal",
    ))
}

/// The floating plugin panes (popups and modal panes), the topmost last.
pub fn floating(cx: &mut DialogCx) -> Vec<AnyElement> {
    let ui = cx.ui;
    cx.app
        .plugin_panes()
        .iter()
        .filter(|p| p.is_modal())
        .map(|p| placeholder::dialog(ui, p.title.clone(), "B8 Terminal panes", "the plugin's terminal"))
        .collect()
}
