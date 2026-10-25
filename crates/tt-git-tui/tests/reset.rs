//! Resetting the current branch with X: soft, mixed or hard, and undoing it.

use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::modal::Modal;

fn read(sb: &Sandbox, path: &str) -> Option<String> {
    std::fs::read_to_string(sb.path().join(path)).ok()
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

fn select_branch(t: &mut TestDriver, name: &str) {
    t.press("3").press("g");
    let idx = t.app().branches().visible().position(|b| b.name == name).unwrap();
    for _ in 0..idx {
        t.press("j");
    }
}

/// `one` (a.txt), `two` (a.txt changed) and `three` (b.txt) on main.
fn three_commits(sb: &Sandbox) -> (String, String, String) {
    let one = sb.commit_file("a.txt", "1\n", "one");
    let two = sb.commit_file("a.txt", "2\n", "two");
    let three = sb.commit_file("b.txt", "3\n", "three");
    (one, two, three)
}

#[test]
fn x_asks_how_to_reset_starting_on_mixed() {
    let sb = Sandbox::new();
    let (one, _, three) = three_commits(&sb);
    let mut t = TestDriver::new(&sb);
    select_commit(&mut t, "one");

    t.press("X").wait_idle();
    insta::assert_snapshot!(t.screen());
    t.press("enter").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(
        t.app().notice(),
        Some(format!("✓ reset --mixed {}", &one[..7]).as_str())
    );
    assert_eq!(sb.rev_parse("HEAD"), one);
    // Mixed: the changes are left in the tree, unstaged.
    assert_eq!(sb.git(&["diff", "--name-only"]), "a.txt");
    assert_eq!(sb.git(&["status", "--porcelain", "--", "b.txt"]), "?? b.txt");
    assert_eq!(&*t.app().graph().selected().unwrap().summary, "one");

    // Undo puts the branch back with nothing left over, without asking.
    t.press("z").wait_idle();
    assert!(t.app().modal().is_none(), "{}", t.screen());
    assert_eq!(
        t.app().notice(),
        Some(format!("✓ undo reset --mixed {}", &one[..7]).as_str())
    );
    assert_eq!(sb.rev_parse("HEAD"), three);
    assert_eq!(sb.git(&["status", "--porcelain"]), "");
}

#[test]
fn soft_keeps_the_changes_staged() {
    let sb = Sandbox::new();
    let (_, two, three) = three_commits(&sb);
    let mut t = TestDriver::new(&sb);
    select_commit(&mut t, "two");

    t.press("X").wait_idle().type_text("soft").press("enter").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.rev_parse("HEAD"), two);
    assert_eq!(sb.git(&["diff", "--cached", "--name-only"]), "b.txt");

    t.press("z").wait_idle();
    assert_eq!(sb.rev_parse("HEAD"), three);
    assert_eq!(sb.git(&["status", "--porcelain"]), "");
}

#[test]
fn hard_asks_first_saying_what_is_lost() {
    let sb = Sandbox::new();
    let (one, _, three) = three_commits(&sb);
    sb.write("a.txt", "dirty\n").write("new.txt", "untracked\n");
    let mut t = TestDriver::new(&sb);
    select_commit(&mut t, "one");

    t.press("X").wait_idle().type_text("hard").press("enter");
    let Some(Modal::Confirm { message, .. }) = t.app().modal() else {
        panic!("no confirm dialog: {}", t.screen());
    };
    assert_eq!(
        *message,
        format!(
            "Reset main to {short} and make the files match it (git reset --hard)?\n\n\
             Uncommitted changes to 1 file are lost for good: undo (z) can't bring them back.\n\
             2 commits leave main. Undo (z) brings them back.\n\
             Untracked files are left alone.",
            short = &one[..7]
        )
    );
    insta::assert_snapshot!(t.screen());

    // `n` changes nothing.
    t.press("n").wait_idle();
    assert_eq!(sb.rev_parse("HEAD"), three);
    assert_eq!(read(&sb, "a.txt").as_deref(), Some("dirty\n"));

    t.press("X")
        .wait_idle()
        .type_text("hard")
        .press("enter")
        .press("y")
        .wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.rev_parse("HEAD"), one);
    assert_eq!(read(&sb, "a.txt").as_deref(), Some("1\n"));
    assert_eq!(read(&sb, "b.txt"), None);
    assert_eq!(read(&sb, "new.txt").as_deref(), Some("untracked\n"));

    // The commits come back; the uncommitted change doesn't. The
    // untracked file is stashed for the step and put back, as for any undo.
    t.press("z").wait_idle();
    assert!(matches!(t.app().modal(), Some(Modal::Confirm { .. })), "{}", t.screen());
    t.press("y").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.rev_parse("HEAD"), three);
    assert_eq!(read(&sb, "a.txt").as_deref(), Some("2\n"));
    assert_eq!(read(&sb, "b.txt").as_deref(), Some("3\n"));
}

#[test]
fn hard_with_a_clean_tree_still_asks() {
    let sb = Sandbox::new();
    let (_, _, three) = three_commits(&sb);
    let mut t = TestDriver::new(&sb);
    select_commit(&mut t, "two");
    t.press("X").wait_idle().type_text("hard").press("enter");
    let Some(Modal::Confirm { message, .. }) = t.app().modal() else {
        panic!("no confirm dialog: {}", t.screen());
    };
    assert!(
        message.contains("There are no uncommitted changes to lose."),
        "{message}"
    );
    assert!(
        message.contains("1 commit leaves main. Undo (z) brings it back."),
        "{message}"
    );
    assert_eq!(sb.rev_parse("HEAD"), three);
}

#[test]
fn x_on_a_branch_resets_to_it() {
    let sb = Sandbox::new();
    sb.commit("one");
    sb.checkout_new("feat");
    let feat = sb.commit("feat work");
    sb.checkout("main");
    sb.commit("main work");
    let mut t = TestDriver::new(&sb);

    select_branch(&mut t, "feat");
    t.press("X").wait_idle();
    let screen = t.screen();
    assert!(screen.contains("Reset main to feat"), "{screen}");
    t.type_text("hard").press("enter").press("y").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.rev_parse("main"), feat);
    assert_eq!(sb.current_branch(), "main");
}

#[test]
fn x_on_the_checked_out_branch_without_a_remote_refuses() {
    let sb = Sandbox::new();
    sb.commit("one");
    let mut t = TestDriver::new(&sb);
    select_branch(&mut t, "main");
    t.press("X").wait_idle();
    assert!(t.app().modal().is_none());
    assert_eq!(t.app().notice(), Some("that's the checked-out branch"));
}
