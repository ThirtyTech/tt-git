//! [`PaneFrame`]: the bordered box every pane draws in, titled `[n] title`.

use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, RenderOnce,
    ScrollDelta, ScrollWheelEvent, SharedString, Styled, Window, div, px,
};
use tt_git_app::mouse::{MouseInput, MouseKind, MouseTarget};
use tt_git_app::panes::PaneId;
use tt_git_app::view::{StyleRole, TextSpan};

use crate::rows::WHEEL_STEP_LINES;
use crate::text::StyledLine;
use crate::ui::Ui;

/// The id a pane's frame is measured as: where the pane was drawn, which
/// the shell reports to the app with `set_layout`.
pub fn frame_id(pane: PaneId) -> SharedString {
    format!("pane:{}", pane.name()).into()
}

/// The id a pane's body (inside the border, below the title) is measured
/// as, for [`Ui::fit`].
pub fn body_id(pane: PaneId) -> SharedString {
    format!("body:{}", pane.name()).into()
}

/// A pane's bordered box: `[n]` and the title on top, in the focus colors
/// when `focused`, and the children below, filling the rest.
///
/// A click or the wheel anywhere in the frame that nothing inside handles
/// goes to the app as on the pane's border (no body row), which focuses it.
#[derive(IntoElement)]
pub struct PaneFrame {
    ui: Ui,
    pane: PaneId,
    focused: bool,
    title: Vec<TextSpan<'static>>,
    children: Vec<AnyElement>,
}

impl PaneFrame {
    pub fn new(ui: &Ui, pane: PaneId, focused: bool) -> Self {
        Self {
            ui: ui.clone(),
            pane,
            focused,
            title: Vec::new(),
            children: Vec::new(),
        }
    }

    /// The title, after `[n] `, in the title look.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = vec![TextSpan::plain(title.into())];
        self
    }

    /// The title as spans, each laid over the title look (as the TUI does
    /// it), for a title with parts of its own, like the branches tabs.
    pub fn title_spans(mut self, spans: Vec<TextSpan<'static>>) -> Self {
        self.title = spans;
        self
    }
}

impl ParentElement for PaneFrame {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for PaneFrame {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self {
            ui,
            pane,
            focused,
            title,
            children,
        } = self;
        let theme = &ui.theme;
        let (border, title_role) = if focused {
            (StyleRole::BorderFocused, StyleRole::TitleFocused)
        } else {
            (StyleRole::Border, StyleRole::Title)
        };
        let mut spans = vec![TextSpan::plain(format!("[{}] ", pane.number()))];
        spans.extend(title);
        let title = StyledLine::new(theme, &spans, Some(title_role));
        let target = MouseTarget::Pane {
            pane,
            body_row: None,
            part: None,
        };
        let click = {
            let (ui, target) = (ui.clone(), target.clone());
            move |ev: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                ui.send_mouse(
                    MouseInput {
                        kind: MouseKind::Click,
                        ctrl: ev.modifiers.control,
                        target: target.clone(),
                    },
                    cx,
                );
            }
        };
        let wheel = {
            let ui = ui.clone();
            let id = frame_id(pane);
            move |ev: &ScrollWheelEvent, _: &mut Window, cx: &mut App| {
                let lines = match ev.delta {
                    ScrollDelta::Lines(d) => d.y,
                    ScrollDelta::Pixels(d) => d.y / ui.font.line_height,
                };
                let steps = ui.measures.wheel_steps(&id, lines, WHEEL_STEP_LINES);
                let kind = if steps > 0 {
                    MouseKind::ScrollUp
                } else {
                    MouseKind::ScrollDown
                };
                for _ in 0..steps.unsigned_abs() {
                    ui.send_mouse(
                        MouseInput {
                            kind,
                            ctrl: ev.modifiers.control,
                            target: target.clone(),
                        },
                        cx,
                    );
                }
            }
        };
        div()
            .id(frame_id(pane))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .border_1()
            .rounded(px(4.))
            .border_color(theme.fg(border))
            .on_mouse_down(MouseButton::Left, click)
            .on_scroll_wheel(wheel)
            .child(ui.measure(frame_id(pane)))
            .child(
                div()
                    .flex_none()
                    .h(ui.font.line_height)
                    .px(ui.font.cell_width)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_color(theme.fg(title_role))
                    .child(title.element()),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .px(ui.font.cell_width / 2.)
                    .overflow_hidden()
                    .child(ui.measure(body_id(pane)))
                    .children(children),
            )
    }
}
