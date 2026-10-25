//! Colors and styles, in tt-git's own [`style`](crate::style) types.

use crate::style::{Color, Modifier, Style};

use crate::syntax::Class;
use crate::themes::Palette;

#[derive(Debug, Clone)]
pub struct Theme {
    pub border: Style,
    pub border_focused: Style,
    pub title: Style,
    pub title_focused: Style,
    pub selected: Style,
    pub selected_unfocused: Style,
    pub help_key: Style,
    pub help_text: Style,
    pub muted: Style,
    pub error: Style,
    /// Graph lane colors, indexed by `Cell::color`.
    pub lanes: [Color; tt_git_graph::PALETTE_SIZE as usize],
    pub label_head: Style,
    pub label_local: Style,
    pub label_remote: Style,
    pub label_tag: Style,
    pub label_detached: Style,
    pub author: Style,
    pub date: Style,
    pub sha: Style,
    pub diff_add: Style,
    pub diff_del: Style,
    pub diff_hunk: Style,
    pub diff_header: Style,
    /// Behind an added or deleted line whose code is highlighted (its text
    /// is in code colors, so the background shows the change).
    pub diff_add_line: Style,
    pub diff_del_line: Style,
    /// Code colors, by [`Theme::code`].
    pub syntax: SyntaxStyles,
    /// Blame's age stripe, newest to oldest (hot to cold).
    pub blame_heat: [Color; 5],
    /// Blame's author colors. Each author starts from the same one every time;
    /// see `panes::blame` for how clashes within a file are avoided.
    pub blame_authors: [Color; 16],
    /// Blame's lines not committed yet: their stripe and author.
    pub blame_uncommitted: Style,
    /// Blame's title when it shows the file as in `HEAD`, and when newer
    /// commits have changed it.
    pub blame_latest: Style,
    pub blame_older: Style,
    /// Text a search matched.
    pub search_match: Style,
    /// The palette's background and text, painted behind everything when
    /// `theme_background` is on. `None` (the default theme) leaves the
    /// terminal's own.
    pub background: Option<Color>,
    pub foreground: Option<Color>,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            border: Style::new().fg(Color::DarkGray),
            border_focused: Style::new().fg(Color::Green),
            title: Style::new().fg(Color::Gray),
            title_focused: Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            selected: Style::new().bg(Color::Rgb(40, 52, 74)).add_modifier(Modifier::BOLD),
            selected_unfocused: Style::new().bg(Color::Rgb(38, 38, 38)),
            help_key: Style::new().fg(Color::Cyan),
            help_text: Style::new().fg(Color::Gray),
            muted: Style::new().fg(Color::DarkGray),
            error: Style::new().fg(Color::Red),
            // GitLens-like lane palette.
            lanes: [
                Color::Rgb(21, 160, 191),
                Color::Rgb(6, 105, 247),
                Color::Rgb(142, 0, 194),
                Color::Rgb(197, 23, 182),
                Color::Rgb(217, 1, 113),
                Color::Rgb(242, 93, 46),
                Color::Rgb(242, 202, 51),
                Color::Rgb(123, 217, 56),
            ],
            label_head: Style::new()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
            label_local: Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            label_remote: Style::new().fg(Color::Magenta),
            label_tag: Style::new().fg(Color::Yellow),
            label_detached: Style::new()
                .fg(Color::Black)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
            author: Style::new().fg(Color::Gray),
            date: Style::new().fg(Color::DarkGray),
            sha: Style::new().fg(Color::Yellow),
            diff_add: Style::new().fg(Color::Green),
            diff_del: Style::new().fg(Color::Red),
            diff_hunk: Style::new().fg(Color::Cyan),
            diff_header: Style::new().add_modifier(Modifier::BOLD),
            diff_add_line: Style::new().bg(Color::Rgb(22, 50, 32)),
            diff_del_line: Style::new().bg(Color::Rgb(62, 28, 32)),
            syntax: SyntaxStyles::default(),
            blame_heat: [
                Color::Rgb(242, 93, 46),
                Color::Rgb(227, 163, 65),
                Color::Rgb(156, 143, 125),
                Color::Rgb(91, 127, 166),
                Color::Rgb(61, 90, 128),
            ],
            blame_authors: [
                Color::Rgb(240, 113, 120),
                Color::Rgb(247, 150, 98),
                Color::Rgb(255, 203, 107),
                Color::Rgb(221, 231, 120),
                Color::Rgb(158, 214, 118),
                Color::Rgb(96, 190, 120),
                Color::Rgb(110, 220, 190),
                Color::Rgb(137, 221, 255),
                Color::Rgb(70, 170, 220),
                Color::Rgb(120, 160, 255),
                Color::Rgb(165, 150, 255),
                Color::Rgb(199, 146, 234),
                Color::Rgb(230, 120, 220),
                Color::Rgb(255, 150, 195),
                Color::Rgb(205, 170, 135),
                Color::Rgb(175, 188, 200),
            ],
            blame_uncommitted: Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            blame_latest: Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            blame_older: Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            search_match: Style::new().fg(Color::Black).bg(Color::Yellow),
            background: None,
            foreground: None,
        }
    }
}

impl Theme {
    /// The theme a palette makes: the accent on whatever has the focus, the
    /// palette's hues where the default theme has the terminal's, and
    /// backgrounds (selection, changed lines) blended from its own colors so
    /// they suit a light palette as well as a dark one.
    pub fn from_palette(p: &Palette) -> Self {
        let fg = |c| Style::new().fg(c);
        let bold = |c| Style::new().fg(c).add_modifier(Modifier::BOLD);
        let on = |c| Style::new().fg(p.background).bg(c).add_modifier(Modifier::BOLD);
        Self {
            border: fg(p.muted),
            border_focused: fg(p.accent),
            title: fg(p.foreground),
            title_focused: bold(p.accent),
            selected: Style::new().bg(p.selection).add_modifier(Modifier::BOLD),
            selected_unfocused: Style::new().bg(mix(p.background, p.selection, 0.55)),
            help_key: fg(p.accent),
            help_text: fg(p.foreground),
            muted: fg(p.dark_foreground),
            error: fg(p.red),
            lanes: [
                p.cyan,
                p.blue,
                p.magenta,
                p.bright_magenta,
                p.red,
                p.orange,
                p.yellow,
                p.green,
            ],
            label_head: on(p.green),
            label_local: bold(p.green),
            label_remote: fg(p.magenta),
            label_tag: fg(p.yellow),
            label_detached: on(p.red),
            author: fg(p.foreground),
            date: fg(p.dark_foreground),
            sha: fg(p.yellow),
            diff_add: fg(p.green),
            diff_del: fg(p.red),
            diff_hunk: fg(p.cyan),
            diff_header: Style::new().add_modifier(Modifier::BOLD),
            diff_add_line: Style::new().bg(mix(p.background, p.green, 0.15)),
            diff_del_line: Style::new().bg(mix(p.background, p.red, 0.15)),
            syntax: SyntaxStyles::from_palette(p),
            blame_heat: [
                p.orange,
                p.yellow,
                mix(p.yellow, p.blue, 0.5),
                p.blue,
                mix(p.blue, p.background, 0.4),
            ],
            blame_authors: {
                let hues = [
                    p.red,
                    p.orange,
                    p.yellow,
                    p.green,
                    p.cyan,
                    p.blue,
                    p.magenta,
                    mix(p.red, p.magenta, 0.5),
                ];
                // The same hues again, paler (or darker on a light palette),
                // so a file with many authors still tells them apart.
                std::array::from_fn(|i| match i {
                    0..8 => hues[i],
                    _ => mix(hues[i - 8], p.foreground, 0.4),
                })
            },
            blame_uncommitted: bold(p.green),
            blame_latest: bold(p.green),
            blame_older: bold(p.yellow),
            search_match: Style::new().fg(p.background).bg(p.yellow),
            background: Some(p.background),
            foreground: Some(p.foreground),
        }
    }

    pub fn lane(&self, color: u8) -> Style {
        Style::new().fg(self.lanes[color as usize % self.lanes.len()])
    }

    /// How highlighted code of `class` looks.
    pub fn code(&self, class: Class) -> Style {
        let s = &self.syntax;
        match class {
            Class::Comment => s.comment,
            Class::String => s.string,
            Class::Keyword => s.keyword,
            Class::Operator => s.operator,
            Class::Type => s.r#type,
            Class::Function => s.function,
            Class::Number => s.number,
            Class::Constant => s.constant,
            Class::Macro => s.r#macro,
            Class::Tag => s.tag,
            Class::Attribute => s.attribute,
            Class::Heading => s.heading,
            Class::Link => s.link,
            Class::Strong => s.strong,
            Class::Emphasis => s.emphasis,
        }
    }
}

/// Code colors, one per [`Class`].
#[derive(Debug, Clone)]
pub struct SyntaxStyles {
    pub comment: Style,
    pub string: Style,
    pub keyword: Style,
    pub operator: Style,
    pub r#type: Style,
    pub function: Style,
    pub number: Style,
    pub constant: Style,
    pub r#macro: Style,
    pub tag: Style,
    pub attribute: Style,
    pub heading: Style,
    pub link: Style,
    pub strong: Style,
    pub emphasis: Style,
}

impl SyntaxStyles {
    /// One Dark's roles on the palette's hues.
    fn from_palette(p: &Palette) -> Self {
        let fg = |c| Style::new().fg(c);
        Self {
            comment: fg(p.dark_foreground).add_modifier(Modifier::ITALIC),
            string: fg(p.green),
            keyword: fg(p.magenta),
            operator: fg(p.cyan),
            r#type: fg(p.yellow),
            function: fg(p.blue),
            number: fg(p.orange),
            constant: fg(p.orange),
            r#macro: fg(p.cyan),
            tag: fg(p.red),
            attribute: fg(p.orange),
            heading: fg(p.blue).add_modifier(Modifier::BOLD),
            link: fg(p.blue).add_modifier(Modifier::UNDERLINED),
            strong: Style::new().add_modifier(Modifier::BOLD),
            emphasis: Style::new().add_modifier(Modifier::ITALIC),
        }
    }
}

impl Default for SyntaxStyles {
    /// One Dark-like, for a dark terminal.
    fn default() -> Self {
        let fg = |r, g, b| Style::new().fg(Color::Rgb(r, g, b));
        Self {
            comment: fg(127, 132, 142).add_modifier(Modifier::ITALIC),
            string: fg(152, 195, 121),
            keyword: fg(198, 120, 221),
            operator: fg(86, 182, 194),
            r#type: fg(229, 192, 123),
            function: fg(97, 175, 239),
            number: fg(209, 154, 102),
            constant: fg(209, 154, 102),
            r#macro: fg(86, 182, 194),
            tag: fg(224, 108, 117),
            attribute: fg(209, 154, 102),
            heading: fg(97, 175, 239).add_modifier(Modifier::BOLD),
            link: fg(97, 175, 239).add_modifier(Modifier::UNDERLINED),
            strong: Style::new().add_modifier(Modifier::BOLD),
            emphasis: Style::new().add_modifier(Modifier::ITALIC),
        }
    }
}

/// `a` moved `t` (0 to 1) of the way to `b`. Only RGB colors blend; for
/// anything else this is `a`.
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
            Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
        }
        _ => a,
    }
}
