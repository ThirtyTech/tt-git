//! Blaming a file: opening it from the status pane and from a commit, the
//! commit behind a line and back, stepping back past a commit, dates,
//! searching the code.

use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::app::Zoom;
use tt_git_tui::panes::{DateStyle, DetailsSubject, PaneId};

/// The commits behind f.ts, oldest first.
struct Repo {
    sb: Sandbox,
    add: String,
    shout_b: String,
    shout_d: String,
}

fn commit_as(sb: &Sandbox, author: &str, msg: &[&str]) -> String {
    sb.git(&["add", "."]);
    sb.tick();
    let mut args = vec!["commit", "-q", "--author", author];
    for part in msg {
        args.extend(["-m", part]);
    }
    sb.git(&args);
    sb.rev_parse("HEAD")
}

/// f.ts from three authors, with line 5 changed in the working tree.
fn blamed() -> Repo {
    let sb = Sandbox::new();
    sb.write("f.ts", "a\nb\nc\nd\ne\n");
    let add = commit_as(&sb, "Mario <mario@lumen.dev>", &["Add f"]);
    sb.write("f.ts", "a\nB\nc\nd\ne\n");
    let shout_b = commit_as(&sb, "Cow <cow@lumen.dev>", &["Shout b"]);
    sb.write("f.ts", "a\nB\nc\nD\ne\n").write("other.txt", "more\n");
    let shout_d = commit_as(
        &sb,
        "Rosalina <rosalina@lumen.dev>",
        &["Shout d and add other", "Other comes along for the ride."],
    );
    sb.write("f.ts", "a\nB\nc\nD\nE\n");
    Repo {
        sb,
        add,
        shout_b,
        shout_d,
    }
}

fn short(oid: &str) -> &str {
    &oid[..7]
}

/// Blames f.ts from the status pane and waits for it.
fn blame_from_status(t: &mut TestDriver) {
    t.press("2").press("g").press("b").wait_idle();
}

#[test]
fn b_in_status_blames_the_working_tree_and_esc_goes_back() {
    let r = blamed();
    let mut t = TestDriver::with_size(Config::default(), &r.sb, 120, 20);
    blame_from_status(&mut t);
    assert_eq!(t.app().details().subject(), Some(&DetailsSubject::Blame("f.ts".into())));
    assert_eq!(t.app().focus(), PaneId::Details);
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Details));
    let title = t.pane_title(PaneId::Details);
    assert!(title.contains("Blame f.ts at working tree  ‹ Status 1/5"), "{title}");
    // At the right: when the file last changed in a commit, that it has
    // changed since, and that it's the latest version.
    assert!(
        title.ends_with(" changed 2024-01-01 12:03 AM (23h ago) + uncommitted changes · latest ╮"),
        "{title}"
    );
    insta::assert_snapshot!(t.pane_lines(PaneId::Details).join("\n"));
    assert!(t.screen().contains("step back past commit"), "{}", t.screen());

    t.press("esc");
    assert_eq!(t.app().focus(), PaneId::Status);
    assert_eq!(t.app().zoom(), Zoom::Off);
    assert!(t.app().details().blame().is_none());
}

#[test]
fn a_narrow_title_leaves_out_the_date_but_not_latest() {
    let r = blamed();
    let mut t = TestDriver::with_size(Config::default(), &r.sb, 80, 20);
    blame_from_status(&mut t);
    let title = t.pane_title(PaneId::Details);
    assert!(title.contains("Blame f.ts at working tree  ‹ Status 1/5"), "{title}");
    assert!(!title.contains("changed"), "{title}");
    assert!(title.ends_with(" latest ╮"), "{title}");
}

#[test]
fn nothing_to_blame_in_an_untracked_file() {
    let r = blamed();
    r.sb.write("new.txt", "fresh\n");
    let mut t = TestDriver::new(&r.sb);
    t.press("2").press("G").press("b");
    assert!(t.app().details().blame().is_none());
    assert_eq!(t.app().notice(), Some("new.txt isn't committed yet: nothing to blame"));
}

#[test]
fn enter_opens_the_lines_commit_and_esc_comes_back_to_the_line() {
    let r = blamed();
    let mut t = TestDriver::with_size(Config::default(), &r.sb, 120, 30);
    blame_from_status(&mut t);
    t.press_all(&["j", "j", "j"]);
    t.press("enter").wait_idle();
    assert_eq!(
        t.app().details().subject(),
        Some(&DetailsSubject::CommitFiles(r.shout_d.clone()))
    );
    // Every file the commit touched, with the blamed one selected and its
    // line picked out.
    let files: Vec<_> = t
        .app()
        .details()
        .commit_files()
        .unwrap()
        .iter()
        .map(|f| f.path.clone())
        .collect();
    assert_eq!(files, ["f.ts", "other.txt"]);
    assert_eq!(t.app().details().selected_file().unwrap().path, "f.ts");
    let title = t.pane_title(PaneId::Details);
    assert!(
        title.contains(&format!(
            "Commit {} files  ‹ Blame f.ts at working tree, line 4",
            short(&r.shout_d)
        )),
        "{title}"
    );
    t.assert_pane_contains(PaneId::Details, "◂ line 4 in blame");
    // Looking at another file the commit changed.
    t.press("j").wait_idle();
    t.assert_pane_contains(PaneId::Details, "+more");

    t.press("esc");
    let blame = t.app().details().blame().expect("back in blame");
    assert_eq!(blame.selected_line(), 4);
    assert_eq!(blame.label(), "working tree");
    t.press("esc");
    assert_eq!(t.app().focus(), PaneId::Status);
}

#[test]
fn an_uncommitted_line_has_no_commit_to_open() {
    let r = blamed();
    let mut t = TestDriver::new(&r.sb);
    blame_from_status(&mut t);
    t.press("G").press("enter");
    assert!(t.app().details().blame().is_some());
    assert_eq!(
        t.app().notice(),
        Some("this line isn't committed yet: there's no commit to open")
    );
}

#[test]
fn brackets_step_back_past_a_commit_and_forward_again() {
    let r = blamed();
    let mut t = TestDriver::with_size(Config::default(), &r.sb, 120, 20);
    blame_from_status(&mut t);
    // Line 2 is "B", from Cow's commit: before it, line 2 was "b", from Mario's.
    t.press("j").press("[").wait_idle();
    let blame = t.app().details().blame().unwrap();
    assert_eq!(blame.depth(), 2);
    assert_eq!(blame.label(), format!("{}^", short(&r.shout_b)));
    assert_eq!(blame.selected_line(), 2);
    let (commit, path, _) = blame.selected().unwrap();
    assert_eq!((commit.oid.as_str(), path), (r.add.as_str(), "f.ts"));
    let title = t.pane_title(PaneId::Details);
    assert!(
        title.contains(&format!("Blame f.ts at {}^  ‹ working tree 2/5", short(&r.shout_b))),
        "{title}"
    );
    // The file as Mario added it, a minute after the base time, and changed
    // by Cow's and Rosalina's commits since.
    assert!(
        title.ends_with(" changed 2024-01-01 12:01 AM (23h ago) · 2 newer commits ╮"),
        "{title}"
    );
    t.assert_pane_contains(PaneId::Details, "▌  2 b");

    // Mario's commit added the file: there's nothing before it.
    t.press("[");
    assert_eq!(t.app().details().blame().unwrap().depth(), 2);
    assert!(
        t.app()
            .notice()
            .unwrap()
            .ends_with("added f.ts: there's nothing before it")
    );

    t.press("]");
    let blame = t.app().details().blame().unwrap();
    assert_eq!((blame.depth(), blame.label()), (1, "working tree"));
    assert_eq!(blame.selected_line(), 2);
    assert!(t.pane_title(PaneId::Details).contains(" changed 2024-01-01 12:03 AM "));
    assert!(t.pane_title(PaneId::Details).ends_with(" · latest ╮"));
    t.press("]");
    assert_eq!(t.app().notice(), Some("this is the newest version"));
}

/// The blame view's depth, the version it shows and the selected line's text.
fn at(t: &TestDriver) -> (usize, String, String) {
    let blame = t.app().details().blame().unwrap();
    let line = blame.blame().unwrap().lines[blame.selected_line() - 1].text.clone();
    (blame.depth(), blame.label().to_string(), line)
}

#[test]
fn braces_step_through_the_files_commits_one_at_a_time() {
    let r = blamed();
    let mut t = TestDriver::with_size(Config::default(), &r.sb, 120, 20);
    blame_from_status(&mut t);
    let v = |depth: usize, label: &str, line: &str| (depth, label.to_string(), line.to_string());
    // Line 2 ("B") last changed in Cow's commit, but { goes one commit of
    // the file at a time: the working tree has changes, so HEAD first.
    t.press("j").press("{").wait_idle();
    assert_eq!(at(&t), v(2, "HEAD", "B"));
    // HEAD's version is Rosalina's; before it, Cow's, which still has "B".
    t.press("{").wait_idle();
    assert_eq!(at(&t), v(3, short(&r.shout_b), "B"));
    assert!(
        t.pane_title(PaneId::Details)
            .contains(&format!("at {}  ‹ HEAD", short(&r.shout_b)))
    );
    t.press("{").wait_idle();
    assert_eq!(at(&t), v(4, short(&r.add), "b"));
    t.press("{").wait_idle();
    assert_eq!(at(&t).0, 4);
    assert_eq!(t.app().notice(), Some("no older commit changed f.ts"));

    // } comes forward the same way, back up to the working tree.
    t.press("}").wait_idle();
    assert_eq!(at(&t), v(3, short(&r.shout_b), "B"));
    t.press("}").wait_idle();
    assert_eq!(at(&t), v(2, "HEAD", "B"));
    t.press("}").wait_idle();
    assert_eq!(at(&t), v(1, "working tree", "B"));
    t.press("}");
    assert_eq!(t.app().notice(), Some("this is the newest version"));
}

#[test]
fn close_brace_after_open_bracket_stops_at_each_commit_in_between() {
    let r = blamed();
    let mut t = TestDriver::with_size(Config::default(), &r.sb, 120, 20);
    blame_from_status(&mut t);
    let v = |depth: usize, label: &str, line: &str| (depth, label.to_string(), line.to_string());
    // Line 2 ("B") is Cow's: [ jumps from the working tree past Rosalina's
    // commit and Cow's, to Mario's version...
    t.press("j").press("[").wait_idle();
    assert_eq!(at(&t), v(2, &format!("{}^", short(&r.shout_b)), "b"));
    // ...which } walks forward through, Cow's then Rosalina's, in place.
    t.press("}").wait_idle();
    assert_eq!(at(&t), v(2, short(&r.shout_b), "B"));
    t.press("}").wait_idle();
    assert_eq!(at(&t), v(2, short(&r.shout_d), "B"));
    // Then the working tree it started from, with its line still selected.
    t.press("}").wait_idle();
    assert_eq!(at(&t), v(1, "working tree", "B"));
}

#[test]
fn t_switches_blame_and_the_graph_to_clock_times() {
    let r = blamed();
    let mut t = TestDriver::with_size(Config::default(), &r.sb, 120, 20);
    blame_from_status(&mut t);
    t.assert_pane_contains(PaneId::Details, "Mario  23h");
    t.press("t");
    assert_eq!(t.app().graph().dates, DateStyle::Clock);
    t.assert_pane_contains(PaneId::Details, "Mario 2024-01-01 12:01 AM");
    t.press("esc");
    // The graph shows them the same way.
    assert_eq!(t.app().graph().dates, DateStyle::Clock);
}

#[test]
fn space_shows_the_lines_commit_with_its_message() {
    let r = blamed();
    let mut t = TestDriver::with_size(Config::default(), &r.sb, 120, 30);
    blame_from_status(&mut t);
    t.press_all(&["j", "j", "j"]).press("space").wait_idle();
    t.assert_pane_contains(PaneId::Details, &format!("Commit {}", short(&r.shout_d)));
    t.assert_pane_contains(PaneId::Details, "Shout d and add other");
    t.assert_pane_contains(PaneId::Details, "Rosalina <rosalina@lumen.dev>");
    t.assert_pane_contains(PaneId::Details, "1 line in this file come from it");
    t.assert_pane_contains(PaneId::Details, "Other comes along for the ride.");

    t.press("c");
    assert_eq!(t.app().notice(), Some(format!("copied {}", r.shout_d).as_str()));
}

#[test]
fn b_in_a_commit_blames_its_file_as_of_that_commit() {
    let r = blamed();
    let mut t = TestDriver::with_size(Config::default(), &r.sb, 120, 30);
    // The graph's newest commit, fullscreen: f.ts is its first file.
    t.press("1").press("g").press("enter").wait_idle();
    assert_eq!(t.app().details().selected_file().unwrap().path, "f.ts");
    t.press("b").wait_idle();
    let blame = t.app().details().blame().expect("blame open");
    assert_eq!(blame.label(), short(&r.shout_d));
    // As of that commit, line 5 is still Mario's "e".
    assert_eq!(blame.blame().unwrap().lines[4].text, "e");
    let title = t.pane_title(PaneId::Details);
    assert!(title.contains(&format!("‹ Commit {}", short(&r.shout_d))), "{title}");
    // Committed with no changes since: the commit's date, and HEAD has the
    // same version.
    assert!(
        title.contains(" changed 2024-01-01 12:03 AM (23h ago) · latest ╮"),
        "{title}"
    );

    t.press("esc");
    assert_eq!(
        t.app().details().subject(),
        Some(&DetailsSubject::CommitFiles(r.shout_d.clone()))
    );
    assert_eq!(t.app().zoom(), Zoom::Full(PaneId::Details));
    t.press("esc");
    assert_eq!(t.app().focus(), PaneId::Graph);
}

/// long.txt, 60 numbered lines in one commit, blamed from the status pane
/// (where it's the only file, changed in the working tree).
fn blame_long(t: &mut TestDriver) {
    t.press("2").press("g").press("b").wait_idle();
    assert_eq!(t.app().details().blame().unwrap().line_count(), 60);
}

fn long_file() -> Sandbox {
    let sb = Sandbox::new();
    let text: String = (1..=60).map(|n| format!("line {n}\n")).collect();
    sb.write("long.txt", &text);
    commit_as(&sb, "Mario <mario@lumen.dev>", &["Add long"]);
    sb.write("long.txt", &text.replacen("line 1\n", "LINE 1\n", 1));
    sb
}

/// notes.txt blamed from the status pane, its last line changed in the
/// working tree.
fn blame_notes() -> (Sandbox, TestDriver) {
    let sb = Sandbox::new();
    let notes = "alpha one\nbeta two\nAlpha three\ngamma four\nalpha five\ndelta six\n";
    sb.commit_file("notes.txt", notes, "Add notes");
    sb.write("notes.txt", &notes.replace("six", "SIX"));
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 20);
    blame_from_status(&mut t);
    (sb, t)
}

fn selected_line(t: &TestDriver) -> usize {
    t.app().details().blame().unwrap().selected_line()
}

#[test]
fn ctrl_g_asks_for_a_line_and_goes_there() {
    let sb = long_file();
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 20);
    blame_long(&mut t);
    assert!(t.screen().contains("go to line"), "{}", t.screen());

    t.press("ctrl+g");
    assert!(t.screen().contains("Go to line (1-60)"), "{}", t.screen());
    assert!(t.screen().contains("type a line number"), "{}", t.screen());
    t.type_text("42");
    assert!(t.screen().contains("Go to line 42"), "{}", t.screen());
    t.press("enter");
    assert!(t.app().modal().is_none());
    assert_eq!(selected_line(&t), 42);
    t.assert_pane_contains(PaneId::Details, "line 42");
    assert!(t.pane_title(PaneId::Details).contains(" 42/60"));

    // Past the end is the last line; esc goes nowhere.
    t.press("ctrl+g").type_text("999").press("enter");
    assert_eq!(selected_line(&t), 60);
    t.press("ctrl+g").type_text("3").press("esc");
    assert!(t.app().modal().is_none());
    assert_eq!(selected_line(&t), 60);
}

#[test]
fn a_count_before_g_or_j_goes_to_a_line_or_moves_that_far() {
    let sb = long_file();
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 20);
    blame_long(&mut t);

    t.type_text("25");
    assert!(
        t.pane_title(PaneId::Details).contains("1/60 go to 25…"),
        "{}",
        t.pane_title(PaneId::Details)
    );
    // Digits count here rather than switching panes.
    assert_eq!(t.app().focus(), PaneId::Details);
    t.press("G");
    assert_eq!(selected_line(&t), 25);
    assert!(!t.pane_title(PaneId::Details).contains("go to"));
    t.assert_pane_contains(PaneId::Details, "line 25");

    t.type_text("10").press("g");
    assert_eq!(selected_line(&t), 10);
    t.type_text("5").press("j");
    assert_eq!(selected_line(&t), 15);
    t.type_text("20").press("k");
    assert_eq!(selected_line(&t), 1);
    // Without a count, G and g are still the ends.
    t.press("G");
    assert_eq!(selected_line(&t), 60);
    t.press("g");
    assert_eq!(selected_line(&t), 1);

    // esc drops a count without leaving blame; another key just uses it up.
    t.type_text("7").press("esc");
    assert!(t.app().details().blame().is_some());
    t.press("G");
    assert_eq!(selected_line(&t), 60);
    t.type_text("7").press("t").press("G");
    assert_eq!(selected_line(&t), 60);
}

#[test]
fn a_number_in_the_palette_goes_to_that_line_in_blame() {
    let sb = long_file();
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 20);
    blame_long(&mut t);
    t.press(":");
    assert!(t.screen().contains("Commands or a line number"), "{}", t.screen());
    t.type_text(":33");
    assert!(t.screen().contains("Go to line 33"), "{}", t.screen());
    t.press("enter");
    assert_eq!(selected_line(&t), 33);

    // Outside blame a number is just a filter.
    t.press("esc").press(":").type_text("33");
    assert!(!t.screen().contains("Go to line"), "{}", t.screen());
}

/// The pane row showing `needle`.
fn row_of(t: &TestDriver, needle: &str) -> usize {
    let lines = t.pane_lines(PaneId::Details);
    lines
        .iter()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no {needle:?} in\n{}", lines.join("\n")))
}

#[test]
fn brackets_follow_the_line_by_its_text_and_keep_its_row() {
    let sb = Sandbox::new();
    let numbered = |i: usize| format!("line {i}");
    let v1: Vec<String> = (1..=60).map(numbered).collect();
    sb.write("f.ts", &(v1.join("\n") + "\n"));
    commit_as(&sb, "Mario <mario@lumen.dev>", &["Add f"]);
    // Five lines added at the top, and line 40 changed.
    let mut v2: Vec<String> = (1..=5).map(|i| format!("new {i}")).collect();
    v2.extend(v1.iter().map(|l| {
        if l == "line 40" {
            "line 40 changed".into()
        } else {
            l.clone()
        }
    }));
    sb.write("f.ts", &(v2.join("\n") + "\n"));
    let change = commit_as(&sb, "Cow <cow@lumen.dev>", &["Change line 40"]);

    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 20);
    t.press("1")
        .press("g")
        .press("enter")
        .wait_idle()
        .press("b")
        .wait_idle();
    // "line 40 changed", line 45 here, five rows down from the top.
    t.press("G");
    for _ in 40..65 {
        t.press("k");
    }
    t.press_all(&["j", "j", "j", "j", "j"]);
    let row = row_of(&t, " 45 line 40 changed");
    assert_eq!(row, 5);

    // Before the commit it was "line 40", on line 40, and on the same row.
    t.press_frame("[");
    // While the older version loads, the newer one stays on screen.
    assert_eq!(row_of(&t, " 45 line 40 changed"), row);
    t.wait_idle();
    let blame = t.app().details().blame().unwrap();
    assert_eq!(blame.label(), format!("{}^", short(&change)));
    assert_eq!(blame.selected_line(), 40);
    assert_eq!(row_of(&t, " 40 line 40"), row);

    // Down two lines and forward again: the same line, "line 42", there.
    t.press("j").press("j").press("]");
    let blame = t.app().details().blame().unwrap();
    assert_eq!(blame.depth(), 1);
    assert_eq!(blame.selected_line(), 47);
    assert_eq!(row_of(&t, " 47 line 42"), row + 2);
}

#[test]
fn slash_searches_as_you_type_and_n_steps_through_the_matches() {
    let (_sb, mut t) = blame_notes();
    t.press("j").press("/");
    assert!(
        t.pane_title(PaneId::Details).contains("2/6 /█─"),
        "{}",
        t.pane_title(PaneId::Details)
    );
    // From line 2: the first "gam" is on line 4.
    t.type_text("gam");
    assert_eq!(selected_line(&t), 4);
    // Lower case ignores case: line 3's "Alpha" matches too, and it's the
    // first match from line 2.
    t.press("backspace")
        .press("backspace")
        .press("backspace")
        .type_text("alpha");
    assert_eq!(selected_line(&t), 3);
    assert!(
        t.pane_title(PaneId::Details).contains(" /alpha█ 2/3"),
        "{}",
        t.pane_title(PaneId::Details)
    );
    t.press("enter");
    let title = t.pane_title(PaneId::Details);
    assert!(title.contains("3/6 /alpha 2/3"), "{title}");

    // Keys are commands again: n and N go through the matches, round the end.
    t.press("n");
    assert_eq!(selected_line(&t), 5);
    t.press("n");
    assert_eq!(selected_line(&t), 1);
    assert_eq!(t.app().notice(), Some("search wrapped to the top"));
    t.press("N");
    assert_eq!(selected_line(&t), 5);
    assert_eq!(t.app().notice(), Some("search wrapped to the bottom"));
    // Off a match, the title counts them.
    t.press("j");
    assert!(t.pane_title(PaneId::Details).contains(" /alpha 3 matches"));
}

#[test]
fn a_capital_letter_makes_the_search_match_case() {
    let (_sb, mut t) = blame_notes();
    t.press("/").type_text("Alpha").press("enter");
    assert_eq!(selected_line(&t), 3);
    assert!(t.pane_title(PaneId::Details).contains(" /Alpha 1/1"));
    // The changed line is searched as it is in the working tree.
    t.press("/").type_text("SIX$").press("enter");
    assert_eq!(selected_line(&t), 6);
}

#[test]
fn esc_drops_the_search_and_goes_back_to_the_line() {
    let (_sb, mut t) = blame_notes();
    t.press("j").press("/").type_text("delta");
    assert_eq!(selected_line(&t), 6);
    t.press("esc");
    assert_eq!(selected_line(&t), 2);
    assert!(!t.pane_title(PaneId::Details).contains(" /"));
    // Still in blame: esc only closed the search.
    assert!(t.app().details().blame().is_some());
    t.press("n");
    assert_eq!(t.app().notice(), Some("nothing to search for yet"));
}

#[test]
fn a_bad_regex_or_no_match_leaves_the_selection_alone() {
    let (_sb, mut t) = blame_notes();
    t.press("j").press("/").type_text("be(");
    assert_eq!(selected_line(&t), 2);
    assert!(t.pane_title(PaneId::Details).contains(" /be(█ invalid regex"));
    t.press("enter");
    assert!(t.app().notice().unwrap().starts_with("invalid regex:"));
    assert!(!t.pane_title(PaneId::Details).contains(" /"));

    t.press("j").press("/").type_text("zeta").press("enter");
    assert_eq!(selected_line(&t), 3);
    assert!(t.pane_title(PaneId::Details).contains(" /zeta no match"));
    t.press("n");
    assert_eq!(t.app().notice(), Some("no match for /zeta"));
}

#[test]
fn matches_are_marked_in_the_code() {
    let (_sb, mut t) = blame_notes();
    t.press("/").type_text("a f").press("enter");
    let theme = tt_git_tui::tui_theme::Theme::default();
    let (x, y) = t.find_text("alpha five").expect("on screen");
    let bg = |dx: u16| t.buffer()[(x + dx, y)].bg;
    assert_eq!(bg(4), theme.search_match.bg.unwrap());
    assert_eq!(bg(6), theme.search_match.bg.unwrap());
    assert_ne!(bg(3), theme.search_match.bg.unwrap());
    assert_ne!(bg(7), theme.search_match.bg.unwrap());
}

#[test]
fn the_search_carries_over_when_stepping_back() {
    let (_sb, mut t) = blame_notes();
    // Line 6 isn't committed: before it, as of HEAD, it says "six".
    t.press("/").type_text("six").press("enter");
    assert_eq!(selected_line(&t), 6);
    t.press("[").wait_idle();
    assert_eq!(t.app().details().blame().unwrap().depth(), 2);
    assert!(
        t.pane_title(PaneId::Details).contains(" /six 1/1"),
        "{}",
        t.pane_title(PaneId::Details)
    );
    t.press("]");
    assert!(t.pane_title(PaneId::Details).contains(" /six 1/1"));
}
