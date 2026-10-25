//! Blame in the details pane: who changed each line, when and why.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tt_git_core::Recency;
use tt_git_core::time::{clock_time, relative};

use super::spans::marked_code_spans;
use crate::panes::DateStyle;
use crate::panes::blame::{BlameView, COMMIT_ROWS, WHO_WIDTH, WHY_MIN_WIDTH};
use crate::tui_theme::Theme;
use crate::widgets::{expand_tabs, fit, fit_right};

/// Whether the version shown is the newest, for the title: `latest`,
/// or how many commits have changed the file since, in a color that
/// stands out. `None` until it's loaded.
pub fn recency(view: &BlameView, theme: &Theme) -> Option<Span<'static>> {
    let (text, style) = match view.level().recency? {
        Recency::Latest => ("latest".to_string(), theme.blame_latest),
        Recency::Behind(1) => ("1 newer commit".to_string(), theme.blame_older),
        Recency::Behind(n) => (format!("{n} newer commits"), theme.blame_older),
        Recency::Elsewhere => ("not on HEAD".to_string(), theme.blame_older),
    };
    Some(Span::styled(text, style))
}

pub fn draw_blame(view: &mut BlameView, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
    let commit_rows = if view.show_commit && view.levels[view.shown()].len() > 0 {
        COMMIT_ROWS.min(area.height / 2)
    } else {
        0
    };
    let [code_area, commit_area] = Layout::vertical([Constraint::Fill(1), Constraint::Length(commit_rows)]).areas(area);
    draw_lines(view, frame, code_area, focused, theme);
    if commit_rows > 0 {
        draw_commit(view, frame, commit_area, theme);
    }
}

/// Each line, reading right to left from the code: when (age stripe and
/// date), who, and why (the commit and its summary, where its lines start).
fn draw_lines(view: &mut BlameView, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
    let now = view.clock.now();
    let dates = view.dates;
    let clock = view.clock.clone();
    let height = area.height as usize;
    view.set_viewport(height);
    let shown = view.shown();
    let search = &view.search;
    let level = &view.levels[shown];
    match &level.blame {
        None => {
            frame.render_widget(Paragraph::new("loading…").style(theme.muted), area);
            return;
        }
        Some(Err(e)) => {
            let text = format!("could not blame {}: {e}", level.path);
            frame.render_widget(Paragraph::new(text).style(theme.error).wrap(Wrap { trim: false }), area);
            return;
        }
        Some(Ok(_)) => {}
    }
    let blame = level.blame().expect("checked above");
    let selected = level.cursor.selected;
    let selected_commit = blame.lines.get(selected).map(|l| l.commit);

    let num_width = blame.lines.len().to_string().len().max(3);
    let when_width = dates.width();
    let summary_width = if area.width >= WHY_MIN_WIDTH {
        (area.width as usize / 6).clamp(12, 30)
    } else {
        0
    };
    // " 1234567  summary" before the author.
    let why_width = if summary_width > 0 {
        1 + 7 + 2 + summary_width
    } else {
        0
    };
    let gutter = why_width + 1 + WHO_WIDTH + 1 + when_width + 1 + 1 + num_width + 1;
    let code_width = (area.width as usize).saturating_sub(gutter);

    let start = level.cursor.offset.min(blame.lines.len());
    let end = (start + height).min(blame.lines.len());
    let mut rows = Vec::with_capacity(end - start);
    for i in start..end {
        let line = &blame.lines[i];
        let commit = &blame.commits[line.commit];
        let uncommitted = commit.is_uncommitted();
        let first = i == 0 || blame.lines[i - 1].commit != line.commit;
        let mut spans = Vec::with_capacity(12);
        if why_width > 0 {
            if !first {
                spans.push(Span::raw(" ".repeat(why_width)));
            } else if uncommitted {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(fit("Not committed yet", why_width - 1), theme.diff_add));
            } else {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(fit(commit.short(), 7), theme.sha));
                spans.push(Span::raw("  "));
                spans.push(Span::styled(fit(&commit.summary, summary_width), theme.author));
            }
        }
        spans.push(Span::raw(" "));
        if uncommitted {
            spans.push(Span::styled(fit_right("You", WHO_WIDTH), theme.blame_uncommitted));
        } else {
            let style = Style::new().fg(author_color(level.authors.get(line.commit).copied(), theme));
            spans.push(Span::styled(fit_right(&commit.author, WHO_WIDTH), style));
        }
        spans.push(Span::raw(" "));
        let when = match dates {
            DateStyle::Relative => relative(now, commit.author_time),
            DateStyle::Clock => clock_time(commit.author_time, clock.utc_offset(commit.author_time)),
        };
        spans.push(Span::styled(fit_right(&when, when_width), theme.date));
        spans.push(Span::raw(" "));
        let stripe = if uncommitted {
            theme.blame_uncommitted
        } else {
            Style::new().fg(heat_color(level.heat.get(line.commit).copied(), theme))
        };
        spans.push(Span::styled("▌", stripe));
        spans.push(Span::styled(format!("{:>num_width$}", i + 1), theme.muted));
        spans.push(Span::raw(" "));
        let tokens = level.syntax.get(i).and_then(Option::as_ref);
        let text = expand_tabs(&line.text);
        let marks = search.ranges(&text);
        let code = marked_code_spans(&text, tokens, Style::new(), theme, code_width, true, &marks);
        spans.extend(code.into_iter().map(|s| Span::styled(s.content.into_owned(), s.style)));
        let row = Line::from(spans);
        rows.push(if i == selected {
            row.patch_style(if focused {
                theme.selected
            } else {
                theme.selected_unfocused
            })
        } else if Some(line.commit) == selected_commit {
            row.patch_style(theme.selected_unfocused)
        } else {
            row
        });
    }
    frame.render_widget(Paragraph::new(rows), area);
}

/// The selected line's commit: subject, who and when, and its message.
fn draw_commit(view: &BlameView, frame: &mut Frame, area: Rect, theme: &Theme) {
    let level = &view.levels[view.shown()];
    let Some((commit, _, _)) = level.selected() else { return };
    let blame = level.blame().expect("a line is selected");
    let index = blame.commits.iter().position(|c| c.oid == commit.oid).unwrap_or(0);
    let count = blame.lines_from(index);
    let title = if commit.is_uncommitted() {
        "Not committed yet".to_string()
    } else {
        format!("Commit {}", commit.short())
    };
    let block = Block::new()
        .borders(Borders::TOP)
        .border_style(theme.border)
        .title(Line::styled(title, theme.muted));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines_here = |what: &str| {
        let s = if count == 1 { "" } else { "s" };
        format!("{count} line{s} in this file {what}")
    };
    let mut lines = Vec::new();
    if commit.is_uncommitted() {
        lines.push(Line::styled("Changed in the working tree", theme.diff_header));
        lines.push(Line::styled(lines_here("aren't committed yet"), theme.author));
    } else {
        let when = format!(
            "{} ({} ago)",
            clock_time(commit.author_time, view.clock.utc_offset(commit.author_time)),
            relative(view.clock.now(), commit.author_time)
        );
        lines.push(Line::styled(commit.summary.clone(), theme.diff_header));
        lines.push(Line::from(vec![
            Span::styled(commit.short().to_string(), theme.sha),
            Span::styled(
                format!(
                    " · {} <{}> · {when} · {}",
                    commit.author,
                    commit.author_mail,
                    lines_here("come from it")
                ),
                theme.author,
            ),
        ]));
        match view.messages.get(&commit.oid) {
            Some(Some(body)) if !body.is_empty() => {
                lines.push(Line::raw(""));
                lines.extend(body.lines().map(|l| Line::raw(expand_tabs(l))));
            }
            Some(None) => {
                lines.push(Line::raw(""));
                lines.push(Line::styled("loading…", theme.muted));
            }
            _ => {}
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn heat_color(bucket: Option<usize>, theme: &Theme) -> Color {
    let last = theme.blame_heat.len() - 1;
    theme.blame_heat[bucket.unwrap_or(last).min(last)]
}

fn author_color(slot: Option<usize>, theme: &Theme) -> Color {
    theme.blame_authors[slot.unwrap_or(0) % theme.blame_authors.len()]
}
