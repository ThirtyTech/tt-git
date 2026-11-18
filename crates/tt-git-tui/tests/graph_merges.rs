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
    // three keeps its own parent, the merge, but side's work is drawn in
    // main's line, where it landed: one lane, with side one's node marking
    // the merge.
    let commits = t.app().graph().commits();
    assert!(!commits[0].is_merge());
    assert_ne!(*commits[0].parents, [commits[1].oid]);
    insta::assert_snapshot!(t.pane_lines(PaneId::Graph)[..4].join("\n"));

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

#[test]
fn a_branch_at_a_hidden_merge_shows_on_the_commit_in_its_place() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "1\n", "one");
    sb.checkout_new("side");
    sb.commit_file("s.txt", "1\n", "side one");
    sb.checkout("main");
    sb.commit_file("a.txt", "2\n", "two");
    sb.merge("side");
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);

    t.press("m");
    assert_eq!(summaries(&t), ["side one", "two", "one"]);
    let lines = t.pane_lines(PaneId::Graph);
    assert!(lines[0].contains("main ↑"), "main moved, marked: {lines:#?}");
    assert!(lines[0].contains("[side]"), "side is really there: {lines:#?}");
}
