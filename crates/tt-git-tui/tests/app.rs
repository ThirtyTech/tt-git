use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::app::Zoom;
use tt_git_tui::panes::{GraphScope, PaneId};

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
    t.press("1");
    assert_eq!(t.app().focus(), PaneId::Status);
    t.press("tab");
    assert_eq!(t.app().focus(), PaneId::Branches);
    t.press("shift+tab").press("shift+tab");
    assert_eq!(t.app().focus(), PaneId::Details);
    t.press("3");
    assert_eq!(t.app().focus(), PaneId::Graph);
}

#[test]
fn help_popup_lists_bindings() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.press("?");
    let screen = t.screen();
    assert!(screen.contains("Keybindings"), "{screen}");
    assert!(screen.contains("all/current"), "{screen}");
    // Keys don't leak through to panes while the popup is open.
    t.press("a").press("esc");
    assert_eq!(t.app().graph().scope, GraphScope::All);
    assert!(!t.screen().contains("Keybindings"));
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
    assert_eq!(graph.commits()[0].summary, "side experiment");
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
        .position(|c| c.summary == "main work")
        .unwrap();
    for _ in 0..target {
        t.press("j");
    }
    assert_eq!(t.app().graph().selected().unwrap().summary, "main work");

    t.press("a");
    let graph = t.app().graph();
    assert_eq!(graph.scope, GraphScope::Current);
    assert!(graph.commits().iter().all(|c| c.summary != "side experiment"));
    assert_eq!(graph.commits().len(), 4);
    assert_eq!(graph.selected().unwrap().summary, "main work", "selection kept");
    assert!(t.pane_title(PaneId::Graph).contains("Graph [current]"));

    t.press("a");
    assert_eq!(t.app().graph().commits().len(), 5);
    assert_eq!(t.app().graph().selected().unwrap().summary, "main work");
}

#[test]
fn details_follow_the_selection() {
    let sb = branchy_repo();
    let mut t = TestDriver::new(&sb);
    t.assert_pane_contains(PaneId::Details, "side experiment");
    t.assert_pane_contains(PaneId::Details, "Author: Test User <test@example.com>");
    t.press("j");
    let selected = t.app().graph().selected().unwrap().summary.clone();
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
    let t = TestDriver::new(&sb);
    t.assert_pane_contains(PaneId::Details, "Author: Real Name <real@example.com>");
    t.assert_pane_contains(PaneId::Details, "Committer: Real Name <real@example.com>");
    t.assert_pane_contains(PaneId::Graph, "Real Name");
}

#[test]
fn zoom_cycles_wide_full_and_back() {
    let sb = Sandbox::new();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 24);
    let normal_graph = t.app().pane_area(PaneId::Graph).unwrap();

    // Wide: graph and details span the screen, the left column is gone.
    t.press("1").press("+");
    assert_eq!(t.app().zoom(), Zoom::Wide);
    assert_eq!(t.app().focus(), PaneId::Graph);
    let graph = t.app().pane_area(PaneId::Graph).unwrap();
    let details = t.app().pane_area(PaneId::Details).unwrap();
    assert_eq!((graph.x, graph.width), (0, 100));
    assert_eq!((details.x, details.width), (0, 100));
    assert!(t.app().pane_area(PaneId::Status).is_none());
    assert!(t.app().pane_area(PaneId::Branches).is_none());

    // Full: the focused pane takes the whole main area.
    t.press("+");
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Graph));
    assert_eq!(t.app().pane_area(PaneId::Graph).unwrap().height, 23);
    assert!(t.app().pane_area(PaneId::Details).is_none());

    t.press("+");
    assert_eq!(t.app().zoom(), Zoom::Off);
    assert_eq!(t.app().pane_area(PaneId::Graph).unwrap(), normal_graph);
}

#[test]
fn zoom_full_uses_details_when_focused() {
    let sb = Sandbox::new();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 24);
    t.press("+").press("4").press("+");
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Details));
    assert_eq!(t.app().pane_area(PaneId::Details).unwrap().width, 100);
    assert!(t.app().pane_area(PaneId::Graph).is_none());
}

#[test]
fn focusing_a_hidden_pane_unzooms() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.press("+").press("+");
    t.press("4");
    assert_eq!(t.app().zoom(), Zoom::Off);
    t.press("+").press("2");
    assert_eq!(t.app().zoom(), Zoom::Off);
    assert!(t.app().pane_area(PaneId::Status).is_some());
}
