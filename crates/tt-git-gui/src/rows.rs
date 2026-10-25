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
    App, Hsla, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, RenderOnce, ScrollDelta,
    ScrollWheelEvent, SharedString, Styled, Window, div, prelude::FluentBuilder,
};
use tt_git_app::mouse::{MouseInput, MouseKind, MouseTarget};
use tt_git_app::panes::PaneId;
use tt_git_app::view::{StyleRole, TextSpan};

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
}

impl StyledRow {
    /// A row of `spans`, all laid over `row` (the look of the whole row:
    /// [`selection`](crate::text::selection) for a picked row, or a role
    /// like [`StyleRole::DiffAddLine`]) when there is one.
    pub fn new(theme: &GuiTheme, spans: &[TextSpan], row: Option<StyleRole>) -> Self {
        Self {
            line: StyledLine::new(theme, spans, row),
            background: row.and_then(|r| theme.bg(r)),
        }
    }

    /// A row of plain text in `role`.
    pub fn text(theme: &GuiTheme, text: impl Into<String>, role: StyleRole) -> Self {
        Self::new(theme, &[TextSpan::new(text.into(), role)], None)
    }
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
        let line_height = ui.font.line_height;
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
            .on_scroll_wheel(wheel)
            .child(ui.measure(id))
            .children(rows.into_iter().map(move |row| {
                div()
                    .h(line_height)
                    .w_full()
                    .overflow_hidden()
                    .when_some(row.background, |d, bg| d.bg(bg))
                    .child(row.line.element())
            }))
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
