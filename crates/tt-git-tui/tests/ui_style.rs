//! The window style is the desktop app's: the TUI leaves the style command
//! out of the palette, and running it only says so. A front end with styles
//! offers it, takes it, and has the app save the style it switched to.

use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Action;
use tt_git_tui::config::UiStyle;
use tt_git_tui::keymap::Command;

#[test]
fn the_tui_leaves_the_style_out_of_the_palette() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.press(":").type_text("terminal style");
    let screen = t.screen();
    assert!(!screen.contains("Native/terminal style"), "{screen}");
}

#[test]
fn the_style_command_in_the_tui_only_shows_a_notice() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.send(Action::Command(Command::WindowStyle));
    assert!(!t.app_mut().take_ui_style());
    assert_eq!(
        t.app().notice(),
        Some("the window style is the desktop app's: the terminal draws its own")
    );
}

#[test]
fn a_front_end_with_styles_offers_and_takes_the_command() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.app_mut().enable_ui_style();
    t.press(":").type_text("terminal style");
    let screen = t.screen();
    assert!(screen.contains("Native/terminal style"), "{screen}");
    t.press("enter");
    assert!(t.app_mut().take_ui_style());
    assert!(!t.app_mut().take_ui_style(), "taken once");

    t.app_mut().ui_styled(UiStyle::Terminal);
    assert_eq!(t.app().notice(), Some("terminal style"));
}
