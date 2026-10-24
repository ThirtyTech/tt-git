use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tt_git_core::{DiscardKind, IgnoreTarget, StashScope, Status, StatusEntry, ignore_pattern};

use super::{PaneId, pane_block};
use crate::action::Action;
use crate::component::{Component, EventState};
use crate::jobs::Ctx;
use crate::keymap::{Command, Context};
use crate::modal::{DiscardCounts, GitOp, Modal, PaletteItem, PaletteTarget, PromptAction};
use crate::theme::Theme;
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
    cursor: Cursor,
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
            self.reshow();
        }
    }

    fn reshow(&mut self) {
        let all = self.all_entries();
        self.shown = match &self.folder {
            Some(f) => all
                .iter()
                .filter(|e| e.is_conflicted() || in_folder(e, f))
                .cloned()
                .collect(),
            None => all.to_vec(),
        };
        let len = self.shown.len();
        self.cursor.clamp(len);
    }

    /// The entries listed: every one, or those in the folder.
    pub fn entries(&self) -> &[StatusEntry] {
        &self.shown
    }

    /// Every changed file, including any hidden outside the folder.
    pub fn all_entries(&self) -> &[StatusEntry] {
        self.status.as_ref().map_or(&[], |s| s.entries.as_slice())
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

    pub fn selected(&self) -> Option<&StatusEntry> {
        self.entries().get(self.cursor.selected)
    }

    /// Tells the app which file is selected so the details pane can show its diff.
    pub fn announce(&self, ctx: &Ctx) {
        if let Some(e) = self.selected() {
            ctx.emit(Action::FileSelected(e.clone()));
        }
    }

    /// Asks where to ignore the selected file, or the folder it's in.
    fn ignore_picker(e: &StatusEntry) -> Modal {
        let mut paths = vec![(e.path.clone(), false)];
        if let Some((dir, _)) = e.path.rsplit_once('/') {
            paths.push((format!("{dir}/"), true));
        }
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
        Modal::picker(format!("Ignore {}", e.path), items, 0)
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
}

impl Component for StatusPane {
    fn context(&self) -> Context {
        Context::Status
    }

    fn on_command(&mut self, cmd: Command, ctx: &Ctx) -> EventState {
        if self.cursor.apply(cmd, self.entries().len()) {
            self.announce(ctx);
            return EventState::Consumed;
        }
        match cmd {
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
                ctx.emit(Action::OpenModal(Self::ignore_picker(e)));
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
        if row >= banner && self.cursor.click(row - banner, self.entries().len()) {
            self.announce(ctx);
        }
    }

    fn commands(&self) -> &[Command] {
        &[
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
            Command::FilterFolder,
            Command::Continue,
            Command::Abort,
        ]
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
        self.cursor.scroll_into_view(height);
        let width = inner.width as usize;
        let entries = self.entries();
        let start = self.cursor.offset.min(entries.len());
        let end = (start + height).min(entries.len());
        let lines: Vec<Line> = (start..end)
            .map(|i| {
                let e = &entries[i];
                let code_style = if e.is_conflicted() {
                    theme.error
                } else if e.has_staged() && !e.has_unstaged() {
                    theme.diff_add
                } else {
                    theme.diff_del
                };
                let path = match &e.orig_path {
                    Some(orig) => format!("{orig} → {}", e.path),
                    None => e.path.clone(),
                };
                let line = Line::from(vec![
                    Span::styled(e.short_code(), code_style),
                    Span::raw(" "),
                    Span::raw(truncate(&path, width.saturating_sub(3))),
                ]);
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
