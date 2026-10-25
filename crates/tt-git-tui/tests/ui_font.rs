//! Choosing the font is the desktop app's: the TUI leaves the font command
//! out of the palette, and running it only says so. A front end that draws
//! its own text lists the fonts, previews the highlighted one and gets the
//! one picked.

use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::Action;
use tt_git_tui::app::FontChoice;
use tt_git_tui::keymap::Command;

fn fonts() -> Vec<FontChoice> {
    let font = |family: &str, monospace| FontChoice {
        family: family.into(),
        monospace,
    };
    vec![
        font("Arial", false),
        font("Fira Code", true),
        font("JetBrains Mono", true),
    ]
}

#[test]
fn the_tui_leaves_the_font_out_of_the_palette() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.press(":").type_text("font");
    let screen = t.screen();
    assert!(!screen.contains("Font"), "{screen}");
}

#[test]
fn the_font_command_in_the_tui_only_shows_a_notice() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.send(Action::Command(Command::Font));
    assert!(!t.app_mut().take_font_picker());
    assert_eq!(
        t.app().notice(),
        Some("choosing a font needs the desktop app: use your terminal's font")
    );
}

#[test]
fn a_front_end_that_chooses_fonts_offers_the_picker() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.app_mut().enable_ui_fonts();
    t.press(":").type_text("font");
    let screen = t.screen();
    assert!(screen.contains("Font"), "{screen}");
    t.press("enter");
    assert!(t.app_mut().take_font_picker());
    assert!(!t.app_mut().take_font_picker(), "taken once");
    assert!(t.app().modal().is_none(), "the front end opens the picker");

    t.app_mut()
        .open_font_picker(fonts(), Some("JetBrains Mono"), "Cascadia Mono");
    t.render();
    let screen = t.screen();
    for line in ["Default (Cascadia Mono)", "Fira Code", "Arial", "proportional"] {
        assert!(screen.contains(line), "{line}: {screen}");
    }
    let mono_first = screen.find("JetBrains Mono").unwrap() < screen.find("Arial").unwrap();
    assert!(mono_first, "monospace fonts first: {screen}");
    assert_eq!(
        t.app().font_preview(),
        Some(Some("JetBrains Mono")),
        "on the current font"
    );

    t.press("up");
    assert_eq!(t.app().font_preview(), Some(Some("Fira Code")));
    t.press("up");
    assert_eq!(t.app().font_preview(), Some(None), "the default");
    t.type_text("arial");
    assert_eq!(t.app().font_preview(), Some(Some("Arial")), "narrowed to what's typed");
    t.press("enter");
    assert!(t.app().modal().is_none());
    assert_eq!(t.app().font_preview(), None);
    assert_eq!(t.app_mut().take_font_pick(), Some(Some("Arial".into())));
    assert_eq!(t.app_mut().take_font_pick(), None, "taken once");
    assert_eq!(t.app().config().ui.font(), Some("Arial"));
    assert_eq!(t.app().notice(), Some("font Arial"));
}

#[test]
fn closing_the_font_picker_keeps_the_font() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.app_mut().enable_ui_fonts();
    t.app_mut().open_font_picker(fonts(), None, "Cascadia Mono");
    assert_eq!(t.app().font_preview(), Some(None), "on the default");
    t.press("down");
    assert_eq!(t.app().font_preview(), Some(Some("Fira Code")));
    t.press("esc");
    assert_eq!(t.app().font_preview(), None);
    assert_eq!(t.app_mut().take_font_pick(), None);
}

#[test]
fn picking_the_default_font() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.app_mut().enable_ui_fonts();
    t.app_mut()
        .open_font_picker(fonts(), Some("Fira Code"), "Cascadia Mono");
    t.press("up").press("enter");
    assert_eq!(t.app_mut().take_font_pick(), Some(None));
    assert_eq!(t.app().config().ui.font(), None);
    assert_eq!(t.app().notice(), Some("default font"));
}

#[test]
fn a_missing_font_is_reported() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.app_mut().ui_font_missing("Nope Mono", "Cascadia Mono");
    assert_eq!(
        t.app().notice(),
        Some("font Nope Mono isn't installed: using Cascadia Mono")
    );
}
