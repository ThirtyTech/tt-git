//! The details pane: a commit, a diff, text, or the staging and blame views.

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tt_git_core::{CommitFile, FileState};
use unicode_width::UnicodeWidthStr;

use super::blame::{draw_blame, recency};
use super::spans::marked_code_spans;
use super::stage::draw_stage;
use super::{Draw, Hits, pane_block};
use crate::panes::details::FilesView;
use crate::panes::{BlameView, DetailsPane, PaneId, StageView};
use crate::syntax::Tokens;
use crate::tui_theme::Theme;
use crate::widgets::truncate;

impl Draw for DetailsPane {
    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme, hits: &mut Hits) {
        let view_title =
            (self.stage.as_ref().map(StageView::title)).or_else(|| self.blame.as_ref().map(BlameView::title));
        let title = if let Some(t) = &view_title {
            t.as_str()
        } else if self.subject.is_some() {
            self.title.as_str()
        } else {
            "Details"
        };
        let mut title = title.to_string();
        // Blame's own steps back name the version before; otherwise, what esc goes back to.
        if let Some(trail) = &self.trail
            && self.blame.as_ref().is_none_or(|v| v.depth() == 1)
        {
            title.push_str(&format!("  ‹ {trail}"));
        }
        if let Some(position) = self.blame.as_ref().and_then(BlameView::position) {
            title.push_str(&format!(" {position}"));
        }
        if let Some(count) = self.blame.as_ref().and_then(BlameView::count) {
            title.push_str(&format!(" go to {count}…"));
        }
        let search = match &self.blame {
            Some(view) => view.search_title(),
            None if self.stage.is_none() => self.search.title(self.found, !self.loading),
            None => None,
        };
        if let Some(search) = search {
            title.push_str(&search);
        }
        if self.loading || self.blame.as_ref().is_some_and(BlameView::loading) {
            title.push_str(" loading…");
        }
        // A blame's date and whether it's the latest version go at the
        // right, where there's room: the date goes first when there isn't.
        let changed = self.blame.as_ref().and_then(BlameView::changed);
        let recency = self.blame.as_ref().and_then(|v| recency(v, theme));
        let used = Line::raw(format!("[{}] {title}", PaneId::Details.number())).width() + 4;
        let mut block = pane_block(PaneId::Details, title, focused, theme);
        let mut options: Vec<Vec<Span>> = Vec::new();
        match (changed, recency) {
            (Some(changed), Some(recency)) => {
                options.push(vec![
                    Span::styled(format!(" {changed} · "), theme.date),
                    recency.clone(),
                    Span::raw(" "),
                ]);
                options.push(vec![Span::raw(" "), recency, Span::raw(" ")]);
            }
            (Some(changed), None) => options.push(vec![Span::styled(format!(" {changed} "), theme.date)]),
            (None, Some(recency)) => options.push(vec![Span::raw(" "), recency, Span::raw(" ")]),
            (None, None) => {}
        }
        if let Some(right) = options
            .into_iter()
            .find(|o| used + Line::from(o.clone()).width() <= area.width as usize)
        {
            block = block.title_top(Line::from(right).right_aligned());
        }
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.subject.is_none() {
            self.set_viewport(inner.height as usize);
            frame.render_widget(Paragraph::new("nothing selected").style(theme.muted), inner);
            return;
        }
        if let Some(view) = &mut self.stage {
            draw_stage(view, frame, inner, focused, theme);
        } else if let Some(view) = &mut self.blame {
            draw_blame(view, frame, inner, focused, theme);
        } else if self.files.is_some() {
            draw_files(self, frame, inner, focused, theme, hits);
        } else {
            draw_text(self, frame, inner, None, theme);
        }
    }
}

/// The files view: the file list on the left when fullscreen, otherwise
/// along the bottom; the commit header and selected file's diff fill the rest.
fn draw_files(pane: &mut DetailsPane, frame: &mut Frame, inner: Rect, focused: bool, theme: &Theme, hits: &mut Hits) {
    let Some(view) = &pane.files else { return };
    let label = if pane.loading {
        "Files".to_string()
    } else {
        format!("Files ({})", view.files.len())
    };
    let title = Line::styled(label, theme.muted);
    let body = if pane.files_beside {
        let longest = view.files.iter().map(|f| file_label(f).width() + 3).max().unwrap_or(0);
        let list_width = (longest as u16 + 1).clamp(20, (inner.width * 2 / 5).max(20));
        let [list_area, right] = Layout::horizontal([Constraint::Length(list_width), Constraint::Fill(1)]).areas(inner);
        let list_block = Block::new().borders(Borders::RIGHT).border_style(theme.border);
        let [title_area, rows_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(list_block.inner(list_area));
        frame.render_widget(list_block, list_area);
        frame.render_widget(Paragraph::new(title), title_area);
        hits.files = Some(rows_area);
        Rect {
            x: right.x + 1,
            width: right.width.saturating_sub(1),
            ..right
        }
    } else {
        // Up to a third of the height, under a rule carrying the title.
        let rows = view.files.len().clamp(1, (inner.height as usize / 3).max(2));
        let [top, list_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(rows as u16 + 1)]).areas(inner);
        let list_block = Block::new()
            .borders(Borders::TOP)
            .border_style(theme.border)
            .title(title);
        hits.files = Some(list_block.inner(list_area));
        frame.render_widget(list_block, list_area);
        top
    };

    // Header (up to a third of the height), a rule, then the diff. Above
    // the bottom list the space can be tiny; under three rows the header
    // and rule give way so the diff still shows.
    let max_header = body.height as usize / 3;
    let max_header = if pane.files_beside {
        max_header.max(1)
    } else {
        max_header
    };
    let header_rows = view.header.len().min(max_header);
    let [header_area, rule_area, diff_area] = Layout::vertical([
        Constraint::Length(header_rows as u16),
        Constraint::Length(u16::from(header_rows > 0)),
        Constraint::Fill(1),
    ])
    .areas(body);
    hits.diff = Some(diff_area);
    let rows_area = hits.files.unwrap_or_default();
    pane.set_files_viewport(rows_area.height as usize, header_rows, diff_area.height as usize);
    let Some(view) = &pane.files else { return };
    draw_file_list(view, frame, rows_area, focused, theme);
    let header: Vec<Line> = view.header[..header_rows]
        .iter()
        .map(|l| style_line(l, None, 0, &[], theme))
        .collect();
    frame.render_widget(Paragraph::new(header), header_area);
    let rule = "─".repeat(rule_area.width as usize);
    frame.render_widget(Paragraph::new(rule).style(theme.border), rule_area);
    let mark = pane.files_mark();
    draw_text(pane, frame, diff_area, mark, theme);
}

/// The text, with `mark`'s row (and new-side line number) picked out:
/// the line blame came from.
fn draw_text(pane: &mut DetailsPane, frame: &mut Frame, area: Rect, mark: Option<(usize, u32)>, theme: &Theme) {
    pane.set_viewport(area.height as usize);
    let start = pane.cursor.offset.min(pane.lines.len());
    let end = (start + area.height as usize).min(pane.lines.len());
    let lines: Vec<Line> = (start..end)
        .map(|row| {
            let text = &pane.lines[row];
            let tokens = pane.syntax.get(row).and_then(Option::as_ref);
            let found = pane.search.ranges(text);
            match mark {
                Some((at, line)) if at == row => {
                    let note = format!("◂ line {line} in blame ");
                    let pad = (area.width as usize).saturating_sub(text.width() + note.width());
                    let mut marked = style_line(text, tokens, 0, &found, theme);
                    marked.spans.push(Span::raw(" ".repeat(pad)));
                    marked.spans.push(Span::styled(note, theme.help_text));
                    marked.patch_style(theme.selected)
                }
                _ if pane.found == Some(row) => {
                    style_line(text, tokens, area.width as usize, &found, theme).patch_style(theme.selected_unfocused)
                }
                _ => style_line(text, tokens, area.width as usize, &found, theme),
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// The changed files, scrolled to keep the selection in `area`.
fn draw_file_list(view: &FilesView, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
    let width = area.width as usize;
    let start = view.cursor.offset.min(view.files.len());
    let end = (start + area.height as usize).min(view.files.len());
    let mut rows = Vec::new();
    for (i, file) in view.files.iter().enumerate().take(end).skip(start) {
        let code = file.state.code().to_string();
        let line = Line::from(vec![
            Span::styled(code, state_style(file.state, theme)),
            Span::raw(" "),
            Span::raw(truncate(&file_label(file), width.saturating_sub(2))),
        ]);
        rows.push(if i == view.cursor.selected {
            line.patch_style(if focused {
                theme.selected
            } else {
                theme.selected_unfocused
            })
        } else {
            line
        });
    }
    frame.render_widget(Paragraph::new(rows), area);
}

fn file_label(file: &CommitFile) -> String {
    match &file.orig_path {
        Some(orig) => format!("{orig} → {}", file.path),
        None => file.path.clone(),
    }
}

fn state_style(state: FileState, theme: &Theme) -> Style {
    match state {
        FileState::Added | FileState::Copied => theme.diff_add,
        FileState::Deleted => theme.diff_del,
        FileState::Renamed => theme.diff_hunk,
        _ => theme.label_tag,
    }
}

/// A line of diff or `git show` output. A hunk line with `tokens` is in
/// code colors, an added or deleted one on a tinted background (padded out
/// to `width` columns) with its `+` or `-` in the usual green or red. The
/// byte ranges `found` are a search's matches.
fn style_line<'a>(
    line: &'a str,
    tokens: Option<&Tokens>,
    width: usize,
    found: &[Range<usize>],
    theme: &Theme,
) -> Line<'a> {
    if let Some(tokens) = tokens
        && let Some(code) = line.get(1..)
    {
        let (sign, back) = match line.as_bytes().first() {
            Some(b'+') => (theme.diff_add, theme.diff_add_line),
            Some(b'-') => (theme.diff_del, theme.diff_del_line),
            _ => (Style::new(), Style::new()),
        };
        let sign_found = found.first().is_some_and(|m| m.start == 0);
        let mut spans = vec![Span::styled(
            &line[..1],
            if sign_found {
                sign.patch(theme.search_match)
            } else {
                sign
            },
        )];
        // The matches in the code, after the sign.
        let in_code: Vec<Range<usize>> = found
            .iter()
            .filter(|m| m.end > 1)
            .map(|m| m.start.max(1) - 1..m.end - 1)
            .collect();
        spans.extend(marked_code_spans(
            code,
            Some(tokens),
            Style::new(),
            theme,
            usize::MAX,
            false,
            &in_code,
        ));
        let pad = width.saturating_sub(line.width());
        if pad > 0 && back != Style::new() {
            spans.push(Span::raw(" ".repeat(pad)));
        }
        return Line::from(spans).style(back);
    }
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
        Style::new()
    };
    Line::from(marked_code_spans(line, None, style, theme, usize::MAX, false, found))
}
