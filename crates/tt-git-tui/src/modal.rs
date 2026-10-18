//! Modal popups: a one-line text prompt and a yes/no confirmation.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use tt_git_core::{CheckoutPlan, DiscardKind, InProgress};

use crate::keymap::{Command, KeyCombo};
use crate::theme::Theme;

/// A git write operation, run as a background job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitOp {
    Checkout(CheckoutPlan),
    CreateBranch {
        name: String,
        start: String,
    },
    DeleteBranch {
        name: String,
        force: bool,
    },
    DeleteRemoteBranch {
        remote: String,
        branch: String,
    },
    RenameBranch {
        old: String,
        new: String,
    },
    FetchAll,
    /// Save the base branch for the merged/unmerged lists (`None`: detect it).
    SetBaseBranch(Option<String>),
    Stage(Vec<String>),
    Unstage(Vec<String>),
    StageAll,
    UnstageAll,
    Discard {
        path: String,
        kind: DiscardKind,
    },
    /// `message: None` with `amend` keeps the previous message.
    Commit {
        message: Option<String>,
        amend: bool,
    },
    Push,
    Pull,
    Merge {
        target: String,
    },
    Rebase {
        onto: String,
    },
    CherryPick {
        oid: String,
        summary: String,
    },
    Continue(InProgress),
    Abort(InProgress),
}

impl GitOp {
    /// Progress text shown while running.
    pub fn label(&self) -> String {
        match self {
            GitOp::Checkout(CheckoutPlan::Local(name)) => format!("checkout {name}"),
            GitOp::Checkout(CheckoutPlan::Track { name, remote_ref }) => {
                format!("checkout {name} (tracking {remote_ref})")
            }
            GitOp::CreateBranch { name, .. } => format!("create branch {name}"),
            GitOp::DeleteBranch { name, .. } => format!("delete branch {name}"),
            GitOp::DeleteRemoteBranch { remote, branch } => format!("delete {remote}/{branch}"),
            GitOp::RenameBranch { old, new } => format!("rename {old} → {new}"),
            GitOp::FetchAll => "fetch --all".into(),
            GitOp::SetBaseBranch(Some(base)) => format!("base branch {base}"),
            GitOp::SetBaseBranch(None) => "base branch auto".into(),
            GitOp::Stage(paths) => format!("stage {}", paths.join(" ")),
            GitOp::Unstage(paths) => format!("unstage {}", paths.join(" ")),
            GitOp::StageAll => "stage all".into(),
            GitOp::UnstageAll => "unstage all".into(),
            GitOp::Discard { path, .. } => format!("discard {path}"),
            GitOp::Commit { amend: false, .. } => "commit".into(),
            GitOp::Commit { amend: true, .. } => "amend".into(),
            GitOp::Push => "push".into(),
            GitOp::Pull => "pull".into(),
            GitOp::Merge { target } => format!("merge {target}"),
            GitOp::Rebase { onto } => format!("rebase onto {onto}"),
            GitOp::CherryPick { oid, .. } => format!("cherry-pick {}", &oid[..oid.len().min(7)]),
            GitOp::Continue(op) => format!("{} --continue", op.command()),
            GitOp::Abort(op) => format!("{} --abort", op.command()),
        }
    }
}

/// What a submitted prompt turns into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptAction {
    CreateBranch { start: String },
    RenameBranch { old: String },
    Commit,
}

impl PromptAction {
    fn op(&self, input: &str) -> GitOp {
        match self {
            PromptAction::CreateBranch { start } => GitOp::CreateBranch {
                name: input.to_string(),
                start: start.clone(),
            },
            PromptAction::RenameBranch { old } => GitOp::RenameBranch {
                old: old.clone(),
                new: input.to_string(),
            },
            PromptAction::Commit => GitOp::Commit {
                message: Some(input.to_string()),
                amend: false,
            },
        }
    }
}

/// What a palette entry runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteTarget {
    Command(Command),
    PluginAction(String),
    /// (plugin id, pane entry point)
    PluginPane(String, String),
    Op(GitOp),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    pub label: String,
    /// Shown dimmed on the right: a key binding or the plugin name.
    pub hint: String,
    pub target: PaletteTarget,
}

const PALETTE_ROWS: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    /// Fuzzy-filtered list: the command palette (built-in commands and
    /// plugin actions) or a picker such as the base branch one.
    Palette {
        title: String,
        input: String,
        items: Vec<PaletteItem>,
        selected: usize,
    },
    Prompt {
        title: String,
        input: String,
        action: PromptAction,
    },
    Confirm {
        title: String,
        message: String,
        op: GitOp,
    },
}

/// Result of feeding a key to a modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModalOutcome {
    /// Still open.
    Open,
    Cancelled,
    Run(GitOp),
    Command(Command),
    PluginAction(String),
    PluginPane(String, String),
}

impl Modal {
    pub fn prompt(title: impl Into<String>, initial: impl Into<String>, action: PromptAction) -> Self {
        Modal::Prompt {
            title: title.into(),
            input: initial.into(),
            action,
        }
    }

    pub fn confirm(title: impl Into<String>, message: impl Into<String>, op: GitOp) -> Self {
        Modal::Confirm {
            title: title.into(),
            message: message.into(),
            op,
        }
    }

    pub fn palette(items: Vec<PaletteItem>) -> Self {
        Self::picker("Commands", items, 0)
    }

    /// A fuzzy-filtered list titled `title`, starting on item `selected`.
    pub fn picker(title: impl Into<String>, items: Vec<PaletteItem>, selected: usize) -> Self {
        Modal::Palette {
            title: title.into(),
            input: String::new(),
            selected: selected.min(items.len().saturating_sub(1)),
            items,
        }
    }

    /// Palette entries matching the typed filter, in order.
    pub fn palette_matches<'a>(items: &'a [PaletteItem], input: &str) -> Vec<&'a PaletteItem> {
        items
            .iter()
            .filter(|i| tt_git_core::branches::matches_filter(&i.label, input))
            .collect()
    }

    pub fn handle_key(&mut self, key: KeyCombo) -> ModalOutcome {
        match self {
            Modal::Palette {
                input, items, selected, ..
            } => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let count = Self::palette_matches(items, input).len();
                match key.code {
                    KeyCode::Esc => return ModalOutcome::Cancelled,
                    KeyCode::Enter => {
                        return match Self::palette_matches(items, input).get(*selected) {
                            Some(item) => match &item.target {
                                PaletteTarget::Command(c) => ModalOutcome::Command(*c),
                                PaletteTarget::PluginAction(a) => ModalOutcome::PluginAction(a.clone()),
                                PaletteTarget::PluginPane(p, e) => ModalOutcome::PluginPane(p.clone(), e.clone()),
                                PaletteTarget::Op(op) => ModalOutcome::Run(op.clone()),
                            },
                            None => ModalOutcome::Open,
                        };
                    }
                    KeyCode::Up => *selected = selected.saturating_sub(1),
                    KeyCode::Char('p') if ctrl => *selected = selected.saturating_sub(1),
                    KeyCode::Down => *selected = (*selected + 1).min(count.saturating_sub(1)),
                    KeyCode::Char('n') if ctrl => *selected = (*selected + 1).min(count.saturating_sub(1)),
                    KeyCode::Backspace => {
                        input.pop();
                        *selected = 0;
                    }
                    KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                        input.push(c);
                        *selected = 0;
                    }
                    _ => {}
                }
                ModalOutcome::Open
            }
            Modal::Prompt { input, action, .. } => match key.code {
                KeyCode::Esc => ModalOutcome::Cancelled,
                KeyCode::Enter => {
                    let value = input.trim();
                    if value.is_empty() {
                        ModalOutcome::Open
                    } else {
                        ModalOutcome::Run(action.op(value))
                    }
                }
                KeyCode::Backspace => {
                    input.pop();
                    ModalOutcome::Open
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.clear();
                    ModalOutcome::Open
                }
                KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                    input.push(c);
                    ModalOutcome::Open
                }
                _ => ModalOutcome::Open,
            },
            Modal::Confirm { op, .. } => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => ModalOutcome::Run(op.clone()),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => ModalOutcome::Cancelled,
                _ => ModalOutcome::Open,
            },
        }
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if let Modal::Palette {
            title,
            input,
            items,
            selected,
        } = self
        {
            return draw_palette(frame, area, theme, title, input, items, *selected);
        }
        let (title, body, hint): (&str, Vec<Line>, &str) = match self {
            Modal::Palette { .. } => unreachable!("handled above"),
            Modal::Prompt { title, input, .. } => (
                title,
                vec![Line::from(vec![
                    Span::raw(input.as_str()),
                    Span::styled("█", theme.help_key),
                ])],
                "enter confirm · esc cancel",
            ),
            Modal::Confirm { title, message, .. } => (
                title,
                message.lines().map(Line::from).collect(),
                "y confirm · n/esc cancel",
            ),
        };
        let width = 60.min(area.width);
        let inner_width = width.saturating_sub(2).max(1) as usize;
        let body_rows: usize = body.iter().map(|l| l.width().div_ceil(inner_width).max(1)).sum();
        let height = (body_rows as u16 + 4).min(area.height);
        let popup = crate::app::centered(area, width, height);
        frame.render_widget(Clear, popup);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(theme.border_focused)
            .title(Span::styled(format!(" {title} "), theme.title_focused));
        let mut lines = body;
        lines.push(Line::raw(""));
        lines.push(Line::styled(hint, theme.muted));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), popup);
    }
}

fn draw_palette(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    title: &str,
    input: &str,
    items: &[PaletteItem],
    selected: usize,
) {
    let matches = Modal::palette_matches(items, input);
    let rows = matches.len().clamp(1, PALETTE_ROWS);
    let width = 70.min(area.width);
    let height = (rows as u16 + 4).min(area.height);
    let popup = Rect {
        y: area.y + area.height.saturating_sub(height) / 4,
        ..crate::app::centered(area, width, height)
    };
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border_focused)
        .title(Span::styled(format!(" {title} "), theme.title_focused));
    let inner_width = popup.width.saturating_sub(2) as usize;
    let mut lines = vec![
        Line::from(vec![
            Span::styled("> ", theme.help_key),
            Span::raw(input.to_string()),
            Span::styled("█", theme.help_key),
        ]),
        Line::raw(""),
    ];
    let start = selected.saturating_sub(PALETTE_ROWS - 1);
    if matches.is_empty() {
        lines.push(Line::styled("no matches", theme.muted));
    }
    for (i, item) in matches.iter().enumerate().skip(start).take(PALETTE_ROWS) {
        let hint_w = item.hint.chars().count();
        let label_w = inner_width.saturating_sub(hint_w + 1);
        let line = Line::from(vec![
            Span::raw(crate::widgets::fit(&item.label, label_w)),
            Span::raw(" "),
            Span::styled(item.hint.clone(), theme.muted),
        ]);
        lines.push(if i == selected {
            line.patch_style(theme.selected)
        } else {
            line
        });
    }
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> KeyCombo {
        s.parse().unwrap()
    }

    #[test]
    fn prompt_edits_and_submits() {
        let mut m = Modal::prompt("New branch", "", PromptAction::CreateBranch { start: "abc".into() });
        assert_eq!(
            m.handle_key(k("enter")),
            ModalOutcome::Open,
            "empty input is not submitted"
        );
        for c in "topix".chars() {
            m.handle_key(KeyCombo::char(c));
        }
        m.handle_key(k("backspace"));
        m.handle_key(KeyCombo::char('c'));
        assert_eq!(
            m.handle_key(k("enter")),
            ModalOutcome::Run(GitOp::CreateBranch {
                name: "topic".into(),
                start: "abc".into()
            })
        );
        m.handle_key(k("ctrl+u"));
        assert_eq!(m.handle_key(k("esc")), ModalOutcome::Cancelled);
    }

    #[test]
    fn palette_filters_and_selects() {
        let items = vec![
            PaletteItem {
                label: "Fetch".into(),
                hint: "f".into(),
                target: PaletteTarget::Command(Command::Fetch),
            },
            PaletteItem {
                label: "Open commit on GitHub".into(),
                hint: "acme".into(),
                target: PaletteTarget::PluginAction("acme.gh.open".into()),
            },
            PaletteItem {
                label: "Push".into(),
                hint: "P".into(),
                target: PaletteTarget::Command(Command::Push),
            },
        ];
        let mut m = Modal::palette(items);
        assert_eq!(m.handle_key(k("enter")), ModalOutcome::Command(Command::Fetch));
        for c in "gh".chars() {
            m.handle_key(KeyCombo::char(c));
        }
        assert_eq!(
            m.handle_key(k("enter")),
            ModalOutcome::PluginAction("acme.gh.open".into())
        );
        m.handle_key(k("backspace"));
        m.handle_key(k("backspace"));
        m.handle_key(k("down"));
        m.handle_key(k("down"));
        m.handle_key(k("down"));
        assert_eq!(m.handle_key(k("enter")), ModalOutcome::Command(Command::Push));
        assert_eq!(m.handle_key(k("esc")), ModalOutcome::Cancelled);
    }

    #[test]
    fn confirm_yes_no() {
        let op = GitOp::DeleteBranch {
            name: "x".into(),
            force: false,
        };
        let mut m = Modal::confirm("Delete", "Delete x?", op.clone());
        assert_eq!(m.handle_key(k("q")), ModalOutcome::Open);
        assert_eq!(m.handle_key(k("y")), ModalOutcome::Run(op));
        assert_eq!(m.handle_key(k("n")), ModalOutcome::Cancelled);
    }
}
