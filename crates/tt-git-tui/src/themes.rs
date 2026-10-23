//! Color themes: palettes in Omarchy's `colors.toml` format, the ones built
//! in, and finding a theme by the name config gives it.
//!
//! A name is looked up in this order:
//!
//! - `default`: tt-git's own colors, in the terminal's palette.
//! - `omarchy`: Omarchy's current theme.
//! - `omarchy:<name>`: one of Omarchy's themes, installed or the user's own.
//! - `<config>/themes/<name>.toml`, so a file there wins over a built-in.
//! - the built-in themes.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ratatui::style::Color;
use serde::Deserialize;

use crate::theme::{Theme, mix};

/// What `theme` means when config leaves it out.
pub const DEFAULT: &str = "default";
/// Follows Omarchy's current theme.
pub const OMARCHY: &str = "omarchy";

/// The themes tt-git ships, by name.
pub const BUILTIN: &[(&str, &str)] = &[
    ("catppuccin-frappe", include_str!("../themes/catppuccin-frappe.toml")),
    ("catppuccin-latte", include_str!("../themes/catppuccin-latte.toml")),
    (
        "catppuccin-macchiato",
        include_str!("../themes/catppuccin-macchiato.toml"),
    ),
    ("catppuccin-mocha", include_str!("../themes/catppuccin-mocha.toml")),
    ("dracula", include_str!("../themes/dracula.toml")),
    ("everforest", include_str!("../themes/everforest.toml")),
    ("gruvbox-dark", include_str!("../themes/gruvbox-dark.toml")),
    ("gruvbox-light", include_str!("../themes/gruvbox-light.toml")),
    ("kanagawa", include_str!("../themes/kanagawa.toml")),
    ("nord", include_str!("../themes/nord.toml")),
    ("one-dark", include_str!("../themes/one-dark.toml")),
    ("rose-pine", include_str!("../themes/rose-pine.toml")),
    ("rose-pine-dawn", include_str!("../themes/rose-pine-dawn.toml")),
    ("solarized-dark", include_str!("../themes/solarized-dark.toml")),
    ("solarized-light", include_str!("../themes/solarized-light.toml")),
    ("tokyo-night", include_str!("../themes/tokyo-night.toml")),
];

/// A theme's colors, as a `colors.toml` gives them, with the ones it leaves
/// out filled in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub light: bool,
    pub accent: Color,
    pub selection: Color,
    /// Borders.
    pub muted: Color,
    pub background: Color,
    pub lighter_background: Color,
    pub foreground: Color,
    /// Dim text: dates, hints, comments.
    pub dark_foreground: Color,
    pub red: Color,
    pub orange: Color,
    pub yellow: Color,
    pub green: Color,
    pub cyan: Color,
    pub blue: Color,
    pub magenta: Color,
    pub bright_red: Color,
    pub bright_yellow: Color,
    pub bright_green: Color,
    pub bright_cyan: Color,
    pub bright_blue: Color,
    pub bright_magenta: Color,
}

/// A `colors.toml` as written. Every key is optional, and keys tt-git doesn't
/// use (`brown`, `dark_background`…) are ignored: Omarchy adds keys over time.
#[derive(Debug, Default, Deserialize)]
struct PaletteFile {
    mode: Option<String>,
    accent: Option<String>,
    selection: Option<String>,
    muted: Option<String>,
    background: Option<String>,
    lighter_background: Option<String>,
    foreground: Option<String>,
    dark_foreground: Option<String>,
    red: Option<String>,
    orange: Option<String>,
    yellow: Option<String>,
    green: Option<String>,
    cyan: Option<String>,
    blue: Option<String>,
    magenta: Option<String>,
    bright_red: Option<String>,
    bright_yellow: Option<String>,
    bright_green: Option<String>,
    bright_cyan: Option<String>,
    bright_blue: Option<String>,
    bright_magenta: Option<String>,
}

impl Palette {
    /// Reads a `colors.toml`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: PaletteFile = toml::from_str(text).map_err(|e| e.message().to_string())?;
        let color = |key: &str, value: &Option<String>| -> Result<Option<Color>, String> {
            value
                .as_deref()
                .map(|v| parse_hex(v).ok_or_else(|| format!("{key} = \"{v}\" isn't a color like \"#1a2b3c\"")))
                .transpose()
        };
        let background = color("background", &file.background)?;
        let light = match file.mode.as_deref() {
            Some("light") => true,
            Some("dark") => false,
            Some(other) => return Err(format!("mode = \"{other}\" isn't \"dark\" or \"light\"")),
            None => background.is_some_and(is_light),
        };
        // Hues a palette leaves out come from One Dark or One Light.
        let base = |dark: (u8, u8, u8), lite: (u8, u8, u8)| {
            let (r, g, b) = if light { lite } else { dark };
            Color::Rgb(r, g, b)
        };
        let background = background.unwrap_or(base((30, 30, 30), (250, 250, 250)));
        let foreground = color("foreground", &file.foreground)?.unwrap_or(base((212, 212, 212), (56, 58, 66)));
        let red = color("red", &file.red)?.unwrap_or(base((224, 108, 117), (228, 86, 73)));
        let yellow = color("yellow", &file.yellow)?.unwrap_or(base((229, 192, 123), (193, 132, 1)));
        let green = color("green", &file.green)?.unwrap_or(base((152, 195, 121), (80, 161, 79)));
        let blue = color("blue", &file.blue)?.unwrap_or(base((97, 175, 239), (64, 120, 242)));
        let cyan = color("cyan", &file.cyan)?.unwrap_or(base((86, 182, 194), (1, 132, 188)));
        let magenta = color("magenta", &file.magenta)?.unwrap_or(base((198, 120, 221), (166, 38, 164)));
        let lighter_background =
            color("lighter_background", &file.lighter_background)?.unwrap_or(mix(background, foreground, 0.08));
        Ok(Self {
            light,
            accent: color("accent", &file.accent)?.unwrap_or(blue),
            selection: color("selection", &file.selection)?.unwrap_or(mix(background, foreground, 0.18)),
            muted: color("muted", &file.muted)?.unwrap_or(mix(background, foreground, 0.3)),
            background,
            lighter_background,
            foreground,
            dark_foreground: color("dark_foreground", &file.dark_foreground)?
                .unwrap_or(mix(foreground, background, 0.45)),
            orange: color("orange", &file.orange)?.unwrap_or(yellow),
            bright_red: color("bright_red", &file.bright_red)?.unwrap_or(red),
            bright_yellow: color("bright_yellow", &file.bright_yellow)?.unwrap_or(yellow),
            bright_green: color("bright_green", &file.bright_green)?.unwrap_or(green),
            bright_cyan: color("bright_cyan", &file.bright_cyan)?.unwrap_or(cyan),
            bright_blue: color("bright_blue", &file.bright_blue)?.unwrap_or(blue),
            bright_magenta: color("bright_magenta", &file.bright_magenta)?.unwrap_or(magenta),
            red,
            yellow,
            green,
            cyan,
            blue,
            magenta,
        })
    }
}

/// `#1a2b3c` (the `#` is optional) or `#abc`.
fn parse_hex(s: &str) -> Option<Color> {
    let hex = s.trim().strip_prefix('#').unwrap_or(s.trim());
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    let nibble = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok().map(|n| n * 17);
    match hex.len() {
        6 => Some(Color::Rgb(byte(0)?, byte(2)?, byte(4)?)),
        3 => Some(Color::Rgb(nibble(0)?, nibble(1)?, nibble(2)?)),
        _ => None,
    }
}

fn is_light(c: Color) -> bool {
    match c {
        Color::Rgb(r, g, b) => 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b) > 140.0,
        _ => false,
    }
}

/// Where themes are looked up.
#[derive(Debug, Clone, Default)]
pub struct ThemeDirs {
    /// tt-git's config directory; its `themes` folder holds the user's own.
    pub config: Option<PathBuf>,
    /// For Omarchy's folders.
    pub home: Option<PathBuf>,
    /// `$OMARCHY_PATH`, where Omarchy is installed (by default
    /// `~/.local/share/omarchy`).
    pub omarchy: Option<PathBuf>,
}

impl ThemeDirs {
    pub fn from_env() -> Self {
        Self {
            config: Some(tt_git_core::paths::config_dir()),
            home: tt_git_core::paths::home_dir(),
            omarchy: std::env::var_os("OMARCHY_PATH")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
        }
    }

    fn user_dir(&self) -> Option<PathBuf> {
        self.config.as_ref().map(|c| c.join("themes"))
    }

    /// Omarchy's current theme folder: where Omarchy 3 keeps it, then where
    /// earlier versions did.
    fn omarchy_current(&self) -> Vec<PathBuf> {
        let Some(home) = &self.home else {
            return Vec::new();
        };
        vec![
            home.join(".local/state/omarchy/current/theme"),
            home.join(".config/omarchy/current/theme"),
        ]
    }

    /// Folders of Omarchy themes: the user's (installed or their own) first,
    /// as Omarchy lays them over its own.
    fn omarchy_themes(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(home) = &self.home {
            dirs.push(home.join(".config/omarchy/themes"));
        }
        match (&self.omarchy, &self.home) {
            (Some(path), _) => dirs.push(path.join("themes")),
            (None, Some(home)) => dirs.push(home.join(".local/share/omarchy/themes")),
            (None, None) => {}
        }
        dirs
    }

    /// The name of Omarchy's current theme, as `omarchy theme set` wrote it.
    pub fn omarchy_current_name(&self) -> Option<String> {
        self.omarchy_current().iter().find_map(|dir| {
            let name = std::fs::read_to_string(dir.with_file_name("theme.name")).ok()?;
            Some(name.trim().to_string()).filter(|n| !n.is_empty())
        })
    }
}

/// Where the app finds themes and saves the one the picker picks.
#[derive(Debug, Clone, Default)]
pub struct ThemeOptions {
    pub dirs: ThemeDirs,
    /// The config file a picked theme is saved to; `None` saves nothing.
    pub config_path: Option<PathBuf>,
}

/// A theme ready to draw with.
#[derive(Debug, Clone)]
pub struct LoadedTheme {
    pub name: String,
    pub theme: Theme,
    /// The file it was read from, watched so changes show at once. `None`
    /// for the built-ins.
    pub file: Option<PathBuf>,
}

impl LoadedTheme {
    pub fn default_theme() -> Self {
        Self {
            name: DEFAULT.into(),
            theme: Theme::default(),
            file: None,
        }
    }
}

/// Finds and reads theme `name`.
pub fn load(name: &str, dirs: &ThemeDirs) -> Result<LoadedTheme, String> {
    let loaded = |file: Option<PathBuf>, text: &str| -> Result<LoadedTheme, String> {
        let palette = Palette::parse(text).map_err(|e| match &file {
            Some(f) => format!("theme {name} ({}): {e}", f.display()),
            None => format!("theme {name}: {e}"),
        })?;
        Ok(LoadedTheme {
            name: name.to_string(),
            theme: Theme::from_palette(&palette),
            file,
        })
    };
    let from_file = |path: PathBuf| -> Result<LoadedTheme, String> {
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("theme {name}: reading {}: {e}", path.display()))?;
        loaded(Some(path), &text)
    };
    if name == DEFAULT {
        return Ok(LoadedTheme::default_theme());
    }
    if name == OMARCHY {
        let candidates: Vec<PathBuf> = dirs.omarchy_current().iter().map(|d| d.join("colors.toml")).collect();
        return match candidates.iter().find(|p| p.is_file()) {
            Some(path) => from_file(path.clone()),
            None => Err(format!(
                "theme omarchy: there's no current Omarchy theme (looked for {})",
                candidates
                    .first()
                    .map_or("~/.local/state/omarchy/current/theme/colors.toml".into(), |p| p
                        .display()
                        .to_string())
            )),
        };
    }
    if let Some(theme) = name.strip_prefix("omarchy:") {
        return match dirs
            .omarchy_themes()
            .iter()
            .map(|d| d.join(theme).join("colors.toml"))
            .find(|p| p.is_file())
        {
            Some(path) => from_file(path),
            None => Err(format!(
                "theme {name}: there's no Omarchy theme called {theme} with a colors.toml"
            )),
        };
    }
    if let Some(path) = dirs.user_dir().map(|d| d.join(format!("{name}.toml")))
        && path.is_file()
    {
        return from_file(path);
    }
    match BUILTIN.iter().find(|(n, _)| *n == name) {
        Some((_, text)) => loaded(None, text),
        None => Err(format!(
            "theme {name}: no such theme. Built in are default, omarchy and {}{}",
            BUILTIN.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", "),
            dirs.user_dir()
                .map(|d| format!("; your own go in {}", d.join(format!("{name}.toml")).display()))
                .unwrap_or_default()
        )),
    }
}

/// A theme the picker offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeEntry {
    pub name: String,
    /// Where it comes from, shown beside it.
    pub hint: String,
}

/// Every theme there is: default, Omarchy's current one and its others when
/// Omarchy is installed, then the built-ins and the user's own by name (a
/// file of the user's replaces the built-in of that name).
pub fn list(dirs: &ThemeDirs) -> Vec<ThemeEntry> {
    let entry = |name: String, hint: &str| ThemeEntry {
        name,
        hint: hint.to_string(),
    };
    let mut out = vec![entry(DEFAULT.into(), "terminal colors")];
    if dirs.omarchy_current().iter().any(|d| d.join("colors.toml").is_file()) {
        let hint = match dirs.omarchy_current_name() {
            Some(current) => format!("follows Omarchy ({current})"),
            None => "follows Omarchy".into(),
        };
        out.push(entry(OMARCHY.into(), &hint));
    }
    let mut omarchy: Vec<String> = Vec::new();
    for dir in dirs.omarchy_themes() {
        for name in names_in(&dir, |p| p.is_dir() && p.join("colors.toml").is_file()) {
            if !omarchy.contains(&name) {
                omarchy.push(name);
            }
        }
    }
    omarchy.sort();
    out.extend(omarchy.into_iter().map(|n| entry(format!("omarchy:{n}"), "Omarchy")));
    let own = dirs
        .user_dir()
        .map(|d| names_in(&d, |p| p.extension().is_some_and(|e| e == "toml")))
        .unwrap_or_default();
    let mut rest: Vec<ThemeEntry> = BUILTIN
        .iter()
        .filter(|(n, _)| !own.iter().any(|o| o == n))
        .map(|(n, _)| entry((*n).into(), "built in"))
        .collect();
    rest.extend(own.into_iter().map(|n| entry(n, "yours")));
    rest.sort_by(|a, b| a.name.cmp(&b.name));
    out.extend(rest);
    out
}

/// The names (file stems) of the entries in `dir` that pass `keep`.
fn names_in(dir: &Path, keep: impl Fn(&Path) -> bool) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| keep(p))
        .filter_map(|p| Some(p.file_stem()?.to_str()?.to_string()))
        .collect()
}

/// What a theme file looks like on disk, to spot a change without reading it.
pub type Stamp = Option<(SystemTime, u64)>;

pub fn stamp(path: &Path) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_parses_and_sets_every_color() {
        for (name, text) in BUILTIN {
            let palette = Palette::parse(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            let file: PaletteFile = toml::from_str(text).unwrap();
            let missing: Vec<&str> = [
                ("mode", file.mode.is_some()),
                ("accent", file.accent.is_some()),
                ("selection", file.selection.is_some()),
                ("muted", file.muted.is_some()),
                ("background", file.background.is_some()),
                ("lighter_background", file.lighter_background.is_some()),
                ("foreground", file.foreground.is_some()),
                ("dark_foreground", file.dark_foreground.is_some()),
                ("orange", file.orange.is_some()),
                ("bright_magenta", file.bright_magenta.is_some()),
            ]
            .into_iter()
            .filter(|(_, set)| !set)
            .map(|(k, _)| k)
            .collect();
            assert!(missing.is_empty(), "{name} leaves out {missing:?}");
            assert_eq!(palette.light, is_light(palette.background), "{name}'s mode");
        }
    }

    #[test]
    fn missing_colors_fall_back() {
        let p = Palette::parse("background = \"#ffffff\"\nforeground = \"#000000\"\nblue = \"#0000ff\"").unwrap();
        assert!(p.light, "a light background means a light palette");
        assert_eq!(p.accent, Color::Rgb(0, 0, 255), "accent falls back to blue");
        assert_eq!(p.bright_blue, p.blue);
        assert_eq!(p.orange, p.yellow);
        assert_eq!(
            p.dark_foreground,
            mix(Color::Rgb(0, 0, 0), Color::Rgb(255, 255, 255), 0.45)
        );
        // Omarchy's keys tt-git doesn't use are fine.
        Palette::parse("brown = \"#75493d\"\ndarker_background = \"#000\"").unwrap();
    }

    #[test]
    fn bad_colors_are_reported() {
        let err = Palette::parse("red = \"crimson\"").unwrap_err();
        assert!(err.contains("red = \"crimson\""), "{err}");
        let err = Palette::parse("mode = \"dim\"").unwrap_err();
        assert!(err.contains("mode"), "{err}");
        assert_eq!(parse_hex("#abc"), Some(Color::Rgb(0xaa, 0xbb, 0xcc)));
        assert_eq!(parse_hex("1a2b3c"), Some(Color::Rgb(0x1a, 0x2b, 0x3c)));
        assert_eq!(parse_hex("#1a2b3"), None);
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn sandbox() -> (tempfile::TempDir, ThemeDirs) {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = ThemeDirs {
            config: Some(tmp.path().join("config")),
            home: Some(tmp.path().join("home")),
            omarchy: None,
        };
        (tmp, dirs)
    }

    #[test]
    fn names_resolve_in_order() {
        let (_tmp, dirs) = sandbox();
        assert!(load("default", &dirs).unwrap().theme.background.is_none());
        let nord = load("nord", &dirs).unwrap();
        assert_eq!(nord.theme.background, Some(Color::Rgb(0x2e, 0x34, 0x40)));
        assert_eq!(nord.file, None);

        // A file of your own replaces the built-in.
        let mine = dirs.config.as_ref().unwrap().join("themes/nord.toml");
        write(&mine, "background = \"#010203\"");
        let nord = load("nord", &dirs).unwrap();
        assert_eq!(nord.theme.background, Some(Color::Rgb(1, 2, 3)));
        assert_eq!(nord.file.as_deref(), Some(mine.as_path()));

        let err = load("nrod", &dirs).unwrap_err();
        assert!(err.contains("no such theme") && err.contains("tokyo-night"), "{err}");
    }

    #[test]
    fn omarchy_current_and_named_themes() {
        let (tmp, dirs) = sandbox();
        let err = load("omarchy", &dirs).unwrap_err();
        assert!(err.contains("no current Omarchy theme"), "{err}");

        let home = tmp.path().join("home");
        // Omarchy before 3 kept the current theme under ~/.config.
        write(
            &home.join(".config/omarchy/current/theme/colors.toml"),
            "accent = \"#111111\"",
        );
        assert_eq!(
            load("omarchy", &dirs).unwrap().theme.border_focused.fg,
            Some(Color::Rgb(17, 17, 17))
        );
        let current = home.join(".local/state/omarchy/current/theme/colors.toml");
        write(&current, "accent = \"#222222\"");
        write(&home.join(".local/state/omarchy/current/theme.name"), "tokyo-night\n");
        let loaded = load("omarchy", &dirs).unwrap();
        assert_eq!(loaded.theme.border_focused.fg, Some(Color::Rgb(34, 34, 34)));
        assert_eq!(loaded.file.as_deref(), Some(current.as_path()));

        write(
            &home.join(".local/share/omarchy/themes/lumon/colors.toml"),
            "accent = \"#333333\"",
        );
        write(
            &home.join(".config/omarchy/themes/lumon/colors.toml"),
            "accent = \"#444444\"",
        );
        write(
            &home.join(".config/omarchy/themes/mine/colors.toml"),
            "accent = \"#555555\"",
        );
        // The user's copy is laid over Omarchy's own.
        let lumon = load("omarchy:lumon", &dirs).unwrap();
        assert_eq!(lumon.theme.border_focused.fg, Some(Color::Rgb(68, 68, 68)));
        assert!(load("omarchy:nope", &dirs).is_err());

        let names: Vec<(String, String)> = list(&dirs).into_iter().map(|e| (e.name, e.hint)).collect();
        assert_eq!(names[0].0, "default");
        assert_eq!(names[1], ("omarchy".into(), "follows Omarchy (tokyo-night)".into()));
        assert_eq!(names[2].0, "omarchy:lumon");
        assert_eq!(names[3].0, "omarchy:mine");
        assert_eq!(names[4].0, "catppuccin-frappe");
    }

    #[test]
    fn your_own_themes_are_listed_among_the_built_ins() {
        let (_tmp, dirs) = sandbox();
        let themes = dirs.config.as_ref().unwrap().join("themes");
        write(&themes.join("nord.toml"), "");
        write(&themes.join("mine.toml"), "");
        let names: Vec<(String, String)> = list(&dirs).into_iter().map(|e| (e.name, e.hint)).collect();
        assert!(
            !names
                .iter()
                .any(|(n, h)| n == "omarchy" || (n == "nord" && h == "built in"))
        );
        let at = |name: &str| names.iter().position(|(n, _)| n == name).unwrap();
        assert_eq!(names[at("nord")].1, "yours");
        assert_eq!(names[at("mine")].1, "yours");
        assert_eq!(at("mine") + 1, at("nord"), "{names:?}");
        assert_eq!(names.len(), 1 + BUILTIN.len() + 1);
    }
}
