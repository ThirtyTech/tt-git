use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::modal::Modal;
use tt_git_tui::panes::{BranchTab, PaneId};

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
    t.press("3");
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
    t.press("3");
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
    t.press("3").press("/").type_text("lgo");
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
    t.press("3").press("n");
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
    t.press("3");
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
    t.press("3");
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
    t.press("3").press("d");
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
    t.press("3");
    let idx = names(&t).iter().position(|n| n == "older").unwrap();
    // Move onto "older" (moving at least once so the selection is announced).
    if idx == 0 {
        t.press("j").press("k");
    } else {
        for _ in 0..idx {
            t.press("j");
        }
    }
    assert_eq!(&*t.app().graph().selected().unwrap().summary, "older tip");
    t.press("1").press("space");
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
    t.press("3").press("f");
    assert!(names(&t).contains(&"brand-new".to_string()), "{:?}", names(&t));
    assert!(t.app().notice().unwrap_or_default().contains("fetch"));
}

/// On origin: `main`, `done` (merged into main), `open` and `archive/old`
/// (not merged), and `development`, which has `open` merged into it.
fn repo_for_base_tabs() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("done");
    sb.commit("done work");
    sb.checkout("main");
    sb.merge("done");
    sb.checkout_new("archive/old");
    sb.commit("old work");
    sb.checkout("main");
    sb.checkout_new("open");
    sb.commit("open work");
    sb.checkout("main");
    sb.checkout_new("development");
    sb.merge("open");
    sb.checkout("main");
    sb.add_remote("origin");
    sb.git(&["branch", "-q", "-D", "done", "open", "archive/old", "development"]);
    sb
}

#[test]
fn tabs_list_branches_merged_and_unmerged_into_the_primary_branch() {
    let sb = repo_for_base_tabs();
    let mut t = TestDriver::with_size(Config::default(), &sb, 160, 40);
    t.press("3");
    assert_eq!(t.app().branches().tab(), BranchTab::All);
    let title = t.pane_title(PaneId::Branches);
    assert!(title.contains("Branches 5 · Tags 0 · Unmerged 2 · Merged 1"), "{title}");

    t.press("]");
    assert_eq!(t.app().branches().tab(), BranchTab::Tags);
    t.press("]");
    assert_eq!(t.app().branches().tab(), BranchTab::Unmerged);
    assert_eq!(names(&t), vec!["development", "open"], "archive is hidden");
    assert!(t.pane_title(PaneId::Branches).contains("(origin/main)"));
    let open = t
        .pane_lines(PaneId::Branches)
        .into_iter()
        .find(|l| l.starts_with("  open "))
        .unwrap();
    assert!(open.contains("open work") && open.contains("Test User"), "{open}");

    t.press("]");
    assert_eq!(names(&t), vec!["done"]);
    t.press("]");
    assert_eq!(t.app().branches().tab(), BranchTab::All, "wraps round");
    t.press("[");
    assert_eq!(t.app().branches().tab(), BranchTab::Merged);
    t.press("{");
    assert_eq!(t.app().branches().tab(), BranchTab::Unmerged);
    t.press("[");
    assert_eq!(t.app().branches().tab(), BranchTab::Tags);
    t.press("]");

    // Selecting a row moves the graph to its tip, as in the full list.
    t.press("j");
    let tip = sb.rev_parse("origin/open");
    assert_eq!(t.app().graph().selected().map(|c| c.oid.to_string()), Some(tip));
}

#[test]
fn brackets_on_the_graph_step_the_branch_tabs() {
    let sb = repo_for_base_tabs();
    let mut t = TestDriver::with_size(Config::default(), &sb, 160, 40);
    assert_eq!(t.app().focus(), PaneId::Graph);
    t.press("]");
    assert_eq!(t.app().branches().tab(), BranchTab::Tags);
    t.press("}");
    assert_eq!(t.app().branches().tab(), BranchTab::Unmerged);
    t.press("[");
    assert_eq!(t.app().branches().tab(), BranchTab::Tags);
    t.press("{");
    assert_eq!(t.app().branches().tab(), BranchTab::All);
    t.press("[");
    assert_eq!(t.app().branches().tab(), BranchTab::Merged, "wraps round");
    assert_eq!(t.app().focus(), PaneId::Graph, "focus stays on the graph");
}

/// The first `n` words of a row.
fn cols(line: &str, n: usize) -> Vec<&str> {
    line.split_whitespace().take(n).collect()
}

fn tag_names(t: &TestDriver) -> Vec<String> {
    t.app()
        .branches()
        .visible_tags()
        .map(|r| r.short_name.clone())
        .collect()
}

#[test]
fn tags_tab_lists_tags_newest_first() {
    let sb = Sandbox::new();
    sb.commit("first");
    sb.git(&["tag", "v1.0"]);
    sb.commit("second");
    sb.git(&["tag", "-a", "v2.0", "-m", "the big release"]);
    sb.commit("third");
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 30);
    t.press("3");
    assert!(t.pane_title(PaneId::Branches).contains("Branches 1 · Tags 2 · "));
    t.press("]");
    assert_eq!(t.app().branches().tab(), BranchTab::Tags);
    assert_eq!(tag_names(&t), vec!["v2.0", "v1.0"]);
    assert!(t.app().branches().selected().is_none(), "no branch on the tags tab");
    assert!(
        t.pane_title(PaneId::Branches).contains("(main)"),
        "distances are from the base"
    );
    let lines = t.pane_lines(PaneId::Branches);
    assert!(
        cols(&lines[0], 2) == ["v2.0", "↓1"] && lines[0].contains("the big release"),
        "{lines:?}"
    );
    assert!(
        cols(&lines[1], 2) == ["v1.0", "↓2"] && lines[1].contains("first"),
        "{lines:?}"
    );

    // Selecting a tag moves the graph to its commit.
    t.press("j");
    assert_eq!(&*t.app().graph().selected().unwrap().summary, "first");

    t.press("/").type_text("2").press("enter");
    assert_eq!(tag_names(&t), vec!["v2.0"]);
    t.press("c");
    assert_eq!(t.app_mut().take_clipboard().as_deref(), Some("v2.0"));

    t.press("n");
    assert!(t.screen().contains("New branch from v2.0"));
    t.type_text("hotfix").press("enter");
    assert_eq!(sb.current_branch(), "hotfix");
    assert_eq!(sb.rev_parse("HEAD"), sb.rev_parse("v2.0^{commit}"));

    t.press("d");
    assert!(t.screen().contains("Delete local tag v2.0?"));
    t.press("y");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.git(&["tag"]), "v1.0");
    t.press("/").press("esc");
    assert_eq!(tag_names(&t), vec!["v1.0"]);
}

#[test]
fn only_the_newest_tags_are_compared_with_the_base() {
    let sb = Sandbox::new();
    for n in 1..=7 {
        sb.commit(&format!("release {n}"));
        sb.git(&["tag", &format!("v{n}")]);
    }
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 30);
    t.press("3").press("]");
    let mut compared: Vec<_> = t.app().branches().base().unwrap().tags.keys().cloned().collect();
    compared.sort();
    assert_eq!(
        compared,
        vec![
            "refs/tags/v3",
            "refs/tags/v4",
            "refs/tags/v5",
            "refs/tags/v6",
            "refs/tags/v7"
        ]
    );
    let lines = t.pane_lines(PaneId::Branches);
    assert!(cols(&lines[0], 2) == ["v7", "✓"], "the base's tip: {lines:?}");
    assert!(cols(&lines[4], 2) == ["v3", "↓4"], "{lines:?}");
    assert!(cols(&lines[5], 2) == ["v2", "release"], "not compared: {lines:?}");
}

#[test]
fn base_branch_follows_origin_head() {
    let sb = repo_for_base_tabs();
    sb.git(&["remote", "set-head", "origin", "development"]);
    let mut t = TestDriver::new(&sb);
    t.press("3").press("]").press("]");
    let base = t.app().branches().base().unwrap();
    assert_eq!(base.base, "origin/development");
    assert!(!base.chosen);
    assert_eq!(
        names(&t),
        Vec::<String>::new(),
        "only archive/old is unmerged, and it's hidden"
    );
    t.press("]");
    let mut merged = names(&t);
    merged.sort();
    assert_eq!(merged, vec!["done", "open"], "main is a safe branch");

    let mut config = Config::default();
    config.branches.safe_branches.clear();
    let mut t = TestDriver::with_config(config, &sb);
    t.press("3").press("]").press("]").press("]");
    let mut merged = names(&t);
    merged.sort();
    assert_eq!(merged, vec!["done", "main", "open"], "listed once nothing is safe");
}

#[test]
fn pick_base_branch_from_a_list_and_go_back_to_detecting() {
    let sb = repo_for_base_tabs();
    let mut t = TestDriver::new(&sb);
    t.press("3").press("]").press("]").press("b");
    assert!(t.screen().contains("Base branch"));
    assert!(t.screen().contains("Detect automatically"));
    t.type_text("origin/dev").press("enter");
    assert!(t.app().modal().is_none());
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.git(&["config", "--get", "tt-git.baseBranch"]), "origin/development");
    let base = t.app().branches().base().unwrap();
    assert_eq!(base.base, "origin/development");
    assert!(base.chosen);
    assert_eq!(base.detected.as_deref(), Some("origin/main"));
    assert!(t.pane_title(PaneId::Branches).contains("(origin/development)"));
    t.press("]");
    let mut merged = names(&t);
    merged.sort();
    assert_eq!(merged, vec!["done", "open"]);

    // The choice is per repo and survives a restart.
    let t2 = TestDriver::new(&sb);
    assert_eq!(t2.app().branches().base().unwrap().base, "origin/development");

    // The picker opens on the current base; the first entry detects again.
    t.press("b");
    t.type_text("detect").press("enter");
    assert!(!sb.git(&["config", "--list", "--local"]).contains("tt-git."));
    let base = t.app().branches().base().unwrap();
    assert_eq!(base.base, "origin/main");
    assert!(!base.chosen);
}

#[test]
fn no_base_branch_without_a_primary_one() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.git(&["branch", "-q", "-m", "topic"]);
    let mut t = TestDriver::new(&sb);
    t.press("3").press("]").press("]");
    assert!(t.app().branches().base().is_none());
    assert!(t.pane_lines(PaneId::Branches)[0].contains("no base branch"));
    let title = t.pane_title(PaneId::Branches);
    assert!(title.contains("Branches 1 · Tags 0 · Unmerged · Merged"), "{title}");
}

#[test]
fn merged_and_unmerged_tabs_show_one_branch_from_fork_to_merge() {
    let sb = repo_for_base_tabs();
    let mut t = TestDriver::with_size(Config::default(), &sb, 160, 40);
    let summaries = |t: &TestDriver| {
        t.app()
            .graph()
            .commits()
            .iter()
            .map(|c| c.summary.to_string())
            .collect::<Vec<_>>()
    };
    let all = summaries(&t).len();
    t.press("3").press("]").press("]").press("j");
    t.wait_idle();
    assert!(t.pane_title(PaneId::Graph).contains("Graph [origin/open]"));
    // The branch's commit and the base commit it branched off, which is
    // still main's tip.
    assert_eq!(summaries(&t), vec!["open work", "Merge branch 'done'"]);
    let tip = sb.rev_parse("origin/open");
    assert_eq!(t.app().graph().selected().map(|c| c.oid.to_string()), Some(tip));
    t.assert_pane_contains(PaneId::Graph, "(branched off origin/main)");

    t.press("]");
    t.wait_idle();
    assert!(t.pane_title(PaneId::Graph).contains("Graph [origin/done]"));
    assert_eq!(
        summaries(&t),
        vec!["Merge branch 'done'", "done work", "base"],
        "merge, branch commits, fork"
    );
    t.assert_pane_contains(PaneId::Graph, "(merged into origin/main)");
    t.assert_pane_contains(PaneId::Graph, "(branched off origin/main)");

    // Back on the full list, the graph shows everything again.
    t.press("]");
    t.wait_idle();
    assert!(t.pane_title(PaneId::Graph).contains("Graph [all]"));
    assert_eq!(summaries(&t).len(), all);

    // `a` in the graph also leaves a focused branch.
    t.press("[");
    t.wait_idle();
    assert!(t.pane_title(PaneId::Graph).contains("Graph [origin/done]"));
    t.press("1").press("a");
    t.wait_idle();
    assert!(t.pane_title(PaneId::Graph).contains("Graph [all]"));
    assert_eq!(summaries(&t).len(), all);
}

#[test]
fn new_branch_as_a_worktree() {
    let sb = Sandbox::new();
    sb.commit("base");
    let mut t = TestDriver::new(&sb);
    t.press("3").press("n");
    assert!(t.screen().contains("switches to it here"));
    t.type_text("side").press("tab");
    assert!(t.screen().contains("checked out in .worktrees/side"), "{}", t.screen());
    t.press("shift+tab");
    assert!(t.screen().contains("switches to it here"));
    t.press("tab").press("enter");
    assert!(t.app().modal().is_none());
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.current_branch(), "main", "stays on the current branch");
    let path = sb.path().join(".worktrees/side");
    assert_eq!(sb.git_in(&path, &["branch", "--show-current"]), "side");
    assert!(names(&t).contains(&"side".to_string()));
    assert!(t.app().status().entries().is_empty(), "the worktree isn't untracked");
    assert!(
        t.app()
            .notice()
            .unwrap_or_default()
            .contains("create worktree .worktrees/side")
    );
}

/// `main` checked out, `feature` tracking `origin/feature` and not checked
/// out, `lonely` with no upstream; `other` is a clone to push from.
fn repo_for_fast_forward() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit("base");
    let remote = sb.add_remote("origin");
    sb.git(&["branch", "-q", "--set-upstream-to=origin/main"]);
    sb.git(&["push", "-q", "origin", "main:feature"]);
    sb.git(&["fetch", "-q", "origin"]);
    sb.git(&["branch", "-q", "--track", "feature", "origin/feature"]);
    sb.git(&["branch", "-q", "lonely"]);
    let other = sb.clone_to(&remote, "other");
    sb.git_in(&other, &["switch", "-q", "feature"]);
    sb.git_in(&other, &["commit", "-q", "--allow-empty", "-m", "their work"]);
    sb.git_in(&other, &["push", "-q", "origin", "feature"]);
    sb
}

fn select_branch(t: &mut TestDriver, name: &str) {
    t.press("3").press("g");
    let idx = t.app().branches().visible().position(|b| b.name == name).unwrap();
    for _ in 0..idx {
        t.press("j");
    }
}

#[test]
fn fast_forward_moves_a_branch_without_checking_it_out() {
    let sb = repo_for_fast_forward();
    let mut t = TestDriver::new(&sb);
    select_branch(&mut t, "feature");
    // Not fetched yet, so the pane still shows it in sync.
    t.press("F");
    assert_eq!(sb.git(&["log", "-1", "--format=%s", "feature"]), "their work");
    assert_eq!(sb.current_branch(), "main");
    let notice = t.app().notice().unwrap_or_default().to_string();
    assert!(notice.contains("fast-forwarded feature to origin/feature"), "{notice}");

    select_branch(&mut t, "feature");
    t.press("F");
    let notice = t.app().notice().unwrap_or_default().to_string();
    assert!(notice.contains("feature is already up to date"), "{notice}");
}

#[test]
fn fast_forward_explains_what_it_cannot_do() {
    let sb = repo_for_fast_forward();
    sb.git(&["switch", "-q", "feature"]);
    sb.git(&["commit", "-q", "--allow-empty", "-m", "my work"]);
    sb.git(&["switch", "-q", "main"]);
    sb.git(&["fetch", "-q", "origin"]);
    let before = sb.git(&["rev-parse", "feature"]);
    let mut t = TestDriver::new(&sb);

    for (branch, says) in [
        ("main", "main is checked out; pull it instead"),
        ("lonely", "lonely has no upstream to fast-forward to"),
        ("feature", "feature has diverged from origin/feature (↑1 ↓1)"),
    ] {
        select_branch(&mut t, branch);
        t.press("F");
        let notice = t.app().notice().unwrap_or_default().to_string();
        assert!(notice.contains(says), "{branch}: {notice}");
    }
    assert_eq!(sb.git(&["rev-parse", "feature"]), before);
    assert!(t.app().error().is_none());
}

#[test]
fn fast_forward_refuses_a_branch_another_worktree_has_checked_out() {
    let sb = repo_for_fast_forward();
    let wt = sb.root().join("feature-wt");
    sb.git(&["worktree", "add", "-q", wt.to_str().unwrap(), "feature"]);
    let mut t = TestDriver::new(&sb);
    select_branch(&mut t, "feature");
    t.press("F");
    let err = t.app().error().unwrap_or_default().to_string();
    assert!(err.contains("feature is checked out in the worktree at"), "{err}");
}

/// `main` tracking `origin/main`, and branches whose remote branch someone
/// else deletes (not fetched yet): `merged-gone` at main, `unmerged-gone`
/// with a commit of its own, and `wt-gone` checked out in a worktree.
/// `kept` still has its remote branch and `lonely` never had one.
fn repo_with_gone_branches() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit("base");
    let remote = sb.add_remote("origin");
    sb.git(&["branch", "-q", "--set-upstream-to=origin/main"]);
    for name in ["merged-gone", "unmerged-gone", "wt-gone", "kept"] {
        sb.git(&["branch", "-q", name]);
    }
    sb.checkout("unmerged-gone");
    sb.commit("unmerged work");
    sb.checkout("main");
    for name in ["merged-gone", "unmerged-gone", "wt-gone", "kept"] {
        sb.git(&["push", "-q", "-u", "origin", name]);
    }
    sb.git(&["branch", "-q", "lonely"]);
    let wt = sb.root().join("wt");
    sb.git(&["worktree", "add", "-q", wt.to_str().unwrap(), "wt-gone"]);
    let other = sb.clone_to(&remote, "other");
    for name in ["merged-gone", "unmerged-gone", "wt-gone"] {
        sb.git_in(&other, &["push", "-q", "origin", "--delete", name]);
    }
    sb
}

fn local_branches(sb: &Sandbox) -> Vec<String> {
    let mut names: Vec<_> = sb
        .git(&["branch", "--format=%(refname:short)"])
        .lines()
        .map(str::to_string)
        .collect();
    names.sort();
    names
}

#[test]
fn prune_gone_fetches_then_deletes_gone_branches_asking_again_for_unmerged() {
    let sb = repo_with_gone_branches();
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 30);
    t.press("3").press("D");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    insta::assert_snapshot!(t.screen());
    // The fetch pruned the remote-tracking branches.
    assert_eq!(sb.git(&["branch", "-r", "--list", "origin/merged-gone"]), "");

    t.press("y");
    let screen = t.screen();
    assert!(screen.contains("Force delete?"), "{screen}");
    assert!(screen.contains("Deleted branch merged-gone."), "{screen}");
    assert!(screen.contains("unmerged-gone"), "{screen}");
    assert_eq!(
        local_branches(&sb),
        ["kept", "lonely", "main", "unmerged-gone", "wt-gone"]
    );

    t.press("y");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(t.app().notice(), Some("✓ deleted branch unmerged-gone"));
    assert_eq!(local_branches(&sb), ["kept", "lonely", "main", "wt-gone"]);
    assert!(!names(&t).contains(&"unmerged-gone".to_string()));
}

#[test]
fn prune_gone_keeps_unmerged_branches_when_force_is_declined() {
    let sb = repo_with_gone_branches();
    let mut t = TestDriver::new(&sb);
    t.press("3").press("D").press("y");
    assert!(t.screen().contains("Force delete?"), "{}", t.screen());
    t.press("n");
    assert!(t.app().modal().is_none());
    assert_eq!(
        local_branches(&sb),
        ["kept", "lonely", "main", "unmerged-gone", "wt-gone"]
    );
}

#[test]
fn prune_gone_says_when_there_is_nothing_to_delete() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.add_remote("origin");
    sb.git(&["branch", "-q", "--set-upstream-to=origin/main"]);
    let mut t = TestDriver::new(&sb);
    t.press("3").press("D");
    assert!(t.app().modal().is_none());
    assert_eq!(
        t.app().notice(),
        Some("✓ fetched all remotes; no branches with a gone upstream")
    );
}

#[test]
fn prune_gone_never_offers_a_checked_out_branch() {
    let sb = repo_with_gone_branches();
    sb.git(&["branch", "-q", "-D", "merged-gone", "unmerged-gone"]);
    let mut t = TestDriver::new(&sb);
    t.press("3").press("D");
    assert!(t.app().modal().is_none());
    assert_eq!(
        t.app().notice(),
        Some("✓ fetched all remotes; only checked-out branches have a gone upstream: wt-gone")
    );
}

fn select(t: &mut TestDriver, name: &str) {
    let idx = names(t).iter().position(|n| n == name).unwrap();
    for _ in 0..idx {
        t.press("j");
    }
}

#[test]
fn push_selected_branch_without_checking_it_out() {
    let sb = repo_with_remote_branch();
    let mut t = TestDriver::with_size(Config::default(), &sb, 240, 30);
    t.press("3");
    assert!(t.screen().contains("P push branch"), "{}", t.screen());
    select(&mut t, "local-work");
    t.press("P");
    t.wait_idle();
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(t.app().notice(), Some("✓ pushed local-work"));
    assert_eq!(sb.current_branch(), "main");
    assert_eq!(
        sb.git(&["rev-parse", "--abbrev-ref", "local-work@{upstream}"]),
        "origin/local-work"
    );
    let pushes: Vec<String> = t
        .app()
        .repo()
        .git()
        .log()
        .entries()
        .iter()
        .filter(|c| c.args.starts_with("push"))
        .map(|c| c.args.clone())
        .collect();
    assert_eq!(pushes, ["push -u origin refs/heads/local-work"]);
}

#[test]
fn push_on_a_remote_only_branch_says_why_not() {
    let sb = repo_with_remote_branch();
    let mut t = TestDriver::new(&sb);
    t.press("3");
    select(&mut t, "their-feature");
    t.press("P");
    assert_eq!(
        t.app().notice(),
        Some("their-feature is only on the remote (origin/their-feature); there's nothing local to push")
    );
}

#[test]
fn push_on_the_checked_out_branch_pushes_as_usual() {
    let sb = repo_with_remote_branch();
    sb.commit("more");
    let mut t = TestDriver::new(&sb);
    t.press("3");
    select(&mut t, "main");
    t.press("P");
    t.wait_idle();
    assert_eq!(t.app().notice(), Some("✓ pushed main → origin/main"));
}

#[test]
fn push_from_the_worktrees_tab_pushes_its_branch() {
    let sb = repo_with_remote_branch();
    let path = sb.root().join("wt-local");
    sb.git(&["worktree", "add", "-q", &path.to_string_lossy(), "local-work"]);
    let mut t = TestDriver::with_size(Config::default(), &sb, 160, 30);
    t.press("3");
    while t.app().branches().tab() != BranchTab::Worktrees {
        t.press("]");
    }
    let idx = t
        .pane_lines(PaneId::Branches)
        .iter()
        .filter(|l| !l.trim().is_empty())
        .position(|l| l.contains("local-work"))
        .unwrap();
    for _ in 0..idx {
        t.press("j");
    }
    t.press("P");
    t.wait_idle();
    assert_eq!(t.app().notice(), Some("✓ pushed local-work"), "{}", t.screen());
}
