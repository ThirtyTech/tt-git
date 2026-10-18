use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tt_git_core::branches::matches_filter;
use tt_git_core::time::{SharedClock, relative};
use tt_git_core::{BranchEntry, BranchState, RefKind};

use super::{PaneId, pane_block};
use crate::action::Action;
use crate::component::{Component, EventState};
use crate::jobs::Ctx;
use crate::keymap::{Command, Context, KeyCombo};
use crate::modal::{GitOp, Modal, PromptAction};
use crate::theme::Theme;
use crate::widgets::{Cursor, fit, fit_right};

const STATE_WIDTH: usize = 9;
const DATE_WIDTH: usize = 4;

/// Unified local + remote branch list sorted by recent activity.
#[derive(Debug)]
pub struct BranchesPane {
    clock: SharedClock,
    entries: Vec<BranchEntry>,
    /// Indices into `entries` that pass the filter.
    visible: Vec<usize>,
    filter: String,
    editing_filter: bool,
    cursor: Cursor,
}

impl BranchesPane {
    pub fn new(clock: SharedClock) -> Self {
        Self {
            clock,
            entries: Vec::new(),
            visible: Vec::new(),
            filter: String::new(),
            editing_filter: false,
            cursor: Cursor::default(),
        }
    }

    /// Replaces the list, keeping the selection on the same branch if it still exists.
    pub fn set_entries(&mut self, entries: Vec<BranchEntry>) {
        let keep = self.selected().map(|e| e.name.clone());
        self.entries = entries;
        self.refilter();
        if let Some(name) = keep
            && let Some(pos) = self.visible.iter().position(|&i| self.entries[i].name == name)
        {
            self.cursor.selected = pos;
        }
    }

    pub fn entries(&self) -> &[BranchEntry] {
        &self.entries
    }

    /// Entries passing the filter, in display order.
    pub fn visible(&self) -> impl Iterator<Item = &BranchEntry> {
        self.visible.iter().map(|&i| &self.entries[i])
    }

    pub fn selected(&self) -> Option<&BranchEntry> {
        self.visible.get(self.cursor.selected).map(|&i| &self.entries[i])
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    fn refilter(&mut self) {
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| matches_filter(&e.name, &self.filter))
            .map(|(i, _)| i)
            .collect();
        self.cursor.clamp(self.visible.len());
    }

    fn announce(&self, ctx: &Ctx) {
        if let Some(oid) = self.selected().and_then(BranchEntry::target) {
            ctx.emit(Action::BranchSelected(oid.to_string()));
        }
    }

    fn start_ref(entry: &BranchEntry) -> String {
        entry
            .local
            .as_ref()
            .or(entry.remotes.first())
            .map(|r| r.short_name.clone())
            .unwrap_or_else(|| entry.name.clone())
    }

    fn delete_modal(entry: &BranchEntry) -> Option<Modal> {
        if let Some(local) = &entry.local {
            if entry.is_head {
                return None;
            }
            let unmerged_note = "\n(If it is not fully merged you will be asked to force-delete.)";
            return Some(Modal::confirm(
                "Delete branch",
                format!("Delete local branch {}?{unmerged_note}", local.short_name),
                GitOp::DeleteBranch {
                    name: local.short_name.clone(),
                    force: false,
                },
            ));
        }
        let remote_ref = entry.remotes.first()?;
        let RefKind::RemoteBranch { remote } = &remote_ref.kind else {
            return None;
        };
        Some(Modal::confirm(
            "Delete remote branch",
            format!(
                "Delete {} on remote {remote}?\nThis runs `git push {remote} --delete {}`.",
                remote_ref.short_name,
                remote_ref.branch_name()
            ),
            GitOp::DeleteRemoteBranch {
                remote: remote.clone(),
                branch: remote_ref.branch_name().to_string(),
            },
        ))
    }

    fn render_entry<'a>(&self, entry: &'a BranchEntry, width: usize, theme: &Theme) -> Line<'a> {
        let marker = if entry.is_head {
            Span::styled("* ", theme.label_local)
        } else {
            Span::raw("  ")
        };
        let name_style = if entry.is_head {
            theme.label_local
        } else if entry.local.is_none() {
            theme.label_remote
        } else {
            Style::new()
        };
        let (state, state_style) = state_text(&entry.state(), theme);
        let name_width = width.saturating_sub(2 + 1 + STATE_WIDTH + 1 + DATE_WIDTH);
        Line::from(vec![
            marker,
            Span::styled(fit(&entry.name, name_width), name_style),
            Span::raw(" "),
            Span::styled(fit(&state, STATE_WIDTH), state_style),
            Span::raw(" "),
            Span::styled(
                fit_right(&relative(self.clock.now(), entry.activity), DATE_WIDTH),
                theme.date,
            ),
        ])
    }
}

/// Short state badge and its style.
fn state_text(state: &BranchState, theme: &Theme) -> (String, Style) {
    match state {
        BranchState::LocalOnly => ("local".into(), theme.muted),
        BranchState::RemoteOnly { remotes } => (remotes.join(","), theme.label_remote),
        BranchState::InSync => ("✓".into(), theme.diff_add),
        BranchState::Diverged { ahead, behind } => {
            let mut s = String::new();
            if *ahead > 0 {
                s.push_str(&format!("↑{ahead}"));
            }
            if *behind > 0 {
                s.push_str(&format!("↓{behind}"));
            }
            (s, theme.label_tag)
        }
        BranchState::Gone => ("gone".into(), theme.error),
        BranchState::Untracked { remotes, same_commit } => {
            let sign = if *same_commit { "=" } else { "≠" };
            (format!("{sign}{}", remotes.join(",")), theme.muted)
        }
    }
}

impl Component for BranchesPane {
    fn context(&self) -> Context {
        Context::Branches
    }

    fn on_command(&mut self, cmd: Command, ctx: &Ctx) -> EventState {
        if self.cursor.apply(cmd, self.visible.len()) {
            self.announce(ctx);
            return EventState::Consumed;
        }
        if cmd == Command::Filter {
            self.editing_filter = true;
            return EventState::Consumed;
        }
        if cmd == Command::Fetch {
            ctx.emit(Action::Run(GitOp::FetchAll));
            return EventState::Consumed;
        }
        let Some(entry) = self.selected() else {
            return EventState::NotConsumed;
        };
        match cmd {
            Command::Checkout => {
                if !entry.is_head
                    && let Some(plan) = entry.checkout_plan()
                {
                    ctx.emit(Action::Run(GitOp::Checkout(plan)));
                }
            }
            Command::NewBranch => ctx.emit(Action::OpenModal(Modal::prompt(
                format!("New branch from {}", Self::start_ref(entry)),
                "",
                PromptAction::CreateBranch {
                    start: Self::start_ref(entry),
                },
            ))),
            Command::Merge | Command::Rebase => {
                if entry.is_head {
                    ctx.emit(Action::Notify("that's the checked-out branch".into()));
                } else {
                    let target = Self::start_ref(entry);
                    let modal = if cmd == Command::Merge {
                        Modal::confirm(
                            "Merge",
                            format!("Merge {target} into the current branch?"),
                            GitOp::Merge { target },
                        )
                    } else {
                        Modal::confirm(
                            "Rebase",
                            format!("Rebase the current branch onto {target}?"),
                            GitOp::Rebase { onto: target },
                        )
                    };
                    ctx.emit(Action::OpenModal(modal));
                }
            }
            Command::DeleteBranch => match Self::delete_modal(entry) {
                Some(modal) => ctx.emit(Action::OpenModal(modal)),
                None => ctx.emit(Action::Notify("can't delete the checked-out branch".into())),
            },
            Command::RenameBranch => match &entry.local {
                Some(local) => ctx.emit(Action::OpenModal(Modal::prompt(
                    format!("Rename {}", local.short_name),
                    local.short_name.clone(),
                    PromptAction::RenameBranch {
                        old: local.short_name.clone(),
                    },
                ))),
                None => ctx.emit(Action::Notify("only local branches can be renamed".into())),
            },
            _ => return EventState::NotConsumed,
        }
        EventState::Consumed
    }

    fn commands(&self) -> &[Command] {
        &[
            Command::Checkout,
            Command::NewBranch,
            Command::DeleteBranch,
            Command::RenameBranch,
            Command::Fetch,
            Command::Filter,
            Command::Merge,
            Command::Rebase,
        ]
    }

    fn on_click(&mut self, row: usize, ctx: &Ctx) {
        if self.cursor.click(row, self.visible.len()) {
            self.announce(ctx);
        }
    }

    fn wants_text_input(&self) -> bool {
        self.editing_filter
    }

    fn on_text_key(&mut self, key: KeyCombo, ctx: &Ctx) {
        match key.code {
            KeyCode::Enter => self.editing_filter = false,
            KeyCode::Esc => {
                self.editing_filter = false;
                self.filter.clear();
                self.refilter();
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.refilter();
            }
            KeyCode::Up | KeyCode::Down => {
                let cmd = if key.code == KeyCode::Up {
                    Command::MoveUp
                } else {
                    Command::MoveDown
                };
                self.cursor.apply(cmd, self.visible.len());
                self.announce(ctx);
            }
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.cursor.selected = 0;
                self.refilter();
            }
            _ => {}
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
        let mut title = format!("Branches {}", self.entries.len());
        if self.editing_filter || !self.filter.is_empty() {
            title = format!(
                "Branches /{}{}",
                self.filter,
                if self.editing_filter { "█" } else { "" }
            );
        }
        let block = pane_block(PaneId::Branches, title, focused, theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if self.visible.is_empty() {
            let msg = if self.entries.is_empty() {
                "no branches"
            } else {
                "no matches"
            };
            frame.render_widget(Paragraph::new(msg).style(theme.muted), inner);
            return;
        }
        let height = inner.height as usize;
        self.cursor.scroll_into_view(height);
        let start = self.cursor.offset.min(self.visible.len());
        let end = (start + height).min(self.visible.len());
        let width = inner.width as usize;
        let lines: Vec<Line> = (start..end)
            .map(|pos| {
                let line = self.render_entry(&self.entries[self.visible[pos]], width, theme);
                if pos == self.cursor.selected {
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
