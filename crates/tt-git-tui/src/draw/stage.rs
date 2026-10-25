//! Staging in the details pane: a file's unstaged and staged changes.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tt_git_core::patch::LineKind;

use super::spans::code_spans;
use crate::panes::stage::{Row, SIDE_BY_SIDE_WIDTH};
use crate::panes::{StageSideId, StageView};
use crate::syntax;
use crate::tui_theme::Theme;
use crate::widgets::truncate;

pub fn draw_stage(stage: &mut StageView, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
    if !stage.loaded() {
        frame.render_widget(Paragraph::new("loading…").style(theme.muted), area);
        return;
    }
    let (u, s) = (stage.unstaged.has_content(), stage.staged.has_content());
    if !u && !s {
        frame.render_widget(Paragraph::new("no changes").style(theme.muted), area);
        return;
    }
    let (first, second) = if !u || !s {
        // The empty side shrinks to its title along the bottom.
        let [main, rest] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
        if u { (main, rest) } else { (rest, main) }
    } else if area.width >= SIDE_BY_SIDE_WIDTH {
        let [left, right] = Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]).areas(area);
        let block = Block::new().borders(Borders::RIGHT).border_style(theme.border);
        let inner = block.inner(left);
        frame.render_widget(block, left);
        (
            inner,
            Rect {
                x: right.x + 1,
                width: right.width.saturating_sub(1),
                ..right
            },
        )
    } else {
        let [top, bottom] = Layout::vertical([Constraint::Fill(1), Constraint::Fill(1)]).areas(area);
        (top, bottom)
    };
    draw_side(stage, frame, StageSideId::Unstaged, first, focused, theme);
    draw_side(stage, frame, StageSideId::Staged, second, focused, theme);
}

fn draw_side(stage: &mut StageView, frame: &mut Frame, id: StageSideId, area: Rect, focused: bool, theme: &Theme) {
    if area.height == 0 {
        return;
    }
    let active = id == stage.side;
    let [title_area, body] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    let side = stage.view(id);
    let summary = match &side.diff {
        Some(Ok(d)) if d.is_empty() => "no changes".to_string(),
        Some(Ok(d)) if d.hunks.len() == 1 => "1 hunk".to_string(),
        Some(Ok(d)) => format!("{} hunks", d.hunks.len()),
        _ => String::new(),
    };
    let title_style = match (active, focused) {
        (true, true) => theme.title_focused,
        (true, false) => theme.title,
        _ => theme.muted,
    };
    let marker = if active { "▌" } else { " " };
    let title = Line::from(vec![
        Span::styled(format!("{marker}{}", id.name()), title_style),
        Span::styled(format!("  {summary}"), theme.muted),
    ]);
    frame.render_widget(Paragraph::new(title), title_area);

    if let Some(Err(why)) = &side.diff {
        frame.render_widget(Paragraph::new(format!(" {why}")).style(theme.muted), body);
        return;
    }
    let height = body.height as usize;
    stage.set_viewport(id, height);
    let view = stage.view(id);
    let Some(diff) = view.diff.as_ref().and_then(|d| d.as_ref().ok()) else {
        return;
    };
    let width = body.width as usize;
    let highlight = if focused {
        theme.selected
    } else {
        theme.selected_unfocused
    };
    let start = view.offset.min(view.rows.len());
    let end = (start + height).min(view.rows.len());
    let lines: Vec<Line> = view.rows[start..end]
        .iter()
        .map(|row| {
            let (line, picked) = match *row {
                Row::Heading(h) => (
                    Line::from(Span::styled(truncate(&diff.hunks[h].heading(), width), theme.diff_hunk)),
                    !stage.line_mode && h == stage.hunk,
                ),
                Row::Line(h, l) => {
                    let line = &diff.hunks[h].lines[l];
                    let (prefix, style, back) = match line.kind {
                        LineKind::Context => (" ", Style::new(), Style::new()),
                        LineKind::Add => ("+", theme.diff_add, theme.diff_add_line),
                        LineKind::Del => ("-", theme.diff_del, theme.diff_del_line),
                    };
                    let body = syntax::shown(&line.text);
                    let picked = if stage.line_mode {
                        line.is_change() && stage.picked((h, l))
                    } else {
                        h == stage.hunk
                    };
                    let tokens = view.syntax.get(h).and_then(|t| t.get(l)).and_then(Option::as_ref);
                    let line = match tokens {
                        Some(tokens) if width > 0 => {
                            let code =
                                code_spans(&body, Some(tokens), Style::new(), theme, width - 1, line.is_change());
                            let mut spans = vec![Span::styled(prefix, style)];
                            spans.extend(code.into_iter().map(|s| Span::styled(s.content.into_owned(), s.style)));
                            Line::from(spans).style(back)
                        }
                        _ => Line::from(Span::styled(truncate(&format!("{prefix}{body}"), width), style)),
                    };
                    (line, picked)
                }
            };
            if active && picked {
                line.patch_style(highlight)
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), body);
}
