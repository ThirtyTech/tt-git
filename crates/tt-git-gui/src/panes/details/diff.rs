//! The details pane's diff and commit text (Phase B4), with syntax
//! highlighting and search matches. A placeholder until B4 draws it.

use gpui::{AnyElement, IntoElement, ParentElement, Styled, div};
use tt_git_app::view::StyleRole;

use crate::panes::PaneCx;

pub fn render(cx: &mut PaneCx) -> AnyElement {
    div()
        .text_color(cx.ui.theme.fg(StyleRole::Muted))
        .child("B4 Details draws the commit, its files and the diff here.")
        .into_any_element()
}
