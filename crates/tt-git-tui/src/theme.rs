//! Colors and styles.

use ratatui::style::{Color, Modifier, Style};

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
        }
    }
}

impl Theme {
    pub fn lane(&self, color: u8) -> Style {
        Style::new().fg(self.lanes[color as usize % self.lanes.len()])
    }
}
