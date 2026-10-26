//! `v` and `V` preview the details pane's file with Quick Look, in a front
//! end that offers it (macOS): the version after the commit, or before.

use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::keymap::Command;
use tt_git_tui::panes::PaneId;
use tt_git_tui::quick_look;

/// The file Quick Look was asked to show, and what's in it.
fn previewed(t: &mut TestDriver) -> (std::path::PathBuf, String) {
    t.wait_idle();
    let file = t.app_mut().take_quick_look().expect("a file to preview");
    let text = std::fs::read_to_string(&file).unwrap();
    (file, text)
}

#[test]
fn only_a_front_end_that_previews_offers_it() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "a\n", "base");
    let mut t = TestDriver::with_size(Default::default(), &sb, 160, 30);
    t.press("1").press("enter").wait_idle();
    assert_eq!(t.app().focus(), PaneId::Details);
    let offered = |t: &TestDriver| t.app().pane_commands().iter().any(|c| c.command == Command::QuickLook);
    assert!(!offered(&t));
    assert!(!t.screen().contains("quick look"));
    t.press("v").wait_idle();
    assert!(t.app_mut().take_quick_look().is_none());

    t.app_mut().enable_quick_look();
    assert!(offered(&t));
    t.render();
    let screen = t.screen();
    assert!(screen.contains("quick look"), "{screen}");
}

#[test]
fn previews_a_commits_files_before_and_after() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "old\n", "base");
    sb.commit_file("gone.txt", "gone\n", "add gone");
    sb.write("a.txt", "new\n");
    sb.write("added.txt", "added\n");
    sb.git(&["add", "a.txt", "added.txt"]);
    sb.git(&["rm", "-q", "gone.txt"]);
    sb.commit_staged("change a, add added, remove gone");
    // Moved on in the working tree, so the copies must come from the commit.
    sb.write("a.txt", "newer\n");
    let mut t = TestDriver::new(&sb);
    t.app_mut().enable_quick_look();

    // The newest commit as files beside a diff: a.txt is first.
    t.press("1").press("enter").wait_idle();
    t.press("v");
    let (file, text) = previewed(&mut t);
    assert_eq!(text, "new\n");
    assert_eq!(file.file_name().unwrap(), "a.txt", "keeps its name for Quick Look");
    assert!(file.starts_with(quick_look::dir()), "{}", file.display());
    t.press("V");
    assert_eq!(previewed(&mut t).1, "old\n");

    // A file the commit added has no version before it.
    t.press("j").wait_idle().press("V").wait_idle();
    assert!(t.app_mut().take_quick_look().is_none());
    assert_eq!(t.app().notice(), Some("added.txt is new in this commit"));
    t.press("v");
    assert_eq!(previewed(&mut t).1, "added\n");

    // A deleted file shows as it was.
    t.press("j").wait_idle().press("v");
    assert_eq!(previewed(&mut t).1, "gone\n");

    // A changed file in the working tree is previewed in place; before is HEAD's.
    t.press("esc").press("2").wait_idle().press("4").wait_idle();
    t.press("v");
    assert_eq!(previewed(&mut t), (sb.path().join("a.txt"), "newer\n".into()));
    t.press("V");
    assert_eq!(previewed(&mut t).1, "new\n");

    // The copies go when the front end quits.
    quick_look::clean_up();
    assert!(!quick_look::dir().exists());
}
