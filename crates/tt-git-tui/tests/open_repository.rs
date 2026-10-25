//! "Open repository" is the desktop app's: the TUI never offers it, and a
//! key bound to it only says so.

use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Config;

#[test]
fn the_tui_palette_leaves_open_repository_out() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.press(":").type_text("open repo");
    let screen = t.screen();
    assert!(!screen.contains("Open repository"), "{screen}");
}

#[test]
fn a_key_bound_to_it_in_the_tui_only_shows_a_notice() {
    let sb = Sandbox::new();
    let cfg = Config::parse("[keys.global]\nopen_repository = \"ctrl+o\"\n").unwrap();
    let mut t = TestDriver::with_config(cfg, &sb);
    t.press("ctrl+o");
    assert!(!t.app_mut().take_open_repository());
    assert_eq!(
        t.app().notice(),
        Some("opening another repository needs the desktop app")
    );
    assert!(!t.app().should_quit());
}

#[test]
fn a_front_end_that_opens_repos_offers_it_and_gets_the_request() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.app_mut().enable_open_repository();
    t.press(":").type_text("open repo");
    let screen = t.screen();
    assert!(screen.contains("Open repository"), "{screen}");
    t.press("enter");
    assert!(t.app_mut().take_open_repository());
    assert!(!t.app_mut().take_open_repository(), "taken once");
}
