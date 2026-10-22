//! The branches pane's Worktrees tab: listing, opening and removing.

use std::path::{Path, PathBuf};

use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::modal::Modal;
use tt_git_tui::panes::{BranchTab, PaneId};
use tt_git_tui::{Action, Config};

/// main, plus `.worktrees/side` (branch side, one commit main doesn't
/// have and one untracked file) and `.worktrees/review` (detached, locked).
fn repo_with_worktrees() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit("base");
    let side = add_worktree(&sb, "side", &["-b", "side"]);
    let side = side.to_str().unwrap();
    sb.git(&["-C", side, "commit", "-q", "--allow-empty", "-m", "side work"]);
    std::fs::write(sb.path().join(".worktrees/side/wip.txt"), "wip\n").unwrap();
    let review = add_worktree(&sb, "review", &["--detach"]);
    sb.git(&["worktree", "lock", "--reason", "reviewing", review.to_str().unwrap()]);
    sb
}

fn add_worktree(sb: &Sandbox, name: &str, how: &[&str]) -> PathBuf {
    let path = sb.path().join(".worktrees").join(name);
    let mut args = vec!["worktree", "add", "-q"];
    args.extend(how);
    args.extend([path.to_str().unwrap(), "main"]);
    sb.git(&args);
    // Like tt-git's own worktrees, out of the main one's status.
    std::fs::write(sb.path().join(".git/info/exclude"), "/.worktrees/\n").unwrap();
    path
}

/// Opens the branches pane on its Worktrees tab.
fn on_worktrees_tab(sb: &Sandbox) -> TestDriver {
    let mut t = TestDriver::new(sb);
    // From the first tab, back one wraps round to the last: Worktrees.
    t.press("3").press("[");
    t.wait_idle();
    assert_eq!(t.app().branches().tab(), BranchTab::Worktrees);
    t
}

fn select(t: &mut TestDriver, name: &str) {
    let lines = t.pane_lines(PaneId::Branches);
    let row = lines
        .iter()
        .position(|l| l.contains(name))
        .unwrap_or_else(|| panic!("no {name} row:\n{}", lines.join("\n")));
    t.press("g");
    for _ in 0..row {
        t.press("j");
    }
}

fn same(a: &Path, b: &Path) -> bool {
    std::fs::canonicalize(a).unwrap() == std::fs::canonicalize(b).unwrap()
}

#[test]
fn lists_every_worktree_with_its_state() {
    let sb = repo_with_worktrees();
    let mut t = on_worktrees_tab(&sb);
    assert!(
        t.pane_title(PaneId::Branches).contains("Worktrees 3"),
        "{}",
        t.pane_title(PaneId::Branches)
    );
    // Sizes depend on git's own files, so the real ones are swapped for fixed ones.
    let sizes = [1024 * 1024 + 512 * 1024, 2048, 512];
    let paths: Vec<PathBuf> = t.app().branches().worktrees().iter().map(|w| w.path.clone()).collect();
    for (path, size) in paths.into_iter().zip(sizes) {
        t.app_mut().update(Action::WorktreeSize(path, Ok(size)));
    }
    t.render();
    insta::assert_snapshot!(t.pane_lines(PaneId::Branches)[..3].join("\n"));
    assert!(t.screen().contains("total 1.5 MB"), "{}", t.screen());
    assert!(t.screen().contains("enter open"), "the help bar shows the tab's keys");
}

#[test]
fn measures_each_worktree_and_the_total() {
    let sb = repo_with_worktrees();
    std::fs::write(sb.path().join(".worktrees/side/big.bin"), vec![0u8; 3 * 1024 * 1024]).unwrap();
    let t = on_worktrees_tab(&sb);
    let lines = t.pane_lines(PaneId::Branches);
    let side = lines.iter().find(|l| l.contains(".worktrees/side")).unwrap();
    assert!(side.trim_end().ends_with("3.0 MB"), "{}", lines.join("\n"));
    // The main worktree's size leaves out the worktrees inside it, so the
    // total counts big.bin once.
    assert!(!lines[0].contains(" MB"), "{}", lines.join("\n"));
    assert!(t.screen().contains("total 3.0 MB"), "{}", t.screen());
}

#[test]
fn tab_shows_only_with_linked_worktrees() {
    let sb = Sandbox::new();
    sb.commit("base");
    let mut t = TestDriver::new(&sb);
    t.press("3");
    assert!(!t.pane_title(PaneId::Branches).contains("Worktrees"));
    add_worktree(&sb, "side", &["-b", "side"]);
    t.press("ctrl+r");
    t.wait_idle();
    assert!(t.pane_title(PaneId::Branches).contains("Worktrees 2"));
}

#[test]
fn enter_opens_another_worktree() {
    let sb = repo_with_worktrees();
    let mut t = on_worktrees_tab(&sb);
    t.press("enter");
    assert!(t.app().reopen().is_none(), "the current one isn't reopened");
    assert!(
        t.app()
            .notice()
            .unwrap_or_default()
            .contains("already in this worktree")
    );

    select(&mut t, ".worktrees/side");
    t.press("enter");
    let repo = t.app().reopen().expect("asked to open the worktree");
    assert!(same(repo.root(), &sb.path().join(".worktrees/side")));
    assert!(t.app().should_quit(), "quits so the caller can open it");
}

#[test]
fn remove_a_clean_worktree_after_asking() {
    let sb = repo_with_worktrees();
    let clean = add_worktree(&sb, "clean", &["-b", "clean"]);
    let mut t = on_worktrees_tab(&sb);
    t.press("ctrl+r");
    t.wait_idle();
    select(&mut t, ".worktrees/clean");
    t.press("d");
    let Some(Modal::Confirm { message, .. }) = t.app().modal() else {
        panic!("no confirmation: {:?}", t.app().modal());
    };
    assert!(message.contains("Remove worktree .worktrees/clean?"), "{message}");
    assert!(message.contains("Branch clean stays"), "{message}");
    t.press("y");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert!(!clean.exists());
    assert_eq!(sb.rev_parse("clean"), sb.rev_parse("main"), "the branch stays");
    assert_eq!(t.app().branches().worktrees().len(), 3);
    assert!(
        t.app()
            .notice()
            .unwrap_or_default()
            .contains("remove worktree .worktrees/clean")
    );
}

#[test]
fn removing_a_worktree_with_changes_asks_again_before_forcing() {
    let sb = repo_with_worktrees();
    let side = sb.path().join(".worktrees/side");
    let mut t = on_worktrees_tab(&sb);
    select(&mut t, ".worktrees/side");
    t.press("d").press("y");
    let Some(Modal::Confirm { title, message, .. }) = t.app().modal() else {
        panic!("no second confirmation: {:?} {:?}", t.app().modal(), t.app().error());
    };
    assert_eq!(title, "Uncommitted changes");
    assert!(message.contains("--force"), "{message}");
    assert!(side.join("wip.txt").is_file(), "nothing deleted yet");

    // No keeps it.
    t.press("n");
    assert!(side.join("wip.txt").is_file());

    t.press("d").press("y").press("y");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert!(!side.exists());
    assert!(
        t.app()
            .notice()
            .unwrap_or_default()
            .contains("force-remove worktree .worktrees/side")
    );
}

#[test]
fn refuses_to_remove_the_main_current_or_a_locked_worktree() {
    let sb = repo_with_worktrees();
    let mut t = on_worktrees_tab(&sb);
    t.press("d");
    assert!(t.app().modal().is_none());
    assert!(t.app().notice().unwrap_or_default().contains("main worktree"));

    select(&mut t, ".worktrees/review");
    t.press("d");
    assert!(t.app().modal().is_none());
    let notice = t.app().notice().unwrap_or_default().to_string();
    assert!(notice.contains("locked (reviewing)"), "{notice}");
    assert!(sb.path().join(".worktrees/review").exists());
}

#[test]
fn refuses_to_remove_the_worktree_tt_git_runs_in() {
    let sb = repo_with_worktrees();
    let side = sb.path().join(".worktrees/side");
    let mut t = TestDriver::at(Config::default(), &sb, &side, 120, 30);
    t.press("3").press("[");
    t.wait_idle();
    // Paths are given from the worktree tt-git runs in.
    let lines = t.pane_lines(PaneId::Branches);
    assert!(lines[0].starts_with("  ../.. "), "{}", lines.join("\n"));
    assert!(lines[1].starts_with("  ../review "), "{}", lines.join("\n"));
    assert!(lines[2].starts_with("* . "), "{}", lines.join("\n"));

    select(&mut t, "* . ");
    t.press("d");
    assert!(t.app().modal().is_none());
    let notice = t.app().notice().unwrap_or_default().to_string();
    assert!(notice.contains("running in this worktree"), "{notice}");
    assert!(side.exists());

    // Opening the main worktree from here works too.
    t.press("g").press("enter");
    let repo = t.app().reopen().expect("asked to open main");
    assert!(same(repo.root(), sb.path()));
}

#[test]
fn n_makes_a_new_worktree_from_the_selected_one() {
    let sb = repo_with_worktrees();
    let mut t = on_worktrees_tab(&sb);
    select(&mut t, ".worktrees/side");
    t.press("n");
    assert!(t.screen().contains("New branch from side"), "{}", t.screen());
    assert!(
        t.screen().contains("checked out in .worktrees/<name>"),
        "{}",
        t.screen()
    );
    t.type_text("next").press("enter");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert!(sb.path().join(".worktrees/next").is_dir());
    assert_eq!(t.app().branches().worktrees().len(), 4);
}

#[test]
fn filter_narrows_by_path() {
    let sb = repo_with_worktrees();
    let mut t = on_worktrees_tab(&sb);
    t.press("/").type_text("rev").press("enter");
    let shown: Vec<_> = t.app().branches().visible_worktrees().map(|w| w.path.clone()).collect();
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(same(&shown[0], &sb.path().join(".worktrees/review")));
}

#[test]
fn a_narrow_title_drops_the_other_tabs_counts() {
    let sb = repo_with_worktrees();
    let mut t = TestDriver::with_size(Config::default(), &sb, 110, 30);
    t.press("3").press("[");
    t.wait_idle();
    let title = t.pane_title(PaneId::Branches);
    assert!(
        title.contains("Branches · Tags · Unmerged · Merged · Worktrees 3"),
        "{title}"
    );
}

#[test]
fn c_copies_the_absolute_path() {
    let sb = repo_with_worktrees();
    let mut t = on_worktrees_tab(&sb);
    select(&mut t, ".worktrees/side");
    t.press("c");
    let copied = t.app_mut().take_clipboard().expect("copied");
    assert!(Path::new(&copied).is_absolute(), "{copied}");
    assert!(same(Path::new(&copied), &sb.path().join(".worktrees/side")), "{copied}");
    if cfg!(windows) {
        assert!(!copied.contains('/'), "Windows separators: {copied}");
    }
}

#[test]
fn measures_more_worktrees_than_run_at_once() {
    let sb = repo_with_worktrees();
    for i in 0..5 {
        add_worktree(&sb, &format!("extra{i}"), &["--detach"]);
    }
    let t = on_worktrees_tab(&sb);
    let lines = t.pane_lines(PaneId::Branches);
    assert_eq!(lines.iter().filter(|l| l.contains(".worktrees/")).count(), 7);
    assert!(!lines.iter().any(|l| l.ends_with('…')), "{}", lines.join("\n"));
    assert!(!t.screen().contains("total …"), "{}", t.screen());
}
