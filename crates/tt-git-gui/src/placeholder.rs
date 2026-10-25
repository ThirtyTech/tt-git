//! Stand-ins for the panes and dialogs Phase B hasn't drawn yet: a frame or
//! box that says which area it is. The app behind them works as usual;
//! keys still go through the keymap.

use gpui::{AnyElement, IntoElement, ParentElement, Styled, div, px};
use tt_git_app::panes::PaneId;
use tt_git_app::view::StyleRole;

use crate::frame::PaneFrame;
use crate::overlay;
use crate::ui::Ui;

/// A pane's frame titled `title`, saying `area` (like `B2 Status`) will
/// draw `what` in it.
pub fn pane(ui: &Ui, pane: PaneId, focused: bool, title: impl Into<String>, area: &str, what: &str) -> AnyElement {
    PaneFrame::new(ui, pane, focused)
        .title(title)
        .child(note(ui, area, what))
        .into_any_element()
}

/// A box over the middle of the main area titled `title`, saying `area`
/// will draw `what` in it.
pub fn dialog(ui: &Ui, title: impl Into<String>, area: &str, what: &str) -> AnyElement {
    let theme = &ui.theme;
    overlay::centered(
        div()
            .min_w(ui.font.cell_width * 50.)
            .max_w(ui.font.cell_width * 80.)
            .p(ui.font.cell_width)
            .rounded(px(6.))
            .border_1()
            .border_color(theme.fg(StyleRole::BorderFocused))
            .bg(theme.background)
            .shadow_lg()
            .child(div().text_color(theme.fg(StyleRole::TitleFocused)).child(title.into()))
            .child(note(ui, area, what)),
    )
    .into_any_element()
}

fn note(ui: &Ui, area: &str, what: &str) -> impl IntoElement {
    div()
        .text_color(ui.theme.fg(StyleRole::Muted))
        .child(format!("{area} draws {what} here."))
}
