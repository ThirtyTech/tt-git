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
fn esc_from_staging_goes_back_to_the_status_column() {
    let sb = two_hunks();
    let mut t = TestDriver::new(&sb);
    t.press("2").press("+");
    assert_eq!(t.app().zoom(), Zoom::Status);
    open(&mut t, "f.txt");
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Details));
    t.press("esc");
    assert_eq!(t.app().zoom(), Zoom::Status);
    assert_eq!(t.app().focus(), PaneId::Status);

    // Unzoomed again, staging goes back to the normal layout.
    t.press("+");
    open(&mut t, "f.txt");
    t.press("esc");
    assert_eq!(t.app().zoom(), Zoom::Off);
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

#[test]
fn brackets_step_to_the_next_and_previous_file() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "a\n", "base");
    sb.commit_file("b.txt", "b\n", "more");
    sb.write("a.txt", "A\n").write("b.txt", "B\n");
    let mut t = TestDriver::with_size(Config::default(), &sb, 160, 30);
    open(&mut t, "a.txt");
    let staging = |t: &TestDriver| t.app().details().stage().map(|v| v.path().to_string());
    assert_eq!(staging(&t).as_deref(), Some("a.txt"));
    let help = t.screen().lines().last().unwrap().to_string();
    assert!(help.contains("[ prev file") && help.contains("] next file"), "{help}");

    t.press("]").wait_idle();
    assert_eq!(staging(&t).as_deref(), Some("b.txt"));
    assert_eq!(t.app().status().selected().unwrap().path, "b.txt");
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Details));
    t.assert_pane_contains(PaneId::Details, "+B");
    // At the last file, ] stays put.
    t.press("]").wait_idle();
    assert_eq!(staging(&t).as_deref(), Some("b.txt"));
    t.press("[").wait_idle();
    assert_eq!(staging(&t).as_deref(), Some("a.txt"));
    t.assert_pane_contains(PaneId::Details, "+A");
}

#[test]
fn brackets_step_through_files_in_a_file_diff() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "a\n", "base");
    sb.commit_file("b.txt", "b\n", "more");
    sb.write("a.txt", "A\n").write("b.txt", "B\n");
    let mut t = TestDriver::with_size(Config::default(), &sb, 160, 30);
    t.press("2").press("g").press("4").wait_idle();
    assert_eq!(t.app().details().subject(), Some(&DetailsSubject::File("a.txt".into())));
    let help = t.screen().lines().last().unwrap().to_string();
    assert!(help.contains("[ prev file") && help.contains("] next file"), "{help}");

    t.press("]").wait_idle();
    assert_eq!(t.app().details().subject(), Some(&DetailsSubject::File("b.txt".into())));
    assert_eq!(t.app().status().selected().unwrap().path, "b.txt");
    assert_eq!(t.app().focus(), PaneId::Details);
    assert!(t.app().details().lines().iter().any(|l| l == "+B"));
    t.press("[").wait_idle();
    assert_eq!(t.app().details().subject(), Some(&DetailsSubject::File("a.txt".into())));
}

#[test]
fn a_click_picks_a_hunk_or_a_line() {
    let sb = two_hunks();
    let mut t = TestDriver::new(&sb);
    open(&mut t, "f.txt");
    let selected = |t: &TestDriver| t.app().details().stage().unwrap().selected_hunk();
    assert_eq!(selected(&t), 0);
    // The second hunk's heading, wherever it was drawn.
    t.click_text("@@ -11,");
    assert_eq!(selected(&t), 1);
    assert_eq!(t.app().focus(), PaneId::Details);
    t.press("space");
    assert!(index(&sb, "f.txt").contains("fourteen"), "the clicked hunk was staged");
    assert!(!index(&sb, "f.txt").contains("two"));

    // In line mode, on a changed line: that line.
    t.press("a");
    t.click_text("+two");
    let stage = t.app().details().stage().unwrap();
    assert!(stage.line_mode());
    assert!(stage.picked((0, 2)) && !stage.picked((0, 1)), "the added line");
}
