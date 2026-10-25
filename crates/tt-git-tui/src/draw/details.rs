//! The details pane: a commit, a diff, text, or the staging and blame views,
//! drawn from its view model.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::blame::draw_blame;
use super::graph::span;
use super::stage::draw_stage;
use super::{Draw, Hits, pane_block};
use crate::panes::{DetailsPane, PaneId};
use crate::tui_theme::Theme;
use crate::view::TextSpan;
use crate::view::details::{TextRow, fit_title_right};

impl Draw for DetailsPane {
    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme, hits: &mut Hits) {
        let title = self.frame_title();
        // A blame's date and whether it's the latest version go at the
        // right, where there's room: the date goes first when there isn't.
        let used = Line::raw(format!("[{}] {title}", PaneId::Details.number())).width() + 4;
        let right = fit_title_right(self.title_right(), used, area.width as usize);
        let mut block = pane_block(PaneId::Details, title, focused, theme);
        if let Some(right) = right {
            let spans: Vec<Span> = right.into_iter().map(|s| span(s, theme)).collect();
            block = block.title_top(Line::from(spans).right_aligned());
        }
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if let Some(msg) = self.empty_message() {
            self.set_viewport(inner.height as usize);
            frame.render_widget(Paragraph::new(msg).style(theme.muted), inner);
            return;
        }
        if let Some(view) = &mut self.stage {
            draw_stage(view, frame, inner, focused, theme, hits);
        } else if let Some(view) = &mut self.blame {
            draw_blame(view, frame, inner, focused, theme);
        } else if self.files.is_some() {
            draw_files(self, frame, inner, focused, theme, hits);
        } else {
            draw_text(self, frame, inner, theme);
        }
    }
}

/// The files view: the file list on the left when fullscreen, otherwise
/// along the bottom; the commit header and selected file's diff fill the rest.
fn draw_files(pane: &mut DetailsPane, frame: &mut Frame, inner: Rect, focused: bool, theme: &Theme, hits: &mut Hits) {
    let Some(label) = pane.files_label() else { return };
    let title = Line::styled(label, theme.muted);
    let body = if pane.files_beside {
        let list_width = pane.files_list_width(inner.width as usize) as u16;
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
        let rows = pane.files_list_rows(inner.height as usize);
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

    // Header, a rule, then the diff.
    let header_rows = pane.files_header_rows(body.height as usize);
    let [header_area, rule_area, diff_area] = Layout::vertical([
        Constraint::Length(header_rows as u16),
        Constraint::Length(u16::from(header_rows > 0)),
        Constraint::Fill(1),
    ])
    .areas(body);
    hits.diff = Some(diff_area);
    let rows_area = hits.files.unwrap_or_default();
    pane.set_files_viewport(rows_area.height as usize, header_rows, diff_area.height as usize);
    let rows: Vec<Line> = pane
        .file_rows(rows_area.height as usize, rows_area.width as usize)
        .into_iter()
        .map(|row| {
            let line = spans_line(row.spans, theme);
            if row.picked {
                line.patch_style(if focused {
                    theme.selected
                } else {
                    theme.selected_unfocused
                })
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(rows), rows_area);
    let header: Vec<Line> = pane
        .header_rows(header_rows)
        .into_iter()
        .map(|spans| spans_line(spans, theme))
        .collect();
    frame.render_widget(Paragraph::new(header), header_area);
    let rule = "─".repeat(rule_area.width as usize);
    frame.render_widget(Paragraph::new(rule).style(theme.border), rule_area);
    draw_text(pane, frame, diff_area, theme);
}

/// The text, or the selected file's diff, from its view model.
fn draw_text(pane: &mut DetailsPane, frame: &mut Frame, area: Rect, theme: &Theme) {
    pane.set_viewport(area.height as usize);
    let lines: Vec<Line> = pane
        .text_rows(area.height as usize, area.width as usize)
        .into_iter()
        .map(|row| text_line(row, theme))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// A line of the text in its theme style: the spans over what's behind
/// the row, picked out when it's picked.
fn text_line<'a>(row: TextRow<'a>, theme: &Theme) -> Line<'a> {
    let mut line = spans_line(row.spans, theme);
    if let Some(back) = row.back {
        line = line.style(theme.role(back));
    }
    match row.pick {
        Some(pick) => line.patch_style(theme.role(pick)),
        None => line,
    }
}

fn spans_line<'a>(spans: Vec<TextSpan<'a>>, theme: &Theme) -> Line<'a> {
    Line::from(spans.into_iter().map(|s| span(s, theme)).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use ratatui::buffer::Buffer;
    use ratatui::style::Style;
    use ratatui::widgets::Widget;
    use unicode_width::UnicodeWidthStr;

    use super::*;
    use crate::draw::spans::marked_code_spans;
    use crate::syntax::{Class, Tokens};
    use crate::view::StyleRole;
    use crate::view::details::diff_row;

    /// How the TUI drew a line of the text before the view model.
    fn old_style_line<'a>(
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

    fn render(line: Line, width: u16) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, 1));
        Paragraph::new(line).render(buf.area, &mut buf);
        buf
    }

    #[test]
    fn text_rows_draw_as_before() {
        let theme = Theme::default();
        let tokens: Tokens = vec![(0..2, Class::Keyword), (5..6, Class::Operator)];
        let lines = [
            "+fn x = 1;",
            "-fn x = 2;",
            " fn x = 3;",
            "diff --git a/x b/x",
            "+++ b/x",
            "@@ -1 +1 @@",
            "commit abc",
            "+plain add",
            "plain",
        ];
        let width = 24;
        for line in lines {
            for tokens in [None, Some(&tokens)] {
                for found in [vec![], std::iter::once(0..2).collect(), vec![3..6, 8..9]] {
                    let old = old_style_line(line, tokens, width, &found, &theme);
                    let new = text_line(diff_row(line, tokens, width, &found), &theme);
                    assert_eq!(
                        render(old.clone(), width as u16),
                        render(new, width as u16),
                        "{line:?} {tokens:?} {found:?}"
                    );
                    // Picked out as the match gone to.
                    let old = old.patch_style(theme.selected_unfocused);
                    let row = TextRow {
                        pick: Some(StyleRole::SelectedUnfocused),
                        ..diff_row(line, tokens, width, &found)
                    };
                    assert_eq!(render(old, width as u16), render(text_line(row, &theme), width as u16));
                }
            }
        }
    }
}
