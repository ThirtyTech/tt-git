//! The graph pane (Phase B1): one row per commit, from the graph's view
//! model. The lanes are drawn as the view model's text for now; B1 paints
//! them.

use gpui::{AnyElement, IntoElement, ParentElement, Styled, div};
use tt_git_app::panes::PaneId;
use tt_git_app::view::StyleRole;

use super::PaneCx;
use crate::frame::PaneFrame;
use crate::rows::{StyledRow, StyledRows};
use crate::text::selection;

/// The id the commit list is measured as.
pub const ROWS: &str = "graph";

pub fn render(cx: &mut PaneCx) -> AnyElement {
    let (ui, focused) = (cx.ui, cx.focused);
    let graph = cx.app.graph_mut();
    let now = graph.clock().now();
    let frame = PaneFrame::new(ui, PaneId::Graph, focused).title(graph.title());

    if let Some(msg) = graph.empty_message() {
        graph.note_drawn(now, 0..0);
        return frame
            .child(div().text_color(ui.theme.fg(StyleRole::Muted)).child(msg))
            .into_any_element();
    }

    let fit = ui.fit(ROWS);
    graph.set_viewport(fit.rows);
    let view = graph.view(fit.rows, fit.cols, now);
    let range = view.range.clone();
    let rows = view
        .rows
        .iter()
        .map(|row| StyledRow::new(&ui.theme, &row.spans, selection(row.picked, focused)))
        .collect();
    graph.note_drawn(now, range);
    frame
        .child(StyledRows::new(ui, ROWS, PaneId::Graph, rows))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::testing::Gui;

    #[gpui::test]
    fn shows_the_sandbox_commits(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        for i in 1..=5 {
            sb.commit_file("a.txt", &i.to_string(), &format!("commit {i}"));
        }
        let mut gui = Gui::open(cx, &sb);
        let fit = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(ROWS));
        assert!(
            fit.rows > 5 && fit.cols > 60,
            "the list was drawn and measured: {fit:?}"
        );
        let text = gui.model.update(&mut gui.cx, |model, _| {
            let graph = model.app_mut().graph_mut();
            let view = graph.view(fit.rows, fit.cols, graph.clock().now());
            assert_eq!(view.range, 0..5);
            view.rows
                .iter()
                .map(|r| r.spans.iter().map(|s| s.text.as_ref()).collect::<String>())
                .collect::<Vec<_>>()
        });
        assert!(
            text[0].contains("commit 5") && text[4].contains("commit 1"),
            "{text:#?}"
        );
        // The pane told the graph how many rows it shows.
        gui.press("ctrl+d");
        assert_eq!(
            gui.read(|app| app.graph().selected_index()),
            4,
            "a page down goes to the end"
        );
    }
}
