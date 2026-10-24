use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::app::Zoom;
use tt_git_tui::panes::{DateStyle, DetailsSubject, GraphScope, PaneId};

#[test]
fn initial_layout_empty_repo() {
    let sb = Sandbox::new();
    let t = TestDriver::with_size(Config::default(), &sb, 100, 24);
    assert_eq!(t.app().focus(), PaneId::Graph);
    insta::assert_snapshot!(t.screen());
}

#[test]
fn focus_keys_and_cycle() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.press("2");
    assert_eq!(t.app().focus(), PaneId::Status);
    t.press("tab");
    assert_eq!(t.app().focus(), PaneId::Branches);
    // Details is closed, so cycling skips it.
    t.press("shift+tab").press("shift+tab");
    assert_eq!(t.app().focus(), PaneId::Graph);
    t.press("tab");
    assert_eq!(t.app().focus(), PaneId::Status);
    // Focusing details opens it in place of status and branches.
    t.press("4");
    assert_eq!(t.app().focus(), PaneId::Details);
    assert!(t.app().shows(PaneId::Details));
    assert!(!t.app().shows(PaneId::Status));
    t.press("1");
    assert_eq!(t.app().focus(), PaneId::Graph);
}

#[test]
fn ctrl_hjkl_moves_between_panes_by_position() {
    let sb = branchy_repo();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 24);
    // Default layout: graph on top, status bottom left, branches bottom right.
    t.press("ctrl+j");
    assert_eq!(t.app().focus(), PaneId::Status);
    t.press("ctrl+l");
    assert_eq!(t.app().focus(), PaneId::Branches);
    t.press("ctrl+l"); // nothing further right
    assert_eq!(t.app().focus(), PaneId::Branches);
    t.press("ctrl+h");
    assert_eq!(t.app().focus(), PaneId::Status);
    t.press("ctrl+k");
    assert_eq!(t.app().focus(), PaneId::Graph);
    t.press("ctrl+k");
    assert_eq!(t.app().focus(), PaneId::Graph);

    // With details open below the graph, ctrl+j goes there.
    t.press("space").press("ctrl+j");
    assert_eq!(t.app().focus(), PaneId::Details);
    t.press("ctrl+k");
    assert_eq!(t.app().focus(), PaneId::Graph);
}

#[test]
fn space_toggles_details_over_the_bottom_row() {
    let sb = branchy_repo();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 40);
    assert!(t.app().pane_area(PaneId::Details).is_none());
    let status = t.app().pane_area(PaneId::Status).unwrap();
    let branches = t.app().pane_area(PaneId::Branches).unwrap();

    t.press("space");
    assert_eq!(t.app().focus(), PaneId::Graph);
    let details = t.app().pane_area(PaneId::Details).unwrap();
    assert_eq!(details, status.union(branches));
    assert!(t.app().pane_area(PaneId::Status).is_none());
    t.assert_pane_contains(PaneId::Details, "side experiment");

    t.press("space");
    assert!(t.app().pane_area(PaneId::Details).is_none());
    assert_eq!(t.app().pane_area(PaneId::Status), Some(status));

    // esc in details closes it and returns to the graph.
    t.press("space").press("4").press("esc");
    assert!(!t.app().shows(PaneId::Details));
    assert_eq!(t.app().focus(), PaneId::Graph);

    // So does space, the key that opened it.
    t.press("space").press("4").press("space");
    assert!(!t.app().shows(PaneId::Details));
    assert_eq!(t.app().focus(), PaneId::Graph);

    // And esc in the graph while details is showing.
    t.press("space").press("esc");
    assert!(!t.app().shows(PaneId::Details));
    assert_eq!(t.app().focus(), PaneId::Graph);
    assert_eq!(t.app().pane_area(PaneId::Status), Some(status));

    // Focusing a pane details covers closes it.
    t.press("space").press("3");
    assert_eq!(t.app().focus(), PaneId::Branches);
    assert!(!t.app().shows(PaneId::Details));
}

#[test]
fn custom_layout_from_config() {
    let sb = Sandbox::new();
    let cfg = Config::parse(
        r#"
        [layout]
        split = "columns"
        children = [
          { pane = "branches", size = 25 },
          { split = "rows", children = [{ pane = "graph" }, { pane = "status" }, { pane = "details" }] },
        ]
        "#,
    )
    .unwrap();
    let t = TestDriver::with_size(cfg, &sb, 100, 24);
    let branches = t.app().pane_area(PaneId::Branches).unwrap();
    assert_eq!((branches.x, branches.width, branches.height), (0, 25, 23));
    for pane in [PaneId::Graph, PaneId::Status, PaneId::Details] {
        let area = t.app().pane_area(pane).unwrap();
        assert_eq!((area.x, area.width), (25, 75), "{pane:?}");
    }
}

#[test]
fn help_popup_lists_bindings() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.press("?");
    let screen = t.screen();
    assert!(screen.contains("Keybindings"), "{screen}");
    assert!(
        screen.contains(concat!("tt-git ", env!("CARGO_PKG_VERSION"))),
        "{screen}"
    );
    assert!(screen.contains("all/current"), "{screen}");
    // Keys don't leak through to panes while the popup is open.
    t.press("a").press("esc");
    assert_eq!(t.app().graph().scope, GraphScope::All);
    assert!(!t.screen().contains("Keybindings"));
}

#[test]
fn help_popup_filters_as_you_type() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.press("?").type_text("cherry");
    let labels: Vec<_> = t.app().help_rows().into_iter().map(|(_, l)| l).collect();
    assert_eq!(labels, ["cherry-pick"]);
    assert!(t.screen().contains("> cherry"), "{}", t.screen());

    // Typing q searches instead of quitting.
    t.press("ctrl+u").press("q");
    assert!(!t.app().should_quit());

    // Keys match as well as labels.
    t.press("backspace").type_text("ctrl+d");
    let labels: Vec<_> = t.app().help_rows().into_iter().map(|(_, l)| l).collect();
    assert_eq!(labels, ["page down"]);

    // With nothing typed, `?` closes the popup again.
    t.press("ctrl+u").press("?");
    assert!(!t.screen().contains("Keybindings"));
    t.press("?");
    assert!(t.screen().contains("type to search"), "{}", t.screen());
}

#[test]
fn help_popup_lists_push_and_pull() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.press("?");
    let rows = t.app().help_rows();
    assert!(rows.contains(&("p".to_string(), "pull")), "{rows:?}");
    assert!(rows.contains(&("P".to_string(), "push")), "{rows:?}");
}

#[test]
fn quit() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    assert!(!t.app().should_quit());
    t.press("q");
    assert!(t.app().should_quit());
}

#[test]
fn user_keybindings_apply() {
    let sb = Sandbox::new();
    let cfg = Config::parse("[keys.graph]\ntoggle_scope = \"b\"\n").unwrap();
    let mut t = TestDriver::with_config(cfg, &sb);
    t.press("a");
    assert_eq!(t.app().graph().scope, GraphScope::All);
    t.press("b");
    assert_eq!(t.app().graph().scope, GraphScope::Current);
    assert!(t.screen().contains("b all/current"));
}

/// main: base ─ m1 ─ merge(feature) ; feature: f1 ; side: s1 (unmerged, newest)
fn branchy_repo() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("feature");
    sb.commit("feature work");
    sb.checkout("main");
    sb.commit("main work");
    sb.merge("feature");
    sb.git(&["tag", "v1.0"]);
    sb.checkout_new("side");
    sb.commit("side experiment");
    sb.checkout("main");
    sb
}

#[test]
fn graph_shows_all_branches_with_labels() {
    let sb = branchy_repo();
    let t = TestDriver::with_size(Config::default(), &sb, 120, 30);
    let graph = t.app().graph();
    assert_eq!(graph.commits().len(), 5);
    assert_eq!(&*graph.commits()[0].summary, "side experiment");
    insta::assert_snapshot!(t.pane_lines(PaneId::Graph).join("\n"));
    assert!(t.pane_title(PaneId::Graph).contains("Graph [all] 1/5"));
}

#[test]
fn toggle_to_current_branch_hides_other_branches() {
    let sb = branchy_repo();
    let mut t = TestDriver::new(&sb);
    // Select "main work" so we can check the selection survives the reload.
    let target = t
        .app()
        .graph()
        .commits()
        .iter()
        .position(|c| &*c.summary == "main work")
        .unwrap();
    for _ in 0..target {
        t.press("j");
    }
    assert_eq!(&*t.app().graph().selected().unwrap().summary, "main work");

    t.press("a");
    let graph = t.app().graph();
    assert_eq!(graph.scope, GraphScope::Current);
    assert!(graph.commits().iter().all(|c| &*c.summary != "side experiment"));
    assert_eq!(graph.commits().len(), 4);
    assert_eq!(&*graph.selected().unwrap().summary, "main work", "selection kept");
    assert!(t.pane_title(PaneId::Graph).contains("Graph [current]"));

    t.press("a");
    assert_eq!(t.app().graph().commits().len(), 5);
    assert_eq!(&*t.app().graph().selected().unwrap().summary, "main work");
}

#[test]
fn t_switches_the_graph_to_clock_times_and_back() {
    let sb = branchy_repo();
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 30);
    assert!(t.pane_lines(PaneId::Graph)[0].ends_with(" 23h b66496d"));

    // The test clock is UTC; the sandbox's commits are a minute apart from
    // 2024-01-01 00:00.
    t.press("t");
    assert_eq!(t.app().graph().dates, DateStyle::Clock);
    insta::assert_snapshot!(t.pane_lines(PaneId::Graph).join(
        "
"
    ));

    t.press("t");
    assert_eq!(t.app().graph().dates, DateStyle::Relative);
    assert!(t.pane_lines(PaneId::Graph)[0].ends_with(" 23h b66496d"));
}

#[test]
fn shift_l_hides_the_graph_lanes_and_back() {
    let sb = branchy_repo();
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 30);
    let with_lanes = t.pane_lines(PaneId::Graph);
    assert!(with_lanes[0].starts_with('●'));

    t.press("L");
    assert!(!t.app().graph().lanes);
    insta::assert_snapshot!(t.pane_lines(PaneId::Graph).join("\n"));

    t.press("L");
    assert!(t.app().graph().lanes);
    assert_eq!(t.pane_lines(PaneId::Graph), with_lanes);
}

#[test]
fn config_starts_the_graph_without_lanes() {
    let sb = branchy_repo();
    let cfg = Config::parse("[graph]\nlanes = false\n").unwrap();
    let t = TestDriver::with_config(cfg, &sb);
    assert!(!t.app().graph().lanes);
    assert!(
        !t.pane_lines(PaneId::Graph)
            .iter()
            .any(|l| l.contains('●') || l.contains('│'))
    );
}

#[test]
fn config_starts_the_graph_on_clock_times() {
    let sb = branchy_repo();
    let cfg = Config::parse(
        "[graph]
dates = \"clock\"
",
    )
    .unwrap();
    let t = TestDriver::with_config(cfg, &sb);
    assert_eq!(t.app().graph().dates, DateStyle::Clock);
    assert!(t.pane_lines(PaneId::Graph)[0].contains("2024-01-01 12:0"));
}

#[test]
fn details_follow_the_selection() {
    let sb = branchy_repo();
    let mut t = TestDriver::new(&sb);
    t.press("space");
    t.assert_pane_contains(PaneId::Details, "side experiment");
    t.assert_pane_contains(PaneId::Details, "Author: Test User <test@example.com>");
    t.press("j");
    let selected = t.app().graph().selected().unwrap().summary.to_string();
    t.assert_pane_contains(PaneId::Details, &selected);
    assert!(t.pane_title(PaneId::Details).contains("Commit "));

    // Scrolling the details pane.
    t.press("4");
    let before = t.pane_lines(PaneId::Details);
    t.press("j");
    let after = t.pane_lines(PaneId::Details);
    if t.app().details().lines().len() > before.len() {
        assert_ne!(before, after);
    }
}

#[test]
fn merge_commit_details_show_first_parent_diff() {
    let sb = branchy_repo();
    let mut t = TestDriver::new(&sb);
    let idx = t
        .app()
        .graph()
        .commits()
        .iter()
        .position(|c| c.parents.len() == 2)
        .unwrap();
    t.press("space");
    for _ in 0..idx {
        t.press("j");
    }
    t.assert_pane_contains(PaneId::Details, "Merge branch 'feature'");
    t.assert_pane_contains(PaneId::Details, "feature-work.txt");
}

#[test]
fn status_title_shows_branch() {
    let sb = branchy_repo();
    sb.write("dirty.txt", "x");
    let t = TestDriver::new(&sb);
    assert!(
        t.pane_title(PaneId::Status).contains("Status: repo · main"),
        "{}",
        t.pane_title(PaneId::Status)
    );
    t.assert_pane_contains(PaneId::Status, "?? dirty.txt");
}

#[test]
fn small_graph_limit_loads_more_when_scrolling() {
    let sb = Sandbox::new();
    for i in 0..30 {
        sb.commit(&format!("commit {i}"));
    }
    let cfg = Config::parse("[graph]\nmax_commits = 10\n").unwrap();
    let mut t = TestDriver::with_config(cfg, &sb);
    assert_eq!(t.app().graph().commits().len(), 10);
    t.press("G");
    assert!(t.app().graph().commits().len() > 10);
    t.press("G").press("G");
    assert_eq!(t.app().graph().commits().len(), 30);
}

#[test]
fn details_respect_mailmap() {
    let sb = Sandbox::new();
    sb.commit("one");
    sb.write(
        ".mailmap",
        "Real Name <real@example.com> Test User <test@example.com>\n",
    );
    let mut t = TestDriver::new(&sb);
    t.press("space");
    t.assert_pane_contains(PaneId::Details, "Author: Real Name <real@example.com>");
    t.assert_pane_contains(PaneId::Details, "Committer: Real Name <real@example.com>");
    t.assert_pane_contains(PaneId::Graph, "Real Name");
}

#[test]
fn c_copies_the_commit_hash_or_branch_name() {
    let sb = Sandbox::new();
    sb.commit("one");
    sb.checkout_new("feature/copy-me");
    let oid = sb.commit("two");
    let mut t = TestDriver::new(&sb);

    t.press("c");
    assert_eq!(t.app_mut().take_clipboard().as_deref(), Some(oid.as_str()));
    assert_eq!(t.app().notice(), Some(format!("copied {oid}").as_str()));

    t.press("3").press("g");
    assert_eq!(t.app().branches().selected().unwrap().name, "feature/copy-me");
    t.press("c");
    assert_eq!(t.app_mut().take_clipboard().as_deref(), Some("feature/copy-me"));
}

#[test]
fn zoom_puts_the_graph_fullscreen_and_back() {
    let sb = Sandbox::new();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 24);
    let normal_graph = t.app().pane_area(PaneId::Graph).unwrap();

    // One `+` from the graph fills the screen, even with details open.
    t.press("space").press("1").press("+");
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Graph));
    assert_eq!(t.app().focus(), PaneId::Graph);
    let graph = t.app().pane_area(PaneId::Graph).unwrap();
    assert_eq!((graph.width, graph.height), (100, 23));
    assert!(t.app().pane_area(PaneId::Details).is_none());
    assert!(t.app().pane_area(PaneId::Status).is_none());

    t.press("+");
    assert_eq!(t.app().zoom(), Zoom::Off);
    assert_eq!(t.app().pane_area(PaneId::Graph).unwrap(), normal_graph);
    assert!(t.app().pane_area(PaneId::Details).is_some());
}

#[test]
fn space_in_a_fullscreen_graph_opens_details_below_it() {
    let sb = Sandbox::new();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 24);
    t.press("+");
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Graph));

    // Wide: graph and details span the screen, the left column is gone.
    t.press("space");
    assert_eq!(t.app().zoom(), Zoom::Wide);
    let graph = t.app().pane_area(PaneId::Graph).unwrap();
    let details = t.app().pane_area(PaneId::Details).unwrap();
    assert_eq!((graph.x, graph.width), (0, 100));
    assert_eq!((details.x, details.width), (0, 100));
    assert!(t.app().pane_area(PaneId::Status).is_none());
    assert!(t.app().pane_area(PaneId::Branches).is_none());

    // Closing details again keeps the graph zoomed; `+` goes back.
    t.press("space");
    assert_eq!(t.app().zoom(), Zoom::Wide);
    let graph = t.app().pane_area(PaneId::Graph).unwrap();
    assert_eq!((graph.width, graph.height), (100, 23));
    t.press("+");
    assert_eq!(t.app().zoom(), Zoom::Off);
}

#[test]
fn zoom_from_details_goes_straight_to_fullscreen() {
    let sb = Sandbox::new();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 24);
    t.press("space").press("4").press("+");
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Details));
    assert_eq!(t.app().focus(), PaneId::Details);
    let details = t.app().pane_area(PaneId::Details).unwrap();
    assert_eq!((details.width, details.height), (100, 23));
    assert!(t.app().pane_area(PaneId::Graph).is_none());

    t.press("+");
    assert_eq!(t.app().zoom(), Zoom::Off);
    assert_eq!(t.app().focus(), PaneId::Details);
}

#[test]
fn focusing_a_hidden_pane_unzooms() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.press("+");
    t.press("4");
    assert_eq!(t.app().zoom(), Zoom::Off);
    t.press("+").press("3");
    assert_eq!(t.app().zoom(), Zoom::Off);
    assert!(t.app().pane_area(PaneId::Status).is_some());
}

#[test]
fn enter_opens_files_beside_a_diff() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "one\n", "base");
    sb.write("a.txt", "two\n").write("b.txt", "bee\n");
    sb.git(&["add", "."]);
    let oid = sb.commit_staged("touch two files");
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);

    t.press("enter");
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Details));
    assert_eq!(t.app().focus(), PaneId::Details);
    assert_eq!(
        t.app().details().subject(),
        Some(&DetailsSubject::CommitFiles(oid.clone()))
    );
    let paths: Vec<_> = t
        .app()
        .details()
        .commit_files()
        .unwrap()
        .iter()
        .map(|f| f.path.clone())
        .collect();
    assert_eq!(paths, ["a.txt", "b.txt"]);
    t.assert_pane_contains(PaneId::Details, "Files (2)");
    t.assert_pane_contains(PaneId::Details, "touch two files");
    t.assert_pane_contains(PaneId::Details, "+two");

    // j picks the next file and shows its diff.
    t.press("j");
    assert_eq!(t.app().details().selected_file().unwrap().path, "b.txt");
    t.assert_pane_contains(PaneId::Details, "+bee");
    assert!(!t.pane_lines(PaneId::Details).iter().any(|l| l.contains("+two")));

    // esc goes back to the normal layout with the plain commit details.
    t.press("esc");
    assert_eq!(t.app().zoom(), Zoom::Off);
    assert_eq!(t.app().focus(), PaneId::Graph);
    assert!(!t.app().shows(PaneId::Details));
    t.press("space");
    assert_eq!(t.app().details().subject(), Some(&DetailsSubject::Commit(oid.clone())));
    t.assert_pane_contains(PaneId::Details, "touch two files");

    // space closes the fullscreen files view too.
    t.press("space").press("enter");
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Details));
    t.press("space");
    assert_eq!(t.app().zoom(), Zoom::Off);
    assert_eq!(t.app().focus(), PaneId::Graph);
    assert!(!t.app().shows(PaneId::Details));
    assert_eq!(t.app().details().subject(), Some(&DetailsSubject::Commit(oid)));
}

#[test]
fn brackets_in_details_step_through_the_graph() {
    let sb = Sandbox::new();
    let first = sb.commit_file("a.txt", "a\n", "first");
    let second = sb.commit_file("a.txt", "a\nb\n", "second");
    let third = sb.commit_file("a.txt", "a\nb\nc\n", "third");
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    let subject = |t: &TestDriver| t.app().details().subject().cloned();
    let selected = |t: &TestDriver| t.app().graph().selected().map(|c| c.oid.to_string());

    // In the plain commit view, ] is the row below and the graph follows.
    t.press("4").wait_idle();
    assert_eq!(subject(&t), Some(DetailsSubject::Commit(third.clone())));
    t.press("]").wait_idle();
    assert_eq!(subject(&t), Some(DetailsSubject::Commit(second.clone())));
    assert_eq!(selected(&t), Some(second.clone()));
    assert_eq!(t.app().focus(), PaneId::Details);
    t.assert_pane_contains(PaneId::Details, "second");
    t.press("[").wait_idle();
    assert_eq!(subject(&t), Some(DetailsSubject::Commit(third.clone())));
    // At the top, [ stays put.
    t.press("[").wait_idle();
    assert_eq!(subject(&t), Some(DetailsSubject::Commit(third.clone())));

    // Fullscreen files view stays open and fullscreen as it steps.
    t.press("1").press("enter").wait_idle();
    t.press("]").press("]").wait_idle();
    assert_eq!(subject(&t), Some(DetailsSubject::CommitFiles(first.clone())));
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Details));
    t.assert_pane_contains(PaneId::Details, "first");
    // At the bottom, ] stays put.
    t.press("]").wait_idle();
    assert_eq!(subject(&t), Some(DetailsSubject::CommitFiles(first.clone())));
    assert_eq!(selected(&t), Some(first));
}

#[test]
fn details_help_bar_lists_paging() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "a\n", "first");
    let mut t = TestDriver::with_size(Config::default(), &sb, 160, 30);
    t.press("4").wait_idle();
    let help = t.screen().lines().last().unwrap().to_string();
    assert!(
        help.contains("pgdn page down") && help.contains("pgup page up"),
        "{help}"
    );
    // The fullscreen files view too.
    t.press("1").press("enter").wait_idle();
    let help = t.screen().lines().last().unwrap().to_string();
    assert!(
        help.contains("pgdn page down") && help.contains("pgup page up"),
        "{help}"
    );
}

#[test]
fn brackets_refuse_a_commit_opened_from_blame() {
    let sb = Sandbox::new();
    let first = sb.commit_file("a.txt", "a\n", "first");
    sb.commit_file("a.txt", "a\nb\n", "second");
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    // Blame the newest commit's file, then open line 1's commit.
    t.press("enter").wait_idle();
    t.press("b").wait_idle();
    t.press("g").press("enter").wait_idle();
    assert_eq!(
        t.app().details().subject(),
        Some(&DetailsSubject::CommitFiles(first.clone()))
    );
    t.press("]").wait_idle();
    assert_eq!(t.app().details().subject(), Some(&DetailsSubject::CommitFiles(first)));
    assert_eq!(
        t.app().notice(),
        Some("this commit isn't selected in the graph: nothing to step from")
    );
}

#[test]
fn files_view_shows_its_list_from_the_first_frame() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "one\n", "base");
    sb.commit_file("a.txt", "two\n", "change a");
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    // The column of the list's right border on the details pane's first row.
    let divider = |t: &TestDriver| {
        let first = t.pane_lines(PaneId::Details).remove(0);
        assert!(first.starts_with("Files"), "no file list:\n{}", t.screen());
        first.chars().position(|c| c == '│')
    };

    // Before the files load, the list is already down the left; the plain
    // commit text loaded for the graph isn't drawn full width meanwhile.
    t.press_frame("enter");
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Details));
    let loading = divider(&t);
    assert!(loading.is_some());
    assert!(!t.pane_lines(PaneId::Details).iter().any(|l| l.contains("Author:")));

    // Once they arrive, everything lands beside the list where it stood.
    t.wait_idle();
    assert_eq!(divider(&t), loading);
    t.assert_pane_contains(PaneId::Details, "Files (1)");
    t.assert_pane_contains(PaneId::Details, "change a");
}

#[test]
fn close_brace_lists_files_below_the_diff() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "one\n", "base");
    sb.write("a.txt", "two\n").write("b.txt", "bee\n");
    sb.git(&["add", "."]);
    let oid = sb.commit_staged("touch two files");
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 60);

    t.press("space").press("4").press("}");
    assert_eq!(
        t.app().details().subject(),
        Some(&DetailsSubject::CommitFiles(oid.clone()))
    );
    assert_eq!(t.app().zoom(), Zoom::Off);
    insta::assert_snapshot!(t.pane_lines(PaneId::Details).join("\n"));

    // The list sits under the diff, and j still picks the file.
    let lines = t.pane_lines(PaneId::Details);
    let row = |needle: &str| lines.iter().position(|l| l.contains(needle)).unwrap();
    assert!(row("Files (2)") > row("+two"), "{lines:#?}");
    t.press("j");
    assert_eq!(t.app().details().selected_file().unwrap().path, "b.txt");
    t.assert_pane_contains(PaneId::Details, "+bee");

    // Fullscreen puts the list beside the diff, on the top row.
    t.press("+");
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Details));
    assert!(t.pane_lines(PaneId::Details)[0].starts_with("Files (2)"));

    // Leaving fullscreen puts the list back below.
    t.press("+");
    assert_eq!(t.app().zoom(), Zoom::Off);
    let lines = t.pane_lines(PaneId::Details);
    let row = |needle: &str| lines.iter().position(|l| l.contains(needle)).unwrap();
    assert!(row("Files (2)") > row("+bee"), "{lines:#?}");

    // } again goes back to the plain commit.
    t.press("}");
    assert_eq!(t.app().details().subject(), Some(&DetailsSubject::Commit(oid)));
    t.assert_pane_contains(PaneId::Details, "touch two files");
    assert!(!t.pane_lines(PaneId::Details).iter().any(|l| l.contains("Files (2)")));
}

#[test]
fn files_below_a_short_diff_keep_the_diff_visible() {
    let sb = Sandbox::new();
    sb.commit_file(
        "a.txt", "one
", "base",
    );
    sb.write(
        "a.txt", "two
",
    )
    .write(
        "b.txt", "bee
",
    )
    .write(
        "c.txt", "sea
",
    );
    sb.git(&["add", "."]);
    sb.commit_staged("touch three files");
    // At 24 rows the details pane has five rows inside its border.
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 24);

    t.press("space").press("4").press("}");
    let lines = t.pane_lines(PaneId::Details);
    assert!(lines.iter().any(|l| l.starts_with("diff --git a/a.txt")), "{lines:#?}");
    assert!(lines.iter().any(|l| l.contains("Files (3)")), "{lines:#?}");
    t.press("j");
    t.assert_pane_contains(PaneId::Details, "diff --git a/b.txt");
}
