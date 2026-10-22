//! Colors and styles.

use ratatui::style::{Color, Modifier, Style};

use crate::syntax::Class;

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
        }
    }
}

impl Theme {
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
