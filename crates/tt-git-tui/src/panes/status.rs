use std::collections::{BTreeMap, HashSet};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tt_git_core::{DiscardKind, IgnoreTarget, StashScope, Status, StatusEntry, ignore_pattern};

use super::{PaneId, pane_block};
use crate::action::Action;
use crate::component::{Component, EventState};
use crate::jobs::Ctx;
use crate::keymap::{Command, Context};
use crate::modal::{DiscardCounts, GitOp, Modal, PaletteItem, PaletteTarget, PromptAction};
use crate::tui_theme::Theme;
use crate::widgets::{Cursor, truncate};

/// Working tree status with a repo/branch summary in the title.
#[derive(Debug, Default)]
pub struct StatusPane {
    pub repo_name: String,
    status: Option<Status>,
    /// Shows only the files under this folder (ending in `/`), plus any
    /// conflicted ones; bulk actions act on it alone.
    folder: Option<String>,
    /// The entries shown: all of them, or those `folder` keeps.
    shown: Vec<StatusEntry>,
    /// List `shown` as a folder tree rather than by full path.
    tree: bool,
    /// Folders in the tree (ending in `/`) whose contents are hidden.
    collapsed: HashSet<String>,
    /// What's listed, one per line: every shown entry, or the tree's rows.
    rows: Vec<Row>,
    cursor: Cursor,
}

/// One line of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    /// `shown[index]`, `depth` folders in.
    File { index: usize, depth: usize },
    /// A folder in the tree. `path` ends in `/`; `name` is what's drawn,
    /// several folders deep when a folder holds only one folder.
    Folder { path: String, name: String, depth: usize },
}

/// What a row is, to find it again after the rows are rebuilt.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RowKey {
    File(String),
    Folder(String),
}

/// A folder's contents, for building the tree.
#[derive(Debug, Default)]
struct Dir {
    dirs: BTreeMap<String, Dir>,
    /// The name each file is listed by, with its index into the entries.
    files: Vec<(String, usize)>,
}

/// The name `e` is listed by in the tree: the last part of its path. An
/// untracked folder keeps its `/`.
fn file_name(e: &StatusEntry) -> String {
    let trimmed = e.path.trim_end_matches('/');
    let name = trimmed.rsplit_once('/').map_or(trimmed, |(_, n)| n);
    if e.path.ends_with('/') {
        format!("{name}/")
    } else {
        name.to_string()
    }
}

impl Dir {
    /// `entries` filed by folder. An untracked folder (`path` ending in `/`)
    /// is listed as a file in its parent.
    fn of(entries: &[StatusEntry]) -> Self {
        let mut root = Dir::default();
        for (i, e) in entries.iter().enumerate() {
            let mut dir = &mut root;
            if let Some((dirs, _)) = e.path.trim_end_matches('/').rsplit_once('/') {
                for part in dirs.split('/') {
                    dir = dir.dirs.entry(part.to_string()).or_default();
                }
            }
            dir.files.push((file_name(e), i));
        }
        root
    }

    /// The rows under this folder (at `prefix`, `depth` in): folders first,
    /// then files, each by name. Collapsed folders' contents are left out.
    fn rows(&self, prefix: &str, depth: usize, collapsed: &HashSet<String>, out: &mut Vec<Row>) {
        for (name, dir) in &self.dirs {
            let mut name = name.clone();
            let mut dir = dir;
            // A folder holding only one folder shares its row.
            while dir.files.is_empty() && dir.dirs.len() == 1 {
                let (child, inner) = dir.dirs.iter().next().expect("one folder");
                name = format!("{name}/{child}");
                dir = inner;
            }
            let path = format!("{prefix}{name}/");
            let open = !collapsed.contains(&path);
            out.push(Row::Folder {
                path: path.clone(),
                name,
                depth,
            });
            if open {
                dir.rows(&path, depth + 1, collapsed, out);
            }
        }
        let mut files = self.files.clone();
        files.sort();
        out.extend(files.into_iter().map(|(_, index)| Row::File { index, depth }));
    }

    /// Every folder row's path, as if all were open.
    fn folders(&self) -> Vec<String> {
        let mut rows = Vec::new();
        self.rows("", 0, &HashSet::new(), &mut rows);
        rows.into_iter()
            .filter_map(|r| match r {
                Row::Folder { path, .. } => Some(path),
                Row::File { .. } => None,
            })
            .collect()
    }
}

/// Whether `e` is under `folder`, or is a rename from or to it.
fn in_folder(e: &StatusEntry, folder: &str) -> bool {
    e.path.starts_with(folder) || e.orig_path.as_deref().is_some_and(|p| p.starts_with(folder))
}

impl StatusPane {
    pub fn new(repo_name: String) -> Self {
        Self {
            repo_name,
            ..Self::default()
        }
    }

    pub fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }

    pub fn set_status(&mut self, status: Status) {
        self.status = Some(status);
        self.reshow();
    }

    pub fn folder(&self) -> Option<&str> {
        self.folder.as_deref()
    }

    /// Narrows the list to `folder`, or shows every file again.
    pub fn set_folder(&mut self, folder: Option<String>) {
        if self.folder != folder {
            self.folder = folder;
            self.cursor = Cursor::default();
            self.refilter();
            self.layout(None);
        }
    }

    /// Whether the files are listed as a folder tree.
    pub fn tree(&self) -> bool {
        self.tree
    }

    /// Lists the files as a folder tree, or by full path.
    pub fn set_tree(&mut self, tree: bool) {
        if self.tree != tree {
            self.tree = tree;
            self.rebuild();
        }
    }

    fn reshow(&mut self) {
        let key = self.selected_key();
        self.refilter();
        self.layout(key);
    }

    /// Picks the entries to show from the status.
    fn refilter(&mut self) {
        let all = self.all_entries();
        self.shown = match &self.folder {
            Some(f) => all
                .iter()
                .filter(|e| e.is_conflicted() || in_folder(e, f))
                .cloned()
                .collect(),
            None => all.to_vec(),
        };
    }

    /// Lays the rows out again, keeping the selection on the same file or
    /// folder.
    fn rebuild(&mut self) {
        let key = self.selected_key();
        self.layout(key);
    }

    /// Builds the rows from `shown` and selects `key` in them.
    fn layout(&mut self, key: Option<RowKey>) {
        self.rows = if self.tree {
            let mut rows = Vec::new();
            Dir::of(&self.shown).rows("", 0, &self.collapsed, &mut rows);
            rows
        } else {
            (0..self.shown.len())
                .map(|index| Row::File { index, depth: 0 })
                .collect()
        };
        self.reselect(key);
    }

    fn key(&self, row: &Row) -> RowKey {
        match row {
            Row::File { index, .. } => RowKey::File(self.shown[*index].path.clone()),
            Row::Folder { path, .. } => RowKey::Folder(path.clone()),
        }
    }

    fn selected_key(&self) -> Option<RowKey> {
        self.rows.get(self.cursor.selected).map(|r| self.key(r))
    }

    /// Selects the row `key` names, or the collapsed folder it's in, or
    /// failing both, whatever row is at the old position.
    fn reselect(&mut self, key: Option<RowKey>) {
        if let Some(key) = key {
            let path = match &key {
                RowKey::File(p) | RowKey::Folder(p) => p.as_str(),
            };
            let found = self.rows.iter().position(|r| self.key(r) == key).or_else(|| {
                self.rows.iter().rposition(|r| match r {
                    Row::Folder { path: f, .. } => self.collapsed.contains(f) && path.starts_with(f.as_str()),
                    Row::File { .. } => false,
                })
            });
            if let Some(i) = found {
                self.cursor.selected = i;
            }
        }
        self.cursor.clamp(self.rows.len());
    }

    /// Collapses or expands the folder `path`.
    fn toggle_collapsed(&mut self, path: &str) {
        if !self.collapsed.remove(path) {
            self.collapsed.insert(path.to_string());
        }
        self.rebuild();
    }

    /// Collapses every folder in the tree, or expands them all.
    fn collapse_all(&mut self, collapse: bool) {
        self.collapsed.clear();
        if collapse {
            self.collapsed.extend(Dir::of(&self.shown).folders());
        }
        self.rebuild();
    }

    /// The entries listed: every one, or those in the folder.
    pub fn entries(&self) -> &[StatusEntry] {
        &self.shown
    }

    /// Every changed file, including any hidden outside the folder.
    pub fn all_entries(&self) -> &[StatusEntry] {
        self.status.as_ref().map_or(&[], |s| s.entries.as_slice())
    }

    /// The front end shows `rows` lines of files (under the banner of a
    /// merge or rebase in progress): keeps the selection among them, and
    /// pages by them. Call before drawing.
    pub fn set_viewport(&mut self, rows: usize) {
        self.cursor.scroll_into_view(rows);
    }

    /// The text of each row as drawn (without colors), for tests.
    pub fn row_texts(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|r| {
                self.row_line(r, usize::MAX, &Theme::default())
                    .spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect()
            })
            .collect()
    }

    /// How many changed files the folder hides, and how many of those
    /// have staged changes (which a commit takes too).
    fn hidden(&self) -> (usize, usize) {
        if self.folder.is_none() {
            return (0, 0);
        }
        let n = self.all_entries().len() - self.shown.len();
        let staged = self.all_entries().iter().filter(|e| e.has_staged()).count()
            - self.shown.iter().filter(|e| e.has_staged()).count();
        (n, staged)
    }

    /// The selected file; `None` when nothing is, or a folder is.
    pub fn selected(&self) -> Option<&StatusEntry> {
        match self.rows.get(self.cursor.selected)? {
            Row::File { index, .. } => self.shown.get(*index),
            Row::Folder { .. } => None,
        }
    }

    /// The selected folder in the tree (ending in `/`).
    pub fn selected_folder(&self) -> Option<&str> {
        match self.rows.get(self.cursor.selected)? {
            Row::Folder { path, .. } => Some(path),
            Row::File { .. } => None,
        }
    }

    /// The shown entries under `folder`.
    pub fn entries_in(&self, folder: &str) -> Vec<StatusEntry> {
        self.shown.iter().filter(|e| in_folder(e, folder)).cloned().collect()
    }

    /// Moves to the file above (`up`) or below the selected row, past any
    /// folders. Returns whether there was one.
    pub fn step_file(&mut self, up: bool) -> bool {
        let at = self.cursor.selected.min(self.rows.len());
        let is_file = |r: &Row| matches!(r, Row::File { .. });
        let found = if up {
            self.rows[..at].iter().rposition(is_file)
        } else {
            self.rows.iter().skip(at + 1).position(is_file).map(|i| at + 1 + i)
        };
        if let Some(i) = found {
            self.cursor.selected = i;
        }
        found.is_some()
    }

    /// Tells the app which file or folder is selected so the details pane
    /// can show its diff.
    pub fn announce(&self, ctx: &Ctx) {
        if let Some(e) = self.selected() {
            ctx.emit(Action::FileSelected(e.clone()));
        } else if let Some(folder) = self.selected_folder() {
            ctx.emit(Action::FolderSelected {
                folder: folder.to_string(),
                entries: self.entries_in(folder),
            });
        }
    }

    /// Asks where to ignore the selected file, or the folder it's in.
    fn ignore_file_picker(e: &StatusEntry) -> Modal {
        let mut paths = vec![(e.path.clone(), false)];
        if let Some((dir, _)) = e.path.rsplit_once('/') {
            paths.push((format!("{dir}/"), true));
        }
        Self::ignore_picker(&e.path, paths)
    }

    /// Asks where to ignore each of `paths` (with whether it's a folder),
    /// titled after `what`.
    fn ignore_picker(what: &str, paths: Vec<(String, bool)>) -> Modal {
        let mut items = Vec::new();
        for (path, dir) in paths {
            let pattern = ignore_pattern(&path, dir);
            for (target, hint) in [
                (IgnoreTarget::Gitignore, "committed"),
                (IgnoreTarget::Exclude, "this clone only"),
            ] {
                items.push(PaletteItem {
                    label: format!("Add {pattern} to {}", target.label()),
                    hint: hint.into(),
                    target: PaletteTarget::Op(GitOp::Ignore {
                        path: path.clone(),
                        target,
                        untrack: false,
                    }),
                });
            }
        }
        Modal::picker(format!("Ignore {what}"), items, 0)
    }

    /// `space`, `d` or `i` on a folder in the tree: acts on everything
    /// under it.
    fn on_folder(&self, cmd: Command, folder: String, ctx: &Ctx) {
        let entries = self.entries_in(&folder);
        match cmd {
            Command::StageToggle => {
                if entries.iter().any(StatusEntry::has_unstaged) {
                    ctx.emit(Action::Run(GitOp::StageAll { folder: Some(folder) }));
                } else if entries.iter().any(StatusEntry::has_staged) {
                    ctx.emit(Action::Run(GitOp::UnstageAll { folder: Some(folder) }));
                }
            }
            Command::Discard => {
                if let Some(op) = self.status.as_ref().and_then(|s| s.in_progress) {
                    ctx.emit(Action::Notify(format!(
                        "a {} is in progress: X aborts it, d on a file discards that file",
                        op.command()
                    )));
                } else {
                    ctx.emit(Action::OpenModal(Modal::discard_all(
                        DiscardCounts::of(&entries),
                        Some(folder),
                    )));
                }
            }
            Command::Ignore => {
                let picker = Self::ignore_picker(&folder, vec![(folder.clone(), true)]);
                ctx.emit(Action::OpenModal(picker));
            }
            _ => {}
        }
    }

    /// Whether anything is staged, in or out of the folder: a commit takes
    /// the whole index.
    fn has_staged(&self) -> bool {
        self.all_entries().iter().any(StatusEntry::has_staged)
    }

    /// `repo · branch → upstream ↑a ↓b`
    pub fn summary(&self) -> String {
        let mut s = self.repo_name.clone();
        let Some(st) = &self.status else {
            return s;
        };
        let b = &st.branch;
        match &b.head {
            Some(head) => s.push_str(&format!(" · {head}")),
            None => s.push_str(" · (detached)"),
        }
        if let Some(op) = st.in_progress {
            s.push_str(&format!(" · {}", op.label()));
        }
        if let Some(up) = &b.upstream {
            s.push_str(&format!(" → {up}"));
            if b.ahead > 0 {
                s.push_str(&format!(" ↑{}", b.ahead));
            }
            if b.behind > 0 {
                s.push_str(&format!(" ↓{}", b.behind));
            }
        }
        if let Some(folder) = &self.folder {
            s.push_str(&format!(" · in {folder}"));
            match self.hidden() {
                (0, _) => {}
                (n, 0) => s.push_str(&format!(" · +{n} outside")),
                (n, staged) => s.push_str(&format!(" · +{n} outside ({staged} staged)")),
            }
        }
        s
    }

    /// One row as drawn: a file's status code and path (its name alone in
    /// the tree), or a folder's open/closed marker and name.
    fn row_line(&self, row: &Row, width: usize, theme: &Theme) -> Line<'static> {
        match row {
            Row::File { index, depth } => {
                let e = &self.shown[*index];
                let indent = "  ".repeat(*depth);
                let name = if self.tree { file_name(e) } else { e.path.clone() };
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
                let marker = if self.collapsed.contains(path) { "▸ " } else { "▾ " };
                let style = code_style(self.shown.iter().filter(|e| in_folder(e, path)), theme);
                let room = width.saturating_sub(indent.len() + 2);
                Line::from(vec![
                    Span::raw(indent),
                    Span::styled(marker, style),
                    Span::styled(truncate(&format!("{name}/"), room), style),
                ])
            }
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

/// The pane's commands, the tree's own last.
const TREE_COMMANDS: &[Command] = &[
    Command::StageToggle,
    Command::StageAll,
    Command::Discard,
    Command::DiscardAll,
    Command::Ignore,
    Command::Commit,
    Command::Amend,
    Command::Stash,
    Command::OpenInEditor,
    Command::Blame,
    Command::ToggleTree,
    Command::FilterFolder,
    Command::Continue,
    Command::Abort,
    Command::CollapseAll,
    Command::ExpandAll,
];

impl Component for StatusPane {
    fn context(&self) -> Context {
        Context::Status
    }

    fn on_command(&mut self, cmd: Command, ctx: &Ctx) -> EventState {
        if self.cursor.apply(cmd, self.rows.len()) {
            self.announce(ctx);
            return EventState::Consumed;
        }
        if let Some(folder) = self.selected_folder().map(str::to_string) {
            match cmd {
                Command::OpenStaging => {
                    self.toggle_collapsed(&folder);
                    return EventState::Consumed;
                }
                Command::StageToggle | Command::Discard | Command::Ignore => {
                    self.on_folder(cmd, folder, ctx);
                    return EventState::Consumed;
                }
                _ => {}
            }
        }
        match cmd {
            Command::CollapseAll | Command::ExpandAll if self.tree => {
                self.collapse_all(cmd == Command::CollapseAll);
                self.announce(ctx);
            }
            Command::CollapseAll | Command::ExpandAll => {
                ctx.emit(Action::Notify("folders fold in the tree view (` shows it)".into()));
            }
            Command::Continue | Command::Abort => {
                let Some(op) = self.status.as_ref().and_then(|s| s.in_progress) else {
                    ctx.emit(Action::Notify("no merge, rebase or cherry-pick in progress".into()));
                    return EventState::Consumed;
                };
                if cmd == Command::Continue {
                    if self.all_entries().iter().any(StatusEntry::is_conflicted) {
                        ctx.emit(Action::Notify("resolve and stage the conflicted files first".into()));
                    } else {
                        ctx.emit(Action::Run(GitOp::Continue(op)));
                    }
                } else {
                    ctx.emit(Action::OpenModal(Modal::confirm(
                        "Abort",
                        format!(
                            "Abort the {} in progress and go back to before it started?",
                            op.command()
                        ),
                        GitOp::Abort(op),
                    )));
                }
            }
            Command::StageAll => {
                let folder = self.folder.clone();
                if self.entries().iter().any(StatusEntry::has_unstaged) {
                    ctx.emit(Action::Run(GitOp::StageAll { folder }));
                } else if self.entries().iter().any(StatusEntry::has_staged) {
                    ctx.emit(Action::Run(GitOp::UnstageAll { folder }));
                }
            }
            Command::Commit => {
                if self.has_staged() {
                    ctx.emit(Action::OpenModal(Modal::prompt(
                        "Commit message",
                        "",
                        PromptAction::Commit,
                    )));
                } else {
                    ctx.emit(Action::Notify(
                        "nothing staged to commit (space stages a file, a stages all)".into(),
                    ));
                }
            }
            Command::Stash => {
                if self.entries().is_empty() {
                    ctx.emit(Action::Notify(match &self.folder {
                        Some(f) => format!("nothing to stash in {f}"),
                        None => "nothing to stash".into(),
                    }));
                } else {
                    let file = self.selected().map(|e| {
                        let mut paths = vec![e.path.clone()];
                        paths.extend(e.orig_path.clone());
                        paths
                    });
                    ctx.emit(Action::OpenModal(Modal::prompt(
                        "Stash",
                        "",
                        PromptAction::Stash {
                            scope: match &self.folder {
                                Some(f) => StashScope::Paths(vec![f.clone()]),
                                None => StashScope::All,
                            },
                            file,
                            folder: self.folder.clone(),
                        },
                    )));
                }
            }
            Command::Amend => ctx.emit(Action::OpenModal(Modal::confirm(
                "Amend",
                "Amend the last commit with the staged changes, keeping its message?",
                GitOp::Commit {
                    message: None,
                    amend: true,
                },
            ))),
            Command::DiscardAll => {
                if self.entries().is_empty() {
                    ctx.emit(Action::Notify(match &self.folder {
                        Some(f) => format!("nothing to discard in {f}"),
                        None => "nothing to discard".into(),
                    }));
                } else if let Some(op) = self.status.as_ref().and_then(|s| s.in_progress) {
                    ctx.emit(Action::Notify(format!(
                        "a {} is in progress: X aborts it, d discards one file",
                        op.command()
                    )));
                } else {
                    ctx.emit(Action::OpenModal(Modal::discard_all(
                        DiscardCounts::of(self.entries()),
                        self.folder.clone(),
                    )));
                }
            }
            Command::Ignore => {
                let Some(e) = self.selected() else {
                    return EventState::NotConsumed;
                };
                ctx.emit(Action::OpenModal(Self::ignore_file_picker(e)));
            }
            Command::StageToggle | Command::Discard => {
                let Some(e) = self.selected() else {
                    return EventState::NotConsumed;
                };
                let mut paths = vec![e.path.clone()];
                if let Some(orig) = &e.orig_path {
                    paths.push(orig.clone());
                }
                let op = if cmd == Command::Discard {
                    ctx.emit(Action::OpenModal(Modal::confirm(
                        "Discard changes",
                        format!("Discard all changes to {}? This cannot be undone.", e.path),
                        GitOp::Discard {
                            path: e.path.clone(),
                            kind: DiscardKind::of(e),
                        },
                    )));
                    return EventState::Consumed;
                } else if e.has_unstaged() {
                    GitOp::Stage(vec![e.path.clone()])
                } else {
                    GitOp::Unstage(paths)
                };
                ctx.emit(Action::Run(op));
            }
            _ => return EventState::NotConsumed,
        }
        EventState::Consumed
    }

    fn on_click(&mut self, row: usize, ctx: &Ctx) {
        // The conflict banner takes the first row while an operation is in progress.
        let banner = usize::from(self.status.as_ref().is_some_and(|s| s.in_progress.is_some()));
        if row >= banner && self.cursor.click(row - banner, self.rows.len()) {
            self.announce(ctx);
        }
    }

    fn commands(&self) -> &[Command] {
        // Folding every folder only means something in the tree.
        if self.tree {
            TREE_COMMANDS
        } else {
            &TREE_COMMANDS[..TREE_COMMANDS.len() - 2]
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
        let block = pane_block(PaneId::Status, format!("Status: {}", self.summary()), focused, theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if self.entries().is_empty() {
            let msg = match (&self.status, &self.folder) {
                (None, _) => "loading…".to_string(),
                (Some(_), Some(f)) if !self.all_entries().is_empty() => format!("no changes in {f} (. shows all)"),
                (Some(_), _) => "working tree clean".to_string(),
            };
            frame.render_widget(Paragraph::new(msg).style(theme.muted), inner);
            return;
        }
        let mut inner = inner;
        if let Some(op) = self.status.as_ref().and_then(|s| s.in_progress) {
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
        let start = self.cursor.offset.min(self.rows.len());
        let end = (start + height).min(self.rows.len());
        let lines: Vec<Line> = (start..end)
            .map(|i| {
                let line = self.row_line(&self.rows[i], width, theme);
                if i == self.cursor.selected {
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
