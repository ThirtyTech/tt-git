//! The error popup (Phase B7). A placeholder until B7 draws it, which
//! shows the message as it is.

use gpui::{AnyElement, IntoElement, ParentElement, Styled, div, px};
use tt_git_app::view::StyleRole;

use super::DialogCx;
use crate::overlay;

pub fn render(cx: &mut DialogCx) -> AnyElement {
    let ui = cx.ui;
    let theme = &ui.theme;
    let message = cx.app.error().unwrap_or_default().to_string();
    overlay::centered(
        div()
            .max_w(ui.font.cell_width * 70.)
            .p(ui.font.cell_width)
            .rounded(px(6.))
            .border_1()
            .border_color(theme.fg(StyleRole::Error))
            .bg(theme.background)
            .shadow_lg()
            .child(div().text_color(theme.fg(StyleRole::Error)).child("Error"))
            .child(div().whitespace_normal().child(message)),
    )
    .into_any_element()
}
