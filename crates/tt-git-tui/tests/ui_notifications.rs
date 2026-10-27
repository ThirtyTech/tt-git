//! The notifications picker is the desktop app's: the TUI leaves it out of
//! the palette, and running the command only says so. A front end that
//! notifies offers it, and each entry flips a setting and saves it to
//! config.toml, keeping what else is there.

use tt_git_app::config::Alert;
use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::keymap::Command;
use tt_git_tui::{Action, Config};

#[test]
fn the_tui_leaves_notifications_out_of_the_palette() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    t.press(":").type_text("notifications");
    let screen = t.screen();
    assert!(!screen.contains("Notifications"), "{screen}");

    t.press("esc");
    t.send(Action::Command(Command::Notifications));
    assert!(t.app().modal().is_none());
    assert_eq!(
        t.app().notice(),
        Some("notifications are the desktop app's: the terminal has none")
    );
}

#[test]
fn the_picker_flips_and_saves_each_setting() {
    let sb = Sandbox::new();
    let config_file = sb.root().join("tt-config").join("config.toml");
    std::fs::create_dir_all(config_file.parent().unwrap()).unwrap();
    std::fs::write(&config_file, "# mine\n[ui]\nzoom = 120 # big\n").unwrap();
    let mut t = TestDriver::new(&sb);
    t.app_mut().enable_themes(tt_git_tui::themes::ThemeOptions {
        dirs: Default::default(),
        config_path: Some(config_file.clone()),
    });
    t.app_mut().enable_notifications();

    t.press(":").type_text("notifications").press("enter");
    let screen = t.screen();
    for line in [
        "Notifications",
        "New commits on remote branches",
        "New commits on local branches and worktrees",
        "Conflicts: badge on the app icon",
        "on (notifications off)",
    ] {
        assert!(screen.contains(line), "{line}: {screen}");
    }

    // The switch for them all, on the first line.
    t.press("enter");
    assert!(t.app().config().ui.notifications);
    assert!(t.app().modal().is_some(), "stays open");
    assert!(t.app().notice().unwrap().starts_with("notifications on, saved to"));
    assert_eq!(t.app_mut().take_alert_settings().map(|s| s.notifications), Some(true));
    assert_eq!(t.app_mut().take_alert_settings(), None, "taken once");

    // One alert, found by typing; the picker stays on it.
    t.type_text("conflicts").press("enter");
    assert!(!t.app().config().ui.alerts_on(Alert::Conflicts));
    assert!(t.app().config().ui.alerts_on(Alert::Stopped));
    t.render();
    assert!(t.screen().contains("off"), "{}", t.screen());
    t.press("enter");
    assert!(t.app().config().ui.alerts_on(Alert::Conflicts), "flipped back");
    t.press("enter").press("esc");
    assert!(t.app().modal().is_none());

    assert_eq!(
        std::fs::read_to_string(&config_file).unwrap(),
        "# mine\n[ui]\nzoom = 120 # big\nnotifications = true\n\n[ui.alerts]\nconflicts = false\n"
    );
    let saved = Config::load(&config_file).unwrap().ui;
    assert!(saved.notifications);
    assert!(!saved.alerts.conflicts);
    assert_eq!(saved.zoom, 120);
}

#[test]
fn settings_from_another_window_are_used() {
    let sb = Sandbox::new();
    let mut t = TestDriver::new(&sb);
    let mut settings = t.app().alert_settings();
    settings.notifications = true;
    settings.alerts.set(Alert::Popup, false);
    t.app_mut().set_alert_settings(settings);
    assert!(t.app().config().ui.alerts_on(Alert::Badge));
    assert!(!t.app().config().ui.alerts_on(Alert::Popup));
    assert_eq!(t.app_mut().take_alert_settings(), None, "not handed back out");
}
