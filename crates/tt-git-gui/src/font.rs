//! The monospace font everything is drawn in, and its cell size. View
//! models fit their text to a width in columns, as the TUI's do, so the GUI
//! draws in one fixed-width font and turns pixels into columns and rows
//! with [`MonoFont::cell`].

use gpui::{App, Font, Pixels, SharedString, Size, font, px, size};

/// The families to try on each OS, best first; the last ships with it.
/// Cascadia Mono and SF Mono draw the graph's box and circle characters
/// that Consolas and Menlo lack.
#[cfg(target_os = "windows")]
const FAMILIES: &[&str] = &["Cascadia Mono", "Consolas"];
#[cfg(target_os = "macos")]
const FAMILIES: &[&str] = &["SF Mono", "Menlo"];
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const FAMILIES: &[&str] = &["Noto Sans Mono", "Liberation Mono", "DejaVu Sans Mono"];

/// The font size at 100% zoom.
const SIZE: f32 = 13.;
/// Line height over font size.
const LINE_SPACING: f32 = 1.4;

/// A monospace font: its family, size and the size of one character cell.
#[derive(Debug, Clone, PartialEq)]
pub struct MonoFont {
    pub family: SharedString,
    pub size: Pixels,
    /// The height of a line.
    pub line_height: Pixels,
    /// How wide one column is: the advance of `0`.
    pub cell_width: Pixels,
}

impl MonoFont {
    /// The default font for this OS (the first of its families that's
    /// installed) at the app's [zoom](crate::zoom), measured with the text
    /// system.
    pub fn new(cx: &App) -> Self {
        let installed = cx.text_system().all_font_names();
        let family = FAMILIES
            .iter()
            .find(|f| installed.iter().any(|name| name == *f))
            .or(FAMILIES.last())
            .copied()
            .unwrap_or_default();
        Self::measured(family.into(), px(SIZE * crate::zoom::scale(cx)), cx)
    }

    /// `family` at `size`, measured with the text system. A text system
    /// that can't measure it (as in tests) gets the usual proportions.
    pub fn measured(family: SharedString, size: Pixels, cx: &App) -> Self {
        let text = cx.text_system();
        let id = text.resolve_font(&font(family.clone()));
        let cell_width = text
            .ch_advance(id, size)
            .ok()
            .filter(|w| *w > px(0.))
            .unwrap_or(size * 0.6);
        Self {
            family,
            size,
            line_height: (size * LINE_SPACING).round(),
            cell_width,
        }
    }

    /// The font, for text elements.
    pub fn font(&self) -> Font {
        font(self.family.clone())
    }

    /// One character cell.
    pub fn cell(&self) -> Size<Pixels> {
        size(self.cell_width, self.line_height)
    }

    /// Whole columns and rows that fit in `space`.
    pub fn fit(&self, space: Size<Pixels>) -> Fit {
        let count = |len: Pixels, cell: Pixels| {
            if cell > px(0.) {
                (len / cell).floor().max(0.) as usize
            } else {
                0
            }
        };
        Fit {
            cols: count(space.width, self.cell_width),
            rows: count(space.height, self.line_height),
        }
    }
}

/// Columns and rows: how much text fits somewhere.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fit {
    pub cols: usize,
    pub rows: usize,
}
