//! Undo with z.

use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::modal::Modal;

fn read(sb: &Sandbox, path: &str) -> Option<String> {
    std::fs::read_to_string(sb.path().join(path)).ok()
}

/// Paths the status pane lists.
fn status_paths(t: &TestDriver) -> Vec<String> {
    t.app().status().entries().iter().map(|e| e.path.clone()).collect()
}

/// Brings in the commit `change` makes on a branch as a fast-forward merge
/// into main: a step undone with `reset --keep`, unlike your own commits.
fn merged(sb: &Sandbox, change: impl FnOnce(&Sandbox)) {
    sb.checkout_new("incoming");
    change(sb);
    sb.checkout("main");
    sb.git(&["merge", "-q", "--ff-only", "incoming"]);
}

#[test]
fn z_undoes_a_commit() {
    let sb = Sandbox::new();
    let one = sb.commit("one");
    let two = sb.commit("two");
    sb.commit("three");
    let mut t = TestDriver::new(&sb);

    t.press("z").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(t.app().notice(), Some("✓ undo commit: three"));
    assert_eq!(sb.rev_parse("HEAD"), two);
    assert_eq!(&*t.app().graph().selected().unwrap().summary, "two");
    assert!(t.app().modal().is_none());
    // The commit's file is back in the status pane, staged.
    assert_eq!(status_paths(&t), ["three.txt"]);
    assert!(t.app().status().entries()[0].has_staged());

    // From any pane, stepping further back.
    t.press("2").press("z").wait_idle();
    assert_eq!(t.app().notice(), Some("✓ undo commit: two"));
    assert_eq!(sb.rev_parse("HEAD"), one);
    assert_eq!(status_paths(&t), ["three.txt", "two.txt"]);
}

#[test]
fn undoing_a_commit_never_asks() {
    let sb = Sandbox::new();
    let one = sb.commit("one");
    sb.commit_file("a.txt", "mine\n", "two");
    sb.write("wip.txt", "wip\n");
    let mut t = TestDriver::new(&sb);

    t.press("z").wait_idle();
    assert!(t.app().modal().is_none(), "{}", t.screen());
    assert_eq!(sb.rev_parse("HEAD"), one);
    assert_eq!(status_paths(&t), ["a.txt", "wip.txt"]);
    assert_eq!(read(&sb, "a.txt").as_deref(), Some("mine\n"));
    assert_eq!(sb.git(&["stash", "list"]), "");
}

#[test]
fn undoes_a_checkout() {
    let sb = Sandbox::new();
    sb.commit("one");
    sb.checkout_new("feat");
    let mut t = TestDriver::new(&sb);
    t.press("z").wait_idle();
    assert_eq!(t.app().notice(), Some("✓ undo checkout of feat"));
    assert_eq!(sb.current_branch(), "main");
}

#[test]
fn nothing_to_undo_before_the_first_commit() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.press("z").wait_idle();
    assert_eq!(t.app().notice(), Some("Nothing to undo: there are no commits yet"));
    assert!(t.app().error().is_none());

    sb.commit("first");
    t.press("ctrl+r").wait_idle();
    t.press("z").wait_idle();
    assert_eq!(t.app().error(), Some("Can't undo the repository's first commit"));
}

#[test]
fn uncommitted_changes_ask_first_and_come_back() {
    let sb = Sandbox::new();
    let one = sb.commit_file("a.txt", "1\n", "one");
    merged(&sb, |sb| {
        sb.commit_file("b.txt", "1\n", "two");
    });
    sb.write("a.txt", "local\n").write("new.txt", "new\n");
    let mut t = TestDriver::new(&sb);

    t.press("z").wait_idle();
    let Some(Modal::Confirm { message, .. }) = t.app().modal() else {
        panic!("no confirm dialog: {}", t.screen());
    };
    assert_eq!(
        *message,
        format!(
            "2 files have uncommitted changes.\n\
             Stash them, undo merge incoming: Fast-forward (reset to {}), and put them back?\n\n\
             If they don't go back cleanly, they stay on the branches pane's Stash tab.",
            &one[..7]
        )
    );
    let screen = t.screen();
    assert!(screen.contains("2 files have uncommitted changes."), "{screen}");
    t.press("n").wait_idle();
    assert_eq!(sb.git(&["log", "-1", "--format=%s"]), "two");

    t.press("z").wait_idle().press("y").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.git(&["log", "-1", "--format=%s"]), "one");
    assert_eq!(read(&sb, "a.txt").as_deref(), Some("local\n"));
    assert_eq!(read(&sb, "new.txt").as_deref(), Some("new\n"));
    assert_eq!(read(&sb, "b.txt"), None);
    assert_eq!(sb.git(&["stash", "list"]), "");
}

#[test]
fn conflicting_changes_stay_stashed() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "1\n", "one");
    merged(&sb, |sb| {
        sb.commit_file("a.txt", "2\n", "two");
    });
    sb.write("a.txt", "3\n");
    let mut t = TestDriver::new(&sb);

    t.press("z").wait_idle().press("y").wait_idle();
    let error = t.app().error().expect("an error popup");
    assert!(
        error.starts_with("Putting your changes back didn't go cleanly"),
        "{error}"
    );
    assert_eq!(sb.git(&["log", "-1", "--format=%s"]), "one");
    assert_eq!(sb.git(&["stash", "list"]).lines().count(), 1);
}

#[test]
fn refuses_during_a_merge() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "base\n", "base");
    sb.checkout_new("feat");
    sb.commit_file("a.txt", "feat\n", "feat");
    sb.checkout("main");
    sb.commit_file("a.txt", "main\n", "main");
    let mut cmd = std::process::Command::new("git");
    cmd.args(["merge", "-q", "feat"]).current_dir(sb.path());
    sb.apply_env(&mut cmd);
    assert!(!cmd.status().unwrap().success());

    let mut t = TestDriver::new(&sb);
    t.press("z").wait_idle();
    let error = t.app().error().expect("an error popup");
    assert!(error.starts_with("A merge is in progress"), "{error}");
    assert_eq!(sb.git(&["log", "-1", "--format=%s"]), "main");
}
