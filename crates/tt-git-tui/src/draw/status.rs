//! The status pane: changed files as a list or a folder tree.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tt_git_core::StatusEntry;

use super::{Draw, Hits, pane_block};
use crate::panes::status::{Row, file_name, in_folder};
use crate::panes::{PaneId, StatusPane};
use crate::tui_theme::Theme;
use crate::widgets::truncate;

impl Draw for StatusPane {
    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme, _hits: &mut Hits) {
        let block = pane_block(PaneId::Status, format!("Status: {}", self.summary()), focused, theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if self.entries().is_empty() {
            let msg = match (self.status(), self.folder()) {
                (None, _) => "loading…".to_string(),
                (Some(_), Some(f)) if !self.all_entries().is_empty() => format!("no changes in {f} (. shows all)"),
                (Some(_), _) => "working tree clean".to_string(),
            };
            frame.render_widget(Paragraph::new(msg).style(theme.muted), inner);
            return;
        }
        let mut inner = inner;
        if let Some(op) = self.status().and_then(|s| s.in_progress) {
            let banner = format!("{}: stage fixes · C continue · X abort", op.label());
            frame.render_widget(
                Paragraph::new(truncate(&banner, inner.width as usize)).style(theme.label_tag),
                Rect { height: 1, ..inner },
            );
            inner.y += 1;
            inner.height = inner.height.saturating_sub(1);
        }
        let height = inner.height as usize;
        self.set_viewport(height);
        let width = inner.width as usize;
        let cursor = self.cursor();
        let rows = self.rows();
        let start = cursor.offset.min(rows.len());
        let end = (start + height).min(rows.len());
        let lines: Vec<Line> = (start..end)
            .map(|i| {
                let line = row_line(self, &rows[i], width, theme);
                if i == cursor.selected {
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
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

/// The text of each row of `pane` as drawn (without colors), for tests.
pub fn row_texts(pane: &StatusPane) -> Vec<String> {
    pane.rows()
        .iter()
        .map(|r| {
            row_line(pane, r, usize::MAX, &Theme::default())
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect()
        })
        .collect()
}

/// One row as drawn: a file's status code and path (its name alone in
/// the tree), or a folder's open/closed marker and name.
fn row_line(pane: &StatusPane, row: &Row, width: usize, theme: &Theme) -> Line<'static> {
    match row {
        Row::File { index, depth } => {
            let e = &pane.entries()[*index];
            let indent = "  ".repeat(*depth);
            let name = if pane.tree() { file_name(e) } else { e.path.clone() };
            let path = match &e.orig_path {
                Some(orig) => format!("{orig} → {name}"),
                None => name,
            };
            let room = width.saturating_sub(indent.len() + 3);
            Line::from(vec![
                Span::raw(indent),
                Span::styled(e.short_code(), code_style([e], theme)),
                Span::raw(" "),
                Span::raw(truncate(&path, room)),
            ])
        }
        Row::Folder { path, name, depth } => {
            let indent = "  ".repeat(*depth);
            let marker = if pane.is_collapsed(path) { "▸ " } else { "▾ " };
            let style = code_style(pane.entries().iter().filter(|e| in_folder(e, path)), theme);
            let room = width.saturating_sub(indent.len() + 2);
            Line::from(vec![
                Span::raw(indent),
                Span::styled(marker, style),
                Span::styled(truncate(&format!("{name}/"), room), style),
            ])
        }
    }
}

/// The color of a file's status code, or of a folder holding `entries`:
/// conflicted, all staged, or with something left to stage.
fn code_style<'a>(entries: impl IntoIterator<Item = &'a StatusEntry>, theme: &Theme) -> Style {
    let (mut conflicted, mut unstaged) = (false, false);
    for e in entries {
        conflicted |= e.is_conflicted();
        unstaged |= !e.has_staged() || e.has_unstaged();
    }
    if conflicted {
        theme.error
    } else if unstaged {
        theme.diff_del
    } else {
        theme.diff_add
    }
}
