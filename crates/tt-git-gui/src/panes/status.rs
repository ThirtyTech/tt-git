//! The status pane (Phase B2): the changed files as a list or a tree, and
//! the folder filter. A placeholder until B2 draws it.

use gpui::AnyElement;
use tt_git_app::panes::PaneId;

use super::PaneCx;
use crate::placeholder;

pub fn render(cx: &mut PaneCx) -> AnyElement {
    placeholder::pane(
        cx.ui,
        PaneId::Status,
        cx.focused,
        "Status",
        "B2 Status",
        "the changed files, as a list or a tree, and the folder filter",
    )
}
