//! The font picker, for a front end that draws its own text (the desktop
//! app). The app doesn't know which fonts are installed or how they look:
//! the front end lists them when the picker is asked for, draws in the
//! highlighted one while it's open, and switches to the one picked, which
//! the app saves as `ui.font` in config.toml.

use super::App;
use crate::modal::{Modal, PaletteItem, PaletteTarget};

/// An installed font family, as the front end lists it for the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontChoice {
    pub family: String,
    /// Every character is as wide as the next. A proportional font's
    /// columns don't line up.
    pub monospace: bool,
}

impl App {
    /// Says the front end can draw in any font, which offers
    /// [`Command::Font`](crate::keymap::Command::Font) in the palette and
    /// the help popup.
    pub fn enable_ui_fonts(&mut self) {
        self.chooses_fonts = true;
    }

    pub fn chooses_fonts(&self) -> bool {
        self.chooses_fonts
    }

    /// Whether the font picker was asked for since the last call: the front
    /// end then lists the fonts with [`open_font_picker`](Self::open_font_picker).
    pub fn take_font_picker(&mut self) -> bool {
        std::mem::take(&mut self.font_picker_wanted)
    }

    /// Opens the font picker on `current` (`None` for the default,
    /// `default`): the default first, then the monospace families, then
    /// the proportional ones, each list in the order given.
    pub fn open_font_picker(&mut self, fonts: Vec<FontChoice>, current: Option<&str>, default: &str) {
        let (mono, proportional): (Vec<_>, Vec<_>) = fonts.into_iter().partition(|f| f.monospace);
        let items: Vec<PaletteItem> = std::iter::once(PaletteItem {
            label: format!("Default ({default})"),
            hint: String::new(),
            target: PaletteTarget::Font(None),
        })
        .chain(mono.into_iter().chain(proportional).map(|f| PaletteItem {
            hint: if f.monospace {
                String::new()
            } else {
                "proportional".into()
            },
            label: f.family.clone(),
            target: PaletteTarget::Font(Some(f.family)),
        }))
        .collect();
        let selected = current
            .and_then(|c| {
                items
                    .iter()
                    .position(|i| i.target == PaletteTarget::Font(Some(c.to_string())))
            })
            .unwrap_or(0);
        self.modal = Some(Modal::picker("Font", items, selected));
        self.dirty = true;
    }

    /// The font to draw in while the font picker is open: the highlighted
    /// one (`Some(None)` for the default). `None` when the picker isn't
    /// open or nothing matches what's typed.
    pub fn font_preview(&self) -> Option<Option<&str>> {
        match self.modal.as_ref()?.palette_selection()? {
            PaletteTarget::Font(family) => Some(family.as_deref()),
            _ => None,
        }
    }

    /// The font picked since the last call, if one was (`Some(None)` for
    /// the default): the front end draws in it from now on.
    pub fn take_font_pick(&mut self) -> Option<Option<String>> {
        self.font_picked.take()
    }

    /// Keeps `family` (`None` for the default), picked in the picker, and
    /// saves it as `ui.font` in config.toml.
    pub(super) fn pick_font(&mut self, family: Option<String>) {
        self.font_picked = Some(family.clone());
        self.config.ui.font = family.clone();
        let name = match &family {
            Some(f) => format!("font {f}"),
            None => "default font".to_string(),
        };
        let saved = match &family {
            Some(f) => self.save_setting("ui", "font", &toml::Value::String(f.clone()).to_string()),
            None => self.remove_setting("ui", "font"),
        };
        match saved {
            None => self.notice = Some(name),
            Some(Ok(path)) => self.notice = Some(format!("{name}, saved to {}", path.display())),
            Some(Err(e)) => self.error = Some(format!("couldn't save the font to {e}")),
        }
    }

    /// The front end couldn't find `family`, the font config names, and
    /// draws in `using` instead: says so.
    pub fn ui_font_missing(&mut self, family: &str, using: &str) {
        self.notice = Some(format!("font {family} isn't installed: using {using}"));
        self.dirty = true;
    }
}
