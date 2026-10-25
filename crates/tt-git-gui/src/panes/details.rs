//! The details pane (Phase B4): the commit or file it shows, its file list
//! beside or below, and the diff (in `details/diff.rs`). A placeholder
//! until B4 draws it.
//!
//! Staging and blame take over the pane; `panes::render` sends those to
//! `details/stage.rs` (B5) and `details/blame.rs` (B6), so each has a file
//! of its own.

use gpui::{AnyElement, IntoElement, ParentElement};
use tt_git_app::panes::PaneId;

use super::PaneCx;
use crate::frame::PaneFrame;

pub mod blame;
pub mod diff;
pub mod stage;

pub fn render(cx: &mut PaneCx) -> AnyElement {
    PaneFrame::new(cx.ui, PaneId::Details, cx.focused)
        .title("Details")
        .child(diff::render(cx))
        .into_any_element()
}
