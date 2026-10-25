//! The branches pane (Phase B3): the branches, tags, merged and unmerged,
//! stashes and worktrees tabs in the title, the current tab's rows with
//! plugin decorations and merge-conflict marks, and below them what the
//! TUI puts on the bottom border (the selected row's conflicts, the
//! worktrees' disk space). All of it from the pane's view model.
//!
//! A click on a tab's name in the title switches to it, as in the TUI.

use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Point,
    Styled, Window, div, px,
};
use tt_git_app::mouse::{MouseInput, MouseKind, MouseTarget, PanePart};
use tt_git_app::panes::PaneId;
use tt_git_app::view::StyleRole;
use tt_git_app::view::branches::{BranchesFooter, BranchesTitle};
use unicode_width::UnicodeWidthStr;

use super::PaneCx;
use crate::frame::{PaneFrame, body_id, frame_id};
use crate::rows::{StyledRow, StyledRows};
use crate::text::{StyledLine, selection};
use crate::ui::Ui;

/// The id the list is measured as.
pub const ROWS: &str = "branches";

/// The border's width, as `PaneFrame` draws it.
const BORDER: Pixels = px(1.);

pub fn render(cx: &mut PaneCx) -> AnyElement {
    let (ui, focused) = (cx.ui, cx.focused);
    let pane = cx.app.branches_mut();
    let now = pane.clock.now();

    let title = pane.title(title_room(ui));
    let footer = pane.footer(ui.fit(&body_id(PaneId::Branches)).cols);
    let fit = ui.fit(ROWS);
    if !pane.visible.is_empty() {
        pane.set_viewport(fit.rows);
    }
    let view = pane.view(fit.rows, fit.cols, now);
    pane.note_drawn(now, view.range.clone());

    let list = match &view.empty {
        Some(msg) => div()
            .text_color(ui.theme.fg(StyleRole::Muted))
            .child(msg.clone())
            .into_any_element(),
        None => {
            let rows = view
                .rows
                .iter()
                .map(|row| StyledRow::new(&ui.theme, &row.spans, selection(row.picked, focused)))
                .collect();
            StyledRows::new(ui, ROWS, PaneId::Branches, rows).into_any_element()
        }
    };
    let body = div()
        .size_full()
        .flex()
        .flex_col()
        .child(div().relative().flex_1().min_h_0().child(list))
        .children(footer_line(ui, footer));

    let frame = PaneFrame::new(ui, PaneId::Branches, focused)
        .title_spans(title.spans.clone())
        .child(body);
    let click = {
        let ui = ui.clone();
        move |ev: &MouseDownEvent, _: &mut Window, cx: &mut App| {
            if ev.button != MouseButton::Left {
                return;
            }
            let Some(tab) = tab_at(&ui, &title, ev.position) else {
                return;
            };
            // The tab, instead of the frame's click on the border.
            cx.stop_propagation();
            ui.send_mouse(
                MouseInput {
                    kind: MouseKind::Click,
                    ctrl: ev.modifiers.control,
                    target: MouseTarget::Pane {
                        pane: PaneId::Branches,
                        body_row: None,
                        part: Some(PanePart::Tab(tab)),
                    },
                },
                cx,
            );
        }
    };
    div()
        .size_full()
        .capture_any_mouse_down(click)
        .child(frame)
        .into_any_element()
}

/// `[3] `, before the title.
fn prefix_width() -> usize {
    format!("[{}] ", PaneId::Branches.number()).width()
}

/// The columns the title has inside the frame's corners, after `[3] `, as
/// laid out last frame.
fn title_room(ui: &Ui) -> usize {
    let Some(frame) = ui.measures.get(&frame_id(PaneId::Branches)) else {
        return 0;
    };
    // PaneFrame pads the title by a cell each side.
    let inside = frame.size.width - BORDER * 2. - ui.font.cell_width * 2.;
    let cols = (inside / ui.font.cell_width).floor().max(0.) as usize;
    cols.saturating_sub(prefix_width())
}

/// The tab whose name is under `pos`, if `pos` is on the title.
fn tab_at(ui: &Ui, title: &BranchesTitle, pos: Point<Pixels>) -> Option<tt_git_app::panes::BranchTab> {
    let frame = ui.measures.get(&frame_id(PaneId::Branches))?;
    let top = frame.top();
    if pos.y < top || pos.y >= top + BORDER + ui.font.line_height {
        return None;
    }
    let start = frame.left() + BORDER + ui.font.cell_width * (1 + prefix_width()) as f32;
    if pos.x < start {
        return None;
    }
    let col = ((pos.x - start) / ui.font.cell_width).floor() as usize;
    if col >= title_room(ui) {
        return None;
    }
    title.tab_at(col)
}

/// The line under the list with what the TUI writes on the bottom border:
/// the conflicts on the left, the worktrees' total on the right.
fn footer_line(ui: &Ui, footer: BranchesFooter) -> Option<AnyElement> {
    if footer.conflicts.is_none() && footer.total.is_none() {
        return None;
    }
    let part = |span: Option<_>| {
        span.map(|span| {
            div()
                .overflow_hidden()
                .child(StyledLine::new(&ui.theme, &[span], None).element())
        })
    };
    Some(
        div()
            .flex_none()
            .h(ui.font.line_height)
            .flex()
            .justify_between()
            .overflow_hidden()
            .whitespace_nowrap()
            .child(div().min_w_0().children(part(footer.conflicts)))
            .children(part(footer.total))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use gpui::{Modifiers, TestAppContext, point};
    use tt_git_app::panes::BranchTab;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::testing::Gui;

    fn texts(gui: &mut Gui) -> Vec<String> {
        let fit = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(ROWS));
        gui.model.update(&mut gui.cx, |model, _| {
            let pane = model.app_mut().branches_mut();
            let view = pane.view(fit.rows, fit.cols, pane.clock.now());
            view.rows
                .iter()
                .map(|r| r.spans.iter().map(|s| s.text.as_ref()).collect::<String>())
                .collect()
        })
    }

    fn ui(gui: &Gui) -> Ui {
        gui.shell.read_with(&gui.cx, |shell, _| shell.ui().clone())
    }

    /// Where the middle of the title's column `col` is drawn.
    fn title_column(gui: &Gui, col: usize) -> Point<Pixels> {
        let ui = ui(gui);
        let frame = ui.measures.get(&frame_id(PaneId::Branches)).expect("drawn");
        let x =
            frame.left() + BORDER + ui.font.cell_width * (1 + prefix_width() + col) as f32 + ui.font.cell_width / 2.;
        point(x, frame.top() + BORDER + ui.font.line_height / 2.)
    }

    fn branches(sb: &Sandbox, n: usize) {
        sb.commit_file("a.txt", "a", "first");
        for i in 0..n {
            sb.git(&["branch", &format!("topic-{i:02}")]);
        }
    }

    #[gpui::test]
    fn lists_the_branches(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        branches(&sb, 3);
        let mut gui = Gui::open(cx, &sb);
        let fit = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(ROWS));
        assert!(
            fit.rows > 0 && fit.cols > 20,
            "the list was drawn and measured: {fit:?}"
        );
        let text = texts(&mut gui);
        assert_eq!(text.len(), 4, "{text:#?}");
        assert!(
            text.iter().any(|t| t.starts_with("* ")),
            "the checked-out one: {text:#?}"
        );
        assert!(text.iter().any(|t| t.contains("topic-01")), "{text:#?}");
        let room = title_room(&ui(&gui));
        assert!(room > 10, "the title has room: {room}");
    }

    #[gpui::test]
    fn a_click_on_a_tab_switches_to_it(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        branches(&sb, 1);
        sb.git(&["tag", "v1"]);
        let mut gui = Gui::open(cx, &sb);
        let title = gui.read(|app| app.branches().title(200));
        let (_, tags) = title
            .tabs
            .iter()
            .find(|(t, _)| *t == BranchTab::Tags)
            .cloned()
            .expect("a tags tab");
        let pos = title_column(&gui, tags.start + 1);
        gui.cx.simulate_click(pos, Modifiers::none());
        gui.wait_idle();
        assert_eq!(gui.read(|app| app.branches().tab()), BranchTab::Tags);
        assert_eq!(
            gui.read(|app| app.focus()),
            PaneId::Branches,
            "the click focused the pane"
        );
        let text = texts(&mut gui);
        assert!(text[0].contains("v1"), "{text:#?}");

        // On the separator: no tab, only the focus.
        let pos = title_column(&gui, tags.start - 2);
        gui.cx.simulate_click(pos, Modifiers::none());
        gui.wait_idle();
        assert_eq!(gui.read(|app| app.branches().tab()), BranchTab::Tags);
    }

    #[gpui::test]
    fn a_click_picks_a_row_and_keys_switch_tabs(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        branches(&sb, 3);
        let mut gui = Gui::open(cx, &sb);
        let ui = ui(&gui);
        let list = ui.measures.get(ROWS).expect("drawn");
        let pos = point(list.left() + px(20.), list.top() + ui.font.line_height * 2.5);
        gui.cx.simulate_click(pos, Modifiers::none());
        gui.wait_idle();
        assert_eq!(
            gui.read(|app| app.focus()),
            PaneId::Branches,
            "the click focused the pane"
        );
        assert_eq!(
            gui.read(|app| app.branches().cursor().selected),
            2,
            "and picked the third row"
        );
        // Keys need no GUI code: the keymap handles them.
        gui.press("]");
        assert_eq!(gui.read(|app| app.branches().tab()), BranchTab::Tags);
    }

    #[gpui::test]
    fn the_wheel_scrolls_the_list(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        branches(&sb, 60);
        let mut gui = Gui::open(cx, &sb);
        let ui = ui(&gui);
        let list = ui.measures.get(ROWS).expect("drawn");
        let pos = point(list.left() + px(20.), list.top() + ui.font.line_height);
        // Three notches down: three steps of three rows each.
        gui.cx.simulate_event(gpui::ScrollWheelEvent {
            position: pos,
            delta: gpui::ScrollDelta::Lines(point(0., -9.)),
            modifiers: Modifiers::none(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        gui.wait_idle();
        assert_eq!(gui.read(|app| app.branches().cursor().selected), 9);
    }
}
