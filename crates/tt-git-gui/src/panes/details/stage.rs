//! Staging in the details pane (Phase B5): a file's hunks or lines, staged
//! and not. A placeholder until B5 draws it.

use gpui::AnyElement;
use tt_git_app::panes::PaneId;

use crate::panes::PaneCx;
use crate::placeholder;

pub fn render(cx: &mut PaneCx) -> AnyElement {
    let title = cx.app.details().stage().map(|s| s.title()).unwrap_or_default();
    placeholder::pane(
        cx.ui,
        PaneId::Details,
        cx.focused,
        title,
        "B5 Stage",
        "the hunks and lines to stage",
    )
}
