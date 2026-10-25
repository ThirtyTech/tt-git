//! Staging in the details pane (Phase B5): a file's unstaged and staged
//! changes by hunk or line, from the staging view's view model
//! (`tt_git_app::view::stage`), laid out as the TUI lays them out: side by
//! side when the pane is wide, one above the other when not, and a side
//! with nothing in it shrunk to its title along the bottom.
//!
//! Every key works through the keymap as in the TUI. The mouse does what
//! it does there too: a click on a row picks its hunk or line (on either
//! side), and the wheel moves the selection on the active side.

use gpui::{AnyElement, Div, IntoElement, ParentElement, Styled, div};
use tt_git_app::mouse::{MouseTarget, PanePart};
use tt_git_app::panes::{PaneId, StageSideId, StageView};
use tt_git_app::view::StyleRole;
use tt_git_app::view::stage::StageLayout;

use crate::frame::{PaneFrame, body_id};
use crate::panes::PaneCx;
use crate::rows::{StyledRow, StyledRows};
use crate::text::{StyledLine, selection};
use crate::ui::Ui;

/// The id the unstaged side's rows are measured as.
pub const UNSTAGED: &str = "stage:unstaged";
/// The id the staged side's rows are measured as.
pub const STAGED: &str = "stage:staged";

/// The id side `id`'s rows are measured as.
pub fn rows_id(id: StageSideId) -> &'static str {
    match id {
        StageSideId::Unstaged => UNSTAGED,
        StageSideId::Staged => STAGED,
    }
}

pub fn render(cx: &mut PaneCx) -> AnyElement {
    let (ui, focused) = (cx.ui, cx.focused);
    let details = cx.app.details_mut();
    let frame = PaneFrame::new(ui, PaneId::Details, focused).title(details.frame_title());
    let Some(stage) = details.stage_mut() else {
        return frame.into_any_element();
    };
    let width = ui.fit(&body_id(PaneId::Details)).cols;
    let body = match stage.layout(width) {
        StageLayout::Message(msg) => {
            return frame
                .child(div().text_color(ui.theme.fg(StyleRole::Muted)).child(msg))
                .into_any_element();
        }
        StageLayout::One { full } => column()
            .child(side(ui, stage, full, focused, 0).flex_1())
            .child(title_only(ui, stage, full.other(), focused)),
        StageLayout::Beside => div()
            .size_full()
            .flex()
            .flex_row()
            .child(
                side(ui, stage, StageSideId::Unstaged, focused, 0)
                    .flex_1()
                    .border_r_1()
                    .border_color(ui.theme.fg(StyleRole::Border))
                    .pr(ui.font.cell_width / 2.),
            )
            .child(
                side(ui, stage, StageSideId::Staged, focused, 0)
                    .flex_1()
                    .pl(ui.font.cell_width),
            ),
        StageLayout::Stacked => {
            // The staged side's title is on the body row after the
            // unstaged side's rows.
            let below = 1 + ui.fit(UNSTAGED).rows;
            column()
                .child(side(ui, stage, StageSideId::Unstaged, focused, 0).flex_1())
                .child(side(ui, stage, StageSideId::Staged, focused, below).flex_1())
        }
    };
    frame.child(body).into_any_element()
}

fn column() -> Div {
    div().size_full().flex().flex_col()
}

/// Side `id`: its title, then why it has no diff or its rows. Its title is
/// on body row `title_row` of the pane, for the mouse.
fn side(ui: &Ui, stage: &mut StageView, id: StageSideId, focused: bool, title_row: usize) -> Div {
    let side = column()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .child(title(ui, stage, id, focused));
    if let Some(why) = stage.side_error(id) {
        return side.child(div().text_color(ui.theme.fg(why.role)).child(why.text.into_owned()));
    }
    let fit = ui.fit(rows_id(id));
    stage.set_viewport(id, fit.rows);
    let rows = stage
        .side_rows(id, fit.rows, fit.cols)
        .iter()
        .map(|row| StyledRow::new(&ui.theme, &row.spans, selection(row.picked, focused).or(row.back)))
        .collect();
    side.child(
        div()
            .flex_1()
            .min_h_0()
            .child(
                StyledRows::new(ui, rows_id(id), PaneId::Details, rows).on_target(move |line| MouseTarget::Pane {
                    pane: PaneId::Details,
                    body_row: Some(title_row + 1 + line),
                    part: Some(PanePart::StageRow { side: id, row: line }),
                }),
            ),
    )
}

/// Side `id` with no room for anything but its title.
fn title_only(ui: &Ui, stage: &mut StageView, id: StageSideId, focused: bool) -> Div {
    if stage.side_error(id).is_none() {
        stage.set_viewport(id, 0);
    }
    title(ui, stage, id, focused)
}

fn title(ui: &Ui, stage: &StageView, id: StageSideId, focused: bool) -> Div {
    let line = StyledLine::new(&ui.theme, &stage.side_title(id, focused), None);
    div()
        .flex_none()
        .h(ui.font.line_height)
        .overflow_hidden()
        .whitespace_nowrap()
        .child(line.element())
}

#[cfg(test)]
mod tests {
    use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, point, px, size};
    use tt_git_app::App;
    use tt_git_app::app::Zoom;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::testing::Gui;

    /// A file of 60 lines with a change every 10, so 6 hunks.
    fn six_hunks() -> Sandbox {
        let sb = Sandbox::new();
        let base: String = (1..=60).map(|i| format!("{i}\n")).collect();
        sb.commit_file("f.txt", &base, "base");
        let changed: String = (1..=60)
            .map(|i| {
                if i % 10 == 5 {
                    format!("{i} changed\n")
                } else {
                    format!("{i}\n")
                }
            })
            .collect();
        sb.write("f.txt", &changed);
        sb
    }

    /// Opens f.txt's staging view from the status pane.
    fn open(gui: &mut Gui) {
        gui.press_all(&["2", "enter"]);
        assert_eq!(gui.read(|app| app.zoom()), Zoom::Full(PaneId::Details));
        assert!(gui.read(|app| app.details().stage().is_some()), "staging f.txt");
    }

    fn stage<R>(gui: &Gui, f: impl FnOnce(&StageView) -> R) -> R {
        gui.read(|app: &App| f(app.details().stage().expect("staging")))
    }

    fn bounds(gui: &Gui, id: &str) -> Option<gpui::Bounds<gpui::Pixels>> {
        gui.shell
            .read_with(&gui.cx, |shell, _| shell.ui().measures.this_frame(id))
    }

    fn rows_text(gui: &mut Gui, id: StageSideId) -> Vec<String> {
        let fit = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(rows_id(id)));
        gui.model.update(&mut gui.cx, |model, _| {
            let stage = model.app_mut().details_mut().stage_mut().expect("staging");
            stage
                .side_rows(id, fit.rows, fit.cols)
                .iter()
                .map(|r| r.spans.iter().map(|s| s.text.as_ref()).collect())
                .collect()
        })
    }

    #[gpui::test]
    fn shows_the_hunks_and_keeps_the_selection_on_screen(cx: &mut TestAppContext) {
        let sb = six_hunks();
        let mut gui = Gui::open(cx, &sb);
        // Short, so the hunks don't all fit.
        gui.cx.simulate_resize(size(px(1000.), px(300.)));
        gui.wait_idle();
        open(&mut gui);
        assert_eq!(
            stage(&gui, |s| s.layout(200)),
            StageLayout::One {
                full: StageSideId::Unstaged
            }
        );
        let fit = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(UNSTAGED));
        assert!(fit.rows > 3 && fit.rows < 40, "the rows were measured: {fit:?}");
        let text = rows_text(&mut gui, StageSideId::Unstaged);
        assert_eq!(text.len(), fit.rows);
        assert!(text[0].starts_with("@@ -2,7 +2,7 @@"), "{text:#?}");
        // The staged side is only its title.
        assert!(bounds(&gui, STAGED).is_none());

        // The last hunk is scrolled into view, by the rows the pane shows.
        gui.press("G");
        assert_eq!(stage(&gui, StageView::selected_hunk), 5);
        assert!(stage(&gui, |s| s.view(StageSideId::Unstaged).offset) > 0);
        let text = rows_text(&mut gui, StageSideId::Unstaged);
        assert!(text.iter().any(|t| t.starts_with("@@ -52,")), "{text:#?}");
    }

    #[gpui::test]
    fn the_wheel_moves_the_selection(cx: &mut TestAppContext) {
        let sb = six_hunks();
        let mut gui = Gui::open(cx, &sb);
        open(&mut gui);
        let rows = bounds(&gui, UNSTAGED).expect("rows drawn");
        let wheel = |lines: f32| ScrollWheelEvent {
            position: rows.center(),
            delta: ScrollDelta::Lines(point(0., lines)),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        };
        // One notch down is three steps, as in the terminal.
        gui.cx.simulate_event(wheel(-3.));
        gui.wait_idle();
        assert_eq!(stage(&gui, StageView::selected_hunk), 3);
        gui.cx.simulate_event(wheel(3.));
        gui.wait_idle();
        assert_eq!(stage(&gui, StageView::selected_hunk), 0);
    }

    #[gpui::test]
    fn a_click_picks_a_hunk_or_a_line(cx: &mut TestAppContext) {
        let sb = six_hunks();
        let mut gui = Gui::open(cx, &sb);
        open(&mut gui);
        let (rows, line_height) = gui.shell.read_with(&gui.cx, |shell, _| {
            let ui = shell.ui();
            (ui.measures.get(UNSTAGED).expect("rows drawn"), ui.font.line_height)
        });
        let at = |line: usize| point(rows.left() + px(30.), rows.top() + line_height * (line as f32 + 0.5));
        // Each hunk is its heading and 8 lines: line 9 is the second's heading.
        gui.cx.simulate_click(at(9), Modifiers::default());
        gui.wait_idle();
        assert_eq!(gui.read(|app| app.focus()), PaneId::Details);
        assert_eq!(stage(&gui, StageView::selected_hunk), 1);
        // In line mode, a click on the hunk's heading picks its first change.
        gui.press("a");
        gui.cx.simulate_click(at(18), Modifiers::default());
        gui.wait_idle();
        assert_eq!(stage(&gui, StageView::selected_hunk), 2);
        assert!(
            stage(&gui, |s| s.picked((2, 3)) && !s.picked((2, 4))),
            "the deleted line"
        );
        // And on a changed line, that line.
        gui.cx.simulate_click(at(23), Modifiers::default());
        gui.wait_idle();
        assert!(stage(&gui, |s| s.picked((2, 4)) && !s.picked((2, 3))), "the added line");
    }

    #[gpui::test]
    fn sides_sit_beside_each_other_when_wide_and_stack_when_not(cx: &mut TestAppContext) {
        let sb = six_hunks();
        let mut gui = Gui::open(cx, &sb);
        open(&mut gui);
        // Stage the first hunk: now both sides have changes.
        gui.press("space");
        let both = |s: &StageView| s.unstaged.has_content() && s.staged.has_content();
        assert!(stage(&gui, both));

        gui.cx.simulate_resize(size(px(2400.), px(800.)));
        gui.wait_idle();
        let (u, s) = (bounds(&gui, UNSTAGED).unwrap(), bounds(&gui, STAGED).unwrap());
        assert!(u.right() <= s.left() && u.top() == s.top(), "beside: {u:?} {s:?}");
        assert_eq!(rows_text(&mut gui, StageSideId::Staged)[0], "@@ -2,7 +2,7 @@");

        gui.cx.simulate_resize(size(px(800.), px(800.)));
        gui.wait_idle();
        let (u, s) = (bounds(&gui, UNSTAGED).unwrap(), bounds(&gui, STAGED).unwrap());
        assert!(u.bottom() <= s.top() && u.left() == s.left(), "stacked: {u:?} {s:?}");
    }

    #[gpui::test]
    fn staging_everything_leaves_the_staged_side(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("f.txt", "a\n", "base");
        sb.write("f.txt", "b\n");
        let mut gui = Gui::open(cx, &sb);
        open(&mut gui);
        gui.press("space");
        assert_eq!(stage(&gui, StageView::side), StageSideId::Staged);
        assert_eq!(
            stage(&gui, |s| s.layout(80)),
            StageLayout::One {
                full: StageSideId::Staged
            }
        );
        assert!(bounds(&gui, STAGED).is_some() && bounds(&gui, UNSTAGED).is_none());
        assert_eq!(
            rows_text(&mut gui, StageSideId::Staged),
            vec!["@@ -1,1 +1,1 @@", "-a", "+b"]
        );
    }
}
