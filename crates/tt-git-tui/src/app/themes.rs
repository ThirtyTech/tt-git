//! Color themes: the one config names, the picker that previews each theme
//! as it's highlighted, and following the theme's file (Omarchy's current
//! theme, or the user's own palette) as it changes.

use std::path::Path;

use super::App;
use crate::modal::{Modal, PaletteItem, PaletteTarget};
use crate::themes::{self, LoadedTheme, ThemeDirs, ThemeOptions};

/// Ticks between looks at the theme's file (about a second).
const THEME_POLL_TICKS: u64 = 4;

impl App {
    /// Uses the theme config names, looking themes up in `options.dirs`, and
    /// lets the picker save to `options.config_path`. Until this runs the app
    /// keeps the default theme, so tests never read the user's files.
    pub fn enable_themes(&mut self, options: ThemeOptions) {
        self.themes = Some(options);
        if let Some(name) = self.config.theme.clone() {
            match themes::load(&name, &self.theme_dirs()) {
                Ok(theme) => self.apply_theme(theme),
                Err(e) => self.error = Some(e),
            }
        }
    }

    /// The name of the theme on screen.
    pub fn theme_name(&self) -> &str {
        &self.theme_name
    }

    fn theme_dirs(&self) -> ThemeDirs {
        self.themes.as_ref().map(|t| t.dirs.clone()).unwrap_or_default()
    }

    fn apply_theme(&mut self, loaded: LoadedTheme) {
        self.theme_file = loaded.file.map(|f| {
            let stamp = themes::stamp(&f);
            (f, stamp)
        });
        self.theme = loaded.theme;
        self.theme_name = loaded.name;
        self.dirty = true;
    }

    fn current_theme(&self) -> LoadedTheme {
        LoadedTheme {
            name: self.theme_name.clone(),
            theme: self.theme.clone(),
            file: self.theme_file.as_ref().map(|(f, _)| f.clone()),
        }
    }

    /// Reads the theme again when its file has changed, as it does when
    /// `omarchy theme set` switches themes. On ticks.
    pub(super) fn poll_theme(&mut self) {
        if !self.ticks.is_multiple_of(THEME_POLL_TICKS) || self.theme_before_picker.is_some() {
            return;
        }
        let Some((path, seen)) = &mut self.theme_file else {
            return;
        };
        let now = themes::stamp(path);
        if now == *seen {
            return;
        }
        *seen = now;
        // Omarchy replaces the whole folder, so the file can be missing for
        // a moment; it's read once it's back.
        if now.is_none() {
            return;
        }
        match themes::load(&self.theme_name.clone(), &self.theme_dirs()) {
            Ok(theme) => self.apply_theme(theme),
            Err(e) => {
                self.error = Some(e);
                self.dirty = true;
            }
        }
    }

    /// Opens the theme picker on the theme in use.
    pub(super) fn open_theme_picker(&mut self) {
        let entries = themes::list(&self.theme_dirs());
        let selected = entries.iter().position(|e| e.name == self.theme_name).unwrap_or(0);
        let items = entries
            .into_iter()
            .map(|e| PaletteItem {
                label: e.name.clone(),
                hint: e.hint,
                target: PaletteTarget::Theme(e.name),
            })
            .collect();
        self.theme_before_picker = Some(self.current_theme());
        self.modal = Some(Modal::picker("Theme", items, selected));
    }

    /// While the theme picker is open, shows the theme it has highlighted;
    /// once it closes without a pick, puts the old theme back. Runs after
    /// every key or click the modal gets.
    pub(super) fn preview_theme(&mut self) {
        if self.theme_before_picker.is_none() {
            return;
        }
        match self.modal.as_ref().map(Modal::palette_selection) {
            None => {
                if let Some(before) = self.theme_before_picker.take() {
                    self.apply_theme(before);
                }
            }
            Some(Some(PaletteTarget::Theme(name))) if *name != self.theme_name => {
                // One that won't load is reported if it's picked.
                if let Ok(theme) = themes::load(&name.clone(), &self.theme_dirs()) {
                    self.apply_theme(theme);
                }
            }
            _ => {}
        }
    }

    /// Keeps theme `name`, picked in the picker, and saves it to config.toml.
    pub(super) fn pick_theme(&mut self, name: String) {
        let before = self.theme_before_picker.take();
        match themes::load(&name, &self.theme_dirs()) {
            Ok(theme) => self.apply_theme(theme),
            Err(e) => {
                if let Some(before) = before {
                    self.apply_theme(before);
                }
                self.error = Some(e);
                return;
            }
        }
        self.config.theme = Some(name.clone());
        let Some(path) = self.themes.as_ref().and_then(|t| t.config_path.clone()) else {
            self.notice = Some(format!("theme {name}"));
            return;
        };
        match save_theme(&path, &name) {
            Ok(()) => self.notice = Some(format!("theme {name}, saved to {}", path.display())),
            Err(e) => self.error = Some(format!("couldn't save the theme to {}: {e}", path.display())),
        }
    }
}

/// Sets `theme = "<name>"` in the config file at `path`, making it if need be.
fn save_theme(path: &Path, name: &str) -> std::io::Result<()> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let value = toml::Value::String(name.to_string()).to_string();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, crate::config::set_top_level(&text, "theme", &value))
}
