//! [`PaneFrame`]: the bordered box every pane draws in, titled `[n] title`.

use std::rc::Rc;

use gpui::component::menu::ContextMenuExt as _;
use gpui::{
    AnyElement, App, Div, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, RenderOnce,
    ScrollDelta, ScrollWheelEvent, SharedString, Styled, Window, div, prelude::FluentBuilder, px,
};
use tt_git_app::mouse::{MouseInput, MouseKind, MouseTarget};
use tt_git_app::panes::PaneId;
use tt_git_app::view::{StyleRole, TextSpan};
use unicode_width::UnicodeWidthStr;

use crate::context_menu;
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

/// The id a pane's title line (inside the frame's padding, `[n] ` and
/// the right-hand part included) is measured as, for [`title_cols`].
pub fn title_id(pane: PaneId) -> SharedString {
    format!("title:{}", pane.name()).into()
}

/// The columns a pane's title has after `[n] `, as its title line was laid
/// out last frame, for a title fitted to its room (the branches tabs, the
/// part of blame's title that goes at the right). Zero before the first
/// frame.
pub fn title_cols(ui: &Ui, pane: PaneId) -> usize {
    ui.fit(&title_id(pane)).cols.saturating_sub(prefix(pane).width())
}

/// `[n] `, before every title.
fn prefix(pane: PaneId) -> String {
    format!("[{}] ", pane.number())
}

/// What a click on column `n` of a pane's title (from its start, after
/// `[n] `) is over, if anything with clicks of its own.
pub type TitleTargetFn = Rc<dyn Fn(usize) -> Option<MouseTarget>>;

/// A pane's bordered box: `[n]` and the title on top, in the focus colors
/// when `focused`, with an optional part pushed to the right of the title
/// line; the children below, filling the rest; and an optional line along
/// the bottom, as the TUI writes on a pane's bottom border.
///
/// A click or the wheel anywhere in the frame that nothing inside handles
/// goes to the app as on the pane's border (no body row), which focuses it.
#[derive(IntoElement)]
pub struct PaneFrame {
    ui: Ui,
    pane: PaneId,
    focused: bool,
    title: Vec<TextSpan<'static>>,
    title_right: Vec<TextSpan<'static>>,
    title_target: Option<TitleTargetFn>,
    bottom_left: Vec<TextSpan<'static>>,
    bottom_right: Vec<TextSpan<'static>>,
    children: Vec<AnyElement>,
}

impl PaneFrame {
    pub fn new(ui: &Ui, pane: PaneId, focused: bool) -> Self {
        Self {
            ui: ui.clone(),
            pane,
            focused,
            title: Vec::new(),
            title_right: Vec::new(),
            title_target: None,
            bottom_left: Vec::new(),
            bottom_right: Vec::new(),
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

    /// Spans at the right end of the title line, each laid over the title
    /// look, like blame's date. The title is cut short before them; fit
    /// what goes here to [`title_cols`] to keep both whole.
    pub fn title_right(mut self, spans: Vec<TextSpan<'static>>) -> Self {
        self.title_right = spans;
        self
    }

    /// Clicks on the title's text: `target(n)` says what column `n` of the
    /// title (after `[n] `) is for the mouse, found from where the text was
    /// laid out, so it's exact whatever the font. A click it returns
    /// `Some` for goes to the app as that, instead of as on the border.
    pub fn on_title_click(mut self, target: impl Fn(usize) -> Option<MouseTarget> + 'static) -> Self {
        self.title_target = Some(Rc::new(target));
        self
    }

    /// A line along the bottom of the frame, under the children, as the
    /// TUI writes on a pane's bottom border: `left` at the left and
    /// `right` pushed to the right. Not drawn while both are empty.
    pub fn bottom(mut self, left: Vec<TextSpan<'static>>, right: Vec<TextSpan<'static>>) -> Self {
        self.bottom_left = left;
        self.bottom_right = right;
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
            title_right,
            title_target,
            bottom_left,
            bottom_right,
            children,
        } = self;
        let theme = &ui.theme;
        let (border, title_role) = if focused {
            (StyleRole::BorderFocused, StyleRole::TitleFocused)
        } else {
            (StyleRole::Border, StyleRole::Title)
        };
        let mut spans = vec![TextSpan::plain(prefix(pane))];
        spans.extend(title);
        let title = StyledLine::new(theme, &spans, Some(title_role));
        let title_text = title.element();
        let title_click = title_target.map(|target| {
            let (ui, layout, text) = (ui.clone(), title_text.layout().clone(), title.text.clone());
            move |ev: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                // On a character, not past the end or between lines.
                let Ok(index) = layout.index_for_position(ev.position) else {
                    return;
                };
                let col = text.get(..index).map_or(0, UnicodeWidthStr::width);
                let Some(target) = col.checked_sub(prefix(pane).width()).and_then(|col| target(col)) else {
                    return;
                };
                cx.stop_propagation();
                ui.send_mouse(
                    MouseInput {
                        kind: MouseKind::Click,
                        ctrl: ev.modifiers.control,
                        target,
                    },
                    cx,
                );
            }
        });
        let right = (!title_right.is_empty()).then(|| {
            div()
                .flex_none()
                .child(StyledLine::new(theme, &title_right, Some(title_role)).element())
        });
        let title_line = div()
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .child(ui.measure(title_id(pane)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .when_some(title_click, |d, click| d.on_mouse_down(MouseButton::Left, click))
                    .child(title_text),
            )
            .children(right);
        let bottom = (!bottom_left.is_empty() || !bottom_right.is_empty()).then(|| {
            let part = |spans: &[TextSpan]| StyledLine::new(theme, spans, None).element();
            line(&ui)
                .flex()
                .flex_row()
                .justify_between()
                .child(div().flex_1().min_w_0().overflow_hidden().child(part(&bottom_left)))
                .child(div().flex_none().child(part(&bottom_right)))
        });
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
        // The context menu opens after this, so it can't stop the event:
        // a list that took the right click says so with `prevent_default`.
        let right_click = {
            let (ui, target) = (ui.clone(), target.clone());
            move |ev: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                if window.default_prevented() {
                    return;
                }
                ui.send_mouse(
                    MouseInput {
                        kind: MouseKind::RightClick,
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
            .on_mouse_down(MouseButton::Right, right_click)
            .on_scroll_wheel(wheel)
            .child(ui.measure(frame_id(pane)))
            .child(line(&ui).text_color(theme.fg(title_role)).child(title_line))
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
            .children(bottom)
            .context_menu(context_menu::builder(ui.clone()))
    }
}

/// A line of the frame outside its body: the title, the bottom line.
fn line(ui: &Ui) -> Div {
    div()
        .flex_none()
        .h(ui.font.line_height)
        .px(ui.font.cell_width)
        .overflow_hidden()
        .whitespace_nowrap()
}
