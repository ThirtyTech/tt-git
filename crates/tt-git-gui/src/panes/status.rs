//! The status pane (Phase B2): the changed files as a list or a folder tree,
//! from the status view model, under the banner of a merge or rebase in
//! progress. The folder filter shows in the title and the empty message.

use gpui::{AnyElement, IntoElement, ParentElement, Styled, div};
use tt_git_app::panes::PaneId;
use tt_git_app::view::{StyleRole, TextSpan};

use super::PaneCx;
use crate::frame::PaneFrame;
use crate::rows::{StyledRow, StyledRows};
use crate::text::{StyledLine, selection};

/// The id the file list is measured as.
pub const ROWS: &str = "status";

pub fn render(cx: &mut PaneCx) -> AnyElement {
    let (ui, focused) = (cx.ui, cx.focused);
    let status = cx.app.status_mut();
    let frame = PaneFrame::new(ui, PaneId::Status, focused).title(status.title());

    if let Some(msg) = status.empty_message() {
        return frame
            .child(div().text_color(ui.theme.fg(StyleRole::Muted)).child(msg))
            .into_any_element();
    }

    let fit = ui.fit(ROWS);
    status.set_viewport(fit.rows);
    let view = status.view(fit.rows, fit.cols);
    let first = status.banner_rows();
    let banner = view.banner.map(|text| {
        let line = StyledLine::new(&ui.theme, &[TextSpan::new(text, StyleRole::LabelTag)], None);
        div()
            .flex_none()
            .h(ui.font.line_height)
            .overflow_hidden()
            .whitespace_nowrap()
            .child(line.element())
    });
    let rows = view
        .rows
        .iter()
        .map(|row| StyledRow::new(&ui.theme, &row.spans, selection(row.picked, focused)))
        .collect();
    frame
        .child(
            div().size_full().flex().flex_col().children(banner).child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(StyledRows::new(ui, ROWS, PaneId::Status, rows).first_body_row(first)),
            ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::testing::Gui;

    /// The rows the pane shows now, as text.
    fn shown(gui: &mut Gui) -> Vec<String> {
        let fit = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(ROWS));
        gui.model.update(&mut gui.cx, |model, _| {
            let status = model.app_mut().status_mut();
            status
                .view(fit.rows, fit.cols)
                .rows
                .iter()
                .map(|r| r.spans.iter().map(|s| s.text.as_ref()).collect::<String>())
                .collect()
        })
    }

    fn selected_path(gui: &Gui) -> Option<String> {
        gui.read(|app| app.status().selected().map(|e| e.path.clone()))
    }

    #[gpui::test]
    fn shows_the_changed_files_and_the_tree(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        sb.commit_file("src/b.rs", "b", "second");
        sb.write("a.txt", "changed");
        sb.write("src/b.rs", "changed");
        sb.write("src/new.rs", "new");
        let mut gui = Gui::open(cx, &sb);
        gui.press("tab");
        assert_eq!(gui.read(|app| app.focus()), PaneId::Status);
        let fit = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(ROWS));
        assert!(
            fit.rows > 3 && fit.cols > 20,
            "the list was drawn and measured: {fit:?}"
        );
        assert_eq!(shown(&mut gui), [" M a.txt", " M src/b.rs", "?? src/new.rs"]);

        // ` lists them as a tree, with the selection kept on its file.
        gui.press("`");
        assert!(gui.read(|app| app.status().tree()));
        assert_eq!(shown(&mut gui), ["▾ src/", "   M b.rs", "  ?? new.rs", " M a.txt"]);
        assert_eq!(selected_path(&gui).as_deref(), Some("a.txt"));
    }

    #[gpui::test]
    fn the_folder_filter_narrows_the_list(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        sb.write("a.txt", "changed");
        sb.write("src/new.rs", "new");
        let mut gui = Gui::open(cx, &sb);
        // What the graph's path filter (f, or . when started in a folder) sets.
        gui.model.update(&mut gui.cx, |model, cx| {
            model.app_mut().status_mut().set_folder(Some("src/".into()));
            cx.notify();
        });
        gui.draw();
        assert_eq!(shown(&mut gui), ["?? src/new.rs"]);
        let title = gui.read(|app| app.status().title());
        assert!(title.ends_with("· in src/ · +1 outside"), "{title}");

        gui.model.update(&mut gui.cx, |model, cx| {
            model.app_mut().status_mut().set_folder(Some("docs/".into()));
            cx.notify();
        });
        gui.draw();
        assert_eq!(
            gui.read(|app| app.status().empty_message()).as_deref(),
            Some("no changes in docs/ (. shows all)")
        );
    }

    #[gpui::test]
    fn clicks_select_and_the_wheel_scrolls(cx: &mut TestAppContext) {
        use gpui::{Modifiers, ScrollDelta, point, px};

        let sb = Sandbox::new();
        sb.commit_file("seed", "s", "first");
        for i in 0..60 {
            sb.write(&format!("f{i:02}.txt"), "new");
        }
        let mut gui = Gui::open(cx, &sb);
        let (bounds, line) = gui.shell.read_with(&gui.cx, |shell, _| {
            let ui = shell.ui();
            (ui.measures.get(ROWS).expect("list measured"), ui.font.line_height)
        });
        let x = bounds.left() + px(20.);
        let at_line = |n: f32| point(x, bounds.top() + line * (n + 0.5));

        // A click on the third line picks the third file, and focuses the pane.
        gui.cx.simulate_click(at_line(2.), Modifiers::none());
        gui.wait_idle();
        assert_eq!(gui.read(|app| app.focus()), PaneId::Status);
        assert_eq!(selected_path(&gui).as_deref(), Some("f02.txt"));

        // A notch down moves the selection down three files.
        gui.cx.simulate_event(gpui::ScrollWheelEvent {
            position: at_line(0.),
            delta: ScrollDelta::Lines(point(0., -3.)),
            ..Default::default()
        });
        gui.wait_idle();
        assert_eq!(selected_path(&gui).as_deref(), Some("f05.txt"));

        // Paging past the screen scrolls the list.
        gui.press("G");
        let rows = shown(&mut gui);
        assert_eq!(rows.last().map(String::as_str), Some("?? f59.txt"), "{rows:?}");
        let offset = gui.read(|app| app.status().cursor().offset);
        assert!(offset > 0, "scrolled");
        // A click now picks the file on that line, not the one at the top.
        gui.cx.simulate_click(at_line(0.), Modifiers::none());
        gui.wait_idle();
        assert_eq!(selected_path(&gui), Some(format!("f{offset:02}.txt")));
    }

    #[gpui::test]
    fn clicks_under_the_merge_banner_pick_the_row_below_it(cx: &mut TestAppContext) {
        use gpui::{Modifiers, point, px};

        let sb = Sandbox::new();
        sb.commit_file("f.txt", "base\n", "base");
        sb.checkout_new("other");
        sb.commit_file("f.txt", "theirs\n", "theirs");
        sb.checkout("main");
        sb.commit_file("f.txt", "ours\n", "ours");
        let mut merge = std::process::Command::new("git");
        sb.apply_env(&mut merge);
        let out = merge.current_dir(sb.path()).args(["merge", "other"]).output().unwrap();
        assert!(!out.status.success(), "the merge stops on the conflict");
        sb.write("g.txt", "new");

        let mut gui = Gui::open(cx, &sb);
        gui.press("2");
        let view = gui.model.update(&mut gui.cx, |model, _| {
            let status = model.app_mut().status_mut();
            let view = status.view(10, 80);
            (view.banner, status.banner_rows())
        });
        assert_eq!(
            view,
            (Some("MERGING: stage fixes · C continue · X abort".to_string()), 1)
        );
        assert_eq!(shown(&mut gui), ["UU f.txt", "?? g.txt"]);

        // The list is laid out under the banner, and its second line is g.txt.
        let (bounds, line, body) = gui.shell.read_with(&gui.cx, |shell, _| {
            let ui = shell.ui();
            let body = ui.measures.get(&crate::frame::body_id(PaneId::Status)).expect("body");
            (ui.measures.get(ROWS).expect("list measured"), ui.font.line_height, body)
        });
        assert_eq!(bounds.top(), body.top() + line, "one line for the banner");
        gui.cx.simulate_click(
            point(bounds.left() + px(20.), bounds.top() + line * 1.5),
            Modifiers::none(),
        );
        gui.wait_idle();
        assert_eq!(selected_path(&gui).as_deref(), Some("g.txt"));
    }
}
