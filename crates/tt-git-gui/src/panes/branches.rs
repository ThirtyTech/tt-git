//! The branches pane (Phase B3): the branches, tags, merged and unmerged,
//! stashes and worktrees tabs in the title, the current tab's rows with
//! plugin decorations and merge-conflict marks, and below them what the
//! TUI puts on the bottom border (the selected row's conflicts, the
//! worktrees' disk space). All of it from the pane's view model.
//!
//! A click on a tab's name in the title switches to it, as in the TUI.

use gpui::{AnyElement, IntoElement, ParentElement, Styled, div};
use tt_git_app::mouse::{MouseTarget, PanePart};
use tt_git_app::panes::PaneId;
use tt_git_app::view::StyleRole;

use super::PaneCx;
use crate::frame::{PaneFrame, body_id, title_cols};
use crate::rows::{StyledRow, StyledRows};
use crate::text::selection;

/// The id the list is measured as.
pub const ROWS: &str = "branches";

pub fn render(cx: &mut PaneCx) -> AnyElement {
    let (ui, focused) = (cx.ui, cx.focused);
    let pane = cx.app.branches_mut();
    let now = pane.clock.now();

    let title = pane.title(title_cols(ui, PaneId::Branches));
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

    let spans = title.spans.clone();
    PaneFrame::new(ui, PaneId::Branches, focused)
        .title_spans(spans)
        // A click on a tab's name switches to it, as in the TUI.
        .on_title_click(move |col| {
            title.tab_at(col).map(|tab| MouseTarget::Pane {
                pane: PaneId::Branches,
                body_row: None,
                part: Some(PanePart::Tab(tab)),
            })
        })
        .bottom(
            footer.conflicts.into_iter().collect(),
            footer.total.into_iter().collect(),
        )
        .child(div().relative().size_full().child(list))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui::{Modifiers, Pixels, Point, TestAppContext, point, px};
    use tt_git_app::panes::BranchTab;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::frame::title_id;
    use crate::testing::Gui;
    use crate::ui::Ui;

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

    /// Where the middle of the title's column `col` (after `[3] `) is drawn.
    fn title_column(gui: &Gui, col: usize) -> Point<Pixels> {
        let ui = ui(gui);
        let line = ui.measures.get(&title_id(PaneId::Branches)).expect("drawn");
        let x = line.left() + ui.font.cell_width * (4 + col) as f32 + ui.font.cell_width / 2.;
        point(x, line.top() + ui.font.line_height / 2.)
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
        let room = title_cols(&ui(&gui), PaneId::Branches);
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
