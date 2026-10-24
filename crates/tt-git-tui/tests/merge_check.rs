//! The branches pane marks branches and worktrees whose merge into the base
//! branch would conflict.

use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::panes::{BranchTab, PaneId};
use tt_git_tui::{Action, Config};

/// main (the base) plus `clean`, which merges into it cleanly, and
/// `clash`, which conflicts with it in two files.
fn repo_with_conflicts() -> Sandbox {
    let sb = Sandbox::new();
    sb.write("f.txt", "a\nb\nc\n").write("sub dir/h.txt", "1\n");
    sb.git(&["add", "."]);
    sb.commit_staged("base");
    sb.branch("clean").branch("clash");
    sb.write("f.txt", "a\nmain\nc\n").write("sub dir/h.txt", "main\n");
    sb.git(&["add", "."]);
    sb.commit_staged("main work");
    sb.checkout("clean");
    sb.commit("clean work");
    sb.checkout("clash");
    sb.write("f.txt", "a\nclash\nc\n").write("sub dir/h.txt", "clash\n");
    sb.git(&["add", "."]);
    sb.commit_staged("clash work");
    sb.checkout("main");
    sb
}

fn row<'a>(lines: &'a [String], name: &str) -> &'a str {
    lines
        .iter()
        .find(|l| l.trim_start_matches(['*', ' ']).starts_with(name))
        .unwrap_or_else(|| panic!("no {name} row:\n{}", lines.join("\n")))
}

fn select(t: &mut TestDriver, name: &str) {
    let lines = t.pane_lines(PaneId::Branches);
    let pos = lines
        .iter()
        .position(|l| l.trim_start_matches(['*', ' ']).starts_with(name))
        .unwrap_or_else(|| panic!("no {name} row:\n{}", lines.join("\n")));
    t.press("g");
    for _ in 0..pos {
        t.press("j");
    }
}

/// The branches pane's bottom border.
fn bottom(t: &TestDriver) -> String {
    let area = t.app().pane_area(PaneId::Branches).unwrap();
    let y = area.bottom() - 1;
    (area.left()..area.right())
        .map(|x| t.buffer()[(x, y)].symbol().to_string())
        .collect()
}

#[test]
fn marks_branches_that_conflict_with_the_base() {
    let sb = repo_with_conflicts();
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 30);
    t.press("3");
    let lines = t.pane_lines(PaneId::Branches);
    assert!(row(&lines, "clash").contains("✗2"), "{}", lines.join("\n"));
    assert!(!row(&lines, "clean").contains('✗'), "{}", lines.join("\n"));
    assert!(!row(&lines, "main").contains('✗'), "{}", lines.join("\n"));
    insta::assert_snapshot!(lines[..3].join("\n"));

    select(&mut t, "clash");
    let border = bottom(&t);
    assert!(
        border.contains("✗ 2 conflicts with main: f.txt, sub dir/h.txt"),
        "{border}"
    );
    select(&mut t, "clean");
    assert!(!bottom(&t).contains('✗'), "{}", bottom(&t));

    // The Unmerged tab marks it too.
    t.press("]").press("]");
    assert_eq!(t.app().branches().tab(), BranchTab::Unmerged);
    let lines = t.pane_lines(PaneId::Branches);
    assert!(row(&lines, "clash").contains("✗2"), "{}", lines.join("\n"));
    assert!(!row(&lines, "clean").contains('✗'), "{}", lines.join("\n"));
}

#[test]
fn no_column_while_nothing_conflicts() {
    let sb = repo_with_conflicts();
    sb.git(&["branch", "-q", "-D", "clash"]);
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 30);
    t.press("3");
    let with = t.pane_lines(PaneId::Branches);
    let mut config = Config::default();
    config.branches.check_merges = false;
    let mut off = TestDriver::with_size(config, &sb, 120, 30);
    off.press("3");
    assert_eq!(with, off.pane_lines(PaneId::Branches));
}

#[test]
fn check_merges_off_marks_nothing() {
    let sb = repo_with_conflicts();
    let mut config = Config::default();
    config.branches.check_merges = false;
    let mut t = TestDriver::with_size(config, &sb, 120, 30);
    t.press("3");
    assert!(!t.pane_lines(PaneId::Branches).join("\n").contains('✗'));
}

#[test]
fn checks_again_when_the_base_moves() {
    let sb = repo_with_conflicts();
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 30);
    t.press("3");
    assert!(row(&t.pane_lines(PaneId::Branches), "clash").contains("✗2"));
    // main takes clash's side, so clash has nothing left to conflict with.
    sb.write("f.txt", "a\nclash\nc\n").write("sub dir/h.txt", "clash\n");
    sb.git(&["add", "."]);
    sb.commit_staged("take clash's side");
    t.send(Action::RepoChanged {
        refs: true,
        status: true,
    });
    t.wait_idle();
    let lines = t.pane_lines(PaneId::Branches);
    assert!(!lines.join("\n").contains('✗'), "{}", lines.join("\n"));
}

#[test]
fn worktree_state_shows_conflicts() {
    let sb = repo_with_conflicts();
    let path = sb.path().join(".worktrees/clash");
    sb.git(&["worktree", "add", "-q", path.to_str().unwrap(), "clash"]);
    std::fs::write(sb.path().join(".git/info/exclude"), "/.worktrees/\n").unwrap();
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 30);
    t.press("3").press("[");
    t.wait_idle();
    assert_eq!(t.app().branches().tab(), BranchTab::Worktrees);
    let lines = t.pane_lines(PaneId::Branches);
    let wt = lines.iter().find(|l| l.contains(".worktrees/clash")).unwrap();
    assert!(wt.contains("↑1 · ✗2 · clean"), "{}", lines.join("\n"));
    select_worktree(&mut t, ".worktrees/clash");
    assert!(bottom(&t).contains("✗ 2 conflicts with main"), "{}", bottom(&t));
}

fn select_worktree(t: &mut TestDriver, name: &str) {
    let lines = t.pane_lines(PaneId::Branches);
    let pos = lines.iter().position(|l| l.contains(name)).unwrap();
    t.press("g");
    for _ in 0..pos {
        t.press("j");
    }
}
