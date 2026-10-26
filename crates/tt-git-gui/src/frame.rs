//! [`PaneFrame`]: the box every pane draws in. In the terminal style, a
//! bordered box titled `[n] title`; in the native style, a rounded card
//! with a header (the pane's number on a chip, then its title in the UI
//! font) and the focus as a colored border and ring.

use std::rc::Rc;

use gpui::component::menu::ContextMenuExt as _;
use gpui::{
    AnyElement, App, BoxShadow, Div, FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, RenderOnce, ScrollDelta, ScrollWheelEvent, SharedString, Styled, Window, div, point,
    prelude::FluentBuilder, px, rems,
};
use tt_git_app::mouse::{MouseInput, MouseKind, MouseTarget};
use tt_git_app::panes::PaneId;
use tt_git_app::view::{StyleRole, TextSpan};
use unicode_width::UnicodeWidthStr;

use crate::context_menu;
use crate::look;
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

/// The columns a pane's title has after `[n] ` (in the native style, after
/// the number's chip), as its title line was laid out last frame, for a
/// title fitted to its room (the branches tabs, the part of blame's title
/// that goes at the right). Zero before the first frame. The native header's
/// UI font is narrower than a column, so what fits in these columns fits.
pub fn title_cols(ui: &Ui, pane: PaneId) -> usize {
    let prefix = if ui.native() { 0 } else { prefix(pane).width() };
    ui.fit(&title_id(pane)).cols.saturating_sub(prefix)
}

/// `[n] `, before every title.
fn prefix(pane: PaneId) -> String {
    format!("[{}] ", pane.number())
}

/// What a click on column `n` of a pane's title (from its start, after
/// `[n] `) is over, if anything with clicks of its own.
pub type TitleTargetFn = Rc<dyn Fn(usize) -> Option<MouseTarget>>;

/// A tab in a native header's tab strip: the branches pane's tabs.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameTab {
    pub label: String,
    /// What it counts, drawn fainter after the label.
    pub count: Option<String>,
    /// The tab shown.
    pub active: bool,
    /// What a click on it is for the app.
    pub target: MouseTarget,
}

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
    /// The title as [`title`](Self::title) gave it, which the native header
    /// splits into the pane's name and the rest.
    title_text: Option<String>,
    tabs: Vec<FrameTab>,
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
            title_text: None,
            tabs: Vec::new(),
            title_right: Vec::new(),
            title_target: None,
            bottom_left: Vec::new(),
            bottom_right: Vec::new(),
            children: Vec::new(),
        }
    }

    /// The title, after `[n] `, in the title look.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        let title = title.into();
        self.title = vec![TextSpan::plain(title.clone())];
        self.title_text = Some(title);
        self
    }

    /// Tabs, which the native header draws as a tab strip before the
    /// title, each clicked as its [`target`](FrameTab::target). The
    /// terminal style ignores them: give it the tabs in the title instead.
    pub fn tabs(mut self, tabs: Vec<FrameTab>) -> Self {
        self.tabs = tabs;
        self
    }

    /// The title as spans, each laid over the title look (as the TUI does
    /// it), for a title with parts of its own, like the branches tabs.
    pub fn title_spans(mut self, spans: Vec<TextSpan<'static>>) -> Self {
        self.title = spans;
        self.title_text = None;
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
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let Self {
            ui,
            pane,
            focused,
            title,
            title_text,
            tabs,
            title_right,
            title_target,
            bottom_left,
            bottom_right,
            children,
        } = self;
        let native = ui.native();
        let theme = &ui.theme;
        let (border, title_role) = if focused {
            (StyleRole::BorderFocused, StyleRole::TitleFocused)
        } else {
            (StyleRole::Border, StyleRole::Title)
        };
        // The native header has the number on a chip, and the pane's name
        // in its own weight.
        let (name, title) = match (&title_text, native) {
            (_, false) => {
                let mut spans = vec![TextSpan::plain(prefix(pane))];
                spans.extend(title);
                (None, spans)
            }
            (Some(text), true) => {
                let (name, rest) = split_title(text);
                (Some(name.to_string()), vec![TextSpan::plain(rest.to_string())])
            }
            (None, true) => (None, title),
        };
        let prefix_width = if native { 0 } else { prefix(pane).width() };
        let title_look = if native { StyleRole::Muted } else { title_role };
        let title = StyledLine::new(theme, &title, Some(title_look));
        let title_text = title.element();
        let title_click = title_target.map(|target| {
            let (ui, layout, text) = (ui.clone(), title_text.layout().clone(), title.text.clone());
            move |ev: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                // On a character, not past the end or between lines.
                let Ok(index) = layout.index_for_position(ev.position) else {
                    return;
                };
                let col = text.get(..index).map_or(0, UnicodeWidthStr::width);
                let Some(target) = col.checked_sub(prefix_width).and_then(|col| target(col)) else {
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
                .child(StyledLine::new(theme, &title_right, Some(title_look)).element())
        });
        let title_line = div()
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .when(native, |d| d.items_center().gap(rems(0.5)))
            .child(ui.measure(title_id(pane)))
            .when(native && !tabs.is_empty(), |d| d.child(tab_strip(&ui, tabs)))
            .when_some(name, |d, name| {
                let color = if focused {
                    theme.foreground
                } else {
                    theme.fg(StyleRole::Muted)
                };
                d.child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color)
                        .child(name),
                )
            })
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
                .when(native, |d| d.px(look::HEADER_PAD))
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
        let frame = div()
            .id(frame_id(pane))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .border_1();
        let frame = if native {
            let s = &theme.surfaces;
            let ring = |window: &Window| BoxShadow {
                color: s.ring,
                offset: point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: look::RING.to_pixels(window.rem_size()),
                inset: false,
            };
            frame
                .rounded(look::CARD_RADIUS)
                .bg(s.card)
                .border_color(if focused { s.focus_border } else { s.hairline })
                .when(focused, |d| d.shadow(vec![ring(window)]))
        } else {
            frame.rounded(rems(0.25)).border_color(theme.fg(border))
        };
        let header = if native {
            div()
                .flex_none()
                .h(look::HEADER_HEIGHT)
                .flex()
                .items_center()
                .gap(rems(0.5))
                .px(look::HEADER_PAD)
                .overflow_hidden()
                .whitespace_nowrap()
                .font_family(look::ui_font_family())
                .child(number_chip(&ui, pane, focused))
                .child(div().flex_1().min_w_0().h_full().child(title_line))
        } else {
            line(&ui).text_color(theme.fg(title_role)).child(title_line)
        };
        let body_pad = if native { look::BODY_PAD } else { rems(0.) };
        frame
            .on_mouse_down(MouseButton::Left, click)
            .on_mouse_down(MouseButton::Right, right_click)
            .on_scroll_wheel(wheel)
            .child(ui.measure(frame_id(pane)))
            .child(header)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .when(!native, |d| d.px(ui.font.cell_width / 2.))
                    .when(native, |d| d.mx(body_pad).mb(body_pad))
                    .overflow_hidden()
                    .child(ui.measure(body_id(pane)))
                    .children(children),
            )
            .children(bottom)
            .context_menu(context_menu::builder(ui.clone()))
    }
}

/// The native header's chip with the pane's number, in the focus color
/// when focused.
fn number_chip(ui: &Ui, pane: PaneId, focused: bool) -> Div {
    let theme = &ui.theme;
    let s = &theme.surfaces;
    let (bg, fg): (Hsla, Hsla) = if focused {
        (theme.fg(StyleRole::BorderFocused), s.card)
    } else {
        (s.tint, theme.fg(StyleRole::Muted))
    };
    div()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .min_w(rems(1.125))
        .h(rems(1.125))
        .px(rems(0.3125))
        .rounded(look::CHIP_RADIUS)
        .bg(bg)
        .text_color(fg)
        .font_family(ui.font.family.clone())
        .text_size(ui.font.size * 0.85)
        .font_weight(FontWeight::SEMIBOLD)
        .child(pane.number().to_string())
}

/// The native header's tab strip: the tabs on a tinted track, the one
/// shown raised on it.
fn tab_strip(ui: &Ui, tabs: Vec<FrameTab>) -> Div {
    let theme = &ui.theme;
    let s = theme.surfaces;
    let muted = theme.fg(StyleRole::Muted);
    div()
        .flex_none()
        .flex()
        .gap(px(2.))
        .p(px(2.))
        .rounded(rems(0.4375))
        .bg(s.tint)
        .children(tabs.into_iter().enumerate().map(|(i, tab)| {
            let label = tab.label.clone();
            let click = {
                let (ui, target) = (ui.clone(), tab.target.clone());
                move |ev: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                    cx.stop_propagation();
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
            div()
                .id(("frame-tab", i))
                .debug_selector(move || format!("tab {label}"))
                .flex()
                .gap(rems(0.25))
                .px(rems(0.5))
                .py(rems(0.0625))
                .rounded(rems(0.3125))
                .when(tab.active, |d| {
                    d.bg(s.card)
                        .text_color(theme.foreground)
                        .font_weight(FontWeight::SEMIBOLD)
                        .shadow(vec![BoxShadow {
                            color: s.shadow.opacity(0.5),
                            offset: point(px(0.), px(1.)),
                            blur_radius: px(2.),
                            spread_radius: px(0.),
                            inset: false,
                        }])
                })
                .when(!tab.active, |d| d.text_color(muted).cursor_pointer())
                .on_mouse_down(MouseButton::Left, click)
                .child(tab.label)
                .children(tab.count.map(|c| {
                    div()
                        .text_color(muted.opacity(0.75))
                        .font_weight(FontWeight::NORMAL)
                        .child(c)
                }))
        }))
}

/// A plain title split for the native header: the pane's name (up to the
/// first space or `:`), and the rest.
fn split_title(title: &str) -> (&str, &str) {
    match title.find([' ', ':']) {
        Some(i) => (&title[..i], title[i + 1..].trim_start()),
        None => (title, ""),
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
