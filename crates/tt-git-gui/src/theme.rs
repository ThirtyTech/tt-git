//! The tt-git theme in GPUI's terms: [`Color`] as [`Hsla`], [`Style`] and
//! every [`StyleRole`] as a [`HighlightStyle`], the native style's
//! [`Surfaces`] mixed from it, and gpui-component's theme set to match.

use gpui::component::{Theme as ComponentTheme, ThemeMode};
use gpui::{App, FontStyle, FontWeight, HighlightStyle, Hsla, Rgba, StrikethroughStyle, UnderlineStyle, px};
use tt_git_app::style::{Color, Modifier, Style};
use tt_git_app::theme::Theme;
use tt_git_app::view::{StyleRole, TextSpan};

use crate::font::MonoFont;
use crate::look::UiStyle;

/// The terminal's 16 named colors, as a dark terminal shows them (VS Code's
/// integrated terminal), for the themes that use them. In order: black,
/// red, green, yellow, blue, magenta, cyan, white (ratatui's `Gray`), then
/// the bright ones (`DarkGray` first).
const ANSI: [u32; 16] = [
    0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, 0x666666, 0xf14c4c, 0x23d18b,
    0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xffffff,
];

/// Text and background where the theme leaves them to the terminal (the
/// default theme does).
const DEFAULT_FOREGROUND: u32 = 0xcccccc;
const DEFAULT_BACKGROUND: u32 = 0x1e1e1e;

/// How much a [`Modifier::DIM`] text fades.
const DIM_FADE: f32 = 0.4;

/// The app's [`Theme`] with the colors it leaves to the front end filled in.
#[derive(Debug, Clone)]
pub struct GuiTheme {
    theme: Theme,
    /// Behind everything.
    pub background: Hsla,
    /// Text with no color of its own.
    pub foreground: Hsla,
    /// What the native style draws on.
    pub surfaces: Surfaces,
}

/// The native style's surfaces, mixed from a theme's background and text
/// so every theme gets them: the panes are cards in the theme's background
/// on a ground a little darker (a little nearer the text on a light theme),
/// with hairlines and tints of the text color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Surfaces {
    /// Behind the cards: the window between and around the panes.
    pub ground: Hsla,
    /// A pane's card: the theme's background.
    pub card: Hsla,
    /// A dialog or popup, raised over the cards.
    pub raised: Hsla,
    /// A card's border.
    pub hairline: Hsla,
    /// A dialog's border.
    pub hairline_strong: Hsla,
    /// Behind a chip, an unpicked tab strip, an unfocused selection.
    pub tint: Hsla,
    /// The focused card's border, in the theme's focus color.
    pub focus_border: Hsla,
    /// The ring around the focused card.
    pub ring: Hsla,
    /// Over the panes while a dialog is open.
    pub scrim: Hsla,
    /// The shadow under a dialog.
    pub shadow: Hsla,
}

impl Surfaces {
    fn new(background: Hsla, foreground: Hsla, accent: Hsla) -> Self {
        let dark = background.l < 0.5;
        let black = gpui::black();
        Self {
            ground: if dark {
                mix(background, black, 0.28)
            } else {
                mix(background, foreground, 0.07)
            },
            card: background,
            raised: if dark {
                mix(background, foreground, 0.06)
            } else {
                background
            },
            hairline: foreground.opacity(if dark { 0.09 } else { 0.12 }),
            hairline_strong: foreground.opacity(if dark { 0.15 } else { 0.18 }),
            tint: foreground.opacity(if dark { 0.07 } else { 0.06 }),
            focus_border: accent.opacity(0.75),
            ring: accent.opacity(0.2),
            scrim: black.opacity(if dark { 0.38 } else { 0.16 }),
            shadow: black.opacity(if dark { 0.5 } else { 0.18 }),
        }
    }
}

/// `a` mixed with `t` of `b`, in RGB.
pub fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    let m = |x: f32, y: f32| x + (y - x) * t;
    Hsla::from(Rgba {
        r: m(a.r, b.r),
        g: m(a.g, b.g),
        b: m(a.b, b.b),
        a: m(a.a, b.a),
    })
}

impl GuiTheme {
    pub fn new(theme: &Theme) -> Self {
        let background = theme
            .background
            .map_or(hex(DEFAULT_BACKGROUND), |c| color(c, hex(DEFAULT_BACKGROUND)));
        let foreground = theme
            .foreground
            .map_or(hex(DEFAULT_FOREGROUND), |c| color(c, hex(DEFAULT_FOREGROUND)));
        let mut gui = Self {
            theme: theme.clone(),
            background,
            foreground,
            surfaces: Surfaces::new(background, foreground, background),
        };
        gui.surfaces = Surfaces::new(background, foreground, gui.fg(StyleRole::BorderFocused));
        gui
    }

    /// The tt-git theme this is made from.
    pub fn app_theme(&self) -> &Theme {
        &self.theme
    }

    /// Whether the background is dark.
    pub fn is_dark(&self) -> bool {
        self.background.l < 0.5
    }

    /// `c` as a color, `None` for [`Color::Reset`] (the default).
    pub fn color(&self, c: Color) -> Option<Hsla> {
        (c != Color::Reset).then(|| color(c, self.foreground))
    }

    /// How `style` looks.
    pub fn style(&self, style: Style) -> HighlightStyle {
        let mut fg = style.fg.and_then(|c| self.color(c));
        let mut bg = style.bg.and_then(|c| self.color(c));
        let on = style.add_modifier;
        let off = style.sub_modifier;
        if on.contains(Modifier::REVERSED) {
            (fg, bg) = (Some(bg.unwrap_or(self.background)), Some(fg.unwrap_or(self.foreground)));
        }
        if on.contains(Modifier::HIDDEN) {
            fg = Some(gpui::transparent_black());
        }
        HighlightStyle {
            color: fg,
            background_color: bg,
            font_weight: pick(on, off, Modifier::BOLD, FontWeight::BOLD, FontWeight::NORMAL),
            font_style: pick(on, off, Modifier::ITALIC, FontStyle::Italic, FontStyle::Normal),
            underline: on.contains(Modifier::UNDERLINED).then(|| UnderlineStyle {
                thickness: px(1.),
                ..Default::default()
            }),
            strikethrough: on.contains(Modifier::CROSSED_OUT).then(|| StrikethroughStyle {
                thickness: px(1.),
                ..Default::default()
            }),
            fade_out: on.contains(Modifier::DIM).then_some(DIM_FADE),
        }
    }

    /// How text in `role` looks.
    pub fn role(&self, role: StyleRole) -> HighlightStyle {
        self.style(self.theme.role(role))
    }

    /// How `span` looks, its [`over`](TextSpan::over) role included.
    pub fn span(&self, span: &TextSpan) -> HighlightStyle {
        self.style(self.theme.span(span))
    }

    /// How `span` looks on a row in `row` (a selected row, say): the row's
    /// look, with the span's own colors and attributes over it, as the TUI
    /// patches a line's style.
    pub fn span_on(&self, span: &TextSpan, row: Option<StyleRole>) -> HighlightStyle {
        match row {
            Some(row) => self.style(self.theme.role(row).patch(self.theme.span(span))),
            None => self.span(span),
        }
    }

    /// The text color of `role`, or the default text color.
    pub fn fg(&self, role: StyleRole) -> Hsla {
        self.role(role).color.unwrap_or(self.foreground)
    }

    /// The background of `role`, if it has one.
    pub fn bg(&self, role: StyleRole) -> Option<Hsla> {
        self.role(role).background_color
    }

    /// Sets gpui-component's theme to match, so its components (resize
    /// handles, scrollbars, menus, and whatever the panes use) fit in, at
    /// the app's [zoom](crate::zoom) and in its [style](crate::look).
    pub fn apply_to_components(&self, font: &MonoFont, style: UiStyle, cx: &mut App) {
        let mode = if self.is_dark() {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };
        if ComponentTheme::global(cx).mode != mode {
            ComponentTheme::change(mode, None, cx);
        }
        let border = self.fg(StyleRole::Border);
        let accent = self.fg(StyleRole::BorderFocused);
        let muted = self.fg(StyleRole::Muted);
        let selection = self.bg(StyleRole::Selected).unwrap_or(accent.opacity(0.3));
        let error = self.fg(StyleRole::Error);
        let (background, foreground) = (self.background, self.foreground);
        let scale = crate::zoom::scale(cx);
        let native = style == UiStyle::Native;
        let s = self.surfaces;
        // Native menus and popups are raised; the border, which is also the
        // resize handles' line, goes, so the gutters between the cards stay
        // clear (a handle still shows its grip when hovered).
        let (border, popover) = if native {
            (gpui::transparent_black(), s.raised)
        } else {
            (border, background)
        };
        ComponentTheme::update(cx, |t| {
            t.background = background;
            t.foreground = foreground;
            t.border = border;
            t.drag_border = accent;
            t.ring = accent;
            t.primary = accent;
            t.accent = selection;
            t.accent_foreground = foreground;
            t.muted_foreground = muted;
            t.list_active = selection;
            t.list_active_border = accent;
            t.popover = popover;
            t.popover_foreground = foreground;
            t.danger = error;
            // What's drawn on `danger`: the close button's X on hover, white
            // as Windows draws it. The dark default is red, lost on red.
            t.danger_foreground = gpui::white();
            t.caret = accent;
            t.mono_font_family = font.family.clone();
            t.mono_font_size = font.size;
            // The rem size, so gpui-component's sizes zoom with the font.
            t.font_size = px(16.) * scale;
            t.radius = px(if native { 8. } else { 6. }) * scale;
            t.radius_lg = px(if native { 12. } else { 8. }) * scale;
        });
    }
}

/// The weight or slant `bit` turns on or off, or `None` to leave it.
fn pick<T>(on: Modifier, off: Modifier, bit: Modifier, set: T, unset: T) -> Option<T> {
    if on.contains(bit) {
        Some(set)
    } else if off.contains(bit) {
        Some(unset)
    } else {
        None
    }
}

/// `c` in GPUI's terms; [`Color::Reset`] is `reset`.
pub fn color(c: Color, reset: Hsla) -> Hsla {
    match c {
        Color::Reset => reset,
        Color::Black => hex(ANSI[0]),
        Color::Red => hex(ANSI[1]),
        Color::Green => hex(ANSI[2]),
        Color::Yellow => hex(ANSI[3]),
        Color::Blue => hex(ANSI[4]),
        Color::Magenta => hex(ANSI[5]),
        Color::Cyan => hex(ANSI[6]),
        Color::Gray => hex(ANSI[7]),
        Color::DarkGray => hex(ANSI[8]),
        Color::LightRed => hex(ANSI[9]),
        Color::LightGreen => hex(ANSI[10]),
        Color::LightYellow => hex(ANSI[11]),
        Color::LightBlue => hex(ANSI[12]),
        Color::LightMagenta => hex(ANSI[13]),
        Color::LightCyan => hex(ANSI[14]),
        Color::White => hex(ANSI[15]),
        Color::Rgb(r, g, b) => rgb(r, g, b),
        Color::Indexed(i) => indexed(i),
    }
}

/// A color of the terminal's 256: the 16 named, a 6×6×6 cube, then 24 grays.
fn indexed(i: u8) -> Hsla {
    match i {
        0..16 => hex(ANSI[usize::from(i)]),
        16..232 => {
            let n = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            rgb(level(n / 36), level(n / 6 % 6), level(n % 6))
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            rgb(v, v, v)
        }
    }
}

fn rgb(r: u8, g: u8, b: u8) -> Hsla {
    Rgba {
        r: f32::from(r) / 255.,
        g: f32::from(g) / 255.,
        b: f32::from(b) / 255.,
        a: 1.,
    }
    .into()
}

fn hex(v: u32) -> Hsla {
    let [_, r, g, b] = v.to_be_bytes();
    rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(a: Hsla, b: Hsla) -> bool {
        let (a, b) = (Rgba::from(a), Rgba::from(b));
        [(a.r, b.r), (a.g, b.g), (a.b, b.b), (a.a, b.a)]
            .iter()
            .all(|(x, y)| (x - y).abs() < 0.01)
    }

    #[test]
    fn colors_convert() {
        let fg = hex(0x123456);
        assert!(same(color(Color::Rgb(255, 0, 0), fg), hex(0xff0000)));
        assert!(same(color(Color::Reset, fg), fg));
        assert!(same(color(Color::Indexed(1), fg), color(Color::Red, fg)));
        assert!(same(color(Color::Indexed(16), fg), hex(0x000000)));
        assert!(same(color(Color::Indexed(231), fg), hex(0xffffff)));
        assert!(same(color(Color::Indexed(196), fg), hex(0xff0000)));
        assert!(same(color(Color::Indexed(232), fg), hex(0x080808)));
        assert!(same(color(Color::Indexed(255), fg), hex(0xeeeeee)));
    }

    #[test]
    fn styles_convert() {
        let theme = GuiTheme::new(&Theme::default());
        let s = theme.style(
            Style::new()
                .fg(Color::Green)
                .bg(Color::Rgb(1, 2, 3))
                .add_modifier(Modifier::BOLD),
        );
        assert!(same(s.color.unwrap(), hex(ANSI[2])));
        assert!(same(s.background_color.unwrap(), hex(0x010203)));
        assert_eq!(s.font_weight, Some(FontWeight::BOLD));
        assert_eq!(s.font_style, None);
        assert_eq!(theme.style(Style::new()), HighlightStyle::default());
        let off = theme.style(Style::new().remove_modifier(Modifier::BOLD));
        assert_eq!(off.font_weight, Some(FontWeight::NORMAL));
        let reversed = theme.style(Style::new().fg(Color::Red).add_modifier(Modifier::REVERSED));
        assert!(same(reversed.color.unwrap(), theme.background));
        assert!(same(reversed.background_color.unwrap(), hex(ANSI[1])));
        assert!(
            theme
                .style(Style::new().add_modifier(Modifier::UNDERLINED))
                .underline
                .is_some()
        );
        assert_eq!(
            theme.style(Style::new().add_modifier(Modifier::DIM)).fade_out,
            Some(DIM_FADE)
        );
    }

    #[test]
    fn every_role_looks_as_the_app_theme_says() {
        let mut themes = vec![Theme::default()];
        for (_, text) in tt_git_app::themes::BUILTIN {
            themes.push(Theme::from_palette(&tt_git_app::themes::Palette::parse(text).unwrap()));
        }
        for app in &themes {
            let theme = GuiTheme::new(app);
            for role in StyleRole::samples() {
                assert_eq!(theme.role(role), theme.style(app.role(role)), "{role:?}");
            }
        }
    }

    #[test]
    fn every_theme_gets_native_surfaces_from_its_own_colors() {
        let mut themes = vec![Theme::default()];
        for (_, text) in tt_git_app::themes::BUILTIN {
            themes.push(Theme::from_palette(&tt_git_app::themes::Palette::parse(text).unwrap()));
        }
        for app in &themes {
            let theme = GuiTheme::new(app);
            let s = theme.surfaces;
            assert!(same(s.card, theme.background), "cards are the background");
            // The ground sets the cards off: darker, or nearer the text on
            // a light theme, but never far.
            let step = s.card.l - s.ground.l;
            assert!(step > 0.01 && step < 0.2, "{step} {:?}", app.background);
            assert!(s.hairline.a > 0. && s.hairline.a < s.hairline_strong.a);
            let focus = Hsla {
                a: 1.,
                ..s.focus_border
            };
            assert!(same(focus, theme.fg(StyleRole::BorderFocused)));
            if theme.is_dark() {
                assert!(s.raised.l > s.card.l, "dialogs lighter than the cards");
            }
        }
    }

    #[test]
    fn mix_goes_between_two_colors() {
        let (black, white) = (gpui::black(), gpui::white());
        assert!(same(mix(black, white, 0.), black));
        assert!(same(mix(black, white, 1.), white));
        assert!((mix(black, white, 0.5).l - 0.5).abs() < 0.01);
    }

    #[test]
    fn a_selected_row_keeps_the_spans_own_colors() {
        let theme = GuiTheme::new(&Theme::default());
        let sha = TextSpan::new("abc1234", StyleRole::Sha);
        let s = theme.span_on(&sha, Some(StyleRole::Selected));
        assert_eq!(s.color, theme.role(StyleRole::Sha).color);
        assert_eq!(s.background_color, theme.bg(StyleRole::Selected));
        assert_eq!(s.font_weight, Some(FontWeight::BOLD));
    }

    #[test]
    fn palettes_set_the_background() {
        let theme = GuiTheme::new(&Theme::default());
        assert!(same(theme.background, hex(DEFAULT_BACKGROUND)));
        assert!(theme.is_dark());
        let latte = tt_git_app::themes::BUILTIN
            .iter()
            .find(|(n, _)| *n == "catppuccin-latte")
            .unwrap();
        let light = GuiTheme::new(&Theme::from_palette(
            &tt_git_app::themes::Palette::parse(latte.1).unwrap(),
        ));
        assert!(!light.is_dark());
    }

    #[gpui::test]
    fn the_close_buttons_x_shows_on_its_hover_color(cx: &mut gpui::TestAppContext) {
        let sb = tt_git_testkit::Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let mut gui = crate::testing::Gui::open(cx, &sb);
        let (danger, x) = gui.cx.update(|_, cx| {
            let t = ComponentTheme::global(cx);
            (t.danger, t.danger_foreground)
        });
        assert!(same(danger, GuiTheme::new(&Theme::default()).fg(StyleRole::Error)));
        assert!(same(x, gpui::white()));
    }
}
