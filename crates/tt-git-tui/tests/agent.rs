//! `A` in the graph asks an agent about the selected commit.

use std::time::Duration;

use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::panes::PaneId;

/// The "agent" is git: it stores the prompt file in the repo as a blob and
/// prints its hash, so tests can read back exactly what it was given.
/// `--no-filters` keeps `core.autocrlf` from converting it or warning.
fn driver(sb: &Sandbox) -> TestDriver {
    let cfg = Config::parse(
        "[agent]\ncommand = \"git hash-object -w --no-filters {prompt_file}\"\nplacement = \"details\"\n",
    )
    .unwrap();
    TestDriver::with_size(cfg, sb, 120, 30)
}

/// Waits for the agent pane to finish and returns the prompt it was given.
fn prompt_given(t: &mut TestDriver, sb: &Sandbox) -> String {
    t.wait_until(Duration::from_secs(20), |app| {
        app.plugin_panes().iter().any(|p| p.exited.is_some())
    });
    let pane = &t.app().plugin_panes()[0];
    assert_eq!(pane.placement, tt_git_plugin::manifest::Placement::Split);
    let screen = t.app().pane_screen(pane.id).unwrap();
    let hash = screen.trim().lines().next().unwrap_or_default().trim().to_string();
    assert_eq!(hash.len(), 40, "pane shows the blob hash: {screen:?}");
    sb.git(&["cat-file", "-p", &hash])
}

#[test]
fn dialog_offers_ready_made_actions() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "a\n", "first");
    let mut t = driver(&sb);
    t.press("A");
    insta::assert_snapshot!(t.screen());
    t.press("tab");
    assert!(t.screen().contains("what the selected commit changes and why"));
    t.press("esc");
    assert!(t.app().modal().is_none());
}

#[test]
fn action_runs_in_place_of_details_with_the_commit_filled_in() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "a\n", "first");
    let oid = sb.rev_parse("HEAD");
    let mut t = driver(&sb);
    t.press("A").press("tab");
    t.type_text("keep it short");
    t.press("enter").wait_idle();
    assert_eq!(t.app().focus(), PaneId::Details);
    assert!(t.pane_title(PaneId::Details).contains("agent: Explain commit"));

    let prompt = prompt_given(&mut t, &sb);
    let root = t.app().repo().root().display().to_string();
    assert!(prompt.contains(&format!("git repository at {root}")), "{prompt}");
    assert!(
        prompt.contains(&format!("Explain commit {oid} (\"first\")")),
        "{prompt}"
    );
    assert!(prompt.ends_with("keep it short"), "{prompt}");
}

#[test]
fn custom_message_needs_text() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "a\n", "first");
    let mut t = driver(&sb);
    t.press("A").press("enter");
    assert!(t.app().modal().is_some(), "nothing typed: still asking");
    t.type_text("what is f.txt for?").press("enter").wait_idle();
    let prompt = prompt_given(&mut t, &sb);
    assert!(prompt.ends_with("\n\nwhat is f.txt for?"), "{prompt}");
}

#[test]
fn conflicts_are_offered_first_while_merging() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "base\n", "base");
    sb.checkout_new("other");
    sb.commit_file("f.txt", "theirs\n", "theirs");
    sb.checkout("main");
    sb.commit_file("f.txt", "ours\n", "ours");
    // The merge stops on the conflict, so it fails.
    let mut merge = std::process::Command::new("git");
    sb.apply_env(&mut merge);
    let out = merge.current_dir(sb.path()).args(["merge", "other"]).output().unwrap();
    assert!(!out.status.success());
    let mut t = driver(&sb);
    t.press("A");
    assert!(t.screen().contains("resolve the conflicts using both sides' history"));
    t.press("enter").wait_idle();
    let prompt = prompt_given(&mut t, &sb);
    assert!(
        prompt.contains("A merge of other into main stopped with conflicts in:\nf.txt"),
        "{prompt}"
    );
}

#[test]
fn a_missing_agent_is_reported() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "a\n", "first");
    let cfg = Config::parse("[agent]\ncommand = \"tt-git-no-such-agent\"\nplacement = \"details\"\n").unwrap();
    let mut t = TestDriver::with_size(cfg, &sb, 120, 30);
    t.press("A").type_text("hi").press("enter").wait_idle();
    t.wait_until(Duration::from_secs(10), |app| {
        app.error().is_some() || app.plugin_panes().iter().any(|p| p.exited.is_some())
    });
    // Unix forks before exec fails, so the pane opens and ends at once;
    // Windows can't start it at all.
    if let Some(err) = t.app().error() {
        assert!(err.contains("[agent] command"), "{err}");
    } else {
        let pane = &t.app().plugin_panes()[0];
        assert!(
            pane.exited.as_deref().unwrap_or_default().contains("exited"),
            "{pane:?}"
        );
    }
}

const BASE: &str = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n";

/// f.txt with two hunks of unstaged changes, lines 2 and 14, open in the
/// staging view.
fn staging(sb: &Sandbox) -> TestDriver {
    sb.commit_file("f.txt", BASE, "base");
    sb.write("f.txt", &BASE.replace("\n2\n", "\ntwo\n").replace("14\n", "fourteen\n"));
    let mut t = driver(sb);
    t.press("2").press("enter").wait_idle();
    assert!(t.app().details().stage().is_some_and(|v| v.loaded()));
    t
}

#[test]
fn a_staging_hunk_goes_to_the_agent() {
    let sb = Sandbox::new();
    let mut t = staging(&sb);
    t.press("j").press("A");
    insta::assert_snapshot!(t.screen());
    t.type_text("why?").press("enter").wait_idle();
    let prompt = prompt_given(&mut t, &sb);
    assert!(
        prompt.contains(
            "This is about lines 11-15 of f.txt, in the unstaged changes (the working tree against the index):"
        ),
        "{prompt}"
    );
    assert!(prompt.contains("```diff\n@@ -11,5 +11,5 @@"), "{prompt}");
    assert!(prompt.contains("-14\n+fourteen\n"), "{prompt}");
    assert!(!prompt.contains("+two"), "only the selected hunk: {prompt}");
    assert!(prompt.ends_with("```\n\nwhy?"), "{prompt}");
}

#[test]
fn picked_lines_go_with_a_ready_made_action() {
    let sb = Sandbox::new();
    let mut t = staging(&sb);
    // Line mode on the first change, then the range down to the next.
    t.press("a").press("v").press("j").press("A");
    assert!(t.screen().contains("Ask an agent about f.txt line 2"), "{}", t.screen());
    t.press("tab");
    assert!(t.screen().contains("what these lines change and why"));
    assert!(!t.screen().contains("Explain branch"), "graph actions aren't offered");
    t.press("enter").wait_idle();
    assert!(t.pane_title(PaneId::Details).contains("agent: Explain change"));
    let prompt = prompt_given(&mut t, &sb);
    assert!(prompt.contains("This is about line 2 of f.txt"), "{prompt}");
    assert!(prompt.contains("@@ -1,5 +1,5 @@\n-2\n+two\n```"), "{prompt}");
    assert!(prompt.contains("Explain what this change does"), "{prompt}");
}

#[test]
fn a_commit_diff_hunk_in_details_goes_to_the_agent() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "a\n", "first");
    sb.commit_file("f.txt", "a\nb\n", "add b");
    let oid = sb.rev_parse("HEAD");
    let mut t = driver(&sb);
    t.press("space").wait_idle();
    t.press("4").press("A");
    assert!(
        t.screen().contains("Ask an agent about f.txt lines 1-2"),
        "{}",
        t.screen()
    );
    t.press("tab").press("tab").press("enter").wait_idle();
    let prompt = prompt_given(&mut t, &sb);
    assert!(
        prompt.contains(&format!("This is about lines 1-2 of f.txt, in commit {}:", &oid[..7])),
        "{prompt}"
    );
    assert!(prompt.contains("@@ -1,1 +1,2 @@\n a\n+b\n```"), "{prompt}");
    assert!(prompt.contains("Review this change for bugs"), "{prompt}");
}

#[test]
fn details_without_a_hunk_says_so() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "a\n", "first");
    sb.git(&["commit", "--allow-empty", "-m", "empty"]);
    let mut t = driver(&sb);
    t.press("space").wait_idle();
    t.press("4").press("A");
    assert!(t.app().modal().is_none());
    assert!(t.screen().contains("no diff hunk here to ask about"), "{}", t.screen());
}
