use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tt_git_core::{DiscardKind, StashScope, Status, StatusEntry};

use super::{PaneId, pane_block};
use crate::action::Action;
use crate::component::{Component, EventState};
use crate::jobs::Ctx;
use crate::keymap::{Command, Context};
use crate::modal::{GitOp, Modal, PromptAction};
use crate::theme::Theme;
use crate::widgets::{Cursor, truncate};

/// Working tree status with a repo/branch summary in the title.
#[derive(Debug, Default)]
pub struct StatusPane {
    pub repo_name: String,
    status: Option<Status>,
    cursor: Cursor,
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
        let len = self.entries().len();
        self.cursor.clamp(len);
    }

    pub fn entries(&self) -> &[StatusEntry] {
        self.status.as_ref().map_or(&[], |s| s.entries.as_slice())
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

    fn has_staged(&self) -> bool {
        self.entries().iter().any(StatusEntry::has_staged)
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
                    if self.entries().iter().any(StatusEntry::is_conflicted) {
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
                let any_unstaged = self.entries().iter().any(StatusEntry::has_unstaged);
                if any_unstaged {
                    ctx.emit(Action::Run(GitOp::StageAll));
                } else if self.has_staged() {
                    ctx.emit(Action::Run(GitOp::UnstageAll));
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
                    ctx.emit(Action::Notify("nothing to stash".into()));
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
                            scope: StashScope::All,
                            file,
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
            Command::Commit,
            Command::Amend,
            Command::Stash,
            Command::OpenInEditor,
            Command::Blame,
            Command::Continue,
            Command::Abort,
        ]
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
        let block = pane_block(PaneId::Status, format!("Status: {}", self.summary()), focused, theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if self.entries().is_empty() {
            let msg = if self.status.is_some() {
                "working tree clean"
            } else {
                "loading…"
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
