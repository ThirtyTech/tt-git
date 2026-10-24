use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::panes::PaneId;

fn conflicting() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "base\n", "base");
    sb.checkout_new("other");
    sb.commit_file("f.txt", "theirs\n", "theirs");
    sb.checkout("main");
    sb.commit_file("f.txt", "ours\n", "ours");
    sb
}

fn select_branch(t: &mut TestDriver, name: &str) {
    t.press("3").press("g");
    let idx = t.app().branches().visible().position(|b| b.name == name).unwrap();
    for _ in 0..idx {
        t.press("j");
    }
}

#[test]
fn merge_conflict_resolve_and_continue() {
    let sb = conflicting();
    let mut t = TestDriver::new(&sb);
    select_branch(&mut t, "other");
    t.press("M");
    assert!(t.screen().contains("Merge other into the current branch?"));
    t.press("y");
    let err = t.app().error().expect("conflict reported").to_string();
    assert!(err.contains("CONFLICT"), "{err}");
    t.press("esc");
    assert!(t.pane_title(PaneId::Status).contains("MERGING"));
    t.assert_pane_contains(PaneId::Status, "MERGING: stage fixes · C continue · X abort");
    t.assert_pane_contains(PaneId::Status, "UU f.txt");

    // Continue is refused while files are still conflicted.
    t.press("2").press("C");
    assert!(t.app().notice().unwrap_or_default().contains("resolve and stage"));

    sb.write("f.txt", "resolved\n");
    t.press("ctrl+r").press("g").press("space");
    t.press("C");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert!(!t.pane_title(PaneId::Status).contains("MERGING"));
    let head = t.app().graph().selected().unwrap();
    assert_eq!(head.parents.len(), 2, "merge commit created and selected");
}

#[test]
fn abort_rebase_restores_branch() {
    let sb = conflicting();
    let before = sb.rev_parse("HEAD");
    let mut t = TestDriver::new(&sb);
    select_branch(&mut t, "other");
    t.press("r").press("y").press("esc");
    assert!(t.pane_title(PaneId::Status).contains("REBASING"));
    t.press("2").press("X");
    assert!(t.screen().contains("Abort the rebase in progress"));
    t.press("y");
    assert!(!t.pane_title(PaneId::Status).contains("REBASING"));
    assert_eq!(sb.rev_parse("HEAD"), before);
}

#[test]
fn cherry_pick_from_graph() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("topic");
    sb.commit("pick me");
    sb.checkout("main");
    sb.commit("main work");
    let mut t = TestDriver::new(&sb);
    let idx = t
        .app()
        .graph()
        .commits()
        .iter()
        .position(|c| &*c.summary == "pick me")
        .unwrap();
    for _ in 0..idx {
        t.press("j");
    }
    t.press("C");
    assert!(t.screen().contains("\"pick me\" onto the current branch?"));
    t.press("y");
    assert_eq!(sb.git(&["log", "-1", "--format=%s"]), "pick me");
    assert_eq!(&*t.app().graph().selected().unwrap().summary, "pick me");
}

#[test]
fn continue_without_operation_explains() {
    let sb = Sandbox::new();
    sb.commit("base");
    let mut t = TestDriver::new(&sb);
    t.press("2").press("C");
    assert!(t.app().notice().unwrap_or_default().contains("no merge, rebase"));
}

/// Moves the graph selection to the commit whose summary is `summary`.
fn select_commit(t: &mut TestDriver, summary: &str) {
    t.press("1").press("g");
    let idx = t
        .app()
        .graph()
        .commits()
        .iter()
        .position(|c| &*c.summary == summary)
        .unwrap();
    for _ in 0..idx {
        t.press("j");
    }
}

#[test]
fn cherry_pick_a_range_from_graph() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("topic");
    sb.commit("one");
    sb.commit("two");
    sb.commit("three");
    sb.checkout("main");
    sb.commit("main work");
    let mut t = TestDriver::new(&sb);
    select_commit(&mut t, "three");
    t.press("v").press("j");
    assert!(
        t.pane_title(PaneId::Graph).contains("· 2 selected"),
        "{}",
        t.pane_title(PaneId::Graph)
    );
    t.press("j");
    assert!(t.pane_title(PaneId::Graph).contains("· 3 selected"));
    // Every row in the range is highlighted, the rest aren't.
    t.render();
    let bg = |t: &TestDriver, text: &str| {
        let (x, y) = t.find_text(text).unwrap();
        t.buffer()[(x, y)].bg
    };
    let highlight = bg(&t, "one");
    assert_eq!(bg(&t, "two"), highlight);
    assert_eq!(bg(&t, "three"), highlight);
    assert_ne!(bg(&t, "main work"), highlight);
    t.press("C");
    assert!(t.screen().contains("Apply 3 commits,"), "{}", t.screen());
    t.press("y");
    assert_eq!(sb.git(&["log", "-4", "--format=%s"]), "three\ntwo\none\nmain work");
    assert!(!t.app().graph().has_range(), "the range ends once applied");
    assert!(!t.pane_title(PaneId::Graph).contains("selected"));
}

#[test]
fn range_selection_ends_with_v_or_esc() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.commit("one");
    sb.commit("two");
    let mut t = TestDriver::new(&sb);
    t.press("1").press("v").press("j");
    assert_eq!(t.app().graph().selected_range(), Some((0, 1)));
    t.press("v");
    assert!(!t.app().graph().has_range());

    t.press("v").press("k");
    assert_eq!(t.app().graph().selected_range(), Some((0, 1)));
    // A cancelled cherry-pick keeps the range.
    t.press("C").press("n");
    assert!(t.app().graph().has_range());
    t.press("esc");
    assert!(!t.app().graph().has_range());

    // The range survives a reload.
    t.press("v").press("j").press("ctrl+r").wait_idle();
    assert_eq!(t.app().graph().selected_range(), Some((0, 1)));
}

#[test]
fn cherry_pick_range_refuses_merge_commits() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("topic");
    sb.commit("topic work");
    sb.checkout("main");
    sb.commit("main work");
    sb.merge("topic");
    let before = sb.rev_parse("HEAD");
    let mut t = TestDriver::new(&sb);
    t.press("1").press("v").press("j").press("C");
    let notice = t.app().notice().unwrap_or_default().to_string();
    assert!(notice.contains("can't apply merges"), "{notice}");
    assert_eq!(sb.rev_parse("HEAD"), before);
}

#[test]
fn cherry_pick_range_continues_after_a_conflict() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "base\n", "base");
    sb.checkout_new("topic");
    sb.commit_file("f.txt", "theirs\n", "one");
    sb.commit("two");
    sb.checkout("main");
    sb.commit_file("f.txt", "ours\n", "ours");
    let mut t = TestDriver::new(&sb);
    select_commit(&mut t, "two");
    t.press("v").press("j").press("C").press("y");
    assert!(t.app().error().is_some(), "conflict reported");
    t.press("esc");
    assert!(t.pane_title(PaneId::Status).contains("CHERRY-PICKING"));

    sb.write("f.txt", "resolved\n");
    t.press("2").press("ctrl+r").press("g").press("space");
    t.press("C");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert!(!t.pane_title(PaneId::Status).contains("CHERRY-PICKING"));
    assert_eq!(sb.git(&["log", "-3", "--format=%s"]), "two\none\nours");
}
