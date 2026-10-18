use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::modal::Modal;
use tt_git_tui::panes::PaneId;

/// main (tracking origin/main) plus a local-only branch, plus a branch that
/// only exists on the remote (pushed by someone else).
fn repo_with_remote_branch() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("local-work");
    sb.commit("local work");
    sb.checkout("main");
    let remote = sb.add_remote("origin");
    sb.git(&["push", "-q", "origin", "--delete", "local-work"]);
    sb.git(&["fetch", "-q", "--prune", "origin"]);
    sb.git(&["branch", "-q", "--set-upstream-to=origin/main"]);
    let other = sb.clone_to(&remote, "other");
    sb.git_in(&other, &["switch", "-q", "-c", "their-feature"]);
    sb.git_in(&other, &["commit", "-q", "--allow-empty", "-m", "their work"]);
    sb.git_in(&other, &["push", "-q", "-u", "origin", "their-feature"]);
    sb.git(&["fetch", "-q", "origin"]);
    sb
}

fn names(t: &TestDriver) -> Vec<String> {
    t.app().branches().visible().map(|e| e.name.clone()).collect()
}

#[test]
fn unified_list_merges_local_and_remote() {
    let sb = repo_with_remote_branch();
    let t = TestDriver::with_size(Config::default(), &sb, 120, 30);
    let names = names(&t);
    assert_eq!(names.len(), 3, "{names:?}");
    assert!(names.contains(&"main".to_string()));
    assert!(names.contains(&"local-work".to_string()));
    assert!(names.contains(&"their-feature".to_string()));
    let lines = t.pane_lines(PaneId::Branches);
    let main = lines.iter().find(|l| l.contains("main")).unwrap();
    assert!(main.starts_with("* main") && main.contains('✓'), "{main}");
    let theirs = lines.iter().find(|l| l.contains("their-feature")).unwrap();
    assert!(theirs.contains("origin"), "{theirs}");
    let local = lines.iter().find(|l| l.contains("local-work")).unwrap();
    assert!(local.contains("local"), "{local}");
}

#[test]
fn checkout_remote_only_branch_creates_local_without_asking() {
    let sb = repo_with_remote_branch();
    let mut t = TestDriver::new(&sb);
    t.press("2");
    let idx = names(&t).iter().position(|n| n == "their-feature").unwrap();
    for _ in 0..idx {
        t.press("j");
    }
    t.press("enter");
    assert!(t.app().modal().is_none(), "no prompt for auto-tracking");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.current_branch(), "their-feature");
    assert_eq!(
        sb.git(&["rev-parse", "--abbrev-ref", "their-feature@{upstream}"]),
        "origin/their-feature"
    );

    // The list refreshed: it is now the checked-out, in-sync branch at the top.
    assert_eq!(names(&t)[0], "their-feature");
    let first = t.pane_lines(PaneId::Branches).remove(0);
    assert!(first.starts_with("* their-feature") && first.contains('✓'), "{first}");
    assert!(t.pane_title(PaneId::Status).contains("their-feature"));
    // The graph labels moved too.
    assert!(
        t.pane_lines(PaneId::Graph)
            .iter()
            .any(|l| l.contains(" their-feature "))
    );
}

#[test]
fn checkout_failure_shows_git_error() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "main\n", "base");
    sb.checkout_new("other");
    sb.commit_file("f.txt", "other\n", "change");
    sb.checkout("main");
    sb.write("f.txt", "dirty\n");
    let mut t = TestDriver::new(&sb);
    t.press("2");
    let idx = names(&t).iter().position(|n| n == "other").unwrap();
    for _ in 0..idx {
        t.press("j");
    }
    t.press("enter");
    let err = t.app().error().expect("error popup").to_string();
    assert!(
        err.contains("checkout other failed") && err.contains("overwritten"),
        "{err}"
    );
    assert!(t.screen().contains("Error"));
    t.press("esc");
    assert!(t.app().error().is_none());
    assert_eq!(sb.current_branch(), "main");
}

#[test]
fn filter_narrows_the_list() {
    let sb = Sandbox::new();
    sb.commit("base");
    for b in ["feature/login", "feature/logout", "bugfix/crash"] {
        sb.branch(b);
    }
    let mut t = TestDriver::new(&sb);
    t.press("2").press("/").type_text("lgo");
    assert_eq!(names(&t), vec!["feature/logout"]);
    assert!(t.pane_title(PaneId::Branches).contains("/lgo"));
    // Keys are text while filtering: "q" does not quit.
    t.press("backspace").press("backspace").type_text("q");
    assert!(!t.app().should_quit());
    t.press("backspace").press("enter");
    assert_eq!(names(&t).len(), 2);
    // Esc while editing clears the filter.
    t.press("/").press("esc");
    assert_eq!(names(&t).len(), 4);
}

#[test]
fn new_branch_from_selected_branch() {
    let sb = Sandbox::new();
    sb.commit("base");
    let mut t = TestDriver::new(&sb);
    t.press("2").press("n");
    assert!(matches!(t.app().modal(), Some(Modal::Prompt { .. })));
    assert!(t.screen().contains("New branch from main"));
    t.type_text("topic").press("enter");
    assert!(t.app().modal().is_none());
    assert_eq!(sb.current_branch(), "topic");
    assert_eq!(names(&t)[0], "topic");
}

#[test]
fn new_branch_at_graph_commit() {
    let sb = Sandbox::new();
    let first = sb.commit("first");
    sb.commit("second");
    let mut t = TestDriver::new(&sb);
    t.press("j").press("n").type_text("from-first").press("enter");
    assert_eq!(sb.current_branch(), "from-first");
    assert_eq!(sb.rev_parse("HEAD"), first);
}

#[test]
fn rename_branch() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.branch("old-name");
    let mut t = TestDriver::new(&sb);
    t.press("2");
    let idx = names(&t).iter().position(|n| n == "old-name").unwrap();
    for _ in 0..idx {
        t.press("j");
    }
    t.press("R").press("ctrl+u").type_text("new-name").press("enter");
    assert!(names(&t).contains(&"new-name".to_string()));
    assert!(!names(&t).contains(&"old-name".to_string()));
}

#[test]
fn deleting_unmerged_branch_asks_to_force() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("wip");
    sb.commit("unmerged");
    sb.checkout("main");
    let mut t = TestDriver::new(&sb);
    t.press("2");
    let idx = names(&t).iter().position(|n| n == "wip").unwrap();
    for _ in 0..idx {
        t.press("j");
    }
    t.press("d");
    assert!(t.screen().contains("Delete local branch wip?"));
    t.press("y");
    assert!(t.screen().contains("Force delete?"), "{}", t.screen());
    t.press("y");
    assert!(!names(&t).contains(&"wip".to_string()));
    assert!(t.app().error().is_none());
}

#[test]
fn cannot_delete_checked_out_branch() {
    let sb = Sandbox::new();
    sb.commit("base");
    let mut t = TestDriver::new(&sb);
    t.press("2").press("d");
    assert!(t.app().modal().is_none());
    assert!(t.app().notice().unwrap_or_default().contains("checked-out"));
}

#[test]
fn selecting_a_branch_moves_the_graph() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("older");
    sb.commit("older tip");
    sb.checkout("main");
    sb.commit("main tip");
    let mut t = TestDriver::new(&sb);
    t.press("2");
    let idx = names(&t).iter().position(|n| n == "older").unwrap();
    // Move onto "older" (moving at least once so the selection is announced).
    if idx == 0 {
        t.press("j").press("k");
    } else {
        for _ in 0..idx {
            t.press("j");
        }
    }
    assert_eq!(t.app().graph().selected().unwrap().summary, "older tip");
    t.press("3").press("enter");
    t.assert_pane_contains(PaneId::Details, "older tip");
}

#[test]
fn fetch_picks_up_new_remote_branches() {
    let sb = repo_with_remote_branch();
    let other = sb.root().join("other");
    sb.git_in(&other, &["switch", "-q", "-c", "brand-new"]);
    sb.git_in(&other, &["push", "-q", "origin", "brand-new"]);
    let mut t = TestDriver::new(&sb);
    assert!(!names(&t).contains(&"brand-new".to_string()));
    t.press("2").press("f");
    assert!(names(&t).contains(&"brand-new".to_string()), "{:?}", names(&t));
    assert!(t.app().notice().unwrap_or_default().contains("fetch"));
}
