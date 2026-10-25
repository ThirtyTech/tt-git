//! The branches pane (Phase B3): the branches, tags, stashes and worktrees
//! tabs, plugin decorations and merge-conflict marks. A placeholder until
//! B3 draws it.

use gpui::AnyElement;
use tt_git_app::panes::PaneId;

use super::PaneCx;
use crate::placeholder;

pub fn render(cx: &mut PaneCx) -> AnyElement {
    placeholder::pane(
        cx.ui,
        PaneId::Branches,
        cx.focused,
        "Branches",
        "B3 Branches",
        "the branches, tags, stashes and worktrees tabs",
    )
}
