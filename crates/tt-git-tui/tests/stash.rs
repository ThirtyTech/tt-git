//! Stashing from the status pane and the branches pane's Stash tab.

use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::app::Zoom;
use tt_git_tui::keymap::Context;
use tt_git_tui::panes::{BranchTab, DetailsSubject, PaneId};

fn read(sb: &Sandbox, path: &str) -> Option<String> {
    std::fs::read_to_string(sb.path().join(path)).ok()
}

fn stash_messages(t: &TestDriver) -> Vec<String> {
    t.app()
        .branches()
        .visible_stashes()
        .map(|s| s.message.clone())
        .collect()
}

/// Focuses the branches pane on its Stash tab.
fn stash_tab(t: &mut TestDriver) {
    t.press("3");
    while t.app().branches().tab() != BranchTab::Stash {
        t.press("]");
    }
}

#[test]
fn s_stashes_everything_with_a_message() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "1\n", "base");
    sb.write("f.txt", "2\n").write("new.txt", "new\n");
    let mut t = TestDriver::new(&sb);
    t.press("2").press("s");
    assert!(t.screen().contains("Everything"), "{}", t.screen());
    t.type_text("wip").press("enter").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert!(t.app().status().entries().is_empty());
    assert_eq!(read(&sb, "new.txt"), None);

    stash_tab(&mut t);
    assert_eq!(t.app().focus(), PaneId::Branches);
    let messages = stash_messages(&t);
    assert_eq!(messages.len(), 1);
    assert!(messages[0].ends_with(": wip"), "{messages:?}");
    t.assert_pane_contains(PaneId::Branches, "stash@{0}");
    assert!(t.pane_title(PaneId::Branches).contains("Stash 1"));
}

#[test]
fn tab_picks_what_to_stash() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "a\n", "base");
    sb.commit_file("b.txt", "b\n", "more");
    sb.write("a.txt", "A\n").write("b.txt", "B\n");
    sb.git(&["add", "a.txt"]);
    let mut t = TestDriver::new(&sb);

    // Staged only, with no message.
    t.press("2").press("s").press("tab");
    assert!(t.screen().contains("only what's staged"), "{}", t.screen());
    t.press("enter").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(read(&sb, "a.txt").as_deref(), Some("a\n"));
    assert_eq!(read(&sb, "b.txt").as_deref(), Some("B\n"));

    // Just the selected file (b.txt is all that's left).
    t.press("s").press("shift+tab");
    assert!(t.screen().contains("only b.txt"), "{}", t.screen());
    t.press("enter").wait_idle();
    assert_eq!(read(&sb, "b.txt").as_deref(), Some("b\n"));
    assert_eq!(sb.git(&["stash", "list"]).lines().count(), 2);
}

#[test]
fn unstaged_leaves_the_staged_changes() {
    let sb = Sandbox::new();
    sb.commit_file(
        "a.txt", "a
", "base",
    );
    sb.commit_file(
        "b.txt", "b
", "more",
    );
    sb.write(
        "a.txt", "A
",
    )
    .write(
        "b.txt", "B
",
    )
    .write(
        "new.txt", "new
",
    );
    sb.git(&["add", "a.txt"]);
    let mut t = TestDriver::new(&sb);

    t.press("2").press("s").press("tab").press("tab");
    let screen = t.screen();
    assert!(
        screen.contains("Everything · Staged · Unstaged · Keep staged"),
        "{screen}"
    );
    assert!(
        screen.contains("unstaged and untracked changes; staged ones stay"),
        "{screen}"
    );
    t.type_text("rest").press("enter").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.git(&["status", "--short"]), "M  a.txt");
    assert_eq!(
        read(&sb, "b.txt").as_deref(),
        Some(
            "b
"
        )
    );
    assert_eq!(read(&sb, "new.txt"), None);
    stash_tab(&mut t);
    let messages = stash_messages(&t);
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(messages[0].ends_with(": rest"), "{messages:?}");
}

#[test]
fn the_selected_file_can_be_a_staged_rename() {
    let sb = Sandbox::new();
    sb.commit_file(
        "old.txt", "o
", "base",
    );
    sb.git(&["mv", "old.txt", "new.txt"]);
    let mut t = TestDriver::new(&sb);
    t.press("2").press("s").press("shift+tab");
    assert!(t.screen().contains("only new.txt, old.txt"), "{}", t.screen());
    t.press("enter").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert!(t.app().status().entries().is_empty());
    assert_eq!(sb.git(&["stash", "list"]).lines().count(), 1);
}

#[test]
fn nothing_to_stash() {
    let sb = Sandbox::new();
    sb.commit("base");
    let mut t = TestDriver::new(&sb);
    t.press("2").press("s");
    assert!(t.app().modal().is_none());
    assert_eq!(t.app().notice(), Some("nothing to stash"));
}

/// Two stashes: `stash@{1}` changes f.txt, `stash@{0}` adds new.txt.
fn two_stashes() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "1\n", "base");
    sb.write("f.txt", "2\n");
    sb.git(&["stash", "push", "-m", "older"]);
    sb.write("new.txt", "new\n");
    sb.git(&["stash", "push", "--include-untracked", "-m", "newer"]);
    sb
}

#[test]
fn stash_tab_has_its_own_keys() {
    let sb = two_stashes();
    let mut t = TestDriver::new(&sb);
    stash_tab(&mut t);
    assert_eq!(t.app().keymap_context(), Context::Stash);
    let help = t.screen();
    assert!(
        help.contains("pop") && help.contains("apply") && help.contains("drop"),
        "{help}"
    );
}

#[test]
fn pop_apply_and_drop() {
    let sb = two_stashes();
    let mut t = TestDriver::new(&sb);
    stash_tab(&mut t);

    // Apply the older one: it stays in the list.
    t.press("j").press("a").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(read(&sb, "f.txt").as_deref(), Some("2\n"));
    assert_eq!(stash_messages(&t).len(), 2);

    // Pop the newer one: it goes.
    t.press("g").press("space").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(read(&sb, "new.txt").as_deref(), Some("new\n"));
    let left = stash_messages(&t);
    assert_eq!(left.len(), 1);
    assert!(left[0].ends_with(": older"), "{left:?}");

    t.press("d");
    assert!(t.screen().contains("Drop stash@{0}"), "{}", t.screen());
    t.press("y").wait_idle();
    assert!(stash_messages(&t).is_empty());
    t.assert_pane_contains(PaneId::Branches, "no stashes");
}

#[test]
fn branch_from_a_stash() {
    let sb = two_stashes();
    let mut t = TestDriver::new(&sb);
    stash_tab(&mut t);
    t.press("j").press("n");
    assert!(t.screen().contains("New branch from stash@{1}"));
    t.type_text("rescued").press("enter").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.git(&["branch", "--show-current"]), "rescued");
    assert_eq!(read(&sb, "f.txt").as_deref(), Some("2\n"));
}

#[test]
fn enter_shows_a_stash_fullscreen() {
    let sb = two_stashes();
    let mut t = TestDriver::new(&sb);
    stash_tab(&mut t);
    t.press("enter").wait_idle();
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Details));
    let oid = t.app().branches().selected_stash().unwrap().oid.clone();
    assert_eq!(t.app().details().subject(), Some(&DetailsSubject::Stash(oid)));
    t.assert_pane_contains(PaneId::Details, "stash@{0}  On ");
    t.assert_pane_contains(PaneId::Details, "── untracked ──");
    t.assert_pane_contains(PaneId::Details, "+new");

    t.press("esc");
    assert_eq!(t.app().focus(), PaneId::Branches);
    assert_eq!(t.app().zoom(), Zoom::Off);
}

#[test]
fn brackets_step_through_stashes_in_details() {
    let sb = two_stashes();
    let mut t = TestDriver::new(&sb);
    stash_tab(&mut t);
    t.press("enter").wait_idle();
    let oids: Vec<_> = t.app().branches().visible_stashes().map(|s| s.oid.clone()).collect();

    t.press("]").wait_idle();
    assert_eq!(
        t.app().details().subject(),
        Some(&DetailsSubject::Stash(oids[1].clone()))
    );
    assert_eq!(t.app().branches().selected_stash().unwrap().oid, oids[1]);
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Details));
    t.assert_pane_contains(PaneId::Details, "stash@{1}  On ");
    t.press("]").wait_idle();
    assert_eq!(
        t.app().details().subject(),
        Some(&DetailsSubject::Stash(oids[1].clone()))
    );
    t.press("[").wait_idle();
    assert_eq!(
        t.app().details().subject(),
        Some(&DetailsSubject::Stash(oids[0].clone()))
    );
}
