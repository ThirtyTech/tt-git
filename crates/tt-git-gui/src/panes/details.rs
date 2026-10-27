//! The details pane (Phase B4): the commit or file it shows, and in the
//! files view its file list beside the diff (fullscreen) or along the
//! bottom, the commit header over the diff. The text itself is
//! `details/diff.rs`'s.
//!
//! Staging and blame take over the pane; `panes::render` sends those to
//! `details/stage.rs` (B5) and `details/blame.rs` (B6), so each has a file
//! of its own.

use gpui::{AnyElement, IntoElement, ParentElement, Styled, div, px};
use tt_git_app::mouse::{MouseTarget, PanePart};
use tt_git_app::panes::{DetailsPane, PaneId};
use tt_git_app::view::details::FilesLayout;
use tt_git_app::view::{StyleRole, TextSpan};

use super::PaneCx;
use crate::frame::{PaneFrame, body_id};
use crate::rows::{StyledRow, StyledRows};
use crate::text::{StyledLine, selection};
use crate::ui::Ui;

pub mod blame;
pub mod diff;
pub mod stage;

/// The id the files view's file list is measured as.
pub const FILES: &str = "details:files";
/// The id the files view's commit header is measured as.
pub const HEADER: &str = "details:header";

pub fn render(cx: &mut PaneCx) -> AnyElement {
    let (ui, focused) = (cx.ui, cx.focused);
    let pane = cx.app.details_mut();
    let frame = PaneFrame::new(ui, PaneId::Details, focused).title(pane.frame_title());
    let body = ui.fit(&body_id(PaneId::Details));

    if let Some(msg) = pane.empty_message() {
        pane.set_viewport(body.rows);
        return frame
            .child(div().text_color(ui.theme.fg(StyleRole::Muted)).child(msg))
            .into_any_element();
    }
    match pane.files_layout(body.cols, body.rows) {
        None => frame
            .child(div().relative().size_full().child(diff::render(ui, pane, 0, None)))
            .into_any_element(),
        Some(layout) => frame
            .child(files_view(ui, pane, focused, layout, body.rows))
            .into_any_element(),
    }
}

/// The files view laid out as `layout` says in a body `height` rows high.
fn files_view(ui: &Ui, pane: &mut DetailsPane, focused: bool, layout: FilesLayout, height: usize) -> AnyElement {
    let theme = &ui.theme;
    let line_height = ui.font.line_height;
    let list_rows = ui.fit(FILES).rows;
    pane.set_files_viewport(list_rows, layout.header_rows, layout.diff_rows);

    let label = pane.files_label().unwrap_or_default();
    let title = StyledLine::new(theme, &[TextSpan::new(label, StyleRole::Muted)], None).element();
    let files = pane
        .file_rows(list_rows, ui.fit(FILES).cols)
        .iter()
        .map(|row| StyledRow::new(theme, &row.spans, selection(row.picked, focused)))
        .collect();
    let list_top = layout.list_top(height);
    let files = StyledRows::new(ui, FILES, PaneId::Details, files).on_target(move |line| MouseTarget::Pane {
        pane: PaneId::Details,
        body_row: Some(list_top + line),
        part: Some(PanePart::FileRow(line)),
    });

    let header: Vec<StyledRow> = pane
        .header_rows(layout.header_rows)
        .iter()
        .map(|spans| StyledRow::new(theme, spans, None))
        .collect();
    let rule = layout.header_rows > 0;
    let diff = diff::render(ui, pane, layout.diff_top(), Some(PanePart::Diff));
    // Native: the rules are hairlines, as the cards' edges are.
    let border = if ui.native() {
        theme.surfaces.hairline_strong
    } else {
        theme.fg(StyleRole::Border)
    };
    let text = div()
        .flex()
        .flex_col()
        .min_w_0()
        .child(
            div()
                .flex_none()
                .h(line_height * layout.header_rows as f32)
                .child(StyledRows::new(ui, HEADER, PaneId::Details, header)),
        )
        .child(
            div()
                .flex_none()
                .h(line_height * usize::from(rule) as f32)
                .flex()
                .items_center()
                .child(div().w_full().h(px(1.)).bg(border)),
        )
        .child(div().relative().flex_1().min_h_0().child(diff));

    if layout.beside {
        let list = div()
            .flex_none()
            .w(ui.font.cell_width * layout.list_width as f32)
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(border)
            .child(div().flex_none().h(line_height).overflow_hidden().child(title))
            .child(div().relative().flex_1().min_h_0().child(files));
        div()
            .size_full()
            .flex()
            .flex_row()
            .child(list)
            .child(text.flex_1().h_full().pl(ui.font.cell_width))
            .into_any_element()
    } else {
        // Along the bottom, under a rule carrying the title.
        let list = div()
            .flex_none()
            .h(line_height * (layout.list_rows + 1) as f32)
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .h(line_height)
                    .flex()
                    .items_center()
                    .gap(ui.font.cell_width / 2.)
                    .whitespace_nowrap()
                    .child(div().flex_none().child(title))
                    .child(div().flex_1().h(px(1.)).bg(border)),
            )
            .child(div().relative().flex_1().min_h_0().child(files));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(text.flex_1().min_h_0().w_full())
            .child(list)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use gpui::{
        Bounds, Modifiers, MouseButton, Pixels, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, point, px,
    };
    use tt_git_app::panes::DetailsSubject;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::testing::Gui;

    /// A commit changing three files, `long.txt` with 200 lines.
    fn sandbox() -> Sandbox {
        let sb = Sandbox::new();
        sb.commit_file("first.txt", "first", "first");
        let long: String = (1..=200).map(|i| format!("line {i}\n")).collect();
        sb.write("long.txt", &long);
        sb.write("short.txt", "short\n");
        sb.write("first.txt", "changed\n");
        sb.git(&["add", "-A"]);
        sb.commit_staged("three files");
        sb
    }

    fn bounds(gui: &Gui, id: &str) -> Bounds<Pixels> {
        gui.shell.read_with(&gui.cx, |shell, _| bounds_of(shell.ui(), id))
    }

    fn bounds_of(ui: &Ui, id: &str) -> Bounds<Pixels> {
        ui.measures.get(id).unwrap_or_else(|| panic!("{id} was drawn"))
    }

    fn texts(gui: &mut Gui) -> Vec<String> {
        let fit = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(diff::TEXT));
        gui.model.update(&mut gui.cx, |model, _| {
            model
                .app_mut()
                .details_mut()
                .text_rows(fit.rows, fit.cols)
                .iter()
                .map(|r| r.spans.iter().map(|s| s.text.as_ref()).collect())
                .collect()
        })
    }

    fn selected_file(gui: &Gui) -> Option<String> {
        gui.read(|app| app.details().selected_file().map(|f| f.path.clone()))
    }

    fn wheel(gui: &mut Gui, at: gpui::Point<Pixels>, lines: f32) {
        gui.cx.simulate_event(ScrollWheelEvent {
            position: at,
            delta: ScrollDelta::Lines(point(0., lines)),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        });
        gui.wait_idle();
    }

    #[gpui::test]
    fn fullscreen_lists_the_files_beside_the_diff(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        gui.press("enter");
        let subject = gui.read(|app| app.details().subject().cloned());
        assert!(matches!(subject, Some(DetailsSubject::CommitFiles(_))), "{subject:?}");
        assert!(gui.read(|app| app.details().files_beside));

        let (files, text) = (bounds(&gui, FILES), bounds(&gui, diff::TEXT));
        assert!(
            files.right() <= text.left(),
            "list left of the diff: {files:?} {text:?}"
        );
        let fit = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(FILES));
        assert!(fit.rows > 3, "{fit:?}");
        assert_eq!(selected_file(&gui).as_deref(), Some("first.txt"));
        let shown = texts(&mut gui);
        assert!(shown.iter().any(|l| l == "+changed"), "{shown:#?}");

        // A click on the second file shows its diff.
        let line = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().font.line_height);
        let at = point(files.left() + px(10.), files.top() + line * 1.5);
        gui.cx.simulate_click(at, Modifiers::default());
        gui.wait_idle();
        assert_eq!(selected_file(&gui).as_deref(), Some("long.txt"));
        assert!(texts(&mut gui).iter().any(|l| l == "+line 1"));
        assert_eq!(gui.read(|app| app.focus()), PaneId::Details);

        // The wheel over the diff scrolls it, and over the list moves
        // through the files.
        let text = bounds(&gui, diff::TEXT);
        wheel(&mut gui, point(text.left() + px(10.), text.top() + px(10.)), -3.);
        assert_eq!(gui.read(|app| app.details().cursor.offset), 3);
        wheel(&mut gui, point(files.left() + px(10.), files.top() + px(5.)), -3.);
        assert_eq!(selected_file(&gui).as_deref(), Some("short.txt"));
    }

    #[gpui::test]
    fn the_files_go_along_the_bottom_in_the_split(cx: &mut TestAppContext) {
        // The test window is small: the terminal style's shorter rows leave
        // a third of the body for all three files.
        cx.update(|cx| crate::look::set(crate::look::UiStyle::Terminal, cx));
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        gui.press("space");
        gui.press("tab");
        if !gui.read(|app| app.details().commit_files().is_some()) {
            gui.press("}");
        }
        assert_eq!(gui.read(|app| app.focus()), PaneId::Details);
        assert!(gui.read(|app| app.details().commit_files().is_some()), "the files view");
        assert!(!gui.read(|app| app.details().files_beside));
        let (files, text, header) = (bounds(&gui, FILES), bounds(&gui, diff::TEXT), bounds(&gui, HEADER));
        assert!(header.bottom() <= text.top(), "header over the diff");
        assert!(text.bottom() <= files.top(), "list under the diff: {text:?} {files:?}");
        let fit = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(FILES));
        assert_eq!(fit.rows, 3, "every file fits");

        // The arrow keys pick files; the diff follows.
        gui.press("j");
        assert_eq!(selected_file(&gui).as_deref(), Some("long.txt"));
        let shown = texts(&mut gui);
        assert!(shown[0].starts_with("diff --git a/long.txt"), "{shown:#?}");
    }

    #[gpui::test]
    fn a_commit_shows_as_text_and_scrolls(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        gui.press("space");
        gui.press("tab");
        if gui.read(|app| app.details().commit_files().is_some()) {
            gui.press("}");
        }
        assert!(gui.read(|app| app.details().commit_files().is_none()), "plain text");
        let shown = texts(&mut gui);
        assert!(shown[0].starts_with("commit "), "{shown:#?}");
        let fit = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(diff::TEXT));
        assert_eq!(gui.read(|app| app.details().cursor.height), fit.rows, "set_viewport");
        gui.press("ctrl+d");
        assert_eq!(gui.read(|app| app.details().cursor.offset), fit.rows, "a page down");

        // A search picks out its matches.
        gui.press("/");
        for c in "line 1".chars() {
            gui.press(&c.to_string());
        }
        gui.press("enter");
        gui.press("n");
        let found = gui.model.update(&mut gui.cx, |model, _| {
            let pane = model.app_mut().details_mut();
            pane.text_rows(fit.rows, fit.cols)
                .iter()
                .any(|r| r.spans.iter().any(|s| s.over == Some(StyleRole::SearchMatch)))
        });
        assert!(found, "a match on screen");
        assert!(gui.read(|app| app.details().frame_title()).contains("/line 1"));
    }

    #[gpui::test]
    fn dragging_selects_text_and_ctrl_c_copies_it(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        gui.press("space");
        gui.press("tab");
        if gui.read(|app| app.details().commit_files().is_some()) {
            gui.press("}");
        }
        let shown = texts(&mut gui);
        assert!(shown[0].starts_with("commit "), "{shown:#?}");
        let (text, cell, line) = gui.shell.read_with(&gui.cx, |shell, _| {
            let ui = shell.ui();
            (bounds_of(ui, diff::TEXT), ui.font.cell_width, ui.font.line_height)
        });
        let at = |row: f32, col: f32| point(text.left() + cell * col, text.top() + line * (row + 0.5));

        // From the start of the first line to the end of "commit", then on
        // to the second line's first two columns.
        gui.cx
            .simulate_mouse_down(at(0., 0.), MouseButton::Left, Modifiers::none());
        gui.cx
            .simulate_mouse_move(at(0., 6.), MouseButton::Left, Modifiers::none());
        gui.wait_idle();
        assert_eq!(gui.read(|app| app.focus()), PaneId::Details);
        let picked = |gui: &Gui| gui.read(|app| app.details().selection().map(|s| s.text()));
        assert_eq!(picked(&gui).as_deref(), Some("commit"));
        gui.cx
            .simulate_mouse_move(at(1., 2.), MouseButton::Left, Modifiers::none());
        gui.cx
            .simulate_mouse_up(at(1., 2.), MouseButton::Left, Modifiers::none());
        gui.wait_idle();
        let second: String = shown[1].chars().take(2).collect();
        assert_eq!(picked(&gui), Some(format!("{}\n{second}", shown[0])));
        // Moving without the button down leaves it as it is.
        gui.cx.simulate_mouse_move(at(0., 3.), None, Modifiers::none());
        gui.wait_idle();
        assert_eq!(picked(&gui), Some(format!("{}\n{second}", shown[0])));

        // ctrl+c (cmd+c on macOS) copies it rather than quitting.
        gui.keystrokes(if cfg!(target_os = "macos") { "cmd-c" } else { "ctrl-c" });
        assert!(gui.is_open(), "still open");
        let copied = gui.cx.read_from_clipboard().and_then(|c| c.text());
        assert_eq!(copied, Some(format!("{}\n{second}", shown[0])));
        assert_eq!(
            gui.read(|app| app.notice().map(str::to_string)).as_deref(),
            Some("copied 2 lines")
        );

        // A click without a drag clears it, and c copies all of the text.
        gui.cx.simulate_click(at(0., 3.), Modifiers::none());
        gui.wait_idle();
        assert_eq!(picked(&gui), None);
        gui.press("c");
        let all = gui.read(|app| app.details().lines().join("\n"));
        assert_eq!(gui.cx.read_from_clipboard().and_then(|c| c.text()), Some(all));
    }

    #[gpui::test]
    fn nothing_selected_says_so(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        let mut gui = Gui::open(cx, &sb);
        gui.press("space");
        assert_eq!(gui.read(|app| app.details().empty_message()), Some("nothing selected"));
    }
}
