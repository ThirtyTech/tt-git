use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::panes::{DetailsSubject, PaneId};

fn repo() -> Sandbox {
    let sb = Sandbox::new();
    for i in 0..10 {
        sb.commit(&format!("commit {i}"));
    }
    sb.branch("alpha");
    sb.branch("beta");
    sb.write("x.txt", "x").write("y.txt", "y");
    sb
}

#[test]
fn click_focuses_and_selects() {
    let sb = repo();
    let mut t = TestDriver::new(&sb);
    t.click_pane_row(PaneId::Graph, 3);
    assert_eq!(t.app().focus(), PaneId::Graph);
    assert_eq!(t.app().graph().selected_index(), 3);
    t.press("enter");
    t.assert_pane_contains(PaneId::Details, "commit 6");
    t.press("enter");

    t.click_pane_row(PaneId::Status, 1);
    assert_eq!(t.app().focus(), PaneId::Status);
    assert_eq!(t.app().details().subject(), Some(&DetailsSubject::File("y.txt".into())));

    t.click_pane_row(PaneId::Branches, 1);
    assert_eq!(t.app().focus(), PaneId::Branches);
    let selected = t.app().branches().visible().nth(1).unwrap().name.clone();
    assert!(t.pane_lines(PaneId::Branches)[1].contains(&selected));

    // Clicking below the last item focuses without changing the selection.
    t.click_pane_row(PaneId::Graph, 3);
    t.click_pane_row(PaneId::Graph, 40);
    assert_eq!(t.app().graph().selected_index(), 3);
}

#[test]
fn wheel_scrolls_the_pane_under_the_pointer() {
    let sb = repo();
    let mut t = TestDriver::new(&sb);
    t.press("1"); // focus elsewhere: the wheel follows the pointer, not focus
    let area = t.app().pane_area(PaneId::Graph).unwrap();
    t.scroll(area.x + 5, area.y + 2, true);
    assert_eq!(t.app().graph().selected_index(), 3);
    assert_eq!(t.app().focus(), PaneId::Status);
    t.scroll(area.x + 5, area.y + 2, false);
    assert_eq!(t.app().graph().selected_index(), 0);
}

#[test]
fn mouse_can_be_disabled() {
    let sb = repo();
    let cfg = Config::parse("mouse = false\n").unwrap();
    let mut t = TestDriver::with_config(cfg, &sb);
    t.click_pane_row(PaneId::Branches, 1);
    assert_eq!(t.app().focus(), PaneId::Graph);
}

#[test]
fn clicks_are_ignored_under_popups() {
    let sb = repo();
    let mut t = TestDriver::new(&sb);
    t.press("?");
    t.click_pane_row(PaneId::Branches, 1);
    assert_eq!(t.app().focus(), PaneId::Graph);
}
