//! Blame in the details pane (Phase B6): each line of the file with when
//! it last changed (the heat stripe and the date), who changed it and in
//! which commit, from the blame view model. The selected line's commit
//! shows below the code when `space` asks for it; the title says which version
//! this is and whether it's the latest. A click selects a line and the
//! wheel scrolls; every key works as in the TUI through the keymap.

use gpui::{AnyElement, IntoElement, ParentElement, Styled, div};
use tt_git_app::panes::PaneId;
use tt_git_app::view::blame::{BlameCommitView, BlameLines};
use tt_git_app::view::details::fit_title_right;
use tt_git_app::view::{StyleRole, TextSpan};
use unicode_width::UnicodeWidthStr;

use crate::frame::{PaneFrame, body_id, title_cols};
use crate::panes::PaneCx;
use crate::rows::{StyledRow, StyledRows};
use crate::text::StyledLine;
use crate::ui::Ui;

/// The id the code list is measured as.
pub const ROWS: &str = "blame";

pub fn render(cx: &mut PaneCx) -> AnyElement {
    let (ui, focused) = (cx.ui, cx.focused);
    let details = cx.app.details_mut();
    let (title, right) = (details.frame_title(), details.title_right());
    let frame = PaneFrame::new(ui, PaneId::Details, focused);
    let Some(view) = details.blame_mut() else {
        return frame.title(title).into_any_element();
    };

    // With a space to spare between the two.
    let right = fit_title_right(right, title.width() + 1, title_cols(ui, PaneId::Details));

    let commit_rows = view.commit_rows(ui.fit(&body_id(PaneId::Details)).rows);
    let fit = ui.fit(ROWS);
    // Before the list's first frame there's nothing to fit: wait for it,
    // so where the selection is put on screen isn't spent on no rows.
    let lines = (fit.rows > 0).then(|| {
        view.set_viewport(fit.rows);
        view.lines_view(fit.rows, fit.cols)
    });
    let code = code_list(ui, lines, focused);
    let commit = (commit_rows > 0).then(|| view.commit_view()).flatten();

    let body = div().size_full().flex().flex_col().child(code);
    let body = match commit {
        Some(commit) => body.child(commit_details(ui, commit, commit_rows)),
        None => body,
    };
    frame
        .title(title)
        .title_right(right.unwrap_or_default())
        .child(body)
        .into_any_element()
}

/// The code rows, or what's shown instead (`loading…`, an error), filling
/// the space above the commit details and measured as [`ROWS`].
fn code_list(ui: &Ui, lines: Option<BlameLines>, focused: bool) -> AnyElement {
    let area = div().relative().flex_1().min_h_0().w_full();
    let lines = match lines {
        Some(lines) if lines.message.is_none() => lines,
        other => {
            let message = other.and_then(|l| l.message).map(|m| {
                div()
                    .text_color(ui.theme.fg(m.role))
                    .child(StyledLine::new(&ui.theme, &[m], None).element())
            });
            return area.child(ui.measure(ROWS)).children(message).into_any_element();
        }
    };
    let rows = lines
        .rows
        .iter()
        .map(|row| StyledRow::new(&ui.theme, &row.spans, row.look.role(focused)))
        .collect();
    area.child(StyledRows::new(ui, ROWS, PaneId::Details, rows))
        .into_any_element()
}

/// The selected line's commit under a rule carrying its title, `rows`
/// lines high (the rule's included, as in the TUI).
fn commit_details(ui: &Ui, commit: BlameCommitView, rows: usize) -> AnyElement {
    let line_height = ui.font.line_height;
    let line = |spans: &[TextSpan]| {
        div()
            .h(line_height)
            .w_full()
            .overflow_hidden()
            .whitespace_nowrap()
            .child(StyledLine::new(&ui.theme, spans, None).element())
    };
    div()
        .flex_none()
        .h(line_height * rows as f32)
        .w_full()
        .overflow_hidden()
        .border_t_1()
        .border_color(ui.theme.fg(StyleRole::Border))
        .child(line(std::slice::from_ref(&commit.title)))
        .children(commit.lines.iter().take(rows.saturating_sub(1)).map(|l| line(l)))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui::{Modifiers, TestAppContext, point, px};
    use tt_git_app::view::blame::RowLook;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::testing::Gui;

    /// f.rs over three commits, line 4 changed in the working tree, blamed
    /// from the status pane. Keep the sandbox while the window is open.
    fn blamed(cx: &mut TestAppContext) -> (Sandbox, Gui) {
        let sb = Sandbox::new();
        sb.commit_file("f.rs", "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\n", "add f");
        sb.commit_file("f.rs", "fn a() {}\nfn B() {}\nfn c() {}\nfn d() {}\n", "shout b");
        sb.write("f.rs", "fn a() {}\nfn B() {}\nfn c() {}\nfn D() {}\n");
        let mut gui = Gui::open(cx, &sb);
        gui.press_all(&["2", "g", "b"]);
        assert!(gui.read(|app| app.details().blame().is_some()), "blaming");
        (sb, gui)
    }

    fn row_texts(gui: &mut Gui) -> Vec<String> {
        let fit = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(ROWS));
        gui.model.update(&mut gui.cx, |model, _| {
            let view = model.app_mut().details_mut().blame_mut().expect("blaming");
            let lines = view.lines_view(fit.rows, fit.cols);
            lines
                .rows
                .iter()
                .map(|r| r.spans.iter().map(|s| s.text.as_ref()).collect())
                .collect()
        })
    }

    #[gpui::test]
    fn shows_each_line_with_its_commit(cx: &mut TestAppContext) {
        let (_sb, mut gui) = blamed(cx);
        let fit = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(ROWS));
        assert!(
            fit.rows > 4 && fit.cols > 40,
            "the code was drawn and measured: {fit:?}"
        );
        let text = row_texts(&mut gui);
        assert_eq!(text.len(), 4, "{text:#?}");
        assert!(text[1].contains("▌  2 fn B() {}"), "{}", text[1]);
        assert!(text[3].contains("You") && text[3].contains("fn D() {}"), "{}", text[3]);
        // The pane told the view how many rows it shows.
        gui.press("G");
        assert_eq!(gui.read(|app| app.details().blame().unwrap().selected_line()), 4);
    }

    #[gpui::test]
    fn a_click_selects_a_line(cx: &mut TestAppContext) {
        let (_sb, mut gui) = blamed(cx);
        let (bounds, line_height) = gui.shell.read_with(&gui.cx, |shell, _| {
            let ui = shell.ui();
            (ui.measures.get(ROWS).expect("measured"), ui.font.line_height)
        });
        let at = point(bounds.left() + px(40.), bounds.top() + line_height * 2.5);
        gui.cx.simulate_click(at, Modifiers::default());
        gui.wait_idle();
        assert_eq!(gui.read(|app| app.details().blame().unwrap().selected_line()), 3);
        let looks: Vec<RowLook> = gui.model.update(&mut gui.cx, |model, _| {
            let view = model.app_mut().details_mut().blame_mut().unwrap();
            view.lines_view(10, 80).rows.iter().map(|r| r.look).collect()
        });
        assert_eq!(looks[2], RowLook::Selected);
    }

    #[gpui::test]
    fn space_shows_the_commit_below_the_code(cx: &mut TestAppContext) {
        let (_sb, mut gui) = blamed(cx);
        let before = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(ROWS));
        gui.press("space");
        assert!(gui.read(|app| app.details().blame().unwrap().show_commit()));
        let after = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(ROWS));
        assert!(
            after.rows < before.rows,
            "the code made room: {before:?} then {after:?}"
        );
        let commit = gui.model.update(&mut gui.cx, |model, _| {
            model.app_mut().details_mut().blame_mut().unwrap().commit_view()
        });
        let commit = commit.expect("a line is selected");
        assert!(commit.title.text.starts_with("Commit "), "{:?}", commit.title);
    }

    #[gpui::test]
    fn bracket_steps_back_before_the_lines_commit(cx: &mut TestAppContext) {
        let (_sb, mut gui) = blamed(cx);
        gui.press_all(&["2", "G"]);
        assert_eq!(gui.read(|app| app.details().blame().unwrap().selected_line()), 2);
        gui.press("[");
        let (title, depth) = gui.read(|app| (app.details().frame_title(), app.details().blame().unwrap().depth()));
        assert_eq!(depth, 2, "{title}");
        assert!(
            title.starts_with("Blame f.rs at ") && title.contains("^  ‹ working tree"),
            "{title}"
        );
        let text = row_texts(&mut gui);
        assert!(text[1].contains("fn b() {}"), "the version before: {text:#?}");
        gui.press("]");
        assert_eq!(gui.read(|app| app.details().blame().unwrap().depth()), 1);
    }
}
