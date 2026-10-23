use std::time::Duration;

use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::panes::{DetailsSubject, PaneId};

fn codes(t: &TestDriver) -> Vec<(String, String)> {
    t.app()
        .status()
        .entries()
        .iter()
        .map(|e| (e.short_code(), e.path.clone()))
        .collect()
}

fn select_file(t: &mut TestDriver, path: &str) {
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
}

#[test]
fn stage_commit_flow() {
    let sb = Sandbox::new();
    sb.commit_file("app.rs", "fn main() {}\n", "base");
    sb.write("app.rs", "fn main() { println!(\"hi\"); }\n");
    sb.write("notes.md", "todo\n");
    let mut t = TestDriver::new(&sb);
    assert_eq!(
        codes(&t),
        vec![(" M".into(), "app.rs".into()), ("??".into(), "notes.md".into())]
    );

    select_file(&mut t, "app.rs");
    t.press("space");
    assert_eq!(codes(&t)[0], ("M ".into(), "app.rs".into()));

    t.press("c");
    assert!(t.screen().contains("Commit message"));
    t.type_text("Say hi").press("enter");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(sb.git(&["log", "-1", "--format=%s"]), "Say hi");
    assert_eq!(codes(&t), vec![("??".into(), "notes.md".into())]);

    // The new commit is selected in the graph.
    let selected = t.app().graph().selected().unwrap();
    assert_eq!(&*selected.summary, "Say hi");
    assert!(t.pane_lines(PaneId::Graph)[0].contains("Say hi"));
}

#[test]
fn commit_with_nothing_staged_explains() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.write("x.txt", "x");
    let mut t = TestDriver::new(&sb);
    t.press("2").press("c");
    assert!(t.app().modal().is_none());
    assert!(t.app().notice().unwrap_or_default().contains("nothing staged"));
}

#[test]
fn stage_all_toggles() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.write("a.txt", "a").write("b.txt", "b");
    let mut t = TestDriver::new(&sb);
    t.press("2").press("a");
    assert!(codes(&t).iter().all(|(c, _)| c == "A "), "{:?}", codes(&t));
    t.press("a");
    assert!(codes(&t).iter().all(|(c, _)| c == "??"), "{:?}", codes(&t));
}

#[test]
fn unstage_with_space() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "1\n", "base");
    sb.write("f.txt", "2\n");
    sb.git(&["add", "f.txt"]);
    let mut t = TestDriver::new(&sb);
    select_file(&mut t, "f.txt");
    assert_eq!(codes(&t)[0].0, "M ");
    t.press("space");
    assert_eq!(codes(&t)[0].0, " M");
}

#[test]
fn discard_asks_then_restores() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "original\n", "base");
    sb.write("f.txt", "oops\n");
    let mut t = TestDriver::new(&sb);
    select_file(&mut t, "f.txt");
    t.press("d");
    assert!(t.screen().contains("Discard all changes to f.txt?"));
    t.press("n");
    assert_eq!(std::fs::read_to_string(sb.path().join("f.txt")).unwrap(), "oops\n");
    t.press("d").press("y");
    assert_eq!(std::fs::read_to_string(sb.path().join("f.txt")).unwrap(), "original\n");
    assert!(t.app().status().entries().is_empty());
    t.assert_pane_contains(PaneId::Status, "working tree clean");
}

#[test]
fn shift_d_asks_what_to_discard_then_discards_it() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "original\n", "base");
    sb.commit_file("g.txt", "original\n", "base2");
    sb.write("f.txt", "staged\n");
    sb.git(&["add", "f.txt"]);
    sb.write("g.txt", "oops\n");
    sb.write("new.txt", "new\n");
    let mut t = TestDriver::new(&sb);
    t.press("2").press("D");
    insta::assert_snapshot!(t.screen());
    t.press("n");
    assert!(t.app().modal().is_none());
    assert_eq!(codes(&t).len(), 3, "nothing discarded");

    // Unstaged + untracked keeps f.txt staged.
    t.press("D").press("tab").press("tab");
    assert!(t.screen().contains("Lost: 1 unstaged file, 1 untracked file."));
    assert!(t.screen().contains("Kept: 1 staged file, ignored files."));
    t.press("y");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(codes(&t), vec![("M ".into(), "f.txt".into())]);
    assert!(!sb.path().join("new.txt").exists());

    t.press("D").press("y");
    assert!(t.app().status().entries().is_empty());
    assert_eq!(std::fs::read_to_string(sb.path().join("f.txt")).unwrap(), "original\n");
    t.press("D");
    assert!(t.app().modal().is_none());
    assert!(t.app().notice().unwrap_or_default().contains("nothing to discard"));
}

#[test]
fn shift_d_refuses_during_a_merge() {
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
    let mut t = TestDriver::new(&sb);
    t.press("2").press("D");
    assert!(t.app().modal().is_none());
    assert!(
        t.app().notice().unwrap_or_default().contains("a merge is in progress"),
        "{:?}",
        t.app().notice()
    );
}

#[test]
fn details_show_file_diff_then_commit_again() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "one\n", "base");
    sb.write("f.txt", "two\n");
    sb.write("new.txt", "brand new\n");
    // A layout with details always on screen, next to status.
    let cfg = Config::parse(
        r#"
        [layout]
        split = "columns"
        children = [
          { split = "rows", children = [{ pane = "status" }, { pane = "branches" }] },
          { split = "rows", children = [{ pane = "graph" }, { pane = "details" }] },
        ]
        "#,
    )
    .unwrap();
    let mut t = TestDriver::with_config(cfg, &sb);
    t.press("2");
    assert_eq!(t.app().details().subject(), Some(&DetailsSubject::File("f.txt".into())));
    t.assert_pane_contains(PaneId::Details, "-one");
    t.assert_pane_contains(PaneId::Details, "+two");
    t.press("j");
    t.assert_pane_contains(PaneId::Details, "+brand new");
    assert!(t.pane_title(PaneId::Details).contains("Diff new.txt"));

    // Back to the graph: details show the selected commit again.
    t.press("1");
    t.assert_pane_contains(PaneId::Details, "Author: Test User");
}

#[test]
fn amend_keeps_message() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.commit_file("x.txt", "x\n", "Add x");
    sb.write("y.txt", "y\n");
    let mut t = TestDriver::new(&sb);
    t.press("2").press("a").press("A");
    assert!(t.screen().contains("Amend the last commit"));
    t.press("y");
    assert_eq!(sb.git(&["log", "-1", "--format=%s"]), "Add x");
    assert_eq!(sb.git(&["rev-list", "--count", "HEAD"]), "2");
    assert!(t.app().status().entries().is_empty());
}

#[test]
fn push_new_branch_then_pull() {
    let sb = Sandbox::new();
    sb.commit("base");
    let remote = sb.add_remote("origin");
    sb.checkout_new("topic");
    sb.commit("topic work");
    let mut t = TestDriver::new(&sb);

    t.press("P");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(
        sb.git(&["rev-parse", "--abbrev-ref", "topic@{upstream}"]),
        "origin/topic"
    );
    assert!(t.pane_title(PaneId::Status).contains("→ origin/topic"));

    let other = sb.clone_to(&remote, "other");
    sb.git_in(&other, &["switch", "-q", "topic"]);
    sb.git_in(&other, &["commit", "-q", "--allow-empty", "-m", "from elsewhere"]);
    sb.git_in(&other, &["push", "-q"]);

    t.press("p");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(&*t.app().graph().selected().unwrap().summary, "from elsewhere");
}

#[test]
fn push_failure_shows_error() {
    let sb = Sandbox::new();
    sb.commit("base");
    let mut t = TestDriver::new(&sb);
    t.press("P");
    let err = t.app().error().expect("error").to_string();
    assert!(err.contains("push failed") && err.contains("no remote"), "{err}");
}

#[test]
fn watcher_refreshes_on_external_changes() {
    let sb = Sandbox::new();
    sb.commit("base");
    let mut t = TestDriver::with_watcher(Config::default(), &sb);
    assert!(t.app().status().entries().is_empty());

    sb.write("from-editor.txt", "edited elsewhere\n");
    t.wait_until(Duration::from_secs(10), |app| {
        app.status().entries().iter().any(|e| e.path == "from-editor.txt")
    });

    // A commit made in another terminal shows up in the graph.
    sb.git(&["add", "-A"]);
    sb.commit_staged("made outside");
    t.wait_until(Duration::from_secs(10), |app| {
        app.graph()
            .commits()
            .first()
            .is_some_and(|c| &*c.summary == "made outside")
    });
}

#[test]
fn ignore_an_untracked_file_from_the_picker() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.write("logs/run.log", "noise\n").write("keep.txt", "k\n");
    let mut t = TestDriver::new(&sb);
    select_file(&mut t, "logs/run.log");
    t.press("i");
    let screen = t.screen();
    assert!(screen.contains("Ignore logs/run.log"), "{screen}");
    for item in [
        "Add /logs/run.log to .gitignore",
        "Add /logs/run.log to .git/info/exclude",
        "Add /logs/ to .gitignore",
        "Add /logs/ to .git/info/exclude",
    ] {
        assert!(screen.contains(item), "{item}\n{screen}");
    }
    t.press("esc");
    assert!(t.app().modal().is_none());
    assert!(!sb.path().join(".gitignore").exists());

    t.press("i").press("enter");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(
        std::fs::read_to_string(sb.path().join(".gitignore")).unwrap(),
        "/logs/run.log\n"
    );
    assert_eq!(
        codes(&t),
        vec![("??".into(), ".gitignore".into()), ("??".into(), "keep.txt".into())]
    );

    // The exclude file, picked by typing.
    select_file(&mut t, "keep.txt");
    t.press("i").type_text("exclude").press("enter");
    let exclude = sb.git(&["rev-parse", "--git-path", "info/exclude"]);
    let exclude = std::fs::read_to_string(sb.path().join(exclude)).unwrap();
    assert!(exclude.lines().any(|l| l == "/keep.txt"), "{exclude}");
    assert_eq!(codes(&t), vec![("??".into(), ".gitignore".into())]);
}

#[test]
fn ignore_a_tracked_file_asks_to_stop_tracking_it() {
    let sb = Sandbox::new();
    sb.commit_file("local.env", "a=1\n", "base");
    sb.write("local.env", "a=2\n");
    let mut t = TestDriver::new(&sb);
    select_file(&mut t, "local.env");
    t.press("i").press("enter");
    assert!(t.screen().contains("local.env is tracked"), "{}", t.screen());
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    t.press("n");
    assert!(!sb.path().join(".gitignore").exists(), "nothing written until asked");

    t.press("i").press("enter").press("y");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(
        std::fs::read_to_string(sb.path().join(".gitignore")).unwrap(),
        "/local.env\n"
    );
    assert!(sb.path().join("local.env").is_file(), "kept on disk");
    assert_eq!(
        codes(&t),
        vec![("D ".into(), "local.env".into()), ("??".into(), ".gitignore".into())]
    );
}
