use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::panes::PaneId;

/// A commit adding `long.txt`, 60 lines with a needle on lines 10, 40 and
/// 55, and `short.txt`, with one more.
fn needles() -> Sandbox {
    let sb = Sandbox::new();
    let long: String = (1..=60)
        .map(|i| match i {
            10 | 40 | 55 => format!("needle {i}\n"),
            _ => format!("hay {i}\n"),
        })
        .collect();
    sb.write("long.txt", &long).write("short.txt", "a Needle\n");
    sb.git(&["add", "."]);
    sb.commit_staged("add hay");
    sb
}

fn title(t: &TestDriver) -> String {
    t.pane_title(PaneId::Details)
}

fn shows(t: &TestDriver, text: &str) -> bool {
    t.pane_lines(PaneId::Details).iter().any(|l| l.contains(text))
}

#[test]
fn slash_searches_the_diff_and_n_steps_through_the_matches() {
    let sb = needles();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    t.press("enter").wait_idle();
    assert!(!shows(&t, "+needle 40"), "below the view to start with");

    t.press("/");
    assert!(title(&t).contains(" /█─"), "{}", title(&t));
    t.type_text("needle");
    assert!(title(&t).contains(" /needle█ 1/3"), "{}", title(&t));
    assert!(shows(&t, "+needle 10"));
    insta::assert_snapshot!("details_search_typed", t.screen());
    t.press("enter");
    assert!(title(&t).contains(" /needle 1/3"), "{}", title(&t));

    // Keys are commands again: n and N go through the matches, round the end.
    t.press("n");
    assert!(title(&t).contains(" /needle 2/3"), "{}", title(&t));
    assert!(shows(&t, "+needle 40"));
    t.press("n").press("n");
    assert!(title(&t).contains(" /needle 1/3"), "{}", title(&t));
    assert_eq!(t.app().notice(), Some("search wrapped to the top"));
    t.press("N");
    assert!(title(&t).contains(" /needle 3/3"), "{}", title(&t));
    assert_eq!(t.app().notice(), Some("search wrapped to the bottom"));

    // The search stays for the next file's diff, matching it afresh. A
    // capital would match case; this one doesn't have one.
    t.press("j").wait_idle();
    assert!(title(&t).contains(" /needle 1 match"), "{}", title(&t));
    t.press("n");
    assert!(title(&t).contains(" /needle 1/1"), "{}", title(&t));
    t.press("/").type_text("Needle").press("enter");
    assert!(title(&t).contains(" /Needle 1/1"), "{}", title(&t));
}

#[test]
fn matches_are_marked_and_esc_goes_back() {
    let sb = needles();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    t.press("enter").wait_idle();
    t.press("/").type_text("dle 1").press("enter");
    let theme = tt_git_tui::theme::Theme::default();
    let (x, y) = t.find_text("+needle 10").expect("on screen");
    let bg = |dx: u16| t.buffer()[(x + dx, y)].bg;
    assert_eq!(bg(4), theme.search_match.bg.unwrap());
    assert_eq!(bg(8), theme.search_match.bg.unwrap());
    assert_ne!(bg(3), theme.search_match.bg.unwrap());

    // esc while typing drops the search and scrolls back.
    t.press("/").type_text("needle 55");
    assert!(shows(&t, "+needle 55"));
    t.press("esc");
    assert!(!shows(&t, "+needle 55"));
    assert!(!title(&t).contains(" /"), "{}", title(&t));
    assert!(t.app().shows(PaneId::Details), "esc only closed the search");
    t.press("n");
    assert_eq!(t.app().notice(), Some("nothing to search for yet"));

    t.press("/").type_text("zeta").press("enter");
    assert!(title(&t).contains(" /zeta no match"), "{}", title(&t));
    t.press("n");
    assert_eq!(t.app().notice(), Some("no match for /zeta"));
}

#[test]
fn the_commit_view_is_searched_too() {
    let sb = needles();
    let mut t = TestDriver::with_size(Config::default(), &sb, 100, 30);
    t.press("4").wait_idle();
    t.press("/").type_text("^\\+a needle").press("enter");
    assert!(title(&t).contains(" /^\\+a needle 1/1"), "{}", title(&t));
    assert!(shows(&t, "+a Needle"));
}
