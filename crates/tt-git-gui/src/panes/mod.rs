//! The four main panes. Each Phase B area draws in its own file; this one
//! only says which file draws what, so nobody needs to edit it.
//!
//! | File | Area | Draws |
//! |---|---|---|
//! | `graph.rs` | B1 Graph | the graph pane |
//! | `status.rs` | B2 Status | the status pane |
//! | `branches.rs` | B3 Branches | the branches pane and its tabs |
//! | `details.rs`, `details/diff.rs` | B4 Details | the details pane: commit, files, diff |
//! | `details/stage.rs` | B5 Stage | details while staging hunks or lines |
//! | `details/blame.rs` | B6 Blame | details while blaming a file |
//!
//! A plugin pane split in place of details is `crate::terminal`'s (B8).

use gpui::AnyElement;
use tt_git_app::App;
use tt_git_app::app::Zoom;
use tt_git_app::panes::PaneId;

use crate::ui::Ui;

pub mod branches;
pub mod details;
pub mod graph;
pub mod status;

/// What a pane draws with: the app (mutable, for `set_viewport` and
/// `note_drawn`), the [`Ui`], and whether the pane has the focus.
pub struct PaneCx<'a> {
    pub app: &'a mut App,
    pub ui: &'a Ui,
    pub pane: PaneId,
    pub focused: bool,
}

/// Draws `cx.pane`, filling the space it's given. Usually a
/// [`PaneFrame`](crate::frame::PaneFrame).
pub fn render(cx: &mut PaneCx) -> AnyElement {
    match cx.pane {
        PaneId::Graph => graph::render(cx),
        PaneId::Status => status::render(cx),
        PaneId::Branches => branches::render(cx),
        PaneId::Details => {
            // Fullscreen has room for the file list beside the diff.
            let beside = cx.app.zoom() == Zoom::Full(PaneId::Details);
            cx.app.details_mut().set_files_beside(beside);
            if cx.app.details().stage().is_some() {
                details::stage::render(cx)
            } else if cx.app.details().blame().is_some() {
                details::blame::render(cx)
            } else {
                details::render(cx)
            }
        }
    }
}
