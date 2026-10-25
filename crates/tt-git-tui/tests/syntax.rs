//! Code colored by language in blame, commit diffs and the staging view,
//! and left plain with `syntax = false`.

use ratatui::style::{Color, Style};
use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;
use tt_git_tui::panes::{DetailsSubject, PaneId};
use tt_git_tui::syntax::Class;
use tt_git_tui::tui_theme::Theme;

const OLD: &str = "// greet\nfn main() {\n    let n = 1;\n}\n";
const NEW: &str = "// greet\nfn main() {\n    let n = 2;\n    println!(\"hi\");\n}\n";

/// main.rs committed, then changed in a second commit.
fn repo() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit_file("main.rs", OLD, "add main");
    sb.commit_file("main.rs", NEW, "say hi");
    sb
}

fn fg(style: Style) -> Color {
    style.fg.expect("a foreground color")
}

/// The foreground and background of the first cell of `text` on screen.
fn colors(t: &TestDriver, text: &str) -> (Color, Color) {
    let (x, y) = t
        .find_text(text)
        .unwrap_or_else(|| panic!("{text:?} is not on screen:\n{}", t.screen()));
    let cell = &t.buffer()[(x, y)];
    (cell.fg, cell.bg)
}

#[test]
fn blame_colors_the_code() {
    let sb = repo();
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 20);
    t.press("enter").wait_idle().press("b").wait_idle();
    assert_eq!(
        t.app().details().subject(),
        Some(&DetailsSubject::Blame("main.rs".into()))
    );
    let theme = Theme::default();
    assert_eq!(colors(&t, "// greet").0, fg(theme.code(Class::Comment)));
    assert_eq!(colors(&t, "\"hi\"").0, fg(theme.code(Class::String)));
    assert_eq!(colors(&t, "println!").0, fg(theme.code(Class::Macro)));
}

#[test]
fn commit_diffs_color_the_code_on_tinted_lines() {
    let sb = repo();
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 30);
    t.press("space").wait_idle().press("4").press("+");
    let theme = Theme::default();
    let add_bg = theme.diff_add_line.bg.unwrap();
    let del_bg = theme.diff_del_line.bg.unwrap();
    // The commit's `git show`.
    t.assert_pane_contains(PaneId::Details, "+    println!(\"hi\");");
    assert_eq!(colors(&t, "+    println!"), (fg(theme.diff_add), add_bg));
    assert_eq!(colors(&t, "println!(\"hi\")"), (fg(theme.code(Class::Macro)), add_bg));
    let (x, y) = t.find_text("-    let n = 1;").unwrap();
    assert_eq!(t.buffer()[(x, y)].fg, fg(theme.diff_del));
    // The tint runs across the line, past the code.
    assert_eq!(t.buffer()[(x + 40, y)].bg, del_bg);
    assert_eq!(colors(&t, "2;").0, fg(theme.code(Class::Number)));

    // The same diff in the commit's files view.
    t.press("+").press("1").press("enter").wait_idle();
    assert!(matches!(
        t.app().details().subject(),
        Some(DetailsSubject::CommitFiles(_))
    ));
    assert_eq!(colors(&t, "println!(\"hi\")"), (fg(theme.code(Class::Macro)), add_bg));
}

#[test]
fn staging_view_colors_the_code() {
    let sb = repo();
    sb.write("main.rs", &NEW.replace("\"hi\"", "\"bye\""));
    let mut t = TestDriver::with_size(Config::default(), &sb, 120, 30);
    t.press("2").press("g").press("enter").wait_idle();
    assert_eq!(
        t.app().details().subject(),
        Some(&DetailsSubject::Stage("main.rs".into()))
    );
    let theme = Theme::default();
    // The selected hunk's highlight shows over the tint.
    assert_eq!(
        colors(&t, "\"bye\""),
        (fg(theme.code(Class::String)), theme.selected.bg.unwrap())
    );
    assert_eq!(colors(&t, "-    println!").0, fg(theme.diff_del));
}

#[test]
fn syntax_off_leaves_diffs_and_blame_as_before() {
    let sb = repo();
    let config = Config {
        syntax: false,
        ..Config::default()
    };
    let mut t = TestDriver::with_size(config.clone(), &sb, 120, 30);
    t.press("space").wait_idle().press("4").press("+");
    let theme = Theme::default();
    assert_eq!(colors(&t, "println!(\"hi\")"), (fg(theme.diff_add), Color::Reset));
    let mut t = TestDriver::with_size(config, &sb, 120, 20);
    t.press("enter").wait_idle().press("b").wait_idle();
    assert_eq!(colors(&t, "// greet").0, Color::Reset);
}
