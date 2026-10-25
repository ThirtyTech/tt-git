//! The theme as ratatui styles, for drawing: [`Theme`](crate::theme::Theme)
//! field for field, converted once.

use ratatui::style::{Color, Modifier, Style};

use crate::style;
use crate::syntax::Class;
use crate::theme;
use crate::view::StyleRole;

pub fn color(c: style::Color) -> Color {
    use style::Color as C;
    match c {
        C::Reset => Color::Reset,
        C::Black => Color::Black,
        C::Red => Color::Red,
        C::Green => Color::Green,
        C::Yellow => Color::Yellow,
        C::Blue => Color::Blue,
        C::Magenta => Color::Magenta,
        C::Cyan => Color::Cyan,
        C::Gray => Color::Gray,
        C::DarkGray => Color::DarkGray,
        C::LightRed => Color::LightRed,
        C::LightGreen => Color::LightGreen,
        C::LightYellow => Color::LightYellow,
        C::LightBlue => Color::LightBlue,
        C::LightMagenta => Color::LightMagenta,
        C::LightCyan => Color::LightCyan,
        C::White => Color::White,
        C::Rgb(r, g, b) => Color::Rgb(r, g, b),
        C::Indexed(i) => Color::Indexed(i),
    }
}

pub fn modifier(m: style::Modifier) -> Modifier {
    Modifier::from_bits_truncate(m.bits())
}

pub fn style(s: style::Style) -> Style {
    let mut out = Style::new();
    out.fg = s.fg.map(color);
    out.bg = s.bg.map(color);
    out.add_modifier = modifier(s.add_modifier);
    out.sub_modifier = modifier(s.sub_modifier);
    out
}

/// [`theme::Theme`] in ratatui's types. See there for what each is for.
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
    pub diff_add_line: Style,
    pub diff_del_line: Style,
    pub syntax: SyntaxStyles,
    pub blame_heat: [Color; 5],
    pub blame_authors: [Color; 16],
    pub blame_uncommitted: Style,
    pub blame_latest: Style,
    pub blame_older: Style,
    pub search_match: Style,
    pub background: Option<Color>,
    pub foreground: Option<Color>,
}

impl Default for Theme {
    fn default() -> Self {
        Self::from(&theme::Theme::default())
    }
}

impl From<&theme::Theme> for Theme {
    fn from(t: &theme::Theme) -> Self {
        Self {
            border: style(t.border),
            border_focused: style(t.border_focused),
            title: style(t.title),
            title_focused: style(t.title_focused),
            selected: style(t.selected),
            selected_unfocused: style(t.selected_unfocused),
            help_key: style(t.help_key),
            help_text: style(t.help_text),
            muted: style(t.muted),
            error: style(t.error),
            lanes: t.lanes.map(color),
            label_head: style(t.label_head),
            label_local: style(t.label_local),
            label_remote: style(t.label_remote),
            label_tag: style(t.label_tag),
            label_detached: style(t.label_detached),
            author: style(t.author),
            date: style(t.date),
            sha: style(t.sha),
            diff_add: style(t.diff_add),
            diff_del: style(t.diff_del),
            diff_hunk: style(t.diff_hunk),
            diff_header: style(t.diff_header),
            diff_add_line: style(t.diff_add_line),
            diff_del_line: style(t.diff_del_line),
            syntax: SyntaxStyles::from(&t.syntax),
            blame_heat: t.blame_heat.map(color),
            blame_authors: t.blame_authors.map(color),
            blame_uncommitted: style(t.blame_uncommitted),
            blame_latest: style(t.blame_latest),
            blame_older: style(t.blame_older),
            search_match: style(t.search_match),
            background: t.background.map(color),
            foreground: t.foreground.map(color),
        }
    }
}

impl Theme {
    pub fn lane(&self, color: u8) -> Style {
        Style::new().fg(self.lanes[color as usize % self.lanes.len()])
    }

    /// How text in `role` looks.
    pub fn role(&self, role: StyleRole) -> Style {
        match role {
            StyleRole::Plain => Style::new(),
            StyleRole::Muted => self.muted,
            StyleRole::Lane(color) => self.lane(color),
            StyleRole::LabelHead => self.label_head,
            StyleRole::LabelDetached => self.label_detached,
            StyleRole::LabelLocal => self.label_local,
            StyleRole::LabelRemote => self.label_remote,
            StyleRole::LabelTag => self.label_tag,
            StyleRole::Author => self.author,
            StyleRole::Date => self.date,
            StyleRole::Sha => self.sha,
        }
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

/// [`theme::SyntaxStyles`] in ratatui's types.
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

impl From<&theme::SyntaxStyles> for SyntaxStyles {
    fn from(s: &theme::SyntaxStyles) -> Self {
        Self {
            comment: style(s.comment),
            string: style(s.string),
            keyword: style(s.keyword),
            operator: style(s.operator),
            r#type: style(s.r#type),
            function: style(s.function),
            number: style(s.number),
            constant: style(s.constant),
            r#macro: style(s.r#macro),
            tag: style(s.tag),
            attribute: style(s.attribute),
            heading: style(s.heading),
            link: style(s.link),
            strong: style(s.strong),
            emphasis: style(s.emphasis),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_look_as_the_app_theme_says() {
        let app = theme::Theme::default();
        let tui = Theme::from(&app);
        let mut roles = vec![
            StyleRole::Plain,
            StyleRole::Muted,
            StyleRole::LabelHead,
            StyleRole::LabelDetached,
            StyleRole::LabelLocal,
            StyleRole::LabelRemote,
            StyleRole::LabelTag,
            StyleRole::Author,
            StyleRole::Date,
            StyleRole::Sha,
        ];
        roles.extend((0..20).map(StyleRole::Lane));
        for role in roles {
            assert_eq!(tui.role(role), style(app.role(role)), "{role:?}");
        }
    }
}
