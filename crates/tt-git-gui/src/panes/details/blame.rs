//! Blame in the details pane (Phase B6): the heat stripe, author colors and
//! stepping through the file's commits. A placeholder until B6 draws it.

use gpui::AnyElement;
use tt_git_app::panes::PaneId;

use crate::panes::PaneCx;
use crate::placeholder;

pub fn render(cx: &mut PaneCx) -> AnyElement {
    let title = cx.app.details().blame().map(|b| b.title()).unwrap_or_default();
    placeholder::pane(
        cx.ui,
        PaneId::Details,
        cx.focused,
        title,
        "B6 Blame",
        "each line with its commit, age and author",
    )
}
