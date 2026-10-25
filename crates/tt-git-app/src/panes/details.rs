use std::collections::HashMap;
use std::sync::Arc;

use tt_git_core::patch::{DiffPosition, diff_text_hunk, diff_text_position, diff_text_row_of_line};
use tt_git_core::{CommitFile, FileState, StageSide, StatusEntry};

use super::blame::BlameView;
use super::stage::StageView;
use crate::action::Action;
use crate::agent::AgentHunk;
use crate::component::{Component, EventState};
use crate::editor::EditTarget;
use crate::jobs::Ctx;
use crate::keymap::{Command, Context, KeyCombo};
use crate::mouse::PanePart;
use crate::search::{LineSearch, SearchKey};
use crate::syntax::{HunkTokens, LineTokens};
use crate::widgets::{Cursor, split_lines};

/// What the details pane is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailsSubject {
    Commit(String),
    /// A commit as a list of changed files beside the selected file's diff.
    CommitFiles(String),
    File(String),
    /// A folder's changes, from the status pane's tree; the path ends in `/`.
    Folder(String),
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
pub struct FilesView {
    oid: String,
    pub header: Vec<String>,
    pub files: Vec<CommitFile>,
    pub cursor: Cursor,
    /// Diff lines and their code already loaded, by file index.
    diffs: HashMap<usize, (Arc<Vec<String>>, Arc<LineTokens>)>,
    /// A file and new-side line to select and scroll to once they load
    /// (the line blame came from).
    focus: Option<(String, u32)>,
    /// The focused line once found: file index, diff row and line number.
    marked: Option<(usize, usize, u32)>,
    /// Scroll the marked line a third of the way down at the next draw.
    center: bool,
    /// `n` or `N` going on through the files for a match.
    seek: Option<Seek>,
}

/// A search going on from file to file in the files view, for `n` (forward)
/// or `N`, waiting on each diff that isn't loaded yet.
#[derive(Debug, Clone, Copy)]
struct Seek {
    forward: bool,
    /// The file it started in: back there, it has been through them all.
    from: usize,
    /// It went round past the last file (or the first).
    wrapped: bool,
}

/// What the file a [`Seek`] reached says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeekStep {
    /// Its diff is still loading.
    Wait,
    /// It stopped: at a match, or with none in any file.
    Done,
    /// No match here; on to the next file.
    Next,
}

/// Commit details or file diff for the current selection.
#[derive(Debug, Default)]
pub struct DetailsPane {
    pub subject: Option<DetailsSubject>,
    pub title: String,
    /// The text shown, or in the files view the selected file's diff
    /// (shared with its entry in the view's `diffs`).
    pub lines: Arc<Vec<String>>,
    /// The code in `lines`, line by line; set along with them.
    pub syntax: Arc<LineTokens>,
    pub cursor: Cursor,
    pub loading: bool,
    pub files: Option<FilesView>,
    /// Rows the files view's header took in the last draw.
    header_rows: usize,
    /// The files view lists the files to the left of the diff (fullscreen)
    /// rather than below it.
    pub files_beside: bool,
    pub stage: Option<StageView>,
    pub blame: Option<BlameView>,
    /// What `esc` goes back to, shown after the title.
    pub trail: Option<String>,
    /// A search of the text, typed after `/`. It stays when the text
    /// changes, and finds its matches in the new text.
    pub search: LineSearch,
    /// The match `n` and `N` went to last, picked out in the text.
    pub found: Option<usize>,
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

    /// The front end shows `rows` lines of the text (in the files view, of
    /// the selected file's diff): pages and searches by them. Call before
    /// drawing.
    pub fn set_viewport(&mut self, rows: usize) {
        self.cursor.height = rows;
    }

    /// The front end shows the files view's list in `list_rows` lines, the
    /// first `header_rows` lines of the commit header, and `diff_rows`
    /// lines of the diff: keeps the selected file in view, and scrolls the
    /// line blame came from a third of the way down once it's found. Call
    /// before [`set_viewport`](Self::set_viewport) and drawing.
    pub fn set_files_viewport(&mut self, list_rows: usize, header_rows: usize, diff_rows: usize) {
        self.header_rows = header_rows;
        let mark = self.files_mark();
        let Some(view) = &mut self.files else { return };
        view.cursor.scroll_into_view(list_rows);
        if let Some((row, _)) = mark
            && std::mem::take(&mut view.center)
        {
            let last_page = self.lines.len().saturating_sub(diff_rows);
            self.cursor.offset = row.saturating_sub(diff_rows / 3).min(last_page);
        }
    }

    /// In the files view, the row of the selected file's diff blame came
    /// from and its new-side line number, once found.
    pub fn files_mark(&self) -> Option<(usize, u32)> {
        let view = self.files.as_ref()?;
        view.marked
            .filter(|(idx, _, _)| *idx == view.cursor.selected)
            .map(|(_, row, line)| (row, line))
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
            self.find_matches();
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
        self.find_matches();
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
    /// selected, and goes on with a search through the files waiting on it.
    pub fn set_file_diff(&mut self, oid: &str, path: &str, text: &str, syntax: LineTokens, ctx: &Ctx) {
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
            self.search.find(lines.iter().map(String::as_str));
            self.found = None;
            // The line blame came from: mark it, a third of the way down.
            if let Some((_, line)) = view.focus.take_if(|(p, _)| *p == path)
                && let Some(row) = diff_text_row_of_line(&lines, line)
            {
                view.marked = Some((idx, row, line));
                view.center = true;
            }
        }
        let seeking = idx == view.cursor.selected && view.seek.is_some();
        view.diffs.insert(idx, (lines, syntax));
        if seeking && self.seek_check(ctx) == SeekStep::Next {
            self.seek_on(ctx);
        }
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
        self.find_matches();
    }

    /// Works out which lines of the text match the search, afresh.
    fn find_matches(&mut self) {
        self.search.find(self.lines.iter().map(String::as_str));
        self.found = None;
    }

    /// Picks out match `row`, scrolling it a third of the way down if it's
    /// out of view.
    fn show_match(&mut self, row: usize) {
        self.found = Some(row);
        let height = self.cursor.height.max(1);
        let offset = &mut self.cursor.offset;
        if row < *offset || row >= *offset + height {
            let last_page = self.lines.len().saturating_sub(height);
            *offset = row.saturating_sub(height / 3).min(last_page);
        }
    }

    /// Goes to the next match, or the one before: after the one gone to
    /// last if it's in view, otherwise from the top of the view. In the
    /// files view, past the diff's last match it goes on to the next file
    /// with one (or back to the previous), round past the end.
    fn next_match(&mut self, forward: bool, ctx: &Ctx) {
        let (top, height) = (self.cursor.offset, self.cursor.height.max(1));
        let found = self.found.filter(|&f| (top..top + height).contains(&f));
        let Some(view) = &mut self.files else {
            let at = match found {
                Some(found) => found,
                None if forward => {
                    if let Some(&first) = self.search.lines().iter().find(|&&l| l >= top) {
                        return self.show_match(first);
                    }
                    top
                }
                None => top,
            };
            if let Some(row) = self.search.step(at, forward, ctx) {
                self.show_match(row);
            }
            return;
        };
        if view.seek.is_some() {
            // Still waiting on a diff.
            return;
        }
        if !self.search.active() {
            ctx.emit(Action::Notify("nothing to search for yet".into()));
            return;
        }
        let next = match found {
            Some(found) => self.search.after(found, forward),
            None if forward => self.search.lines().iter().find(|&&l| l >= top).copied(),
            None => self.search.after(top, false),
        };
        if let Some(row) = next {
            return self.show_match(row);
        }
        view.seek = Some(Seek {
            forward,
            from: view.cursor.selected,
            wrapped: false,
        });
        self.seek_on(ctx);
    }

    /// Moves the seek on a file at a time, showing each, until one has a
    /// match or a diff to wait for.
    fn seek_on(&mut self, ctx: &Ctx) {
        loop {
            let Some(view) = &mut self.files else { return };
            let n = view.files.len();
            let Some(seek) = &mut view.seek else { return };
            if n == 0 {
                view.seek = None;
                return;
            }
            let at = view.cursor.selected;
            let next = if seek.forward { (at + 1) % n } else { (at + n - 1) % n };
            if (seek.forward && next <= at) || (!seek.forward && next >= at) {
                seek.wrapped = true;
            }
            view.cursor.selected = next;
            self.show_selected_file(ctx);
            if self.seek_check(ctx) != SeekStep::Next {
                return;
            }
        }
    }

    /// Where the seek stands with the file it reached shown: at its first
    /// match (last, going back), or back where it started with no match
    /// anywhere.
    fn seek_check(&mut self, ctx: &Ctx) -> SeekStep {
        let Some(view) = &mut self.files else {
            return SeekStep::Done;
        };
        let Some(seek) = view.seek else { return SeekStep::Done };
        let at = view.cursor.selected;
        if !view.diffs.contains_key(&at) {
            return SeekStep::Wait;
        }
        let lines = self.search.lines();
        let row = if seek.forward { lines.first() } else { lines.last() };
        if let Some(&row) = row {
            view.seek = None;
            if seek.wrapped {
                let why = match (view.files.len(), seek.forward) {
                    (1, true) => "search wrapped to the top",
                    (1, false) => "search wrapped to the bottom",
                    (_, true) => "search wrapped to the first file",
                    (_, false) => "search wrapped to the last file",
                };
                ctx.emit(Action::Notify(why.into()));
            }
            self.show_match(row);
            return SeekStep::Done;
        }
        if at == seek.from {
            view.seek = None;
            ctx.emit(Action::Notify(format!(
                "no match for /{} in any file",
                self.search.pattern()
            )));
            return SeekStep::Done;
        }
        SeekStep::Next
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
            DetailsSubject::Text(_)
            | DetailsSubject::Stage(_)
            | DetailsSubject::Blame(_)
            | DetailsSubject::Folder(_) => Err(nothing()),
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

    /// What to ask an agent about here: the staging view's selection, or
    /// the diff hunk at the top of the view (the line `e` opens). `None`
    /// for what isn't a diff (blame, plain text); an error when the diff
    /// has no hunk there.
    pub fn agent_hunk(&self) -> Option<Result<AgentHunk, String>> {
        let none = || "no diff hunk here to ask about".to_string();
        if let Some(view) = &self.stage {
            return Some(view.agent_hunk().ok_or_else(none));
        }
        if self.blame.is_some() {
            return None;
        }
        let subject = self.subject.as_ref()?;
        if matches!(
            subject,
            DetailsSubject::Text(_) | DetailsSubject::Blame(_) | DetailsSubject::Stage(_)
        ) {
            return None;
        }
        let Some(found) = diff_text_hunk(self.lines.as_slice(), self.cursor.offset) else {
            return Some(Err(none()));
        };
        let hunk = &found.hunk;
        let picks = [(hunk, 0, hunk.lines.len().saturating_sub(1))];
        let short = |oid: &str| oid.chars().take(7).collect::<String>();
        Some(Ok(match subject {
            DetailsSubject::File(path) | DetailsSubject::Folder(path) => {
                // The file's (or folder's) diff is in staged and unstaged sections.
                let staged = self.lines[..found.rows.start]
                    .iter()
                    .rev()
                    .find(|l| l.starts_with("── "))
                    .is_some_and(|l| l.contains("staged") && !l.contains("unstaged"));
                let source = if staged {
                    "the staged changes (the index against HEAD)"
                } else {
                    "the unstaged changes (the working tree against the index)"
                };
                AgentHunk::new(found.path.as_deref().unwrap_or(path), source.into(), &picks)
            }
            DetailsSubject::Stash(oid) => {
                let path = found.path.clone().unwrap_or_default();
                AgentHunk::new(&path, format!("the stash at commit {}", short(oid)), &picks).in_commit(oid, None)
            }
            DetailsSubject::Commit(oid) | DetailsSubject::CommitFiles(oid) => {
                let path = found
                    .path
                    .clone()
                    .or_else(|| self.selected_file().map(|f| f.path.clone()))
                    .unwrap_or_default();
                AgentHunk::new(&path, format!("commit {}", short(oid)), &picks).in_commit(oid, None)
            }
            DetailsSubject::Text(_) | DetailsSubject::Blame(_) | DetailsSubject::Stage(_) => return None,
        }))
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
        self.find_matches();
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
        match cmd {
            Command::OpenInEditor => {
                ctx.emit(match self.edit_target() {
                    Ok(target) => Action::OpenInEditor(target),
                    Err(why) => Action::Notify(why),
                });
                return EventState::Consumed;
            }
            Command::Search => {
                self.search = LineSearch::start(self.cursor.offset);
                self.found = None;
                if let Some(view) = &mut self.files {
                    view.seek = None;
                }
                return EventState::Consumed;
            }
            Command::SearchNext | Command::SearchPrev => {
                self.next_match(cmd == Command::SearchNext, ctx);
                return EventState::Consumed;
            }
            _ => {}
        }
        // In the files view, line movement picks a file and paging scrolls its diff.
        if let Some(view) = &mut self.files
            && matches!(
                cmd,
                Command::MoveUp | Command::MoveDown | Command::Top | Command::Bottom
            )
        {
            let before = view.cursor.selected;
            view.seek = None;
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

    /// While a search is typed, in blame or here.
    fn wants_text_input(&self) -> bool {
        self.stage.is_none() && self.blame.as_ref().map_or(self.search.typing(), BlameView::searching)
    }

    /// Each change to the search goes to its first match from where it
    /// started, or back there when nothing matches.
    fn on_text_key(&mut self, key: KeyCombo, ctx: &Ctx) {
        if let Some(view) = &mut self.blame {
            view.on_search_key(key, ctx);
            return;
        }
        match self.search.on_key(key, ctx) {
            SearchKey::Changed => {
                self.find_matches();
                let to = self.search.landing();
                if self.search.lines().is_empty() {
                    self.cursor.offset = to;
                } else {
                    self.show_match(to);
                }
            }
            SearchKey::Cancelled(origin) => {
                self.found = None;
                self.cursor.offset = origin;
            }
            SearchKey::Done => {}
        }
    }

    /// In blame, a click selects a line.
    fn on_click(&mut self, row: usize, ctx: &Ctx) {
        if let Some(view) = &mut self.blame {
            view.click(row, ctx);
        }
    }

    /// In the files view, a click on a file shows its diff; staging, a
    /// click on a row picks its hunk or line.
    fn on_click_part(&mut self, part: PanePart, ctx: &Ctx) -> bool {
        if let PanePart::StageRow { side, row } = part {
            if let Some(view) = &mut self.stage {
                view.click(side, row);
            }
            return true;
        }
        if self.stage.is_some() || self.blame.is_some() {
            return false;
        }
        let Some(view) = &mut self.files else { return false };
        let PanePart::FileRow(row) = part else {
            return false;
        };
        let before = view.cursor.selected;
        view.seek = None;
        if view.cursor.click(row, view.files.len()) && view.cursor.selected != before {
            self.show_selected_file(ctx);
        }
        true
    }

    /// In the files view, the wheel scrolls the diff when over it (and
    /// moves through the files elsewhere, like the arrow keys).
    fn on_scroll_part(&mut self, part: PanePart, down: bool, _ctx: &Ctx) -> bool {
        if self.stage.is_some() || self.blame.is_some() || self.files.is_none() || part != PanePart::Diff {
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
                Command::Agent,
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
            Command::Search,
            Command::SearchNext,
            Command::SearchPrev,
            Command::Agent,
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
