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
fn revert_from_graph() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "one\n", "base");
    sb.commit_file("f.txt", "two\n", "change it");
    sb.commit("later");
    let before = sb.rev_parse("HEAD");
    let mut t = TestDriver::new(&sb);
    select_commit(&mut t, "change it");

    // `n` keeps everything as it was.
    t.press("R");
    let screen = t.screen();
    assert!(screen.contains("\"change it\"?"), "{screen}");
    t.press("n");
    assert_eq!(sb.rev_parse("HEAD"), before);

    t.press("R").press("y");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.git(&["log", "-1", "--format=%s"]), "Revert \"change it\"");
    assert_eq!(sb.rev_parse("HEAD~1"), before);
    assert_eq!(&*t.app().graph().selected().unwrap().summary, "Revert \"change it\"");
}

#[test]
fn revert_merge_asks_which_parent() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("topic");
    sb.commit("topic work");
    sb.checkout("main");
    sb.commit("main work");
    sb.merge("topic");
    let mut t = TestDriver::new(&sb);
    select_commit(&mut t, "Merge branch 'topic'");
    t.press("R");
    let screen = t.screen();
    assert!(screen.contains("keep which side?"), "{screen}");
    assert!(
        screen.contains("Keep parent 1:") && screen.contains("\"main work\""),
        "{screen}"
    );
    assert!(
        screen.contains("Keep parent 2:") && screen.contains("\"topic work\""),
        "{screen}"
    );
    t.press("enter");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert!(
        sb.git(&["log", "-1", "--format=%s"])
            .starts_with("Revert \"Merge branch 'topic'")
    );
    assert!(!sb.path().join("topic-work.txt").exists(), "topic's side undone");
    assert!(sb.path().join("main-work.txt").is_file(), "main's side kept");
}

#[test]
fn revert_conflict_continues_from_status() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "one\n", "base");
    sb.commit_file("f.txt", "two\n", "change it");
    sb.commit_file("f.txt", "three\n", "change it again");
    let before = sb.rev_parse("HEAD");
    let mut t = TestDriver::new(&sb);
    select_commit(&mut t, "change it");
    t.press("R").press("y");
    let err = t.app().error().expect("conflict reported").to_string();
    assert!(err.contains("could not revert"), "{err}");
    t.press("esc");
    assert!(t.pane_title(PaneId::Status).contains("REVERTING"));

    sb.write("f.txt", "one\n");
    t.press("2").press("ctrl+r").press("g").press("space");
    t.press("C");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert!(!t.pane_title(PaneId::Status).contains("REVERTING"));
    assert_eq!(sb.rev_parse("HEAD~1"), before);
    assert_eq!(sb.git(&["log", "-1", "--format=%s"]), "Revert \"change it\"");
}

#[test]
fn continue_without_operation_explains() {
    let sb = Sandbox::new();
    sb.commit("base");
    let mut t = TestDriver::new(&sb);
    t.press("2").press("C");
    assert!(t.app().notice().unwrap_or_default().contains("no merge, rebase"));
}
