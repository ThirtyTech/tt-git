//! Checking out the commit selected in the graph.

use tt_git_testkit::{Sandbox, TestDriver};

fn select_commit(t: &mut TestDriver, summary: &str) {
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

fn read(sb: &Sandbox, path: &str) -> Option<String> {
    std::fs::read_to_string(sb.path().join(path)).ok()
}

#[test]
fn checks_out_a_commit_detached() {
    let sb = Sandbox::new();
    let base = sb.commit("base");
    sb.commit("next");
    let mut t = TestDriver::new(&sb);
    select_commit(&mut t, "base");
    t.press("o").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.rev_parse("HEAD"), base);
    assert_eq!(sb.current_branch(), "");
    assert_eq!(&*t.app().graph().selected().unwrap().summary, "base");
    assert!(t.screen().contains(" HEAD "), "{}", t.screen());
}

#[test]
fn prefers_the_local_branch_on_the_commit() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("topic");
    sb.commit("topic work");
    sb.checkout("main");
    sb.commit("main work");
    let mut t = TestDriver::new(&sb);
    select_commit(&mut t, "topic work");
    t.press("o").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.current_branch(), "topic");
}

#[test]
fn head_commit_is_already_checked_out() {
    let sb = Sandbox::new();
    sb.commit("base");
    let mut t = TestDriver::new(&sb);
    t.press("o").wait_idle();
    assert_eq!(t.app().notice(), Some("already on main"));
    assert!(t.app().modal().is_none());
}

#[test]
fn uncommitted_changes_ask_to_stash_first() {
    let sb = Sandbox::new();
    let base = sb.commit_file("f.txt", "1\n", "base");
    sb.commit_file("f.txt", "2\n", "next");
    sb.write("f.txt", "local\n").write("new.txt", "new\n");
    let mut t = TestDriver::new(&sb);
    select_commit(&mut t, "base");

    t.press("o");
    let screen = t.screen();
    assert!(screen.contains("2 files have uncommitted changes."), "{screen}");
    assert!(screen.contains("Stash them and check out"), "{screen}");
    t.press("n").wait_idle();
    assert_eq!(sb.current_branch(), "main");
    assert_eq!(read(&sb, "f.txt").as_deref(), Some("local\n"));

    t.press("o").press("y").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.rev_parse("HEAD"), base);
    assert_eq!(read(&sb, "f.txt").as_deref(), Some("1\n"));
    assert_eq!(read(&sb, "new.txt"), None);
    assert!(t.app().status().entries().is_empty());
    assert_eq!(sb.git(&["stash", "list"]).lines().count(), 1);
}
