use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::filter::FilterTab;
use tt_git_tui::modal::Modal;
use tt_git_tui::panes::PaneId;

const ALICE: &str = "Alice <alice@example.com>";

/// Commits a change to `file` as `author` (`Name <email>`).
fn commit_as(sb: &Sandbox, author: &str, file: &str, msg: &str) {
    sb.write(file, &format!("{msg}\n"));
    sb.git(&["add", "--", file]);
    sb.tick();
    sb.git(&["commit", "-q", &format!("--author={author}"), "-m", msg]);
}

/// Alice and Test User both commit on main and on a side branch.
fn two_authors() -> Sandbox {
    let sb = Sandbox::new();
    commit_as(&sb, ALICE, "src/lib.rs", "alice lib");
    sb.commit_file("README.md", "readme\n", "test readme");
    sb.checkout_new("side");
    commit_as(&sb, ALICE, "docs/guide.md", "alice guide");
    sb.commit_file("src/lib.rs", "changed\n", "test lib");
    sb.checkout("main");
    sb
}

fn summaries(t: &TestDriver) -> Vec<String> {
    t.app()
        .graph()
        .commits()
        .iter()
        .map(|c| c.summary.to_string())
        .collect()
}

#[test]
fn filter_panel_lists_authors_and_files() {
    let sb = two_authors();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    t.press("f");
    assert!(matches!(t.app().modal(), Some(Modal::Filter(_))));
    insta::assert_snapshot!("filter_panel_authors", t.screen());
    t.press("tab");
    insta::assert_snapshot!("filter_panel_files", t.screen());
    t.press("esc");
    assert!(t.app().modal().is_none());
    assert_eq!(t.app().graph().commits().len(), 4, "cancelling leaves the graph alone");
}

#[test]
fn filter_graph_by_author_then_file_then_clear() {
    let sb = two_authors();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    assert_eq!(t.app().graph().commits().len(), 4);

    t.press("f").type_text("alice").press("enter");
    assert_eq!(summaries(&t), ["alice guide", "alice lib"]);
    assert!(t.pane_title(PaneId::Graph).contains("Graph [all · author Alice]"));

    // Adding a file keeps the author.
    t.press("f").press("tab").type_text("src/lib").press("enter");
    assert_eq!(summaries(&t), ["alice lib"]);
    assert!(
        t.pane_title(PaneId::Graph)
            .contains("Graph [all · author Alice · file src/lib.rs]")
    );

    // "Any author" leads the author list once one is picked.
    t.press("f").press("enter");
    assert_eq!(summaries(&t), ["test lib", "alice lib"]);
    // Path filters keep the history connected.
    let commits = t.app().graph().commits();
    assert_eq!(*commits[0].parents, [commits[1].oid]);

    // The filter applies to the current branch too.
    t.press("a");
    assert_eq!(summaries(&t), ["alice lib"]);
    t.press("a");

    // esc (with details closed) shows everything again.
    t.press("esc");
    assert_eq!(t.app().graph().commits().len(), 4);
    assert!(t.pane_title(PaneId::Graph).contains("Graph [all] "));
}

#[test]
fn filtering_to_nothing_says_so() {
    let sb = two_authors();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    t.press("f").press("tab").type_text("no/such/file").press("enter");
    assert!(t.app().graph().commits().is_empty());
    t.assert_pane_contains(PaneId::Graph, "no commits match the filter");
    // With details open, esc closes them first and keeps the filter.
    t.press("space").press("esc");
    assert!(!t.app().shows(PaneId::Details));
    assert!(t.app().graph().is_filtered());
    t.press("esc");
    assert!(!t.app().graph().is_filtered());
}

#[test]
fn filter_panel_takes_clicks() {
    let sb = two_authors();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    let tab = |t: &TestDriver| match t.app().modal() {
        Some(Modal::Filter(panel)) => Some(panel.tab()),
        _ => None,
    };

    t.press("f");
    t.click_text("File: any");
    assert_eq!(tab(&t), Some(FilterTab::File));
    t.click_text("Author: any");
    assert_eq!(tab(&t), Some(FilterTab::Author));

    // Clicking outside the panel closes it.
    t.click(0, 0, false);
    assert!(t.app().modal().is_none());
    assert_eq!(t.app().graph().commits().len(), 4);

    // Clicking a choice applies it.
    t.press("f");
    t.click_text("Alice <alice@example.com>");
    assert!(t.app().modal().is_none());
    assert_eq!(summaries(&t), ["alice guide", "alice lib"]);
    assert_eq!(t.app().focus(), PaneId::Graph);

    t.press("f").press("tab");
    t.click_text("src/lib.rs");
    assert_eq!(summaries(&t), ["alice lib"]);

    // The wheel moves the selection: down past the end, then back to
    // "Any file" at the top.
    t.press("f").press("tab");
    let (x, y) = t.find_text("Any file").unwrap();
    t.scroll(x, y, true).scroll(x, y, false).scroll(x, y, false);
    t.press("enter");
    assert_eq!(summaries(&t), ["alice guide", "alice lib"]);
}
