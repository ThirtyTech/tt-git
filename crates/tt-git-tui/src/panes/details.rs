use std::collections::HashMap;
use std::sync::Arc;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tt_git_core::patch::{DiffPosition, diff_text_position, diff_text_row_of_line};
use tt_git_core::{CommitFile, FileState, StageSide, StatusEntry};
use unicode_width::UnicodeWidthStr;

use super::blame::BlameView;
use super::stage::StageView;
use super::{PaneId, pane_block};
use crate::action::Action;
use crate::component::{Component, EventState};
use crate::editor::EditTarget;
use crate::jobs::Ctx;
use crate::keymap::{Command, Context, KeyCombo};
use crate::syntax::{HunkTokens, LineTokens, Tokens};
use crate::theme::Theme;
use crate::widgets::{Cursor, code_spans, split_lines, truncate};

/// What the details pane is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailsSubject {
    Commit(String),
    /// A commit as a list of changed files beside the selected file's diff.
    CommitFiles(String),
    File(String),
    /// One file's changes, staged and unstaged by hunk or line.
    Stage(String),
    /// A stash, by its commit.
    Stash(String),
    /// Free text produced by the app (e.g. plugin logs).
    Text(String),
    /// A file's blame, by its path.
    Blame(String),
}

/// The changed-files view of a commit.
#[derive(Debug, Default)]
struct FilesView {
    oid: String,
    header: Vec<String>,
    files: Vec<CommitFile>,
    cursor: Cursor,
    /// Diff lines and their code already loaded, by file index.
    diffs: HashMap<usize, (Arc<Vec<String>>, Arc<LineTokens>)>,
    /// Where the file rows and the diff were drawn last frame, for the mouse.
    rows_area: Rect,
    diff_area: Rect,
    /// A file and new-side line to select and scroll to once they load
    /// (the line blame came from).
    focus: Option<(String, u32)>,
    /// The focused line once found: file index, diff row and line number.
    marked: Option<(usize, usize, u32)>,
    /// Scroll the marked line a third of the way down at the next draw.
    center: bool,
}

/// Commit details or file diff for the current selection.
#[derive(Debug, Default)]
pub struct DetailsPane {
    subject: Option<DetailsSubject>,
    title: String,
    /// The text shown, or in the files view the selected file's diff
    /// (shared with its entry in the view's `diffs`).
    lines: Arc<Vec<String>>,
    /// The code in `lines`, line by line; set along with them.
    syntax: Arc<LineTokens>,
    cursor: Cursor,
    loading: bool,
    files: Option<FilesView>,
    /// Rows the files view's header took in the last draw.
    header_rows: usize,
    /// The files view lists the files to the left of the diff (fullscreen)
    /// rather than below it.
    files_beside: bool,
    stage: Option<StageView>,
    blame: Option<BlameView>,
    /// What `esc` goes back to, shown after the title.
    trail: Option<String>,
}

/// Everything the details pane shows, put aside to come back to.
#[derive(Debug, Default)]
pub struct SavedDetails {
    subject: Option<DetailsSubject>,
    title: String,
    lines: Arc<Vec<String>>,
    syntax: Arc<LineTokens>,
    cursor: Cursor,
    loading: bool,
    files: Option<FilesView>,
    header_rows: usize,
    stage: Option<StageView>,
    blame: Option<BlameView>,
    trail: Option<String>,
}

impl SavedDetails {
    pub fn subject(&self) -> Option<&DetailsSubject> {
        self.subject.as_ref()
    }
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
                self.lines = Arc::default();
                self.syntax = Arc::default();
            }
            _ => self.files = None,
        }
        self.stage = None;
        self.blame = None;
        self.subject = Some(subject);
        self.title = title;
        self.loading = true;
        self.cursor = Cursor::default();
    }

    /// Sets content if it is for the current subject (stale results are
    /// dropped). Text replaces the files view (e.g. its files failed to load).
    /// `syntax` is the code in it, if any, line by line.
    pub fn set_content(&mut self, subject: &DetailsSubject, text: &str, syntax: LineTokens) {
        if self.subject.as_ref() == Some(subject) {
            self.lines = Arc::new(split_lines(text));
            self.syntax = Arc::new(syntax);
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
        self.lines = Arc::new(if files.is_empty() {
            vec!["no file changes".into()]
        } else {
            Vec::new()
        });
        self.syntax = Arc::default();
        let focus = self.files.take().and_then(|v| v.focus);
        let selected = focus
            .as_ref()
            .and_then(|(path, _)| files.iter().position(|f| f.path == *path))
            .unwrap_or(0);
        self.files = Some(FilesView {
            oid: oid.to_string(),
            header: split_lines(header.trim_end()),
            files,
            cursor: Cursor {
                selected,
                ..Cursor::default()
            },
            focus,
            ..FilesView::default()
        });
        self.show_selected_file(ctx);
    }

    /// Opens the staging view for `entry`; its diffs arrive via
    /// [`set_stage_diffs`](Self::set_stage_diffs).
    pub fn request_stage(&mut self, entry: StatusEntry) {
        let subject = DetailsSubject::Stage(entry.path.clone());
        if self.subject.as_ref() == Some(&subject) {
            return;
        }
        self.files = None;
        self.blame = None;
        self.lines = Arc::default();
        self.syntax = Arc::default();
        self.title = format!("Stage {}", entry.path);
        self.stage = Some(StageView::new(entry));
        self.subject = Some(subject);
        self.loading = true;
        self.cursor = Cursor::default();
    }

    /// Fills the staging view, if it's still showing `entry`.
    pub fn set_stage_diffs(
        &mut self,
        entry: StatusEntry,
        unstaged: StageSide,
        staged: StageSide,
        syntax: (HunkTokens, HunkTokens),
    ) {
        if self.subject != Some(DetailsSubject::Stage(entry.path.clone())) {
            return;
        }
        if let Some(view) = &mut self.stage {
            view.set_diffs(entry, unstaged, staged, syntax);
            self.loading = false;
        }
    }

    pub fn stage(&self) -> Option<&StageView> {
        self.stage.as_ref()
    }

    pub fn stage_mut(&mut self) -> Option<&mut StageView> {
        self.stage.as_mut()
    }

    /// Stores a file's diff and its code, showing it if that file is still
    /// selected.
    pub fn set_file_diff(&mut self, oid: &str, path: &str, text: &str, syntax: LineTokens) {
        let Some(view) = &mut self.files else { return };
        if view.oid != oid {
            return;
        }
        let Some(idx) = view.files.iter().position(|f| f.path == path) else {
            return;
        };
        let lines = Arc::new(split_lines(text));
        let syntax = Arc::new(syntax);
        if idx == view.cursor.selected {
            self.lines = lines.clone();
            self.syntax = syntax.clone();
            self.cursor.offset = 0;
            // The line blame came from: mark it, a third of the way down.
            if let Some((_, line)) = view.focus.take_if(|(p, _)| *p == path)
                && let Some(row) = diff_text_row_of_line(&lines, line)
            {
                view.marked = Some((idx, row, line));
                view.center = true;
            }
        }
        view.diffs.insert(idx, (lines, syntax));
    }

    fn show_selected_file(&mut self, ctx: &Ctx) {
        let Some(view) = &self.files else { return };
        let idx = view.cursor.selected;
        let Some(file) = view.files.get(idx) else { return };
        self.cursor.offset = 0;
        match view.diffs.get(&idx) {
            Some((lines, syntax)) => {
                self.lines = lines.clone();
                self.syntax = syntax.clone();
            }
            None => {
                self.lines = Arc::new(vec!["loading…".into()]);
                self.syntax = Arc::default();
                ctx.emit(Action::CommitFileSelected {
                    oid: view.oid.clone(),
                    file: file.clone(),
                });
            }
        }
    }

    /// The file shown and the line at the top of the view, to open in an
    /// editor, or why there isn't one. A file from a commit carries the
    /// commit, to open as it was there if it isn't in the working tree: a
    /// deleted file, as it was just before.
    pub fn edit_target(&self) -> Result<EditTarget, String> {
        let nothing = || "no file to open here".to_string();
        let row = self.cursor.offset;
        let pos = diff_text_position(self.lines.as_slice(), row);
        let line = pos.as_ref().and_then(|p| p.line);
        let before = |oid: &str| Some(format!("{oid}^"));
        if let Some(view) = &self.blame {
            return view.edit_target();
        }
        match self.subject.as_ref().ok_or_else(nothing)? {
            DetailsSubject::Text(_) | DetailsSubject::Stage(_) | DetailsSubject::Blame(_) => Err(nothing()),
            DetailsSubject::File(path) => {
                // An untracked file is shown whole, one `+` line per line
                // under a `new file` heading.
                let untracked = self.lines.first().is_some_and(|l| l.starts_with("new file "));
                let line = line.or_else(|| untracked.then(|| row.max(1) as u32));
                Ok(EditTarget {
                    path: path.clone(),
                    line,
                    rev: None,
                })
            }
            DetailsSubject::CommitFiles(oid) if self.files.is_some() => {
                let file = self.selected_file().ok_or_else(nothing)?;
                let deleted = file.state == FileState::Deleted;
                Ok(EditTarget {
                    path: file.path.clone(),
                    line: if deleted { None } else { line },
                    rev: if deleted { before(oid) } else { Some(oid.clone()) },
                })
            }
            DetailsSubject::Commit(oid) | DetailsSubject::CommitFiles(oid) | DetailsSubject::Stash(oid) => match pos {
                Some(DiffPosition {
                    path: Some(path),
                    deleted,
                    line,
                }) => Ok(EditTarget {
                    path,
                    line: if deleted { None } else { line },
                    rev: if deleted { before(oid) } else { Some(oid.clone()) },
                }),
                _ => Err(nothing()),
            },
        }
    }

    pub fn clear(&mut self) {
        self.subject = None;
        self.lines = Arc::default();
        self.syntax = Arc::default();
        self.files = None;
        self.stage = None;
        self.blame = None;
        self.trail = None;
        self.loading = false;
    }

    /// Shows `view` in place of whatever was showing.
    pub fn open_blame(&mut self, view: BlameView) {
        self.files = None;
        self.stage = None;
        self.lines = Arc::default();
        self.syntax = Arc::default();
        self.cursor = Cursor::default();
        self.loading = false;
        self.subject = Some(DetailsSubject::Blame(view.path().to_string()));
        self.title = view.title();
        self.blame = Some(view);
    }

    pub fn blame(&self) -> Option<&BlameView> {
        self.blame.as_ref()
    }

    pub fn blame_mut(&mut self) -> Option<&mut BlameView> {
        self.blame.as_mut()
    }

    /// Whether the subject's content is still on its way.
    pub fn loading(&self) -> bool {
        self.loading
    }

    /// Notes what `esc` goes back to, for the title.
    pub fn set_trail(&mut self, trail: Option<String>) {
        self.trail = trail;
    }

    /// In the files view loading now, selects `path` and marks new-side
    /// line `line` in its diff once that's loaded.
    pub fn focus_file(&mut self, path: String, line: u32) {
        if let Some(view) = &mut self.files {
            view.focus = Some((path, line));
            view.marked = None;
        }
    }

    /// How a view opened from here names this, to come back to.
    pub fn back_label(&self) -> String {
        let short = |oid: &str| oid.chars().take(7).collect::<String>();
        if let Some(view) = &self.blame {
            return view.back_label();
        }
        match &self.subject {
            Some(DetailsSubject::Commit(oid) | DetailsSubject::CommitFiles(oid)) => format!("Commit {}", short(oid)),
            Some(DetailsSubject::File(path)) => format!("Diff {path}"),
            _ => self.title.clone(),
        }
    }

    /// Puts everything shown aside, leaving the pane empty.
    pub fn save(&mut self) -> SavedDetails {
        SavedDetails {
            subject: self.subject.take(),
            title: std::mem::take(&mut self.title),
            lines: std::mem::take(&mut self.lines),
            syntax: std::mem::take(&mut self.syntax),
            cursor: std::mem::take(&mut self.cursor),
            loading: std::mem::take(&mut self.loading),
            files: self.files.take(),
            header_rows: std::mem::take(&mut self.header_rows),
            stage: self.stage.take(),
            blame: self.blame.take(),
            trail: self.trail.take(),
        }
    }

    /// Shows what [`save`](Self::save) put aside, as it was, asking again
    /// for anything whose load finished while it was away.
    pub fn restore(&mut self, saved: SavedDetails, ctx: &Ctx) {
        let SavedDetails {
            subject,
            title,
            lines,
            syntax,
            cursor,
            loading,
            files,
            header_rows,
            stage,
            blame,
            trail,
        } = saved;
        self.subject = subject;
        self.title = title;
        self.lines = lines;
        self.syntax = syntax;
        self.cursor = cursor;
        self.loading = loading;
        self.files = files;
        self.header_rows = header_rows;
        self.stage = stage;
        self.blame = blame;
        self.trail = trail;
        if let Some(view) = &mut self.blame {
            view.reload_missing(ctx);
        }
        let diff_missing = self
            .files
            .as_ref()
            .is_some_and(|v| !v.files.is_empty() && !v.diffs.contains_key(&v.cursor.selected));
        if diff_missing && !self.loading {
            let offset = self.cursor.offset;
            self.show_selected_file(ctx);
            self.cursor.offset = offset;
        }
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
            view.rows_area = rows_area;
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
            view.rows_area = list_block.inner(list_area);
            draw_file_list(view, frame, view.rows_area, focused, theme);
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
            .map(|l| style_line(l, None, 0, theme))
            .collect();
        frame.render_widget(Paragraph::new(header), header_area);
        let rule = "─".repeat(rule_area.width as usize);
        frame.render_widget(Paragraph::new(rule).style(theme.border), rule_area);
        view.diff_area = diff_area;
        let mark = view
            .marked
            .filter(|(idx, _, _)| *idx == view.cursor.selected)
            .map(|(_, row, line)| (row, line));
        if let Some((row, _)) = mark
            && std::mem::take(&mut view.center)
        {
            let height = diff_area.height as usize;
            let last_page = self.lines.len().saturating_sub(height);
            self.cursor.offset = row.saturating_sub(height / 3).min(last_page);
        }
        self.draw_text(frame, diff_area, mark, theme);
    }

    /// The text, with `mark`'s row (and new-side line number) picked out:
    /// the line blame came from.
    fn draw_text(&mut self, frame: &mut Frame, area: Rect, mark: Option<(usize, u32)>, theme: &Theme) {
        self.cursor.height = area.height as usize;
        let start = self.cursor.offset.min(self.lines.len());
        let end = (start + area.height as usize).min(self.lines.len());
        let lines: Vec<Line> = (start..end)
            .map(|row| {
                let text = &self.lines[row];
                let tokens = self.syntax.get(row).and_then(Option::as_ref);
                match mark {
                    Some((at, line)) if at == row => {
                        let note = format!("◂ line {line} in blame ");
                        let pad = (area.width as usize).saturating_sub(text.width() + note.width());
                        let mut marked = style_line(text, tokens, 0, theme);
                        marked.spans.push(Span::raw(" ".repeat(pad)));
                        marked.spans.push(Span::styled(note, theme.help_text));
                        marked.patch_style(theme.selected)
                    }
                    _ => style_line(text, tokens, area.width as usize, theme),
                }
            })
            .collect();
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
/// to `width` columns) with its `+` or `-` in the usual green or red.
fn style_line<'a>(line: &'a str, tokens: Option<&Tokens>, width: usize, theme: &Theme) -> Line<'a> {
    if let Some(tokens) = tokens
        && let Some(code) = line.get(1..)
    {
        let (sign, back) = match line.as_bytes().first() {
            Some(b'+') => (theme.diff_add, theme.diff_add_line),
            Some(b'-') => (theme.diff_del, theme.diff_del_line),
            _ => (Style::new(), Style::new()),
        };
        let mut spans = vec![Span::styled(&line[..1], sign)];
        spans.extend(code_spans(code, Some(tokens), Style::new(), theme, usize::MAX, false));
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
    Line::from(Span::styled(line, style))
}

impl Component for DetailsPane {
    fn context(&self) -> Context {
        if self.stage.is_some() {
            Context::Stage
        } else if self.blame.is_some() {
            Context::Blame
        } else {
            Context::Details
        }
    }

    fn on_command(&mut self, cmd: Command, ctx: &Ctx) -> EventState {
        if let Some(view) = &mut self.stage {
            return view.on_command(cmd, ctx);
        }
        if let Some(view) = &mut self.blame {
            return view.on_command(cmd, ctx);
        }
        if cmd == Command::OpenInEditor {
            ctx.emit(match self.edit_target() {
                Ok(target) => Action::OpenInEditor(target),
                Err(why) => Action::Notify(why),
            });
            return EventState::Consumed;
        }
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

    /// In blame, digits count lines, vim style.
    fn on_key(&mut self, key: KeyCombo, _ctx: &Ctx) -> bool {
        self.stage.is_none() && self.blame.as_mut().is_some_and(|view| view.on_key(key))
    }

    /// In blame, while a search is typed.
    fn wants_text_input(&self) -> bool {
        self.stage.is_none() && self.blame.as_ref().is_some_and(BlameView::searching)
    }

    fn on_text_key(&mut self, key: KeyCombo, ctx: &Ctx) {
        if let Some(view) = &mut self.blame {
            view.on_search_key(key, ctx);
        }
    }

    /// In blame, a click selects a line.
    fn on_click(&mut self, row: usize, ctx: &Ctx) {
        if let Some(view) = &mut self.blame {
            view.click(row, ctx);
        }
    }

    /// In the files view, a click on a file shows its diff.
    fn on_click_at(&mut self, pos: Position, ctx: &Ctx) -> bool {
        if self.stage.is_some() || self.blame.is_some() {
            return false;
        }
        let Some(view) = &mut self.files else { return false };
        if !view.rows_area.contains(pos) {
            return false;
        }
        let before = view.cursor.selected;
        let row = (pos.y - view.rows_area.y) as usize;
        if view.cursor.click(row, view.files.len()) && view.cursor.selected != before {
            self.show_selected_file(ctx);
        }
        true
    }

    /// In the files view, the wheel scrolls the diff when over it (and
    /// moves through the files elsewhere, like the arrow keys).
    fn on_scroll_at(&mut self, pos: Position, down: bool, _ctx: &Ctx) -> bool {
        if self.stage.is_some()
            || self.blame.is_some()
            || !self.files.as_ref().is_some_and(|v| v.diff_area.contains(pos))
        {
            return false;
        }
        self.scroll(if down { Command::MoveDown } else { Command::MoveUp });
        true
    }

    fn commands(&self) -> &[Command] {
        if self.stage.is_some() {
            return &[
                Command::StageToggle,
                Command::SwitchSide,
                Command::ToggleLineMode,
                Command::SelectRange,
                Command::Discard,
                Command::Commit,
                Command::OpenInEditor,
                Command::PrevFile,
                Command::NextFile,
                Command::CloseDetails,
            ];
        }
        if self.blame.is_some() {
            return &[
                Command::GoToLine,
                Command::OpenCommit,
                Command::ToggleDetails,
                Command::BlameBack,
                Command::BlameForward,
                Command::BlameOlder,
                Command::BlameNewer,
                Command::Search,
                Command::SearchNext,
                Command::SearchPrev,
                Command::ToggleDates,
                Command::OpenInEditor,
                Command::Copy,
                Command::CloseDetails,
            ];
        }
        &[
            Command::PageDown,
            Command::PageUp,
            Command::OpenLink,
            Command::OpenInEditor,
            Command::Blame,
            Command::ToggleFiles,
            Command::NewerCommit,
            Command::OlderCommit,
            Command::CloseDetails,
        ]
    }

    /// On a working-tree file's diff, `[` `]` step through the status
    /// pane's files rather than commits.
    fn label(&self, cmd: Command) -> &'static str {
        match (cmd, &self.subject) {
            (Command::NewerCommit, Some(DetailsSubject::File(_))) => Command::PrevFile.label(),
            (Command::OlderCommit, Some(DetailsSubject::File(_))) => Command::NextFile.label(),
            _ => cmd.label(),
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
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
        if let Some(search) = self.blame.as_ref().and_then(BlameView::search_title) {
            title.push_str(&search);
        }
        if self.loading || self.blame.as_ref().is_some_and(BlameView::loading) {
            title.push_str(" loading…");
        }
        // A blame's date and whether it's the latest version go at the
        // right, where there's room: the date goes first when there isn't.
        let changed = self.blame.as_ref().and_then(BlameView::changed);
        let recency = self.blame.as_ref().and_then(|v| v.recency(theme));
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
            self.cursor.height = inner.height as usize;
            frame.render_widget(Paragraph::new("nothing selected").style(theme.muted), inner);
            return;
        }
        if let Some(view) = &mut self.stage {
            view.draw(frame, inner, focused, theme);
        } else if let Some(view) = &mut self.blame {
            view.draw(frame, inner, focused, theme);
        } else if self.files.is_some() {
            self.draw_files(frame, inner, focused, theme);
        } else {
            self.draw_text(frame, inner, None, theme);
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
        pane.set_content(&DetailsSubject::Commit("a".into()), "commit a\nAuthor: me", Vec::new());

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
        pane.set_content(
            &DetailsSubject::CommitFiles("c".into()),
            "could not load commit",
            Vec::new(),
        );
        assert_eq!(paths(&pane), None);
        pane.request(DetailsSubject::CommitFiles("a".into()), "Commit a files".into());
        pane.request(DetailsSubject::Commit("a".into()), "Commit a".into());
        assert_eq!(paths(&pane), None);
    }
}
