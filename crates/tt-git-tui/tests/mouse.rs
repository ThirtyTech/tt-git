use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::app::Zoom;
use tt_git_tui::panes::{BranchTab, DetailsSubject, PaneId};

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
    t.press("space");
    t.assert_pane_contains(PaneId::Details, "commit 6");
    t.press("space");

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
    t.press("2"); // focus elsewhere: the wheel follows the pointer, not focus
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

/// Clicks the start of `text` in `pane`'s title.
fn click_title(t: &mut TestDriver, pane: PaneId, text: &str) {
    let area = t.app().pane_area(pane).unwrap();
    let title = t.pane_title(pane);
    let at = title.find(text).unwrap_or_else(|| panic!("{text:?} not in {title:?}"));
    // Every character in titles here is one cell wide.
    let col = title[..at].chars().count() as u16;
    t.click(area.x + col, area.y, false);
}

#[test]
fn clicking_a_branch_tab_switches_to_it() {
    let sb = repo();
    sb.git(&["tag", "v1.0"]);
    sb.git(&["stash", "push", "--include-untracked", "-m", "wip"]);
    let mut t = TestDriver::new(&sb);
    assert_eq!(t.app().branches().tab(), BranchTab::All);

    click_title(&mut t, PaneId::Branches, "Tags 1");
    assert_eq!(t.app().focus(), PaneId::Branches);
    assert_eq!(t.app().branches().tab(), BranchTab::Tags);
    t.assert_pane_contains(PaneId::Branches, "v1.0");

    // The last letter of a tab counts too.
    let area = t.app().pane_area(PaneId::Branches).unwrap();
    let title = t.pane_title(PaneId::Branches);
    let end = title[..title.find("Stash 1").unwrap()].chars().count() + "Stash 1".len() - 1;
    t.click(area.x + end as u16, area.y, false);
    assert_eq!(t.app().branches().tab(), BranchTab::Stash);
    t.assert_pane_contains(PaneId::Branches, "wip");

    // Elsewhere on the title only focuses.
    t.press("1");
    t.click(area.x + 2, area.y, false);
    assert_eq!(t.app().focus(), PaneId::Branches);
    assert_eq!(t.app().branches().tab(), BranchTab::Stash);

    click_title(&mut t, PaneId::Branches, "Branches");
    assert_eq!(t.app().branches().tab(), BranchTab::All);
}

#[test]
fn clicking_a_file_in_the_files_view_shows_its_diff() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "one\n", "base");
    let long: String = (1..=60).map(|i| format!("line {i}\n")).collect();
    sb.write("a.txt", "two\n").write("b.txt", &long);
    sb.git(&["add", "."]);
    sb.commit_staged("touch two files");
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    t.press("enter");
    assert_eq!(t.app().details().selected_file().unwrap().path, "a.txt");

    // Fullscreen, the list is down the left.
    t.click_text("b.txt");
    assert_eq!(t.app().details().selected_file().unwrap().path, "b.txt");
    t.assert_pane_contains(PaneId::Details, "+line 1");

    // The wheel scrolls the diff when over it, keeping the file...
    t.assert_pane_contains(PaneId::Details, "diff --git a/b.txt");
    let (x, y) = t.find_text("+line 1").unwrap();
    t.scroll(x, y, true);
    assert_eq!(t.app().details().selected_file().unwrap().path, "b.txt");
    assert!(
        !t.pane_lines(PaneId::Details).iter().any(|l| l.contains("diff --git")),
        "{}",
        t.screen()
    );
    // ...and moves through the files over the list.
    let (x, y) = t.find_text("b.txt").unwrap();
    t.scroll(x, y, false);
    assert_eq!(t.app().details().selected_file().unwrap().path, "a.txt");

    // Out of fullscreen the list is along the bottom.
    t.press("+");
    t.click_text("b.txt");
    assert_eq!(t.app().details().selected_file().unwrap().path, "b.txt");
    t.click_text("a.txt");
    assert_eq!(t.app().details().selected_file().unwrap().path, "a.txt");
    t.assert_pane_contains(PaneId::Details, "diff --git a/a.txt");
}

#[test]
fn double_clicking_a_commit_opens_its_files() {
    let sb = repo();
    let mut t = TestDriver::new(&sb);

    // Clicks on different rows aren't a double click.
    t.click_pane_row(PaneId::Graph, 2).click_pane_row(PaneId::Graph, 3);
    assert_eq!(t.app().zoom(), Zoom::Off);
    // Nor are two below the last commit.
    t.click_pane_row(PaneId::Graph, 40).click_pane_row(PaneId::Graph, 40);
    assert_eq!(t.app().zoom(), Zoom::Off);

    t.click_pane_row(PaneId::Graph, 3).click_pane_row(PaneId::Graph, 3);
    let oid = t.app().graph().commits()[3].oid.to_string();
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Details));
    assert_eq!(t.app().focus(), PaneId::Details);
    assert_eq!(t.app().details().subject(), Some(&DetailsSubject::CommitFiles(oid)));
    t.press("esc");
    assert_eq!(t.app().zoom(), Zoom::Off);
}

#[test]
fn double_clicking_does_what_enter_does() {
    let sb = repo();
    let mut t = TestDriver::new(&sb);
    // A file: its staging view.
    t.click_pane_row(PaneId::Status, 1).click_pane_row(PaneId::Status, 1);
    t.wait_idle();
    assert_eq!(
        t.app().details().subject(),
        Some(&DetailsSubject::Stage("y.txt".into()))
    );
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Details));
    t.press("esc");
    // Nothing below the last file.
    t.click_pane_row(PaneId::Status, 30).click_pane_row(PaneId::Status, 30);
    assert_eq!(t.app().zoom(), Zoom::Off);

    // A branch: checked out.
    let row = t.app().branches().visible().position(|b| b.name == "alpha").unwrap() as u16;
    t.click_pane_row(PaneId::Branches, row)
        .click_pane_row(PaneId::Branches, row);
    t.wait_idle();
    assert_eq!(sb.git(&["branch", "--show-current"]).trim(), "alpha");
}

#[test]
fn double_clicking_a_folder_folds_it() {
    let sb = Sandbox::new();
    sb.commit("first");
    sb.write("src/a.txt", "a").write("src/b.txt", "b");
    let mut t = TestDriver::new(&sb);
    t.press("2");
    t.press("`");
    let rows = |t: &TestDriver| {
        t.pane_lines(PaneId::Status)
            .iter()
            .filter(|l| l.contains(".txt"))
            .count()
    };
    assert_eq!(rows(&t), 2, "{:#?}", t.pane_lines(PaneId::Status));
    t.press("g");
    assert!(t.app().status().selected_folder().is_some(), "the folder is first");
    t.click_pane_row(PaneId::Status, 0).click_pane_row(PaneId::Status, 0);
    assert_eq!(rows(&t), 0, "folded: {:#?}", t.pane_lines(PaneId::Status));
    t.click_pane_row(PaneId::Status, 0).click_pane_row(PaneId::Status, 0);
    assert_eq!(rows(&t), 2, "unfolded");
}

#[test]
fn the_focused_pane_lists_its_commands_with_their_keys() {
    let sb = repo();
    let mut t = TestDriver::new(&sb);
    let commands = |t: &TestDriver| -> Vec<(String, String)> {
        t.app().pane_commands().into_iter().map(|c| (c.label, c.key)).collect()
    };
    let graph = commands(&t);
    assert!(graph.contains(&("Checkout".into(), "o".into())), "{graph:#?}");
    assert!(graph.contains(&("Cherry-pick".into(), "C".into())), "{graph:#?}");
    t.press("2");
    let status = commands(&t);
    assert!(
        status.contains(&("Stage/unstage".into(), "space".into())),
        "{status:#?}"
    );
    assert!(!status.iter().any(|(l, _)| l == "Cherry-pick"));
    // Nothing while a popup is open.
    t.press("?");
    assert!(commands(&t).is_empty());
}

#[test]
fn a_right_click_picks_the_row_but_never_makes_a_double_click() {
    use tt_git_tui::mouse::{MouseInput, MouseKind, MouseTarget};
    let sb = repo();
    let mut t = TestDriver::new(&sb);
    t.press("2");
    let right = |t: &mut TestDriver, row| {
        t.app_mut().handle_mouse(MouseInput {
            kind: MouseKind::RightClick,
            ctrl: false,
            target: MouseTarget::Pane {
                pane: PaneId::Graph,
                body_row: Some(row),
                part: None,
            },
        })
    };
    right(&mut t, 4);
    assert_eq!(t.app().focus(), PaneId::Graph);
    assert_eq!(t.app().graph().selected_index(), 4);
    right(&mut t, 4);
    t.click_pane_row(PaneId::Graph, 4);
    assert_eq!(t.app().zoom(), Zoom::Off, "no double click");
}
