//! [`StyledRows`]: a pane's list of text rows, one line each, with the
//! selection shown, clicks and the wheel sent to the app.
//!
//! The app owns the scroll position, as it does for the TUI: a pane's
//! `Cursor` keeps the offset, and its view model gives only the rows on
//! screen. So a pane draws a list like this:
//!
//! ```ignore
//! let fit = cx.ui.fit(ID);              // rows and columns measured last frame
//! pane.set_viewport(fit.rows);          // scrolls the selection into view
//! let view = pane.view(fit.rows, fit.cols, now);
//! let rows = view.rows.iter().map(|r| {
//!     StyledRow::new(&cx.ui.theme, &r.spans, selection(r.picked, cx.focused))
//! });
//! StyledRows::new(&cx.ui, ID, PaneId::Graph, rows.collect())
//! ```
//!
//! The list measures itself as `ID`, so the next frame's `fit` is right
//! (the window draws again when it changes). A click on line `n` of the
//! list sends [`MouseKind::Click`] at body row `n` (plus
//! [`first_body_row`](StyledRows::first_body_row)); the wheel sends
//! [`MouseKind::ScrollUp`] and [`MouseKind::ScrollDown`], one per notch
//! ([`WHEEL_STEP_LINES`]), and the app scrolls as it does in the terminal.

use std::rc::Rc;

use gpui::{
    AnyElement, App, Div, Hsla, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    RenderOnce, ScrollDelta, ScrollWheelEvent, SharedString, Styled, Window, div, prelude::FluentBuilder,
};
use tt_git_app::mouse::{MouseInput, MouseKind, MouseTarget};
use tt_git_app::panes::PaneId;
use tt_git_app::view::{StyleRole, TextSpan};

use crate::look;
use crate::text::StyledLine;
use crate::theme::GuiTheme;
use crate::ui::Ui;

/// How many lines of wheel movement make one scroll step, as one notch of
/// a mouse wheel does on most systems. Each step is one wheel event for the
/// app, which scrolls as it does for a terminal's.
pub const WHEEL_STEP_LINES: f32 = 3.;

/// One row of a [`StyledRows`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyledRow {
    pub line: StyledLine,
    /// Behind the whole row, past the end of its text too.
    pub background: Option<Hsla>,
    /// The background is the selection, which the native style rounds.
    pub selected: bool,
    /// The row as parts, the graph's labels as pills, when it has any
    /// ([`with_pills`](Self::with_pills)); drawn in place of `line`.
    pub parts: Vec<RowPart>,
}

/// A piece of a row drawn in parts.
#[derive(Debug, Clone, PartialEq)]
pub enum RowPart {
    Text(StyledLine),
    /// A label drawn as a rounded pill, taking the `cols` columns its text
    /// (brackets and all) has in the view model, so what follows it lines
    /// up as in the terminal.
    Pill {
        line: StyledLine,
        cols: usize,
        fill: Hsla,
    },
}

impl StyledRow {
    /// A row of `spans`, all laid over `row` (the look of the whole row:
    /// [`selection`](crate::text::selection) for a picked row, or a role
    /// like [`StyleRole::DiffAddLine`]) when there is one.
    pub fn new(theme: &GuiTheme, spans: &[TextSpan], row: Option<StyleRole>) -> Self {
        Self {
            line: StyledLine::new(theme, spans, row),
            background: row.and_then(|r| theme.bg(r)),
            selected: matches!(row, Some(StyleRole::Selected | StyleRole::SelectedUnfocused)),
            parts: Vec::new(),
        }
    }

    /// As [`new`](Self::new), with the graph's ref labels (`[main]`,
    /// `<v1>`, ` HEAD `) drawn as pills without their brackets: tinted in
    /// their color, or filled as the theme fills the checked-out one.
    pub fn with_pills(theme: &GuiTheme, spans: &[TextSpan], row: Option<StyleRole>) -> Self {
        let mut this = Self::new(theme, spans, row);
        if !spans.iter().any(|s| is_label(s.role)) {
            return this;
        }
        let mut parts = Vec::new();
        let mut text: Vec<TextSpan> = Vec::new();
        for span in spans {
            if !is_label(span.role) {
                text.push(span.clone());
                continue;
            }
            if !text.is_empty() {
                parts.push(RowPart::Text(StyledLine::new(theme, &text, row)));
                text.clear();
            }
            let name = label_name(&span.text);
            let look = theme.span_on(span, row);
            let color = look.color.unwrap_or(theme.foreground);
            let fill = match theme.bg(span.role) {
                // The checked-out branch: filled, its text in the fill's
                // opposite as the theme has it.
                Some(bg) => bg,
                None => color.opacity(0.16),
            };
            let mut line = StyledLine::new(theme, &[TextSpan::new(name.to_string(), span.role)], row);
            for (_, h) in &mut line.highlights {
                h.background_color = None;
            }
            parts.push(RowPart::Pill {
                line,
                cols: unicode_width::UnicodeWidthStr::width(span.text.as_ref()),
                fill,
            });
        }
        if !text.is_empty() {
            parts.push(RowPart::Text(StyledLine::new(theme, &text, row)));
        }
        this.parts = parts;
        this
    }

    /// A row of plain text in `role`.
    pub fn text(theme: &GuiTheme, text: impl Into<String>, role: StyleRole) -> Self {
        Self::new(theme, &[TextSpan::new(text.into(), role)], None)
    }
}

/// Whether `role` is one of the graph's ref labels.
fn is_label(role: StyleRole) -> bool {
    matches!(
        role,
        StyleRole::LabelHead
            | StyleRole::LabelDetached
            | StyleRole::LabelLocal
            | StyleRole::LabelRemote
            | StyleRole::LabelTag
    )
}

/// A label's name without the brackets (or the spaces round the
/// checked-out one) the view model puts round it. A label cut short keeps
/// what's left.
fn label_name(text: &str) -> &str {
    let text = text.strip_prefix(['[', '<', ' ']).unwrap_or(text);
    text.strip_suffix([']', '>', ' ']).unwrap_or(text)
}

/// A row's text, its parts laid side by side when it has them.
pub fn row_content(ui: &Ui, row: &StyledRow) -> AnyElement {
    if row.parts.is_empty() {
        return row.line.element().into_any_element();
    }
    let cell = ui.font.cell_width;
    div()
        .flex()
        .flex_row()
        .items_center()
        .h_full()
        .children(row.parts.iter().map(|part| match part {
            RowPart::Text(line) => div().flex_none().child(line.element()).into_any_element(),
            RowPart::Pill { line, cols, fill } => {
                let name = line.text.chars().count();
                // The pill's text is a little smaller, so it fits inside
                // the room the brackets left with some to spare.
                let pad = (cell * (cols.saturating_sub(name) as f32 - 0.4) / 2.).max(gpui::px(0.));
                div()
                    .flex_none()
                    .w(cell * *cols as f32)
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h(ui.font.line_height * 0.8)
                            .px(pad)
                            .rounded(look::CHIP_RADIUS)
                            .bg(*fill)
                            .text_size(ui.font.size * 0.92)
                            .overflow_hidden()
                            .child(line.element()),
                    )
                    .into_any_element()
            }
        }))
        .into_any_element()
}

/// A row's box: the native style rounds a selection and keeps the text
/// clear of its edges.
pub fn row_box(ui: &Ui, row: &StyledRow) -> Div {
    div()
        .h(ui.font.line_height)
        .w_full()
        .overflow_hidden()
        .when(ui.native(), |d| {
            d.px(look::ROW_INSET)
                .when(row.selected, |d| d.rounded(look::ROW_RADIUS))
        })
        .when_some(row.background, |d, bg| d.bg(bg))
}

/// What a click or wheel event on line `n` of a list (from the top of the
/// list; it may be past the last row) was over.
pub type TargetFn = Rc<dyn Fn(usize) -> MouseTarget>;

/// A list of one-line rows in the monospace font, filling its parent. See
/// the [module docs](self) for how a pane uses it.
#[derive(IntoElement)]
pub struct StyledRows {
    ui: Ui,
    id: SharedString,
    rows: Vec<StyledRow>,
    target: TargetFn,
}

impl StyledRows {
    /// `rows`, top to bottom, in pane `pane`; measured as `id`, which must be
    /// unique in the window (the pane's name will do for its main list).
    pub fn new(ui: &Ui, id: impl Into<SharedString>, pane: PaneId, rows: Vec<StyledRow>) -> Self {
        Self {
            ui: ui.clone(),
            id: id.into(),
            rows,
            target: Rc::new(move |line| MouseTarget::Pane {
                pane,
                body_row: Some(line),
                part: None,
            }),
        }
    }

    /// The pane body row the list's first line is on, when the pane draws
    /// something above it (the TUI counts body rows from the first line
    /// inside the border). Ignored after [`on_target`](Self::on_target).
    pub fn first_body_row(self, first: usize) -> Self {
        let target = self.target.clone();
        Self {
            target: Rc::new(move |line| target(line + first)),
            ..self
        }
    }

    /// Says what line `n` of the list is for the mouse instead, for a list
    /// that's a part of its pane with clicks of its own (the details pane's
    /// file list is `PanePart::FileRow(n)`).
    pub fn on_target(self, target: impl Fn(usize) -> MouseTarget + 'static) -> Self {
        Self {
            target: Rc::new(target),
            ..self
        }
    }
}

impl RenderOnce for StyledRows {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self { ui, id, rows, target } = self;
        let inset = if ui.native() { look::ROW_INSET } else { gpui::rems(0.) };
        let click = {
            let (ui, id, target) = (ui.clone(), id.clone(), target.clone());
            move |ev: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                let Some(line) = line_at(&ui, &id, ev.position.y) else {
                    return;
                };
                cx.stop_propagation();
                ui.send_mouse(
                    MouseInput {
                        kind: MouseKind::Click,
                        ctrl: ev.modifiers.control,
                        target: target(line),
                    },
                    cx,
                );
            }
        };
        let right_click = {
            let (ui, id, target) = (ui.clone(), id.clone(), target.clone());
            move |ev: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                let Some(line) = line_at(&ui, &id, ev.position.y) else {
                    return;
                };
                // Not stopped: the pane's context menu opens on it next.
                window.prevent_default();
                ui.send_mouse(
                    MouseInput {
                        kind: MouseKind::RightClick,
                        ctrl: ev.modifiers.control,
                        target: target(line),
                    },
                    cx,
                );
            }
        };
        let wheel = {
            let (ui, id) = (ui.clone(), id.clone());
            move |ev: &ScrollWheelEvent, _: &mut Window, cx: &mut App| {
                let lines = match ev.delta {
                    ScrollDelta::Lines(d) => d.y,
                    ScrollDelta::Pixels(d) => d.y / ui.font.line_height,
                };
                cx.stop_propagation();
                let steps = ui.measures.wheel_steps(&id, lines, WHEEL_STEP_LINES);
                let line = line_at(&ui, &id, ev.position.y).unwrap_or(0);
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
                            target: target(line),
                        },
                        cx,
                    );
                }
            }
        };
        div()
            .id(id.clone())
            .relative()
            .size_full()
            .overflow_hidden()
            .whitespace_nowrap()
            .on_mouse_down(MouseButton::Left, click)
            .on_mouse_down(MouseButton::Right, right_click)
            .on_scroll_wheel(wheel)
            // Native rows keep their text clear of the highlight's edges:
            // the columns that fit are the ones inside.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(inset)
                    .right(inset)
                    .child(ui.measure(id)),
            )
            .children(
                rows.into_iter()
                    .map(move |row| row_box(&ui, &row).child(row_content(&ui, &row))),
            )
    }
}

/// The line of list `id` at window height `y`, if it's inside the list.
fn line_at(ui: &Ui, id: &str, y: gpui::Pixels) -> Option<usize> {
    let bounds = ui.measures.get(id)?;
    if y < bounds.top() || y >= bounds.bottom() {
        return None;
    }
    Some(((y - bounds.top()) / ui.font.line_height).floor() as usize)
}

#[cfg(test)]
mod tests {
    use tt_git_app::theme::Theme;

    use super::*;

    #[test]
    fn labels_lose_their_brackets_but_keep_their_columns() {
        assert_eq!(label_name("[main]"), "main");
        assert_eq!(label_name("<v1.0>"), "v1.0");
        assert_eq!(label_name(" HEAD "), "HEAD");
        assert_eq!(label_name("[origin/fe…"), "origin/fe…");

        let theme = GuiTheme::new(&Theme::default());
        let spans = [
            TextSpan::plain("  "),
            TextSpan::new("[main]", StyleRole::LabelLocal),
            TextSpan::plain(" "),
            TextSpan::new(" topic ", StyleRole::LabelHead),
            TextSpan::plain(" fix it"),
        ];
        let row = StyledRow::with_pills(&theme, &spans, Some(StyleRole::Selected));
        assert!(row.selected);
        let shape: Vec<(String, Option<usize>)> = row
            .parts
            .iter()
            .map(|p| match p {
                RowPart::Text(l) => (l.text.to_string(), None),
                RowPart::Pill { line, cols, .. } => (line.text.to_string(), Some(*cols)),
            })
            .collect();
        assert_eq!(
            shape,
            [
                ("  ".into(), None),
                ("main".into(), Some(6)),
                (" ".into(), None),
                ("topic".into(), Some(7)),
                (" fix it".into(), None),
            ]
        );
        // The checked-out one is filled as the theme fills it, the others
        // tinted; a pill's text has no background of its own.
        let fills: Vec<Hsla> = row
            .parts
            .iter()
            .filter_map(|p| match p {
                RowPart::Pill { fill, line, .. } => {
                    assert!(line.highlights.iter().all(|(_, h)| h.background_color.is_none()));
                    Some(*fill)
                }
                RowPart::Text(_) => None,
            })
            .collect();
        assert_eq!(Some(fills[1]), theme.bg(StyleRole::LabelHead));
        assert!(fills[0].a < 1.);
    }

    #[test]
    fn a_row_without_labels_has_no_parts() {
        let theme = GuiTheme::new(&Theme::default());
        let row = StyledRow::with_pills(&theme, &[TextSpan::plain("a commit")], None);
        assert!(row.parts.is_empty() && !row.selected);
    }
}
