use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::modal::Modal;

fn sandbox() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "a", "first");
    sb
}

/// `screen` with the About dialog's version line written as
/// `tt-git <version>`, centered in the box as the real one is, so the
/// snapshot outlives a version bump. On a border (the keys popup's
/// title, right-aligned), the dashes before it give or take the
/// difference in length, as they would for a version that long.
fn without_version(screen: &str) -> String {
    const PLACEHOLDER: &str = "tt-git <version>";
    let heading = concat!("tt-git ", env!("CARGO_PKG_VERSION"));
    screen
        .lines()
        .map(|line| {
            let Some(at) = line.find(heading) else {
                return line.to_string();
            };
            // The box's borders either side of it.
            let Some(left) = line[..at].rfind('│') else {
                let before = line[..at].trim_end_matches(' ');
                let dashes = before.len() - before.trim_end_matches('─').len();
                let dashes = dashes / '─'.len_utf8();
                let kept = (dashes + heading.len()).saturating_sub(PLACEHOLDER.len());
                let start = before.len() - dashes * '─'.len_utf8();
                return format!(
                    "{}{}{}{PLACEHOLDER}{}",
                    &line[..start],
                    "─".repeat(kept),
                    &line[before.len()..at],
                    &line[at + heading.len()..]
                );
            };
            let right = at + line[at..].find('│').expect("right border");
            let inside = line[left + '│'.len_utf8()..right].chars().count();
            // A column of padding each side, as the dialog has.
            let text = format!(" {PLACEHOLDER:^w$} ", w = inside - 2);
            format!("{}{text}{}", &line[..left + '│'.len_utf8()], &line[right..])
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_palette_opens_about() {
    let sb = sandbox();
    let mut t = TestDriver::new(&sb);
    t.press(":").type_text("about").press("enter");
    assert_eq!(t.app().modal(), Some(&Modal::About));
    let screen = t.screen();
    assert!(
        screen.contains(concat!("tt-git ", env!("CARGO_PKG_VERSION"))),
        "{screen}"
    );
    insta::assert_snapshot!(without_version(&screen));
}

#[test]
fn esc_enter_q_or_space_close_it_and_other_keys_wait() {
    let sb = sandbox();
    let mut t = TestDriver::new(&sb);
    for key in ["esc", "enter", "q", "space"] {
        t.press(":").type_text("about").press("enter");
        t.press("tab");
        assert_eq!(t.app().modal(), Some(&Modal::About), "tab waits");
        t.press(key);
        assert_eq!(t.app().modal(), None, "{key} closes it");
        assert!(!t.app().should_quit(), "{key} doesn't quit");
    }
}

#[test]
fn the_keys_popup_has_an_about_tab() {
    let sb = sandbox();
    let mut t = TestDriver::new(&sb);
    t.press("?");
    assert!(t.screen().contains("Keys · About   tab switch"), "{}", t.screen());
    t.press("tab");
    let screen = t.screen();
    assert!(screen.contains("Developer: Jonathan Sheely"), "{screen}");
    assert!(!screen.contains("type to search"), "{screen}");
    insta::assert_snapshot!(without_version(&screen));
    t.press("tab");
    assert!(t.screen().contains("type to search"), "back on Keys");
}
