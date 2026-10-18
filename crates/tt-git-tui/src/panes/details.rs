use std::collections::HashMap;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tt_git_core::{CommitFile, FileState};
use unicode_width::UnicodeWidthStr;

use super::{PaneId, pane_block};
use crate::action::Action;
use crate::component::{Component, EventState};
use crate::jobs::Ctx;
use crate::keymap::{Command, Context};
use crate::theme::Theme;
use crate::widgets::{Cursor, truncate};

/// What the details pane is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailsSubject {
    Commit(String),
    /// A commit as a list of changed files beside the selected file's diff.
    CommitFiles(String),
    File(String),
    /// Free text produced by the app (e.g. plugin logs).
    Text(String),
}

/// The changed-files view of a commit.
#[derive(Debug, Default)]
struct FilesView {
    oid: String,
    header: Vec<String>,
    files: Vec<CommitFile>,
    cursor: Cursor,
    /// Diff lines already loaded, by file index.
    diffs: HashMap<usize, Vec<String>>,
}

/// Commit details or file diff for the current selection.
#[derive(Debug, Default)]
pub struct DetailsPane {
    subject: Option<DetailsSubject>,
    title: String,
    /// The text shown, or in the files view the selected file's diff.
    lines: Vec<String>,
    cursor: Cursor,
    loading: bool,
    files: Option<FilesView>,
    /// Rows the files view's header took in the last draw.
    header_rows: usize,
    /// The files view lists the files to the left of the diff (fullscreen)
    /// rather than below it.
    files_beside: bool,
}

impl DetailsPane {
    pub fn subject(&self) -> Option<&DetailsSubject> {
        self.subject.as_ref()
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The text rows currently on screen, top to bottom. In the files view
    /// these are the header, separator, then diff, without the file list.
    pub fn visible_lines(&self) -> Vec<String> {
        let start = self.cursor.offset.min(self.lines.len());
        let end = (start + self.cursor.height.max(1)).min(self.lines.len());
        let diff = self.lines[start..end].iter().cloned();
        match &self.files {
            Some(view) => {
                let mut rows: Vec<String> = view.header.iter().take(self.header_rows).cloned().collect();
                if self.header_rows > 0 {
                    rows.push(String::new());
                }
                rows.extend(diff);
                rows
            }
            None => diff.collect(),
        }
    }

    /// The changed files, in the files view.
    pub fn commit_files(&self) -> Option<&[CommitFile]> {
        self.files.as_ref().map(|v| v.files.as_slice())
    }

    /// The file selected in the files view.
    pub fn selected_file(&self) -> Option<&CommitFile> {
        let view = self.files.as_ref()?;
        view.files.get(view.cursor.selected)
    }

    /// Puts the files view's list beside the diff instead of below it.
    pub fn set_files_beside(&mut self, beside: bool) {
        self.files_beside = beside;
    }

    /// Marks `subject` as the thing to show; content arrives via
    /// [`set_content`](Self::set_content) or, for the files view,
    /// [`set_commit_files`](Self::set_commit_files).
    pub fn request(&mut self, subject: DetailsSubject, title: String) {
        if self.subject.as_ref() == Some(&subject) {
            return;
        }
        // The old content stays up until the new arrives, and so does the
        // files view's layout: drawing a loading files view as plain text
        // would show it full width for a frame, then jump right when the
        // list appears. Coming from another view, an empty list stands in.
        match &subject {
            DetailsSubject::CommitFiles(_) if self.files.is_some() => {}
            DetailsSubject::CommitFiles(oid) => {
                self.files = Some(FilesView {
                    oid: oid.clone(),
                    ..FilesView::default()
                });
                self.lines.clear();
            }
            _ => self.files = None,
        }
        self.subject = Some(subject);
        self.title = title;
        self.loading = true;
        self.cursor = Cursor::default();
    }

    /// Sets content if it is for the current subject (stale results are
    /// dropped). Text replaces the files view (e.g. its files failed to load).
    pub fn set_content(&mut self, subject: &DetailsSubject, text: &str) {
        if self.subject.as_ref() == Some(subject) {
            self.lines = split_lines(text);
            self.loading = false;
            self.files = None;
        }
    }

    /// Fills the files view for `oid` and asks for the first file's diff.
    pub fn set_commit_files(&mut self, oid: &str, header: &str, files: Vec<CommitFile>, ctx: &Ctx) {
        if self.subject != Some(DetailsSubject::CommitFiles(oid.to_string())) {
            return;
        }
        self.loading = false;
        self.lines = if files.is_empty() {
            vec!["no file changes".into()]
        } else {
            Vec::new()
        };
        self.files = Some(FilesView {
            oid: oid.to_string(),
            header: split_lines(header.trim_end()),
            files,
            ..FilesView::default()
        });
        self.show_selected_file(ctx);
    }

    /// Stores a file's diff, showing it if that file is still selected.
    pub fn set_file_diff(&mut self, oid: &str, path: &str, text: &str) {
        let Some(view) = &mut self.files else { return };
        if view.oid != oid {
            return;
        }
        let Some(idx) = view.files.iter().position(|f| f.path == path) else {
            return;
        };
        let lines = split_lines(text);
        if idx == view.cursor.selected {
            self.lines = lines.clone();
            self.cursor.offset = 0;
        }
        view.diffs.insert(idx, lines);
    }

    fn show_selected_file(&mut self, ctx: &Ctx) {
        let Some(view) = &self.files else { return };
        let idx = view.cursor.selected;
        let Some(file) = view.files.get(idx) else { return };
        self.cursor.offset = 0;
        match view.diffs.get(&idx) {
            Some(lines) => self.lines = lines.clone(),
            None => {
                self.lines = vec!["loading…".into()];
                ctx.emit(Action::CommitFileSelected {
                    oid: view.oid.clone(),
                    file: file.clone(),
                });
            }
        }
    }

    pub fn clear(&mut self) {
        self.subject = None;
        self.lines.clear();
        self.files = None;
        self.loading = false;
    }

    fn scroll(&mut self, cmd: Command) -> EventState {
        // The diff scrolls rather than selecting lines: keep the cursor at
        // the top of the viewport.
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

    /// The files view: the file list on the left when fullscreen, otherwise
    /// along the bottom; the commit header and selected file's diff fill the rest.
    fn draw_files(&mut self, frame: &mut Frame, inner: Rect, focused: bool, theme: &Theme) {
        let Some(view) = &mut self.files else { return };
        let label = if self.loading {
            "Files".to_string()
        } else {
            format!("Files ({})", view.files.len())
        };
        let title = Line::styled(label, theme.muted);
        let body = if self.files_beside {
            let longest = view.files.iter().map(|f| file_label(f).width() + 3).max().unwrap_or(0);
            let list_width = (longest as u16 + 1).clamp(20, (inner.width * 2 / 5).max(20));
            let [list_area, right] =
                Layout::horizontal([Constraint::Length(list_width), Constraint::Fill(1)]).areas(inner);
            let list_block = Block::new().borders(Borders::RIGHT).border_style(theme.border);
            let [title_area, rows_area] =
                Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(list_block.inner(list_area));
            frame.render_widget(list_block, list_area);
            frame.render_widget(Paragraph::new(title), title_area);
            draw_file_list(view, frame, rows_area, focused, theme);
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
            draw_file_list(view, frame, list_block.inner(list_area), focused, theme);
            frame.render_widget(list_block, list_area);
            top
        };

        // Header (up to a third of the height), a rule, then the diff. Above
        // the bottom list the space can be tiny; under three rows the header
        // and rule give way so the diff still shows.
        let max_header = body.height as usize / 3;
        let max_header = if self.files_beside {
            max_header.max(1)
        } else {
            max_header
        };
        let header_rows = view.header.len().min(max_header);
        self.header_rows = header_rows;
        let [header_area, rule_area, diff_area] = Layout::vertical([
            Constraint::Length(header_rows as u16),
            Constraint::Length(u16::from(header_rows > 0)),
            Constraint::Fill(1),
        ])
        .areas(body);
        let header: Vec<Line> = view.header[..header_rows]
            .iter()
            .map(|l| style_line(l, theme))
            .collect();
        frame.render_widget(Paragraph::new(header), header_area);
        let rule = "─".repeat(rule_area.width as usize);
        frame.render_widget(Paragraph::new(rule).style(theme.border), rule_area);
        self.draw_text(frame, diff_area, theme);
    }

    fn draw_text(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        self.cursor.height = area.height as usize;
        let start = self.cursor.offset.min(self.lines.len());
        let end = (start + area.height as usize).min(self.lines.len());
        let lines: Vec<Line> = self.lines[start..end].iter().map(|l| style_line(l, theme)).collect();
        frame.render_widget(Paragraph::new(lines), area);
    }
}

/// The changed files, scrolled to keep the selection in `area`.
fn draw_file_list(view: &mut FilesView, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
    let width = area.width as usize;
    view.cursor.scroll_into_view(area.height as usize);
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

fn split_lines(text: &str) -> Vec<String> {
    text.lines().map(|l| l.replace('\t', "    ")).collect()
}

fn file_label(file: &CommitFile) -> String {
    match &file.orig_path {
        Some(orig) => format!("{orig} → {}", file.path),
        None => file.path.clone(),
    }
}

fn state_style(state: FileState, theme: &Theme) -> ratatui::style::Style {
    match state {
        FileState::Added | FileState::Copied => theme.diff_add,
        FileState::Deleted => theme.diff_del,
        FileState::Renamed => theme.diff_hunk,
        _ => theme.label_tag,
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

    fn on_command(&mut self, cmd: Command, ctx: &Ctx) -> EventState {
        // In the files view, line movement picks a file and paging scrolls its diff.
        if let Some(view) = &mut self.files
            && matches!(
                cmd,
                Command::MoveUp | Command::MoveDown | Command::Top | Command::Bottom
            )
        {
            let before = view.cursor.selected;
            view.cursor.apply(cmd, view.files.len());
            if view.cursor.selected != before {
                self.show_selected_file(ctx);
            }
            return EventState::Consumed;
        }
        self.scroll(cmd)
    }

    fn commands(&self) -> &[Command] {
        &[Command::OpenLink, Command::ToggleFiles, Command::CloseDetails]
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

        if self.subject.is_none() {
            self.cursor.height = inner.height as usize;
            frame.render_widget(Paragraph::new("nothing selected").style(theme.muted), inner);
            return;
        }
        if self.files.is_some() {
            self.draw_files(frame, inner, focused, theme);
        } else {
            self.draw_text(frame, inner, theme);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> CommitFile {
        CommitFile {
            state: FileState::Modified,
            path: path.into(),
            orig_path: None,
        }
    }

    fn paths(pane: &DetailsPane) -> Option<Vec<String>> {
        pane.commit_files()
            .map(|files| files.iter().map(|f| f.path.clone()).collect())
    }

    #[test]
    fn a_loading_files_view_keeps_its_layout() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = Ctx::new(tx);
        let mut pane = DetailsPane::default();
        pane.request(DetailsSubject::Commit("a".into()), "Commit a".into());
        pane.set_content(&DetailsSubject::Commit("a".into()), "commit a\nAuthor: me");

        // From plain text, an empty list stands in and the text goes.
        pane.request(DetailsSubject::CommitFiles("a".into()), "Commit a files".into());
        assert_eq!(paths(&pane), Some(vec![]));
        assert!(pane.lines().is_empty());
        pane.set_commit_files("a", "subject a", vec![file("a.txt")], &ctx);
        assert_eq!(paths(&pane), Some(vec!["a.txt".into()]));

        // Another commit's files keep the old list up until they arrive.
        pane.request(DetailsSubject::CommitFiles("b".into()), "Commit b files".into());
        assert_eq!(paths(&pane), Some(vec!["a.txt".into()]));
        pane.set_commit_files("b", "subject b", vec![file("b.txt")], &ctx);
        assert_eq!(paths(&pane), Some(vec!["b.txt".into()]));

        // Text for the files view (a failed load) and other subjects drop the list.
        pane.request(DetailsSubject::CommitFiles("c".into()), "Commit c files".into());
        pane.set_content(&DetailsSubject::CommitFiles("c".into()), "could not load commit");
        assert_eq!(paths(&pane), None);
        pane.request(DetailsSubject::CommitFiles("a".into()), "Commit a files".into());
        pane.request(DetailsSubject::Commit("a".into()), "Commit a".into());
        assert_eq!(paths(&pane), None);
    }
}
