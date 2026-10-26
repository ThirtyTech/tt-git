use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::panes::PaneId;

/// main: one, two, a merge of side (side one), then three.
fn merged() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "1\n", "one");
    sb.checkout_new("side");
    sb.commit_file("s.txt", "1\n", "side one");
    sb.checkout("main");
    sb.commit_file("a.txt", "2\n", "two");
    sb.merge("side");
    sb.commit_file("a.txt", "3\n", "three");
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
fn m_hides_and_shows_merge_commits() {
    let sb = merged();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    assert_eq!(t.app().graph().commits().len(), 5);

    t.press("m");
    assert_eq!(summaries(&t), ["three", "side one", "two", "one"]);
    assert!(t.pane_title(PaneId::Graph).contains("Graph [all · no merges]"));
    // The line through the merge joins its first parent.
    let commits = t.app().graph().commits();
    assert_eq!(*commits[0].parents, [commits[2].oid]);

    // It stays hidden through a filter, and esc clears only the filter.
    t.press("f").press("tab").type_text("a.txt").press("enter");
    assert_eq!(summaries(&t), ["three", "two", "one"]);
    t.press("esc");
    assert_eq!(t.app().graph().commits().len(), 4);
    assert!(t.pane_title(PaneId::Graph).contains("Graph [all · no merges]"));

    t.press("m");
    assert_eq!(t.app().graph().commits().len(), 5);
    assert!(t.pane_title(PaneId::Graph).contains("Graph [all] "));
}
