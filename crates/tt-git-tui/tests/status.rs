use std::time::Duration;

use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::panes::{DetailsSubject, PaneId};
use tt_git_tui::{App, Config};

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

/// Where inotify watches folders one by one (Linux), folders made or
/// stopped being ignored after the start are watched too.
#[test]
fn watcher_sees_into_folders_made_or_unignored_later() {
    let sb = Sandbox::new();
    sb.write(".gitignore", "build/\n");
    sb.git(&["add", ".gitignore"]);
    sb.commit_staged("base");
    sb.write("build/out.txt", "x");
    let mut t = TestDriver::with_watcher(Config::default(), &sb);
    let shows = |path: &'static str| move |app: &App| app.status().entries().iter().any(|e| e.path == path);

    // A new folder, then a file in a new folder in it once that's settled.
    sb.write("src/a.txt", "a\n");
    t.wait_until(Duration::from_secs(10), shows("src/a.txt"));
    std::fs::create_dir(sb.path().join("src/deep")).unwrap();
    std::thread::sleep(Duration::from_millis(800));
    t.wait_idle();
    sb.write("src/deep/b.txt", "b\n");
    t.wait_until(Duration::from_secs(10), shows("src/deep/b.txt"));

    // build/ stops being ignored, then a file in it changes.
    sb.write(".gitignore", "");
    t.wait_until(Duration::from_secs(10), shows("build/out.txt"));
    std::thread::sleep(Duration::from_millis(800));
    t.wait_idle();
    sb.write("build/later.txt", "x");
    t.wait_until(Duration::from_secs(10), shows("build/later.txt"));

    // A branch in a new folder under refs/heads, then another in it.
    sb.git(&["branch", "feature/one"]);
    t.wait_until(Duration::from_secs(10), |app| {
        app.branches().entries().iter().any(|b| b.name == "feature/one")
    });
    std::thread::sleep(Duration::from_millis(800));
    t.wait_idle();
    sb.git(&["branch", "feature/two"]);
    t.wait_until(Duration::from_secs(10), |app| {
        app.branches().entries().iter().any(|b| b.name == "feature/two")
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

/// A config with the status tree on, and details always on screen.
fn tree_config() -> Config {
    Config::parse(
        r#"
        [status]
        tree = true
        [layout]
        split = "columns"
        children = [
          { split = "rows", children = [{ pane = "status" }, { pane = "branches" }] },
          { split = "rows", children = [{ pane = "graph" }, { pane = "details" }] },
        ]
        "#,
    )
    .unwrap()
}

/// Changes in nested folders: `src/` holds files and `src/ui/`, and
/// `docs/guide/` holds one file under a folder with nothing else.
fn tree_sandbox() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit_file("src/main.rs", "fn main() {}\n", "base");
    sb.commit_file("src/ui/view.rs", "view\n", "ui");
    sb.write("src/main.rs", "fn main() { run(); }\n");
    sb.write("src/ui/view.rs", "view 2\n");
    sb.write("src/lib.rs", "pub fn run() {}\n");
    sb.write("docs/guide/intro.md", "hi\n");
    sb.write("README.md", "readme\n");
    sb
}

fn rows(t: &TestDriver) -> Vec<String> {
    tt_git_tui::draw::status::row_texts(t.app().status())
}

/// Selects the row whose text, trimmed, is `text`.
fn select_row(t: &mut TestDriver, text: &str) {
    t.press("2").press("g");
    let idx = rows(t)
        .iter()
        .position(|r| r.trim() == text)
        .unwrap_or_else(|| panic!("{text} not in {:?}", rows(t)));
    for _ in 0..idx {
        t.press("j");
    }
}

#[test]
fn tree_lists_files_under_their_folders() {
    let sb = tree_sandbox();
    let mut t = TestDriver::with_config(tree_config(), &sb);
    t.press("2");
    assert_eq!(
        rows(&t),
        vec![
            "▾ docs/guide/",
            "  ?? intro.md",
            "▾ src/",
            "  ▾ ui/",
            "     M view.rs",
            "  ?? lib.rs",
            "   M main.rs",
            "?? README.md",
        ]
    );
    insta::assert_snapshot!(t.pane_lines(PaneId::Status).join("\n"));
}

#[test]
fn backtick_switches_between_tree_and_flat() {
    let sb = tree_sandbox();
    let mut t = TestDriver::new(&sb);
    t.press("2");
    assert!(!t.app().status().tree());
    select_file(&mut t, "src/lib.rs");
    t.press("`");
    assert!(t.app().status().tree());
    assert!(t.app().notice().unwrap_or_default().contains("status as a tree"));
    // The same file stays selected.
    assert_eq!(t.app().status().selected().unwrap().path, "src/lib.rs");
    t.press("`");
    assert!(!t.app().status().tree());
    assert_eq!(rows(&t)[0], " M src/main.rs");
    assert_eq!(t.app().status().selected().unwrap().path, "src/lib.rs");
}

#[test]
fn enter_folds_a_folder_and_dash_and_equals_fold_them_all() {
    let sb = tree_sandbox();
    let mut t = TestDriver::with_config(tree_config(), &sb);
    select_row(&mut t, "▾ src/");
    t.press("enter");
    assert!(t.app().details().stage().is_none(), "enter on a folder doesn't stage");
    assert_eq!(
        rows(&t),
        vec!["▾ docs/guide/", "  ?? intro.md", "▸ src/", "?? README.md"]
    );
    assert_eq!(t.app().status().selected_folder(), Some("src/"));
    t.press("enter");
    assert_eq!(rows(&t).len(), 8);

    // From a file inside, collapsing all lands on its top folder.
    select_row(&mut t, "M view.rs");
    t.press("-");
    assert_eq!(rows(&t), vec!["▸ docs/guide/", "▸ src/", "?? README.md"]);
    assert_eq!(t.app().status().selected_folder(), Some("src/"));
    t.press("=");
    assert_eq!(rows(&t).len(), 8);

    // In the flat list there's nothing to fold.
    t.press("`").press("-");
    assert!(t.app().notice().unwrap_or_default().contains("tree view"));
}

#[test]
fn space_on_a_folder_stages_and_unstages_everything_under_it() {
    let sb = tree_sandbox();
    let mut t = TestDriver::with_config(tree_config(), &sb);
    select_row(&mut t, "▾ src/");
    t.press("space");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    let mut staged = codes(&t);
    staged.sort_by(|a, b| a.1.cmp(&b.1));
    let want: Vec<(String, String)> = [
        ("??", "README.md"),
        ("??", "docs/guide/intro.md"),
        ("A ", "src/lib.rs"),
        ("M ", "src/main.rs"),
        ("M ", "src/ui/view.rs"),
    ]
    .map(|(c, p)| (c.into(), p.into()))
    .into();
    assert_eq!(staged, want);
    assert_eq!(t.app().status().selected_folder(), Some("src/"));

    // Nothing left to stage under it: space unstages it all.
    t.press("space");
    let mut unstaged = codes(&t);
    unstaged.sort_by(|a, b| a.1.cmp(&b.1));
    let want: Vec<(String, String)> = [
        ("??", "README.md"),
        ("??", "docs/guide/intro.md"),
        ("??", "src/lib.rs"),
        (" M", "src/main.rs"),
        (" M", "src/ui/view.rs"),
    ]
    .map(|(c, p)| (c.into(), p.into()))
    .into();
    assert_eq!(unstaged, want);
}

#[test]
fn d_on_a_folder_discards_only_under_it() {
    let sb = tree_sandbox();
    let mut t = TestDriver::with_config(tree_config(), &sb);
    select_row(&mut t, "▾ ui/");
    t.press("d");
    let screen = t.screen();
    assert!(screen.contains("src/ui/"), "{screen}");
    t.press("y");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(
        std::fs::read_to_string(sb.path().join("src/ui/view.rs")).unwrap(),
        "view\n"
    );
    assert_eq!(
        std::fs::read_to_string(sb.path().join("src/main.rs")).unwrap(),
        "fn main() { run(); }\n"
    );
}

#[test]
fn i_on_a_folder_ignores_the_folder() {
    let sb = tree_sandbox();
    let mut t = TestDriver::with_config(tree_config(), &sb);
    select_row(&mut t, "▾ docs/guide/");
    t.press("i");
    let screen = t.screen();
    assert!(screen.contains("Add /docs/guide/ to .gitignore"), "{screen}");
    assert!(screen.contains("Add /docs/guide/ to .git/info/exclude"), "{screen}");
    t.press("enter");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(
        std::fs::read_to_string(sb.path().join(".gitignore")).unwrap(),
        "/docs/guide/\n"
    );
    assert!(!codes(&t).iter().any(|(_, p)| p.starts_with("docs/")));
}

#[test]
fn a_folder_shows_its_diff_in_details() {
    let sb = tree_sandbox();
    let mut t = TestDriver::with_config(tree_config(), &sb);
    select_row(&mut t, "▾ src/");
    assert_eq!(
        t.app().details().subject(),
        Some(&DetailsSubject::Folder("src/".into()))
    );
    t.assert_pane_contains(PaneId::Details, "+fn main() { run(); }");
    t.assert_pane_contains(PaneId::Details, "+view 2");
    t.assert_pane_contains(PaneId::Details, "src/lib.rs");
    assert!(t.pane_title(PaneId::Details).contains("Diff src/"));
    // e and b need a file.
    t.press("b");
    assert!(t.app().notice().unwrap_or_default().contains("no file"));
}

#[test]
fn stepping_files_from_details_skips_folders() {
    let sb = tree_sandbox();
    let mut t = TestDriver::with_config(tree_config(), &sb);
    select_row(&mut t, "?? intro.md");
    t.press("enter");
    assert!(t.app().details().stage().is_some());
    t.press("]");
    assert_eq!(t.app().status().selected().unwrap().path, "src/ui/view.rs");
    t.press("[");
    assert_eq!(t.app().status().selected().unwrap().path, "docs/guide/intro.md");
}

#[test]
fn a_folder_diff_follows_staging_it() {
    let sb = tree_sandbox();
    let mut t = TestDriver::with_config(tree_config(), &sb);
    select_row(&mut t, "▾ ui/");
    t.assert_pane_contains(PaneId::Details, "── unstaged ──");
    t.press("space");
    t.assert_pane_contains(PaneId::Details, "── staged ──");
    assert!(
        !t.pane_lines(PaneId::Details)
            .iter()
            .any(|l| l.contains("── unstaged ──")),
        "{:?}",
        t.pane_lines(PaneId::Details)
    );
}

#[test]
fn the_tree_or_flat_choice_is_saved_to_config() {
    let sb = tree_sandbox();
    let config_file = sb.root().join("tt-config").join("config.toml");
    let mut t = TestDriver::new(&sb);
    t.app_mut().enable_themes(tt_git_tui::themes::ThemeOptions {
        dirs: Default::default(),
        config_path: Some(config_file.clone()),
    });
    t.press("2").press("`");
    assert!(t.app().error().is_none(), "{:?}", t.app().error());
    assert_eq!(
        std::fs::read_to_string(&config_file).unwrap(),
        "[status]\ntree = true\n"
    );
    assert!(Config::load(&config_file).unwrap().status.tree);
    t.press("`");
    assert_eq!(
        std::fs::read_to_string(&config_file).unwrap(),
        "[status]\ntree = false\n"
    );
}
