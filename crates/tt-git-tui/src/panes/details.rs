use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{PaneId, pane_block};
use crate::component::{Component, EventState};
use crate::jobs::Ctx;
use crate::keymap::{Command, Context};
use crate::theme::Theme;
use crate::widgets::Cursor;

/// What the details pane is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailsSubject {
    Commit(String),
    File(String),
    /// Free text produced by the app (e.g. plugin logs).
    Text(String),
}

/// Commit details or file diff for the current selection.
#[derive(Debug, Default)]
pub struct DetailsPane {
    subject: Option<DetailsSubject>,
    title: String,
    lines: Vec<String>,
    cursor: Cursor,
    loading: bool,
}

impl DetailsPane {
    pub fn subject(&self) -> Option<&DetailsSubject> {
        self.subject.as_ref()
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The lines currently scrolled into view.
    pub fn visible_lines(&self) -> &[String] {
        let start = self.cursor.offset.min(self.lines.len());
        let end = (start + self.cursor.height.max(1)).min(self.lines.len());
        &self.lines[start..end]
    }

    /// Marks `subject` as the thing to show; content arrives via [`set_content`](Self::set_content).
    pub fn request(&mut self, subject: DetailsSubject, title: String) {
        if self.subject.as_ref() != Some(&subject) {
            self.subject = Some(subject);
            self.title = title;
            self.loading = true;
            self.cursor = Cursor::default();
        }
    }

    /// Sets content if it is for the current subject (stale results are dropped).
    pub fn set_content(&mut self, subject: &DetailsSubject, text: &str) {
        if self.subject.as_ref() == Some(subject) {
            self.lines = text.lines().map(|l| l.replace('\t', "    ")).collect();
            self.loading = false;
        }
    }

    pub fn clear(&mut self) {
        self.subject = None;
        self.lines.clear();
        self.loading = false;
    }
}

fn style_line<'a>(line: &'a str, theme: &Theme) -> Line<'a> {
    let style = if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff --git") {
        theme.diff_header
    } else if line.starts_with('+') {
        theme.diff_add
    } else if line.starts_with('-') {
        theme.diff_del
    } else if line.starts_with("@@") {
        theme.diff_hunk
    } else if line.starts_with("commit ") {
        theme.sha
    } else {
        ratatui::style::Style::new()
    };
    Line::from(Span::styled(line, style))
}

impl Component for DetailsPane {
    fn context(&self) -> Context {
        Context::Details
    }

    fn on_command(&mut self, cmd: Command, _ctx: &Ctx) -> EventState {
        // The details pane scrolls rather than selecting lines: keep the
        // cursor at the top of the viewport.
        let len = self.lines.len();
        let max_offset = len.saturating_sub(self.cursor.height.max(1));
        let page = self.cursor.height.max(1);
        let offset = &mut self.cursor.offset;
        *offset = match cmd {
            Command::MoveUp => offset.saturating_sub(1),
            Command::MoveDown => (*offset + 1).min(max_offset),
            Command::PageUp => offset.saturating_sub(page),
            Command::PageDown => (*offset + page).min(max_offset),
            Command::Top => 0,
            Command::Bottom => max_offset,
            _ => return EventState::NotConsumed,
        };
        EventState::Consumed
    }

    fn commands(&self) -> &[Command] {
        &[Command::OpenLink, Command::CloseDetails]
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
        let title = if self.subject.is_some() {
            self.title.as_str()
        } else {
            "Details"
        };
        let title = if self.loading {
            format!("{title} loading…")
        } else {
            title.to_string()
        };
        let block = pane_block(PaneId::Details, title, focused, theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.cursor.height = inner.height as usize;

        if self.subject.is_none() {
            frame.render_widget(Paragraph::new("nothing selected").style(theme.muted), inner);
            return;
        }
        let start = self.cursor.offset.min(self.lines.len());
        let end = (start + inner.height as usize).min(self.lines.len());
        let lines: Vec<Line> = self.lines[start..end].iter().map(|l| style_line(l, theme)).collect();
        frame.render_widget(Paragraph::new(lines), inner);
    }
}
