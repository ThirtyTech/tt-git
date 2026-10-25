//! Blame in the details pane: who changed each line, when and why, drawn
//! from its view model.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::graph::span;
use crate::panes::BlameView;
use crate::tui_theme::Theme;
use crate::view::TextSpan;

pub fn draw_blame(view: &mut BlameView, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
    let commit_rows = view.commit_rows(area.height as usize) as u16;
    let [code_area, commit_area] = Layout::vertical([Constraint::Fill(1), Constraint::Length(commit_rows)]).areas(area);
    draw_lines(view, frame, code_area, focused, theme);
    if commit_rows > 0 {
        draw_commit(view, frame, commit_area, theme);
    }
}

/// Each line, reading right to left from the code: when (age stripe and
/// date), who, and why (the commit and its summary, where its lines start).
fn draw_lines(view: &mut BlameView, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
    let height = area.height as usize;
    view.set_viewport(height);
    let lines = view.lines_view(height, area.width as usize);
    if let Some(message) = lines.message {
        let style = theme.span(&message);
        frame.render_widget(
            Paragraph::new(message.text).style(style).wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let rows: Vec<Line> = lines
        .rows
        .into_iter()
        .map(|row| {
            let line = spans_line(row.spans, theme);
            match row.look.role(focused) {
                Some(role) => line.patch_style(theme.role(role)),
                None => line,
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(rows), area);
}

/// The selected line's commit: subject, who and when, and its message.
fn draw_commit(view: &BlameView, frame: &mut Frame, area: Rect, theme: &Theme) {
    let Some(details) = view.commit_view() else { return };
    let block = Block::new()
        .borders(Borders::TOP)
        .border_style(theme.border)
        .title(Line::from(span(details.title, theme)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines: Vec<Line> = details.lines.into_iter().map(|l| spans_line(l, theme)).collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn spans_line<'a>(spans: Vec<TextSpan<'a>>, theme: &Theme) -> Line<'a> {
    Line::from(spans.into_iter().map(|s| span(s, theme)).collect::<Vec<_>>())
}
