//! Zooming is the desktop app's: the TUI leaves the zoom commands out of
//! the palette and the help popup, and their keys only say so.

use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::config::ZoomStep;

#[test]
fn the_tui_leaves_zoom_out_of_the_palette_and_help() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.press(":").type_text("zoom");
    let screen = t.screen();
    assert!(!screen.contains("Zoom in"), "{screen}");
    t.press("esc").press("?").type_text("zoom");
    let screen = t.screen();
    assert!(!screen.contains("zoom in"), "{screen}");
}

#[test]
fn the_zoom_keys_in_the_tui_only_show_a_notice() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.press("ctrl+=");
    assert_eq!(t.app_mut().take_ui_zoom(), None);
    assert_eq!(
        t.app().notice(),
        Some("zooming needs the desktop app: use your terminal's font size")
    );
}

#[test]
fn a_front_end_that_zooms_offers_it_and_gets_the_request() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.app_mut().enable_ui_zoom();
    t.press(":").type_text("zoom in");
    let screen = t.screen();
    assert!(screen.contains("Zoom in"), "{screen}");
    t.press("enter");
    assert_eq!(t.app_mut().take_ui_zoom(), Some(ZoomStep::In));
    assert_eq!(t.app_mut().take_ui_zoom(), None, "taken once");
    t.press("ctrl+-");
    assert_eq!(t.app_mut().take_ui_zoom(), Some(ZoomStep::Out));
    t.press("ctrl+0");
    assert_eq!(t.app_mut().take_ui_zoom(), Some(ZoomStep::Reset));

    t.app_mut().ui_zoomed(110);
    assert_eq!(t.app().notice(), Some("zoom 110%"));
    assert_eq!(t.app().config().ui.zoom, 110);
}

#[test]
fn a_front_end_that_zooms_lists_the_keys_in_help() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.app_mut().enable_ui_zoom();
    t.press("?").type_text("zoom");
    let screen = t.screen();
    assert!(screen.contains("zoom in"), "{screen}");
}
