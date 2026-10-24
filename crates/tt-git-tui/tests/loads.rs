//! Which git loads run: graph reloads skipped when refs changed nothing the
//! graph walks, and details loads that follow the selection one at a time
//! and only while details is on screen.

use std::time::{Duration, SystemTime};

use tt_git_core::git::LoggedCommand;
use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::panes::{BranchTab, DetailsSubject, PaneId};
use tt_git_tui::{Action, Config};

fn commands(t: &TestDriver) -> Vec<LoggedCommand> {
    t.app().repo().git().log().entries()
}

/// The commands logged after the first `skip`, whose args start with `prefix`.
fn since(t: &TestDriver, skip: usize, prefix: &str) -> Vec<LoggedCommand> {
    commands(t)
        .into_iter()
        .skip(skip)
        .filter(|c| c.args.starts_with(prefix))
        .collect()
}

const GRAPH_LOG: &str = "log --topo-order";
const SHOW: &str = "show \"--format=commit";

fn refs_changed(t: &mut TestDriver) -> &mut TestDriver {
    t.send(Action::RepoChanged {
        refs: true,
        status: false,
    })
}

fn messages(t: &TestDriver) -> Vec<String> {
    t.app()
        .graph()
        .commits()
        .iter()
        .map(|c| c.summary.to_string())
        .collect()
}

/// Asserts no two of `cmds` ran at the same time.
#[track_caller]
fn one_at_a_time(cmds: &[LoggedCommand]) {
    // Logged start times are wall-clock and taken a moment after the
    // duration's start; allow for that.
    const SLACK: Duration = Duration::from_millis(2);
    let mut spans: Vec<(SystemTime, SystemTime)> = cmds
        .iter()
        .map(|c| (c.started, c.started + c.duration.expect("finished")))
        .collect();
    spans.sort();
    for pair in spans.windows(2) {
        assert!(pair[0].1 <= pair[1].0 + SLACK, "overlapping: {cmds:#?}");
    }
}

#[test]
fn a_refs_change_that_moves_nothing_skips_the_graph_log() {
    let sb = Sandbox::new();
    for i in 0..3 {
        sb.commit(&format!("commit {i}"));
    }
    let mut t = TestDriver::new(&sb);
    let start = commands(&t).len();

    // E.g. a fetch that brought nothing, or a reflog append.
    refs_changed(&mut t);
    assert!(since(&t, start, GRAPH_LOG).is_empty(), "{:#?}", commands(&t));
    assert!(!since(&t, start, "for-each-ref").is_empty(), "refs still reload");
    assert_eq!(messages(&t), ["commit 2", "commit 1", "commit 0"]);

    // A branch moving does reload.
    let start = commands(&t).len();
    sb.commit("commit 3");
    refs_changed(&mut t);
    assert_eq!(since(&t, start, GRAPH_LOG).len(), 1);
    assert_eq!(messages(&t)[0], "commit 3");

    // As does a new branch on an old commit: it adds no commits, but the
    // graph doesn't know that without looking.
    let start = commands(&t).len();
    sb.git(&["branch", "side", "HEAD~2"]);
    refs_changed(&mut t);
    assert_eq!(since(&t, start, GRAPH_LOG).len(), 1);
}

#[test]
fn switching_to_a_branch_on_a_shown_commit_reloads_the_current_branch_graph() {
    let sb = Sandbox::new();
    sb.commit("a");
    sb.branch("feature");
    sb.commit("b");
    let mut t = TestDriver::new(&sb);
    t.press("a");
    assert_eq!(messages(&t), ["b", "a"]);

    // Every ref still points where it did; only HEAD moved.
    let start = commands(&t).len();
    sb.checkout("feature");
    refs_changed(&mut t);
    assert_eq!(since(&t, start, GRAPH_LOG).len(), 1);
    assert_eq!(messages(&t), ["a"]);
}

#[test]
fn deepening_a_shallow_history_reloads_the_graph() {
    let sb = Sandbox::new();
    let first = sb.commit("first");
    let second = sb.commit("second");
    sb.commit("third");
    let shallow = sb.path().join(".git").join("shallow");
    std::fs::write(&shallow, format!("{second}\n")).unwrap();
    let mut t = TestDriver::new(&sb);
    assert_eq!(messages(&t), ["third", "second"]);

    // As `git fetch --deepen` does: no tip moves, and FETCH_HEAD (a refs
    // change to the watcher) is written.
    let start = commands(&t).len();
    std::fs::write(&shallow, format!("{first}\n")).unwrap();
    refs_changed(&mut t);
    assert_eq!(since(&t, start, GRAPH_LOG).len(), 1);
    assert_eq!(messages(&t), ["third", "second", "first"]);
}

#[test]
fn a_new_stash_reloads_the_graph() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "one\n", "base");
    let mut t = TestDriver::new(&sb);
    let start = commands(&t).len();
    sb.write("a.txt", "two\n");
    sb.git(&["stash", "-q"]);
    refs_changed(&mut t);
    assert_eq!(since(&t, start, GRAPH_LOG).len(), 1);
}

#[test]
fn a_skipped_reload_leaves_the_graph_as_a_reload_would() {
    let sb = Sandbox::new();
    for i in 0..30 {
        sb.commit(&format!("commit {i}"));
    }
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 20);
    // Far enough down that a reload's recentring shows.
    t.press_all(&["j"; 15]);
    let selected = t.app().graph().selected().unwrap().oid.to_string();
    let screen = t.screen();

    let start = commands(&t).len();
    refs_changed(&mut t);
    assert!(since(&t, start, GRAPH_LOG).is_empty());
    let skipped = t.screen();

    // An explicit refresh always reloads; it must end up the same.
    t.press("ctrl+r");
    assert_eq!(t.app().graph().selected().unwrap().oid, selected);
    assert_eq!(skipped, t.screen());
    assert_ne!(screen, skipped, "recentred on the selection");
}

#[test]
fn details_load_nothing_while_hidden_and_the_selection_once_shown() {
    let sb = Sandbox::new();
    for i in 0..5 {
        sb.commit(&format!("commit {i}"));
    }
    let mut t = TestDriver::new(&sb);
    assert!(!t.app().shows(PaneId::Details));
    let start = commands(&t).len();
    t.press_all(&["j", "j", "j"]);
    assert!(since(&t, start, "show").is_empty(), "{:#?}", commands(&t));
    // Details still knows what it would show.
    let oid = t.app().graph().selected().unwrap().oid.to_string();
    assert_eq!(t.app().details().subject(), Some(&DetailsSubject::Commit(oid.clone())));

    t.press("space");
    let shows = since(&t, start, "show");
    assert_eq!(shows.len(), 1, "{shows:#?}");
    assert!(shows[0].args.contains(&oid));
    t.assert_pane_contains(PaneId::Details, "commit 1");

    // Moving with details open loads as it goes; closing stops that.
    t.press("j");
    t.assert_pane_contains(PaneId::Details, "commit 0");
    t.press("space").press("k");
    assert_eq!(since(&t, start, "show").len(), 2);
    t.press("space");
    assert_eq!(since(&t, start, "show").len(), 3);
    t.assert_pane_contains(PaneId::Details, "commit 1");
}

#[test]
fn rapid_moves_run_one_git_show_at_a_time_and_end_on_the_last() {
    let sb = Sandbox::new();
    for i in 0..12 {
        sb.commit(&format!("commit {i}"));
    }
    let mut t = TestDriver::new(&sb);
    t.press("space");
    let start = commands(&t).len();
    for _ in 0..8 {
        t.press_frame("j");
    }
    t.wait_idle();
    let shows = since(&t, start, SHOW);
    assert!(!shows.is_empty() && shows.len() <= 8, "{shows:#?}");
    one_at_a_time(&shows);
    let oid = t.app().graph().selected().unwrap().oid.to_string();
    assert_eq!(t.app().details().subject(), Some(&DetailsSubject::Commit(oid.clone())));
    assert!(shows.last().unwrap().args.contains(&oid));
    t.assert_pane_contains(PaneId::Details, "commit 3");
}

#[test]
fn rapid_moves_through_unmerged_branches_end_on_the_last() {
    let sb = Sandbox::new();
    sb.commit("base");
    for name in ["one", "two", "three", "four"] {
        sb.checkout_new(name);
        sb.commit(&format!("{name} work"));
        sb.checkout("main");
    }
    sb.add_remote("origin");
    sb.git(&["branch", "-q", "-D", "one", "two", "three", "four"]);
    let mut t = TestDriver::with_size(Config::default(), &sb, 160, 40);
    t.press("3").press("]").press("]");
    assert_eq!(t.app().branches().tab(), BranchTab::Unmerged);

    for key in ["j", "j", "k", "j", "j"] {
        t.press_frame(key);
    }
    t.wait_idle();
    let focus = t.app().branches().focus().expect("a branch is focused");
    assert_eq!(t.app().graph().focus.as_ref(), Some(&focus));
    assert!(!t.app().graph().is_loading());
    let work = format!("{} work", focus.branch.trim_start_matches("origin/"));
    assert_eq!(messages(&t)[0], work);
    assert_eq!(t.app().graph().selected().map(|c| c.oid.to_string()), Some(focus.tip));
}

/// The args of the commands logged after the first `skip`.
fn args_since(t: &TestDriver, skip: usize) -> Vec<String> {
    commands(t).into_iter().skip(skip).map(|c| c.args).collect()
}

#[test]
fn a_refs_load_reads_each_thing_with_one_command() {
    let sb = Sandbox::new();
    sb.commit("base");
    for i in 0..6 {
        sb.tick();
        sb.commit(&format!("release {i}"));
        sb.git(&["tag", "-a", &format!("v{i}"), "-m", "release"]);
    }
    sb.checkout_new("feature");
    sb.git(&[
        "-c",
        "user.name=Other Person",
        "-c",
        "user.email=other@example.com",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "theirs",
    ]);
    sb.checkout("main");
    sb.add_remote("origin");
    sb.write(
        ".mailmap",
        "Real Name <real@example.com> Test User <test@example.com>\n",
    );
    let mut t = TestDriver::new(&sb);
    let base = t.app().branches().base().cloned().expect("a base branch");
    assert_eq!(base.tags.len(), 5, "{base:?}");

    let start = commands(&t).len();
    refs_changed(&mut t);
    let args = args_since(&t, start);
    let count = |prefix: &str| args.iter().filter(|a| a.starts_with(prefix)).count();
    // `git remote` and the base setting's `config --get` are one config read.
    assert_eq!(count("remote"), 0, "{args:#?}");
    assert_eq!(count("config --get "), 0, "{args:#?}");
    assert_eq!(count("config -z --get-regexp"), 1, "{args:#?}");
    // The refs (with their authors), the merged ones, and the tags' distance
    // from the base (rather than a rev-list each).
    assert_eq!(count("for-each-ref"), 3, "{args:#?}");
    assert_eq!(count("rev-list"), 0, "{args:#?}");
    assert_eq!(count("check-mailmap"), 1, "{args:#?}");
    // Same result as before.
    assert_eq!(t.app().branches().base(), Some(&base));
    assert_eq!(base.authors["refs/remotes/origin/main"], "Real Name");
    assert_eq!(base.authors["refs/remotes/origin/feature"], "Other Person");
}

#[test]
fn a_build_in_an_ignored_dir_does_not_reload_the_status() {
    let sb = Sandbox::new();
    sb.write(
        ".gitignore",
        "target/
",
    );
    sb.git(&["add", ".gitignore"]);
    sb.commit_staged("base");
    sb.write("target/debug/first", "1");
    let mut t = TestDriver::with_watcher(Config::default(), &sb);
    let runs = |t: &TestDriver, prefix: &str| commands(t).iter().filter(|c| c.args.starts_with(prefix)).count();

    // The first change there has the watcher load the ignore rules.
    sb.write("target/debug/one", "1");
    t.wait_until(Duration::from_secs(10), |app| {
        app.repo()
            .git()
            .log()
            .entries()
            .iter()
            .any(|c| c.args.starts_with("ls-files"))
    });
    std::thread::sleep(Duration::from_millis(800));
    t.wait_idle();

    let statuses = runs(&t, "status ");
    let loads = runs(&t, "ls-files");
    for i in 0..5 {
        sb.write(&format!("target/debug/step{i}"), "x");
        sb.write(&format!("target/debug/deps/{i}.o"), "x");
        std::thread::sleep(Duration::from_millis(100));
    }
    std::fs::remove_file(sb.path().join("target/debug/one")).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    t.wait_idle();
    assert_eq!(runs(&t, "status "), statuses, "{:#?}", commands(&t));
    assert_eq!(runs(&t, "ls-files"), loads, "rules loaded once");

    // Anything else still reloads, with the status as plain as ever.
    sb.write("visible.txt", "x");
    t.wait_until(Duration::from_secs(10), |app| {
        app.status().entries().iter().any(|e| e.path == "visible.txt")
    });
    assert!(runs(&t, "status ") > statuses);
    assert!(commands(&t).iter().all(|c| !c.args.contains("--ignored")));
}

#[test]
fn without_a_watcher_no_rules_are_loaded() {
    let sb = Sandbox::new();
    sb.write(
        ".gitignore",
        "target/
",
    );
    sb.git(&["add", ".gitignore"]);
    sb.commit_staged("base");
    sb.write("target/debug/first", "1");
    let mut t = TestDriver::new(&sb);
    for i in 0..3 {
        sb.write(&format!("target/debug/step{i}"), "x");
        t.send(Action::Refresh);
    }
    sb.write("src.txt", "x");
    t.send(Action::Refresh);
    assert!(t.app().status().entries().iter().any(|e| e.path == "src.txt"));
    assert!(
        commands(&t).iter().all(|c| !c.args.starts_with("ls-files")),
        "{:#?}",
        commands(&t)
    );
}

/// About 30 commits on three long-lived branches merged back now and then,
/// so lines of history are open across any cut.
fn branchy(sb: &Sandbox) {
    sb.commit("root");
    for round in 0..6 {
        for b in ["a", "b", "c"] {
            if round == 0 {
                sb.checkout_new(b);
            } else {
                sb.checkout(b);
            }
            sb.commit(&format!("{b} {round}"));
            sb.checkout("main");
        }
        sb.commit(&format!("main {round}"));
        if round % 2 == 1 {
            sb.merge("a");
        }
    }
}

/// The graph's commits, rows and screen.
fn graph_state(t: &TestDriver) -> (Vec<String>, String, usize, String) {
    let graph = t.app().graph();
    let oids = graph.commits().iter().map(|c| c.oid.to_string()).collect();
    (oids, format!("{:?}", graph.rows()), graph.selected_index(), t.screen())
}

fn small_limit(sb: &Sandbox) -> TestDriver {
    let cfg = Config::parse("[graph]\nmax_commits = 10\n").unwrap();
    TestDriver::with_size(cfg, sb, 120, 20)
}

#[test]
fn loading_more_streams_only_the_rest_and_ends_as_a_full_load() {
    let sb = Sandbox::new();
    branchy(&sb);
    let mut t = small_limit(&sb);
    assert_eq!(t.app().graph().commits().len(), 10);

    let start = commands(&t).len();
    t.press("G");
    assert_eq!(t.app().graph().commits().len(), 20);
    let logs = since(&t, start, GRAPH_LOG);
    assert_eq!(logs.len(), 1, "{logs:#?}");
    assert!(logs[0].args.contains("--max-count=10 --skip=10"), "{logs:#?}");
    let more = graph_state(&t);
    assert_eq!(more.2, 9, "still on the commit it was on");

    // A refresh lists all 20 from the start; it must end up the same.
    let start = commands(&t).len();
    t.press("ctrl+r");
    let logs = since(&t, start, GRAPH_LOG);
    assert!(logs.iter().all(|c| !c.args.contains("--skip")), "{logs:#?}");
    assert_eq!(graph_state(&t), more);

    // And again past the end of the history.
    t.press("G");
    let more = graph_state(&t);
    assert_eq!(more.0.len(), messages(&t).len());
    assert!(more.0.len() > 20 && more.0.len() < 30, "{}", more.0.len());
    t.press("ctrl+r");
    assert_eq!(graph_state(&t), more);
}

#[test]
fn loading_more_after_the_refs_changed_lists_everything_again() {
    let sb = Sandbox::new();
    branchy(&sb);
    let mut t = small_limit(&sb);
    // A commit the graph hasn't heard of yet.
    sb.checkout("b");
    sb.commit("b late");

    let start = commands(&t).len();
    t.press("G");
    let logs = since(&t, start, GRAPH_LOG);
    assert_eq!(logs.len(), 1, "{logs:#?}");
    assert!(!logs[0].args.contains("--skip"), "{logs:#?}");
    assert_eq!(messages(&t)[0], "b late");
    assert_eq!(t.app().graph().commits().len(), 20);
    let more = graph_state(&t);

    // The refresh also shows the checkout in the other panes; the graph
    // must end up the same.
    t.press("ctrl+r");
    let (oids, rows, selected, _) = graph_state(&t);
    assert_eq!((oids, rows, selected), (more.0, more.1, more.2));
}
