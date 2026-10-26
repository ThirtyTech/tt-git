//! The font everything is drawn in, and its cell size. View models fit
//! their text to a width in columns, as the TUI's do, so the GUI draws in
//! one font and turns pixels into columns and rows with [`MonoFont::cell`].
//!
//! The font is one setting for the whole app, every window, as the
//! [zoom](crate::zoom) is: [`UiFont`], a GPUI global started from `[ui] font`
//! and `[ui] font_size` in the global config and changed by the font picker.
//! A font that isn't installed falls back to the OS's usual one.

use std::collections::HashMap;

use gpui::{App, Font, Global, Pixels, SharedString, Size, font, px, size};
use tt_git_app::app::FontChoice;
use tt_git_app::config::DEFAULT_FONT_SIZE;

/// The families to try on each OS, best first; the last ships with it.
/// Cascadia Mono and SF Mono draw the graph's box and circle characters
/// that Consolas and Menlo lack.
#[cfg(target_os = "windows")]
const FAMILIES: &[&str] = &["Cascadia Mono", "Consolas"];
#[cfg(target_os = "macos")]
const FAMILIES: &[&str] = &["SF Mono", "Menlo"];
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const FAMILIES: &[&str] = &["Noto Sans Mono", "Liberation Mono", "DejaVu Sans Mono"];

/// The font the app draws in: a family (`None` for the OS's usual one) and
/// the size at 100% zoom, in pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct UiFont {
    pub family: Option<SharedString>,
    pub size: f32,
}

impl Default for UiFont {
    fn default() -> Self {
        Self {
            family: None,
            size: DEFAULT_FONT_SIZE,
        }
    }
}

impl Global for UiFont {}

/// Sets the font every window draws in.
pub fn set(font: UiFont, cx: &mut App) {
    if cx.try_global::<UiFont>() != Some(&font) {
        cx.set_global(font);
    }
}

/// Switches every window to `family` (`None` for the default), at the
/// same size.
pub fn set_family(family: Option<String>, cx: &mut App) {
    let size = ui_font(cx).size;
    set(
        UiFont {
            family: family.map(Into::into),
            size,
        },
        cx,
    );
}

/// The font set with [`set`]; the default before.
pub fn ui_font(cx: &App) -> UiFont {
    cx.try_global::<UiFont>().cloned().unwrap_or_default()
}

/// The installed families.
fn installed(cx: &App) -> Vec<String> {
    #[cfg(test)]
    if let Some(fonts) = cx.try_global::<TestFonts>() {
        return fonts.0.iter().map(|f| f.family.clone()).collect();
    }
    cx.text_system().all_font_names()
}

/// `family` as it's installed (the case can differ), if it is.
fn find_installed(family: &str, cx: &App) -> Option<String> {
    installed(cx).into_iter().find(|name| name.eq_ignore_ascii_case(family))
}

/// The OS's usual font: the first of its families that's installed.
pub fn default_family(cx: &App) -> &'static str {
    let installed = installed(cx);
    FAMILIES
        .iter()
        .find(|f| installed.iter().any(|name| name == *f))
        .or(FAMILIES.last())
        .copied()
        .unwrap_or_default()
}

/// The family to draw in for `wanted` (`None` for the default): `wanted`
/// as installed, or the default if it isn't.
fn resolve(wanted: Option<&str>, cx: &App) -> SharedString {
    wanted
        .and_then(|f| find_installed(f, cx))
        .map_or_else(|| default_family(cx).into(), Into::into)
}

/// The family [`UiFont`] names, if it isn't installed.
pub fn missing(cx: &App) -> Option<SharedString> {
    let family = ui_font(cx).family?;
    find_installed(&family, cx).is_none().then_some(family)
}

/// What the font picker lists: every installed family, and whether it's
/// monospace. Measuring each family takes a while, so what's measured is
/// kept for the next time.
pub fn choices(cx: &mut App) -> Vec<FontChoice> {
    #[cfg(test)]
    if let Some(fonts) = cx.try_global::<TestFonts>() {
        return fonts.0.clone();
    }
    let names = installed(cx);
    let mut measured = cx.try_global::<Monospace>().map(|m| m.0.clone()).unwrap_or_default();
    let choices = names
        .into_iter()
        // Names starting with a dot are the OS's own (macOS's system UI font).
        .filter(|name| !name.starts_with('.'))
        .map(|family| {
            let monospace = *measured
                .entry(family.clone())
                .or_insert_with(|| is_monospace(&family, cx));
            FontChoice { family, monospace }
        })
        .collect();
    cx.set_global(Monospace(measured));
    choices
}

/// Whether `family` is monospace, as far as its widest and narrowest
/// letters tell.
fn is_monospace(family: &str, cx: &App) -> bool {
    let text = cx.text_system();
    let id = text.resolve_font(&font(SharedString::from(family.to_string())));
    let size = px(DEFAULT_FONT_SIZE);
    let width = |c| text.advance(id, size, c).map(|s| s.width).ok();
    match (width('i'), width('W'), width('0')) {
        (Some(i), Some(w), Some(zero)) => (i - w).abs() < px(0.5) && (i - zero).abs() < px(0.5),
        _ => false,
    }
}

/// Which families [`choices`] has measured as monospace.
struct Monospace(HashMap<String, bool>);

impl Global for Monospace {}

/// The installed fonts as a test sets them, in place of the text system's
/// (which has none in tests).
#[cfg(test)]
pub struct TestFonts(pub Vec<FontChoice>);

#[cfg(test)]
impl Global for TestFonts {}

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
    /// The app's [`UiFont`] at its [zoom](crate::zoom), measured with the
    /// text system.
    pub fn new(cx: &App) -> Self {
        Self::with_family(ui_font(cx).family.as_deref(), cx)
    }

    /// `family` (`None` for the default) in place of the app's font, at
    /// its size and zoom, as the font picker previews it.
    pub fn with_family(family: Option<&str>, cx: &App) -> Self {
        let size = ui_font(cx).size * crate::zoom::scale(cx);
        Self::measured(resolve(family, cx), px(size), cx)
    }

    /// `family` at `size`, measured with the text system, with the line
    /// height the [style](crate::look) gives it. A text system that can't
    /// measure it (as in tests) gets the usual proportions.
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
            line_height: (size * crate::look::line_spacing(crate::look::style(cx))).round(),
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

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::testing::Gui;

    fn installed_fonts(cx: &mut TestAppContext) {
        let font = |family: &str, monospace| FontChoice {
            family: family.into(),
            monospace,
        };
        cx.update(|cx| {
            cx.set_global(TestFonts(vec![
                font("Arial", false),
                font("Fira Code", true),
                font("JetBrains Mono", true),
            ]))
        });
    }

    fn drawn_family(gui: &Gui) -> String {
        gui.shell
            .read_with(&gui.cx, |shell, _| shell.ui().font.family.to_string())
    }

    fn open(cx: &mut TestAppContext) -> (Gui, Sandbox) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let gui = Gui::open(cx, &sb);
        (gui, sb)
    }

    #[gpui::test]
    fn the_picker_previews_the_highlighted_font_and_esc_goes_back(cx: &mut TestAppContext) {
        installed_fonts(cx);
        let (mut gui, _sb) = open(cx);
        let default = gui.cx.read(default_family);
        assert_eq!(drawn_family(&gui), default);
        gui.model.update(&mut gui.cx, |m, cx| {
            m.send(tt_git_app::Action::Command(tt_git_app::keymap::Command::Font), cx)
        });
        gui.wait_idle();
        assert_eq!(
            gui.read(|app| app.font_preview().map(|f| f.map(str::to_string))),
            Some(None)
        );

        gui.press("down");
        assert_eq!(drawn_family(&gui), "Fira Code", "drawn in the highlighted font");
        gui.press("down").press("down").press("down");
        assert_eq!(drawn_family(&gui), "Arial", "proportional fonts come last");
        gui.press("esc");
        assert!(gui.read(|app| app.modal().is_none()));
        assert_eq!(drawn_family(&gui), default, "back to the font it had");
        assert_eq!(gui.cx.read(ui_font).family, None);
    }

    #[gpui::test]
    fn picking_a_font_switches_the_app_to_it(cx: &mut TestAppContext) {
        installed_fonts(cx);
        let (mut gui, _sb) = open(cx);
        gui.model.update(&mut gui.cx, |m, cx| {
            m.send(tt_git_app::Action::Command(tt_git_app::keymap::Command::Font), cx)
        });
        gui.wait_idle();
        for c in "jetbrains".chars() {
            gui.press(&c.to_string());
        }
        gui.press("enter");
        assert!(gui.read(|app| app.modal().is_none()));
        assert_eq!(gui.cx.read(ui_font).family.as_deref(), Some("JetBrains Mono"));
        assert_eq!(drawn_family(&gui), "JetBrains Mono");
        assert_eq!(
            gui.read(|app| app.notice().map(str::to_string)).as_deref(),
            Some("font JetBrains Mono")
        );

        // Opened again, it starts on the font in use.
        gui.model.update(&mut gui.cx, |m, cx| {
            m.send(tt_git_app::Action::Command(tt_git_app::keymap::Command::Font), cx)
        });
        gui.wait_idle();
        assert_eq!(
            gui.read(|app| app.font_preview().map(|f| f.map(str::to_string))),
            Some(Some("JetBrains Mono".into()))
        );
    }

    #[gpui::test]
    fn a_font_that_is_not_installed_falls_back_to_the_default(cx: &mut TestAppContext) {
        installed_fonts(cx);
        cx.update(|cx| {
            set(
                UiFont {
                    family: Some("Nope Mono".into()),
                    size: 15.,
                },
                cx,
            )
        });
        let (gui, _sb) = open(cx);
        let default = gui.cx.read(default_family);
        assert_eq!(drawn_family(&gui), default);
        let size = gui.shell.read_with(&gui.cx, |shell, _| f32::from(shell.ui().font.size));
        assert_eq!(size, 15., "the size still counts");
        assert_eq!(
            gui.read(|app| app.notice().map(str::to_string)),
            Some(format!("font Nope Mono isn't installed: using {default}"))
        );
    }

    #[gpui::test]
    fn the_installed_name_is_used_whatever_the_case(cx: &mut TestAppContext) {
        installed_fonts(cx);
        cx.update(|cx| set_family(Some("fira code".into()), cx));
        let (gui, _sb) = open(cx);
        assert_eq!(drawn_family(&gui), "Fira Code");
        assert_eq!(gui.read(|app| app.notice().map(str::to_string)), None);
    }
}
