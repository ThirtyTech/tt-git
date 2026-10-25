use std::path::{Path, PathBuf};

use ratatui::style::Color;
use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::keymap::Command;
use tt_git_tui::themes::{ThemeDirs, ThemeOptions};
use tt_git_tui::{Action, Config};

/// Theme folders and a config file of the test's own, so nothing reads the
/// user's.
struct Dirs {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    config_file: PathBuf,
    options: ThemeOptions,
}

fn dirs() -> Dirs {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let config = tmp.path().join("config");
    let config_file = config.join("config.toml");
    let options = ThemeOptions {
        dirs: ThemeDirs {
            config: Some(config),
            home: Some(home.clone()),
            omarchy: None,
        },
        config_path: Some(config_file.clone()),
    };
    Dirs {
        _tmp: tmp,
        home,
        config_file,
        options,
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn themed(config: Config, sb: &Sandbox, d: &Dirs) -> TestDriver {
    let mut t = TestDriver::with_config(config, sb);
    t.app_mut().enable_themes(d.options.clone());
    t.render();
    t
}

fn accent(t: &TestDriver) -> Option<Color> {
    t.app().theme().border_focused.fg.map(tt_git_tui::tui_theme::color)
}

#[test]
fn config_names_the_theme() {
    let sb = Sandbox::new();
    let d = dirs();
    let config = Config {
        theme: Some("nord".into()),
        ..Config::default()
    };
    let t = themed(config, &sb, &d);
    assert_eq!(t.app().theme_name(), "nord");
    assert_eq!(accent(&t), Some(Color::Rgb(0x88, 0xc0, 0xd0)));
    // The terminal's background is left alone unless asked.
    assert_eq!(t.buffer()[(0, 0)].bg, Color::Reset);
}

#[test]
fn an_unknown_theme_is_reported_and_the_default_kept() {
    let sb = Sandbox::new();
    let d = dirs();
    let config = Config {
        theme: Some("nrod".into()),
        ..Config::default()
    };
    let t = themed(config, &sb, &d);
    assert_eq!(t.app().theme_name(), "default");
    let err = t.app().error().unwrap();
    assert!(err.contains("theme nrod: no such theme"), "{err}");
}

#[test]
fn theme_background_paints_the_whole_screen() {
    let sb = Sandbox::new();
    let d = dirs();
    let config = Config {
        theme: Some("nord".into()),
        theme_background: true,
        ..Config::default()
    };
    let t = themed(config, &sb, &d);
    let nord = Color::Rgb(0x2e, 0x34, 0x40);
    let buf = t.buffer();
    let left_alone = buf
        .content
        .iter()
        .filter(|c| c.bg == Color::Reset || c.fg == Color::Reset)
        .count();
    assert_eq!(left_alone, 0);
    assert_eq!(buf[(buf.area.width / 2, buf.area.height / 2)].bg, nord);
}

#[test]
fn the_picker_previews_restores_and_saves() {
    let sb = Sandbox::new();
    let d = dirs();
    write(&d.config_file, "# mine\nmouse = true\n\n[graph]\nmax_lanes = 8\n");
    let mut t = themed(Config::default(), &sb, &d);
    let default_accent = accent(&t);

    t.send(Action::Command(Command::Theme));
    assert!(t.app().modal().is_some());
    // Moving through the list shows each theme at once.
    t.press("down");
    assert_eq!(t.app().theme_name(), "catppuccin-frappe");
    assert_ne!(accent(&t), default_accent);
    t.press("down");
    assert_eq!(t.app().theme_name(), "catppuccin-latte");
    // esc puts the theme back.
    t.press("esc");
    assert!(t.app().modal().is_none());
    assert_eq!(t.app().theme_name(), "default");
    assert_eq!(accent(&t), default_accent);

    // Picking one keeps it and saves it to config.toml.
    t.send(Action::Command(Command::Theme));
    t.type_text("nord").press("enter");
    assert_eq!(t.app().theme_name(), "nord");
    assert_eq!(accent(&t), Some(Color::Rgb(0x88, 0xc0, 0xd0)));
    assert!(t.app().notice().unwrap().contains("theme nord, saved to"));
    let saved = std::fs::read_to_string(&d.config_file).unwrap();
    assert_eq!(
        saved,
        "# mine\nmouse = true\ntheme = \"nord\"\n\n[graph]\nmax_lanes = 8\n"
    );
    Config::parse(&saved).unwrap();

    // The picker opens on the theme in use.
    t.send(Action::Command(Command::Theme));
    let selected = t.app().modal().unwrap().palette_selection().cloned();
    assert_eq!(selected, Some(tt_git_tui::modal::PaletteTarget::Theme("nord".into())));
}

#[test]
fn the_omarchy_theme_follows_omarchy() {
    let sb = Sandbox::new();
    let d = dirs();
    let current = d.home.join(".local/state/omarchy/current/theme");
    write(&current.join("colors.toml"), "accent = \"#111111\"\n");
    write(&current.with_file_name("theme.name"), "lumon\n");
    let config = Config {
        theme: Some("omarchy".into()),
        ..Config::default()
    };
    let mut t = themed(config, &sb, &d);
    assert_eq!(accent(&t), Some(Color::Rgb(17, 17, 17)));

    // `omarchy theme set` removes the folder and moves the new one in.
    std::fs::remove_dir_all(&current).unwrap();
    t.tick(4);
    assert_eq!(t.app().error(), None, "a theme that's gone for a moment isn't an error");
    assert_eq!(accent(&t), Some(Color::Rgb(17, 17, 17)));
    write(&current.join("colors.toml"), "# tokyo night\naccent = \"#7aa2f7\"\n");
    t.tick(4);
    assert_eq!(accent(&t), Some(Color::Rgb(0x7a, 0xa2, 0xf7)));

    // The picker says which theme Omarchy has.
    t.send(Action::Command(Command::Theme));
    let screen = t.screen();
    assert!(screen.contains("follows Omarchy (lumon)"), "{screen}");
}
