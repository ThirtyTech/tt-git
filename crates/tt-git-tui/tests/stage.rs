//! Staging a file by hunk or line from the fullscreen staging view.

use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::app::Zoom;
use tt_git_tui::panes::{DetailsSubject, PaneId, StageSideId};

const BASE: &str = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n";

/// `text` with line `n` (from 1) replaced.
fn with_line(text: &str, n: usize, new: &str) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    lines[n - 1] = new;
    lines.join("\n") + "\n"
}

/// f.txt with two hunks of unstaged changes: lines 2 and 14.
fn two_hunks() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", BASE, "base");
    sb.write("f.txt", &with_line(&with_line(BASE, 2, "two"), 14, "fourteen"));
    sb
}

fn index(sb: &Sandbox, path: &str) -> String {
    sb.git(&["show", &format!(":{path}")])
}

fn open(t: &mut TestDriver, path: &str) {
    t.press("2");
    let idx = t
        .app()
        .status()
        .entries()
        .iter()
        .position(|e| e.path == path)
        .unwrap_or_else(|| panic!("{path} not in status"));
    t.press("g");
    for _ in 0..idx {
        t.press("j");
    }
    t.press("enter");
    t.wait_idle();
}

fn side(t: &TestDriver) -> StageSideId {
    t.app().details().stage().expect("staging view open").side()
}

#[test]
fn enter_opens_the_file_fullscreen() {
    let sb = two_hunks();
    let mut t = TestDriver::new(&sb);
    open(&mut t, "f.txt");
    assert_eq!(
        t.app().details().subject(),
        Some(&DetailsSubject::Stage("f.txt".into()))
    );
    assert_eq!(t.app().focus(), PaneId::Details);
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Details));
    assert!(t.pane_title(PaneId::Details).contains("Stage f.txt · hunks"));
    t.assert_pane_contains(PaneId::Details, "Unstaged  2 hunks");
    t.assert_pane_contains(PaneId::Details, "Staged  no changes");
    t.assert_pane_contains(PaneId::Details, "+fourteen");
    assert!(t.screen().contains("stage/unstage"), "{}", t.screen());

    t.press("esc");
    assert_eq!(t.app().focus(), PaneId::Status);
    assert_eq!(t.app().zoom(), Zoom::Off);
    assert!(t.app().details().stage().is_none());
}

#[test]
fn stage_a_hunk_then_unstage_it() {
    let sb = two_hunks();
    let mut t = TestDriver::new(&sb);
    open(&mut t, "f.txt");
    t.press("j").press("space").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(index(&sb, "f.txt"), with_line(BASE, 14, "fourteen").trim_end());
    t.assert_pane_contains(PaneId::Details, "Unstaged  1 hunk");
    t.assert_pane_contains(PaneId::Details, "Staged  1 hunk");
    // The working tree is untouched and the view stays on the unstaged side.
    assert_eq!(side(&t), StageSideId::Unstaged);

    t.press("tab");
    assert_eq!(side(&t), StageSideId::Staged);
    t.press("space").wait_idle();
    assert_eq!(index(&sb, "f.txt"), BASE.trim_end());
    // Nothing left staged: back on the unstaged side.
    assert_eq!(side(&t), StageSideId::Unstaged);
    t.assert_pane_contains(PaneId::Details, "Unstaged  2 hunks");
}

#[test]
fn staging_the_last_hunk_moves_to_the_staged_side() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "a\n", "base");
    sb.write("f.txt", "b\n");
    let mut t = TestDriver::new(&sb);
    open(&mut t, "f.txt");
    t.press("space").wait_idle();
    assert_eq!(index(&sb, "f.txt"), "b");
    assert_eq!(side(&t), StageSideId::Staged);

    // Committing from here leaves nothing to stage: back to status.
    t.press("c");
    assert!(t.screen().contains("Commit message"));
    t.type_text("Stage by hunk").press("enter").wait_idle();
    assert_eq!(sb.git(&["log", "-1", "--format=%s"]), "Stage by hunk");
    assert_eq!(t.app().focus(), PaneId::Status);
    assert!(t.app().details().stage().is_none());
}

#[test]
fn line_mode_stages_one_line_or_a_range() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "a\nb\n", "base");
    sb.write("f.txt", "a\nx\ny\nz\nb\n");
    let mut t = TestDriver::new(&sb);
    open(&mut t, "f.txt");
    t.press("a");
    assert!(t.app().details().stage().unwrap().line_mode());
    assert!(t.pane_title(PaneId::Details).contains("· lines"));

    // The second added line on its own.
    t.press("j").press("space").wait_idle();
    assert_eq!(index(&sb, "f.txt"), "a\ny\nb");

    // Then the remaining two as a range.
    t.press("g").press("v").press("j").press("space").wait_idle();
    assert_eq!(index(&sb, "f.txt"), "a\nx\ny\nz\nb");
}

#[test]
fn discard_a_hunk_after_confirming() {
    let sb = two_hunks();
    let mut t = TestDriver::new(&sb);
    open(&mut t, "f.txt");
    t.press("d");
    assert!(t.screen().contains("Discard the selected hunk of f.txt?"));
    t.press("n");
    t.press("d").press("y").wait_idle();
    assert_eq!(
        std::fs::read_to_string(sb.path().join("f.txt")).unwrap(),
        with_line(BASE, 14, "fourteen")
    );
    t.assert_pane_contains(PaneId::Details, "Unstaged  1 hunk");
}

#[test]
fn part_of_an_untracked_file() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.write("new.txt", "one\ntwo\n");
    let mut t = TestDriver::new(&sb);
    open(&mut t, "new.txt");
    t.press("a").press("space").wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(index(&sb, "new.txt"), "one");
}

#[test]
fn sides_sit_side_by_side_when_wide() {
    let sb = two_hunks();
    sb.git(&["add", "f.txt"]);
    sb.write("f.txt", &with_line(&with_line(BASE, 2, "TWO"), 14, "fourteen"));
    let mut t = TestDriver::with_size(Config::default(), &sb, 150, 20);
    open(&mut t, "f.txt");
    insta::assert_snapshot!(t.pane_lines(PaneId::Details).join("\n"));
}
