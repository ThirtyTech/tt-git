//! Modal popups: a one-line text prompt, a yes/no confirmation, a
//! fuzzy-filtered list and the graph's filter panel.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use tt_git_core::{
    CheckoutPlan, DiscardKind, DiscardScope, IgnoreTarget, InProgress, LogFilter, PatchOp, StashScope, StatusEntry,
    UndoStep, message_regex,
};

use crate::agent::{AgentAction, AgentRequest};
use crate::filter::FilterPanel;
use crate::keymap::{Command, KeyCombo};
use crate::theme::Theme;

/// A git write operation, run as a background job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitOp {
    /// With `stash`, every change is stashed first (and popped back if the
    /// checkout fails).
    Checkout {
        plan: CheckoutPlan,
        stash: bool,
    },
    /// With `worktree`, the branch is checked out in a new worktree under
    /// `.worktrees/` instead of switching to it here.
    CreateBranch {
        name: String,
        start: String,
        worktree: bool,
    },
    DeleteBranch {
        name: String,
        force: bool,
    },
    DeleteRemoteBranch {
        remote: String,
        branch: String,
    },
    /// With `remote`, the tag is deleted there first, then here.
    DeleteTag {
        name: String,
        remote: Option<String>,
    },
    /// A lightweight tag, or with `message` an annotated one.
    CreateTag {
        name: String,
        target: String,
        message: Option<String>,
    },
    PushTag {
        name: String,
    },
    RenameBranch {
        old: String,
        new: String,
    },
    FetchAll,
    /// Fast-forward local branch `name` to `upstream` without checking it out.
    FastForward {
        name: String,
        upstream: String,
    },
    /// Save the base branch for the merged/unmerged lists (`None`: detect it).
    SetBaseBranch(Option<String>),
    Stage(Vec<String>),
    Unstage(Vec<String>),
    /// Every change, or with `folder` the ones under it.
    StageAll {
        folder: Option<String>,
    },
    UnstageAll {
        folder: Option<String>,
    },
    Discard {
        path: String,
        kind: DiscardKind,
    },
    /// Every change `scope` covers, in every file or with `folder` every
    /// file under it.
    DiscardAll {
        scope: DiscardScope,
        folder: Option<String>,
    },
    /// Adds `path` (a directory ends in `/`) to `target`. A tracked path
    /// is refused unless `untrack`, which also removes it from the index.
    Ignore {
        path: String,
        target: IgnoreTarget,
        untrack: bool,
    },
    /// Applies part of one file's changes (`patch`, built for `op`) by hunk, or
    /// by line with `lines`.
    ApplyPatch {
        path: String,
        patch: String,
        op: PatchOp,
        untracked: bool,
        lines: bool,
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
    /// Applies `oids`, oldest first; `summary` is the last one's.
    CherryPick {
        oids: Vec<String>,
        summary: String,
    },
    /// `git revert`. A merge commit needs `mainline`, the parent (from 1)
    /// whose side is kept.
    Revert {
        oid: String,
        summary: String,
        mainline: Option<usize>,
    },
    Continue(InProgress),
    Abort(InProgress),
    StashPush {
        message: Option<String>,
        scope: StashScope,
    },
    /// Applies `stash@{index}`, dropping it afterwards with `pop`.
    StashApply {
        index: usize,
        pop: bool,
    },
    StashDrop {
        index: usize,
    },
    /// `git stash branch <name> stash@{index}`.
    StashBranch {
        name: String,
        index: usize,
    },
    /// An undo. With `stash`, every change is stashed first: popped
    /// back after a reset, left on the Stash tab after a switch.
    Undo {
        step: UndoStep,
        stash: bool,
    },
    /// `git worktree remove` (with `--force`, even with changes). `name`
    /// is how the worktrees tab shows it; `branch` is what it had checked out.
    RemoveWorktree {
        path: std::path::PathBuf,
        name: String,
        branch: Option<String>,
        force: bool,
    },
    /// Not git: installs tt-git release `tag` over the running binary.
    SelfUpdate {
        tag: String,
    },
}

impl GitOp {
    /// Progress text shown while running.
    pub fn label(&self) -> String {
        match self {
            GitOp::Checkout { plan, stash } => {
                let target = match plan {
                    CheckoutPlan::Local(name) => name.clone(),
                    CheckoutPlan::Track { name, remote_ref } => format!("{name} (tracking {remote_ref})"),
                    CheckoutPlan::Detach(oid) => format!("{} (detached)", &oid[..oid.len().min(7)]),
                };
                if *stash {
                    format!("stash and checkout {target}")
                } else {
                    format!("checkout {target}")
                }
            }
            GitOp::CreateBranch {
                name, worktree: false, ..
            } => format!("create branch {name}"),
            GitOp::CreateBranch {
                name, worktree: true, ..
            } => {
                format!("create worktree {}", worktree_rel_path(name))
            }
            GitOp::DeleteBranch { name, .. } => format!("delete branch {name}"),
            GitOp::DeleteRemoteBranch { remote, branch } => format!("delete {remote}/{branch}"),
            GitOp::DeleteTag { name, remote: None } => format!("delete tag {name}"),
            GitOp::DeleteTag {
                name,
                remote: Some(remote),
            } => format!("delete tag {name} here and on {remote}"),
            GitOp::CreateTag { name, .. } => format!("create tag {name}"),
            GitOp::PushTag { name } => format!("push tag {name}"),
            GitOp::RenameBranch { old, new } => format!("rename {old} → {new}"),
            GitOp::FetchAll => "fetch --all".into(),
            GitOp::FastForward { name, .. } => format!("fast-forward {name}"),
            GitOp::SetBaseBranch(Some(base)) => format!("base branch {base}"),
            GitOp::SetBaseBranch(None) => "base branch auto".into(),
            GitOp::Stage(paths) => format!("stage {}", paths.join(" ")),
            GitOp::Unstage(paths) => format!("unstage {}", paths.join(" ")),
            GitOp::StageAll { folder: None } => "stage all".into(),
            GitOp::StageAll { folder: Some(f) } => format!("stage all in {f}"),
            GitOp::UnstageAll { folder: None } => "unstage all".into(),
            GitOp::UnstageAll { folder: Some(f) } => format!("unstage all in {f}"),
            GitOp::Discard { path, .. } => format!("discard {path}"),
            GitOp::DiscardAll { scope, folder } => {
                let what = match scope {
                    DiscardScope::Everything => "discard everything",
                    DiscardScope::Unstaged => "discard unstaged changes",
                    DiscardScope::UnstagedAndUntracked => "discard unstaged changes and untracked files",
                    DiscardScope::Untracked => "delete untracked files",
                };
                match folder {
                    Some(f) => format!("{what} in {f}"),
                    None => what.into(),
                }
            }
            GitOp::Ignore {
                path,
                target,
                untrack: false,
            } => format!("add {path} to {}", target.label()),
            GitOp::Ignore {
                path,
                target,
                untrack: true,
            } => format!("add {path} to {} and stop tracking it", target.label()),
            GitOp::ApplyPatch { path, op, lines, .. } => {
                let verb = match op {
                    PatchOp::Stage => "stage",
                    PatchOp::Unstage => "unstage",
                    PatchOp::Discard => "discard",
                };
                let part = if *lines { "lines" } else { "hunk" };
                format!("{verb} {part} of {path}")
            }
            GitOp::Commit { amend: false, .. } => "commit".into(),
            GitOp::Commit { amend: true, .. } => "amend".into(),
            GitOp::Push => "push".into(),
            GitOp::Pull => "pull".into(),
            GitOp::Merge { target } => format!("merge {target}"),
            GitOp::Rebase { onto } => format!("rebase onto {onto}"),
            GitOp::CherryPick { oids, .. } => match &oids[..] {
                [oid] => format!("cherry-pick {}", &oid[..oid.len().min(7)]),
                _ => format!("cherry-pick {} commits", oids.len()),
            },
            GitOp::Revert {
                oid, mainline: None, ..
            } => format!("revert {}", &oid[..oid.len().min(7)]),
            GitOp::Revert {
                oid, mainline: Some(n), ..
            } => format!("revert {} -m {n}", &oid[..oid.len().min(7)]),
            GitOp::Continue(op) => format!("{} --continue", op.command()),
            GitOp::Abort(op) => format!("{} --abort", op.command()),
            GitOp::StashPush { scope, .. } => match scope {
                StashScope::All => "stash".into(),
                StashScope::Staged => "stash staged changes".into(),
                StashScope::Unstaged => "stash unstaged changes".into(),
                StashScope::KeepStaged => "stash, keeping staged changes".into(),
                StashScope::Paths(paths) => format!("stash {}", paths.join(" ")),
            },
            GitOp::StashApply { index, pop: true } => format!("stash pop stash@{{{index}}}"),
            GitOp::StashApply { index, pop: false } => format!("stash apply stash@{{{index}}}"),
            GitOp::StashDrop { index } => format!("drop stash@{{{index}}}"),
            GitOp::StashBranch { name, index } => format!("branch {name} from stash@{{{index}}}"),
            GitOp::Undo { step, stash: false } => step.label(),
            GitOp::Undo { step, stash: true } => format!("stash and {}", step.label()),
            GitOp::RemoveWorktree { name, force: false, .. } => format!("remove worktree {name}"),
            GitOp::RemoveWorktree { name, force: true, .. } => format!("force-remove worktree {name}"),
            GitOp::SelfUpdate { tag } => format!("update tt-git to {tag}"),
        }
    }
}

/// Where a new worktree for branch `name` goes, relative to the main
/// working tree.
pub fn worktree_rel_path(name: &str) -> String {
    format!("{}/{name}", tt_git_core::repo::WORKTREE_DIR)
}

/// What a submitted prompt turns into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptAction {
    /// `tab` switches `worktree` on and off.
    CreateBranch {
        start: String,
        worktree: bool,
    },
    RenameBranch {
        old: String,
    },
    /// The tag's name. `tab` switches `annotated` on and off; an annotated
    /// tag asks for its message next.
    CreateTag {
        target: String,
        annotated: bool,
    },
    /// The message of annotated tag `name`.
    TagMessage {
        name: String,
        target: String,
    },
    Commit,
    /// A stash message, which may be left empty. `tab` cycles through what
    /// to stash; `file` (the selected file's paths) and `folder` (the one
    /// Status is narrowed to) add themselves as choices.
    Stash {
        scope: StashScope,
        file: Option<Vec<String>>,
        folder: Option<String>,
    },
    StashBranch {
        index: usize,
    },
    /// A request for an AI agent. `tab` steps from a custom message
    /// (`selected` none) through the ready-made `actions`.
    Agent {
        actions: Vec<AgentAction>,
        selected: Option<usize>,
    },
    /// A regex for the graph's commit subjects, added to `filter` (the
    /// graph's filter now). `tab` puts the next of `presets` in the input.
    FilterMessage {
        filter: LogFilter,
        presets: Vec<String>,
    },
}

impl PromptAction {
    /// The preset after the one typed (or the first), or before it with `back`.
    fn next_preset<'a>(presets: &'a [String], input: &str, back: bool) -> Option<&'a String> {
        let n = presets.len();
        let next = match presets.iter().position(|p| p == input) {
            Some(at) if back => (at + n - 1) % n,
            Some(at) => (at + 1) % n,
            None if back => n.checked_sub(1)?,
            None => 0,
        };
        presets.get(next)
    }

    /// The next choice of what to stash after `scope`.
    fn next_scope(scope: &StashScope, file: Option<&Vec<String>>, folder: Option<&String>, back: bool) -> StashScope {
        let mut choices: Vec<StashScope> = folder.map(|f| StashScope::Paths(vec![f.clone()])).into_iter().collect();
        choices.extend([
            StashScope::All,
            StashScope::Staged,
            StashScope::Unstaged,
            StashScope::KeepStaged,
        ]);
        if let Some(paths) = file {
            choices.push(StashScope::Paths(paths.clone()));
        }
        let at = choices.iter().position(|c| c == scope).unwrap_or(0);
        let n = choices.len();
        choices.swap_remove(if back { (at + n - 1) % n } else { (at + 1) % n })
    }

    fn op(&self, input: &str) -> GitOp {
        match self {
            PromptAction::Stash { scope, .. } => GitOp::StashPush {
                message: (!input.is_empty()).then(|| input.to_string()),
                scope: scope.clone(),
            },
            PromptAction::StashBranch { index } => GitOp::StashBranch {
                name: input.to_string(),
                index: *index,
            },
            PromptAction::CreateBranch { start, worktree } => GitOp::CreateBranch {
                name: input.to_string(),
                start: start.clone(),
                worktree: *worktree,
            },
            PromptAction::RenameBranch { old } => GitOp::RenameBranch {
                old: old.clone(),
                new: input.to_string(),
            },
            PromptAction::CreateTag { target, .. } => GitOp::CreateTag {
                name: input.to_string(),
                target: target.clone(),
                message: None,
            },
            PromptAction::TagMessage { name, target } => GitOp::CreateTag {
                name: name.clone(),
                target: target.clone(),
                message: Some(input.to_string()),
            },
            PromptAction::Commit => GitOp::Commit {
                message: Some(input.to_string()),
                amend: false,
            },
            PromptAction::Agent { .. } => unreachable!("an agent request isn't a git op"),
            PromptAction::FilterMessage { .. } => unreachable!("a filter isn't a git op"),
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
    /// A color theme, by name.
    Theme(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    pub label: String,
    /// Shown dimmed on the right: a key binding or the plugin name.
    pub hint: String,
    pub target: PaletteTarget,
}

const PALETTE_ROWS: usize = 12;

/// How many files the discard-all dialog would touch, from the status
/// pane's entries. A file with staged and unstaged changes counts in both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiscardCounts {
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    /// Untracked folders holding a repository of their own, which `git
    /// clean -fd` leaves alone.
    pub nested: usize,
}

impl DiscardCounts {
    pub fn of(entries: &[StatusEntry]) -> Self {
        let mut counts = Self::default();
        for e in entries {
            if e.is_untracked() {
                // With --untracked-files=all a folder only shows up when
                // it's a repository of its own.
                if e.path.ends_with('/') {
                    counts.nested += 1;
                } else {
                    counts.untracked += 1;
                }
                continue;
            }
            counts.staged += usize::from(e.has_staged());
            counts.unstaged += usize::from(e.has_unstaged());
        }
        counts
    }

    /// What `scope` throws away, as `2 staged files`, `1 untracked file`;
    /// empty if nothing.
    fn lost(&self, scope: DiscardScope) -> Vec<String> {
        let mut parts = Vec::new();
        if scope.drops_staged() && self.staged > 0 {
            parts.push(files(self.staged, "staged"));
        }
        if scope.drops_unstaged() && self.unstaged > 0 {
            parts.push(files(self.unstaged, "unstaged"));
        }
        if scope.drops_untracked() && self.untracked > 0 {
            parts.push(files(self.untracked, "untracked"));
        }
        parts
    }

    /// What stays that the user might expect to go.
    fn kept(&self, scope: DiscardScope) -> Vec<String> {
        let mut parts = Vec::new();
        if !scope.drops_staged() && self.staged > 0 {
            parts.push(files(self.staged, "staged"));
        }
        if !scope.drops_unstaged() && self.unstaged > 0 {
            parts.push(files(self.unstaged, "unstaged"));
        }
        if !scope.drops_untracked() && self.untracked > 0 {
            parts.push(files(self.untracked, "untracked"));
        }
        if scope.drops_untracked() && self.nested > 0 {
            let s = if self.nested == 1 { "repository" } else { "repositories" };
            parts.push(format!("{} nested {s}", self.nested));
        }
        parts.push("ignored files".into());
        parts
    }
}

/// `1 staged file`, `2 staged files`.
fn files(n: usize, what: &str) -> String {
    format!("{n} {what} file{}", if n == 1 { "" } else { "s" })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    /// Fuzzy-filtered list: the command palette (built-in commands and
    /// plugin actions) or a picker such as the base branch one.
    Palette {
        title: String,
        input: String,
        items: Vec<PaletteItem>,
        selected: usize,
        /// How many lines a typed number can go to (blame's); 0 for none.
        lines: usize,
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
        /// A second answer besides `y`: its key, its hint text and what it runs.
        alt: Option<Box<(char, String, GitOp)>>,
    },
    /// Pick an author or file to narrow the graph to.
    Filter(Box<FilterPanel>),
    /// Discard every change (under `folder`, when set): `tab` cycles
    /// through what to discard, `y` does it.
    DiscardAll {
        scope: DiscardScope,
        counts: DiscardCounts,
        folder: Option<String>,
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
    /// Show only the graph's commits that match.
    Filter(LogFilter),
    /// Go to this line (from 1), typed as a number in the palette.
    GoToLine(usize),
    /// Start an agent.
    Agent(AgentRequest),
    /// Keep this color theme.
    Theme(String),
}

impl Modal {
    pub fn prompt(title: impl Into<String>, initial: impl Into<String>, action: PromptAction) -> Self {
        Modal::Prompt {
            title: title.into(),
            input: initial.into(),
            action,
        }
    }

    /// The new tag prompt for commit or ref `target`, shown as `at`.
    pub fn new_tag(at: &str, target: String) -> Self {
        Self::prompt(
            format!("New tag at {at}"),
            "",
            PromptAction::CreateTag {
                target,
                annotated: false,
            },
        )
    }

    pub fn confirm(title: impl Into<String>, message: impl Into<String>, op: GitOp) -> Self {
        Modal::Confirm {
            title: title.into(),
            message: message.into(),
            op,
            alt: None,
        }
    }

    /// [`confirm`](Self::confirm) with a second answer: key `key`, shown
    /// as `hint`, runs `alt_op`.
    pub fn confirm_or(self, key: char, hint: impl Into<String>, alt_op: GitOp) -> Self {
        match self {
            Modal::Confirm { title, message, op, .. } => Modal::Confirm {
                title,
                message,
                op,
                alt: Some(Box::new((key, hint.into(), alt_op))),
            },
            other => other,
        }
    }

    /// The discard-all dialog, starting on discarding everything. `counts`
    /// are of the files under `folder`, when set.
    pub fn discard_all(counts: DiscardCounts, folder: Option<String>) -> Self {
        Modal::DiscardAll {
            scope: DiscardScope::Everything,
            counts,
            folder,
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
            lines: 0,
        }
    }

    /// Asks for a line to go to, from 1 to `count`.
    pub fn go_to_line(count: usize) -> Self {
        let mut modal = Self::picker("Go to line", Vec::new(), 0).with_lines(count);
        if let Modal::Palette { title, .. } = &mut modal {
            *title = format!("Go to line (1-{count})");
        }
        modal
    }

    /// Lets a number typed in the palette go to one of `count` lines.
    pub fn with_lines(mut self, count: usize) -> Self {
        if let Modal::Palette { title, lines, .. } = &mut self
            && count > 0
        {
            *lines = count;
            title.push_str(" or a line number");
        }
        self
    }

    /// The line a number typed in the palette goes to, as vim's `:42` does
    /// (the leading `:` is optional): from 1, past the end the last.
    pub fn palette_line(input: &str, lines: usize) -> Option<usize> {
        let input = input.trim();
        let digits = input.strip_prefix(':').unwrap_or(input);
        if lines == 0 || digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some(digits.parse::<usize>().unwrap_or(usize::MAX).clamp(1, lines))
    }

    /// Palette entries matching the typed filter, in order.
    pub fn palette_matches<'a>(items: &'a [PaletteItem], input: &str) -> Vec<&'a PaletteItem> {
        items
            .iter()
            .filter(|i| tt_git_core::branches::matches_filter(&i.label, input))
            .collect()
    }

    /// What the palette's highlighted entry runs.
    pub fn palette_selection(&self) -> Option<&PaletteTarget> {
        match self {
            Modal::Palette {
                input, items, selected, ..
            } => Self::palette_matches(items, input).get(*selected).map(|i| &i.target),
            _ => None,
        }
    }

    pub fn handle_key(&mut self, key: KeyCombo) -> ModalOutcome {
        match self {
            Modal::Palette {
                input,
                items,
                selected,
                lines,
                ..
            } => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let count = Self::palette_matches(items, input).len();
                match key.code {
                    KeyCode::Esc => return ModalOutcome::Cancelled,
                    KeyCode::Enter if Self::palette_line(input, *lines).is_some() => {
                        return ModalOutcome::GoToLine(Self::palette_line(input, *lines).expect("checked"));
                    }
                    KeyCode::Enter => {
                        return match Self::palette_matches(items, input).get(*selected) {
                            Some(item) => match &item.target {
                                PaletteTarget::Command(c) => ModalOutcome::Command(*c),
                                PaletteTarget::PluginAction(a) => ModalOutcome::PluginAction(a.clone()),
                                PaletteTarget::PluginPane(p, e) => ModalOutcome::PluginPane(p.clone(), e.clone()),
                                PaletteTarget::Op(op) => ModalOutcome::Run(op.clone()),
                                PaletteTarget::Theme(name) => ModalOutcome::Theme(name.clone()),
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
                    if let PromptAction::Agent { actions, selected } = action {
                        let picked = selected.and_then(|i| actions.get(i)).cloned();
                        return if value.is_empty() && picked.is_none() {
                            ModalOutcome::Open
                        } else {
                            ModalOutcome::Agent(AgentRequest {
                                action: picked,
                                message: value.to_string(),
                            })
                        };
                    }
                    if let PromptAction::FilterMessage { filter, .. } = action {
                        // Empty clears the message part; an invalid regex stays open.
                        if !value.is_empty() && message_regex(value).is_err() {
                            return ModalOutcome::Open;
                        }
                        return ModalOutcome::Filter(LogFilter {
                            message: (!value.is_empty()).then(|| value.to_string()),
                            ..filter.clone()
                        });
                    }
                    if value.is_empty() && !matches!(action, PromptAction::Stash { .. }) {
                        ModalOutcome::Open
                    } else if let PromptAction::CreateTag {
                        target,
                        annotated: true,
                    } = action
                    {
                        *self = Modal::prompt(
                            format!("Message for tag {value}"),
                            "",
                            PromptAction::TagMessage {
                                name: value.to_string(),
                                target: target.clone(),
                            },
                        );
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
                KeyCode::Tab | KeyCode::BackTab => {
                    match action {
                        PromptAction::CreateBranch { worktree, .. } => *worktree = !*worktree,
                        PromptAction::CreateTag { annotated, .. } => *annotated = !*annotated,
                        PromptAction::Stash { scope, file, folder } => {
                            *scope = PromptAction::next_scope(
                                scope,
                                file.as_ref(),
                                folder.as_ref(),
                                key.code == KeyCode::BackTab,
                            );
                        }
                        PromptAction::Agent { actions, selected } => {
                            // None (custom message), then each action, round again.
                            let n = actions.len() + 1;
                            let at = selected.map_or(0, |i| i + 1);
                            let next = if key.code == KeyCode::BackTab {
                                (at + n - 1) % n
                            } else {
                                (at + 1) % n
                            };
                            *selected = next.checked_sub(1);
                        }
                        PromptAction::FilterMessage { presets, .. } => {
                            if let Some(preset) =
                                PromptAction::next_preset(presets, input.trim(), key.code == KeyCode::BackTab)
                            {
                                *input = preset.clone();
                            }
                        }
                        _ => {}
                    }
                    ModalOutcome::Open
                }
                KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                    input.push(c);
                    ModalOutcome::Open
                }
                _ => ModalOutcome::Open,
            },
            Modal::Confirm { op, alt, .. } => match key.code {
                KeyCode::Char(c) if alt.as_ref().is_some_and(|alt| alt.0 == c) => {
                    ModalOutcome::Run(alt.as_ref().expect("matched").2.clone())
                }
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => ModalOutcome::Run(op.clone()),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => ModalOutcome::Cancelled,
                _ => ModalOutcome::Open,
            },
            Modal::Filter(panel) => panel.handle_key(key),
            Modal::DiscardAll { scope, counts, folder } => match key.code {
                KeyCode::Tab | KeyCode::BackTab => {
                    let all = DiscardScope::ALL;
                    let at = all.iter().position(|s| s == scope).unwrap_or(0);
                    let n = all.len();
                    *scope = all[if key.code == KeyCode::BackTab {
                        (at + n - 1) % n
                    } else {
                        (at + 1) % n
                    }];
                    ModalOutcome::Open
                }
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter if !counts.lost(*scope).is_empty() => {
                    ModalOutcome::Run(GitOp::DiscardAll {
                        scope: *scope,
                        folder: folder.clone(),
                    })
                }
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
            lines,
        } = self
        {
            return draw_palette(frame, area, theme, title, input, items, *selected, *lines);
        }
        if let Modal::Filter(panel) = self {
            return panel.draw(frame, area, theme);
        }
        let discard_title = match self {
            Modal::DiscardAll { folder: Some(f), .. } => format!("Discard all in {f}"),
            _ => "Discard all".into(),
        };
        let (title, body, hint): (&str, Vec<Line>, String) = match self {
            Modal::Palette { .. } | Modal::Filter(_) => unreachable!("handled above"),
            Modal::Prompt { title, input, action } => {
                let input_line = Line::from(vec![Span::raw(input.as_str()), Span::styled("█", theme.help_key)]);
                let tab = |label: String, on: bool| {
                    if on {
                        Span::styled(label, theme.help_key.add_modifier(Modifier::UNDERLINED))
                    } else {
                        Span::styled(label, theme.muted)
                    }
                };
                match action {
                    PromptAction::Stash { scope, file, folder } => {
                        let mut tabs = Vec::new();
                        if let Some(f) = folder {
                            let picked = matches!(scope, StashScope::Paths(p) if p.len() == 1 && p[0] == *f);
                            tabs.extend([tab(f.clone(), picked), Span::styled(" · ", theme.muted)]);
                        }
                        tabs.extend([
                            tab("Everything".into(), *scope == StashScope::All),
                            Span::styled(" · ", theme.muted),
                            tab("Staged".into(), *scope == StashScope::Staged),
                            Span::styled(" · ", theme.muted),
                            tab("Unstaged".into(), *scope == StashScope::Unstaged),
                            Span::styled(" · ", theme.muted),
                            tab("Keep staged".into(), *scope == StashScope::KeepStaged),
                        ]);
                        if let Some(paths) = file {
                            let name = paths[0].rsplit('/').next().unwrap_or(&paths[0]).to_string();
                            tabs.push(Span::styled(" · ", theme.muted));
                            tabs.push(tab(name, matches!(scope, StashScope::Paths(p) if p == paths)));
                        }
                        let note = match scope {
                            StashScope::All => "staged, unstaged and untracked changes".to_string(),
                            StashScope::Staged => "only what's staged".to_string(),
                            StashScope::Unstaged => "unstaged and untracked changes; staged ones stay".to_string(),
                            StashScope::KeepStaged => "everything, leaving staged changes in place".to_string(),
                            StashScope::Paths(paths) => format!("only {}", paths.join(", ")),
                        };
                        let body = vec![
                            Line::from(tabs),
                            input_line,
                            Line::styled(format!("{note} · message optional"), theme.muted),
                        ];
                        (title, body, "enter stash · tab what to stash · esc cancel".into())
                    }
                    PromptAction::CreateBranch { worktree, .. } => {
                        let note = if *worktree {
                            let name = input.trim();
                            let name = if name.is_empty() { "<name>" } else { name };
                            format!("checked out in {}", worktree_rel_path(name))
                        } else {
                            "switches to it here".to_string()
                        };
                        let body = vec![
                            Line::from(vec![
                                tab("Branch".into(), !worktree),
                                Span::styled(" · ", theme.muted),
                                tab("Worktree".into(), *worktree),
                            ]),
                            input_line,
                            Line::styled(note, theme.muted),
                        ];
                        (title, body, "enter confirm · tab branch/worktree · esc cancel".into())
                    }
                    PromptAction::CreateTag { annotated, .. } => {
                        let note = if *annotated {
                            "asks for a message next"
                        } else {
                            "just a name for the commit"
                        };
                        let body = vec![
                            Line::from(vec![
                                tab("Lightweight".into(), !annotated),
                                Span::styled(" · ", theme.muted),
                                tab("Annotated".into(), *annotated),
                            ]),
                            input_line,
                            Line::styled(note, theme.muted),
                        ];
                        let hint = if *annotated {
                            "enter next · tab lightweight/annotated · esc cancel"
                        } else {
                            "enter create · tab lightweight/annotated · esc cancel"
                        };
                        (title, body, hint.into())
                    }
                    PromptAction::Agent { actions, selected } => {
                        let mut tabs = vec![tab("Custom message".into(), selected.is_none())];
                        for (i, a) in actions.iter().enumerate() {
                            tabs.push(Span::styled(" · ", theme.muted));
                            tabs.push(tab(a.name.clone(), *selected == Some(i)));
                        }
                        let picked = selected.and_then(|i| actions.get(i));
                        let note = match picked {
                            Some(a) => {
                                let what = a.description.clone().unwrap_or_else(|| a.name.clone());
                                format!("{what} · anything typed is added to it")
                            }
                            None => "type what you want the agent to do".into(),
                        };
                        let body = vec![Line::from(tabs), input_line, Line::styled(note, theme.muted)];
                        (title, body, "enter start · tab ready-made actions · esc cancel".into())
                    }
                    PromptAction::FilterMessage { presets, .. } => {
                        let typed = input.trim();
                        let mut tabs = Vec::new();
                        for p in presets {
                            if !tabs.is_empty() {
                                tabs.push(Span::styled(" · ", theme.muted));
                            }
                            tabs.push(tab(p.clone(), p == typed));
                        }
                        let note = match message_regex(typed) {
                            _ if typed.is_empty() => Line::styled("empty shows every subject again", theme.muted),
                            Ok(_) if typed.chars().any(char::is_uppercase) => {
                                Line::styled("regex on the subject · matches case", theme.muted)
                            }
                            Ok(_) => Line::styled("regex on the subject · ignores case", theme.muted),
                            Err(e) => {
                                let why = e.to_string();
                                // The last line of the error says what's wrong, after a
                                // drawing of where.
                                let last = why.lines().last().unwrap_or_default().trim();
                                let last = last.strip_prefix("error: ").unwrap_or(last);
                                Line::styled(format!("invalid regex: {last}"), theme.error)
                            }
                        };
                        let mut body = Vec::new();
                        if !tabs.is_empty() {
                            body.push(Line::from(tabs));
                        }
                        body.extend([input_line, note]);
                        let hint = if presets.is_empty() {
                            "enter filter · esc cancel"
                        } else {
                            "enter filter · tab presets · esc cancel"
                        };
                        (title, body, hint.into())
                    }
                    _ => (title, vec![input_line], "enter confirm · esc cancel".into()),
                }
            }
            Modal::Confirm {
                title, message, alt, ..
            } => (
                title,
                message.lines().map(Line::from).collect(),
                match alt.as_deref() {
                    Some((key, hint, _)) => format!("y confirm · {key} {hint} · n/esc cancel"),
                    None => "y confirm · n/esc cancel".into(),
                },
            ),
            Modal::DiscardAll { scope, counts, folder } => {
                let tab = |label: &'static str, s: DiscardScope| {
                    if s == *scope {
                        Span::styled(label, theme.help_key.add_modifier(Modifier::UNDERLINED))
                    } else {
                        Span::styled(label, theme.muted)
                    }
                };
                let sep = || Span::styled(" · ", theme.muted);
                let what = match scope {
                    DiscardScope::Everything => "every file back to HEAD, untracked files deleted",
                    DiscardScope::Unstaged => "files back to what's staged; staged changes and untracked files stay",
                    DiscardScope::UnstagedAndUntracked => {
                        "files back to what's staged, untracked files deleted; staged changes stay"
                    }
                    DiscardScope::Untracked => "untracked files deleted; changes to tracked files stay",
                };
                let lost = counts.lost(*scope);
                let mut body = vec![
                    Line::from(vec![
                        tab("Everything", DiscardScope::Everything),
                        sep(),
                        tab("Unstaged", DiscardScope::Unstaged),
                        sep(),
                        tab("Unstaged + untracked", DiscardScope::UnstagedAndUntracked),
                        sep(),
                        tab("Untracked", DiscardScope::Untracked),
                    ]),
                    Line::styled(what, theme.muted),
                    Line::raw(""),
                ];
                if lost.is_empty() {
                    body.push(Line::raw("Nothing to discard here."));
                } else {
                    body.push(Line::raw(format!("Lost: {}.", lost.join(", "))));
                }
                let mut kept = counts.kept(*scope);
                if let Some(f) = folder {
                    kept.push(format!("everything outside {f}"));
                }
                body.push(Line::raw(format!("Kept: {}.", kept.join(", "))));
                let hint = if lost.is_empty() {
                    "tab what to discard · n/esc cancel"
                } else {
                    body.push(Line::styled("This cannot be undone.", theme.error));
                    "y discard · tab what to discard · n/esc cancel"
                };
                (&discard_title, body, hint.into())
            }
        };
        // The stash dialog's row of choices ends in a file name.
        let wide = matches!(
            self,
            Modal::Prompt {
                action: PromptAction::Stash { .. } | PromptAction::Agent { .. } | PromptAction::FilterMessage { .. },
                ..
            } | Modal::DiscardAll { .. }
        );
        let width = if wide { 72 } else { 60 }.min(area.width);
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

#[allow(clippy::too_many_arguments)]
fn draw_palette(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    title: &str,
    input: &str,
    items: &[PaletteItem],
    selected: usize,
    line_count: usize,
) {
    // (label, hint) per row. A line number is the one thing to do, in
    // place of the commands.
    let (matches, selected) = match Modal::palette_line(input, line_count) {
        Some(line) => (vec![(format!("Go to line {line}"), format!("of {line_count}"))], 0),
        None => (
            Modal::palette_matches(items, input)
                .into_iter()
                .map(|i| (i.label.clone(), i.hint.clone()))
                .collect(),
            selected,
        ),
    };
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
        let empty = if items.is_empty() && line_count > 0 {
            "type a line number"
        } else {
            "no matches"
        };
        lines.push(Line::styled(empty, theme.muted));
    }
    for (i, (label, hint)) in matches.into_iter().enumerate().skip(start).take(PALETTE_ROWS) {
        let hint_w = hint.chars().count();
        let label_w = inner_width.saturating_sub(hint_w + 1);
        let line = Line::from(vec![
            Span::raw(crate::widgets::fit(&label, label_w)),
            Span::raw(" "),
            Span::styled(hint, theme.muted),
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
    fn a_number_in_the_palette_is_a_line_when_there_are_lines() {
        assert_eq!(Modal::palette_line("42", 100), Some(42));
        assert_eq!(Modal::palette_line(" :42 ", 100), Some(42));
        assert_eq!(Modal::palette_line("420", 100), Some(100));
        assert_eq!(Modal::palette_line("0", 100), Some(1));
        assert_eq!(Modal::palette_line("99999999999999999999999", 100), Some(100));
        assert_eq!(Modal::palette_line("42", 0), None);
        for not_a_line in ["", ":", "4a", "push", "-3"] {
            assert_eq!(Modal::palette_line(not_a_line, 100), None, "{not_a_line}");
        }
        let mut m = Modal::go_to_line(10);
        m.handle_key(KeyCombo::char('7'));
        assert_eq!(m.handle_key(k("enter")), ModalOutcome::GoToLine(7));
    }

    #[test]
    fn prompt_edits_and_submits() {
        let mut m = Modal::prompt(
            "New branch",
            "",
            PromptAction::CreateBranch {
                start: "abc".into(),
                worktree: false,
            },
        );
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
                start: "abc".into(),
                worktree: false,
            })
        );
        m.handle_key(k("tab"));
        assert_eq!(
            m.handle_key(k("enter")),
            ModalOutcome::Run(GitOp::CreateBranch {
                name: "topic".into(),
                start: "abc".into(),
                worktree: true,
            }),
            "tab switches to a worktree"
        );
        m.handle_key(k("shift+tab"));
        assert!(matches!(
            m.handle_key(k("enter")),
            ModalOutcome::Run(GitOp::CreateBranch { worktree: false, .. })
        ));
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
    fn message_filter_prompt_steps_through_presets() {
        let author = LogFilter {
            author: Some("Alice <alice@example.com>".into()),
            ..LogFilter::default()
        };
        let mut m = Modal::prompt(
            "Filter commits by message",
            "",
            PromptAction::FilterMessage {
                filter: author.clone(),
                presets: vec!["^feat".into(), "^fix".into(), "^docs".into()],
            },
        );
        let input = |m: &Modal| match m {
            Modal::Prompt { input, .. } => input.clone(),
            _ => unreachable!(),
        };
        m.handle_key(k("tab"));
        assert_eq!(input(&m), "^feat");
        m.handle_key(k("tab"));
        assert_eq!(input(&m), "^fix");
        m.handle_key(k("shift+tab"));
        m.handle_key(k("shift+tab"));
        assert_eq!(input(&m), "^docs", "shift+tab goes round to the last");
        m.handle_key(k("tab"));
        assert_eq!(input(&m), "^feat", "tab goes round to the first");
        // A preset can be edited before enter.
        m.handle_key(KeyCombo::char('('));
        assert_eq!(
            m.handle_key(k("enter")),
            ModalOutcome::Open,
            "an invalid regex stays open"
        );
        m.handle_key(k("backspace"));
        assert_eq!(
            m.handle_key(k("enter")),
            ModalOutcome::Filter(LogFilter {
                message: Some("^feat".into()),
                ..author.clone()
            }),
            "keeps the author"
        );
        m.handle_key(k("ctrl+u"));
        assert_eq!(
            m.handle_key(k("enter")),
            ModalOutcome::Filter(author),
            "empty clears only the message"
        );
    }

    #[test]
    fn agent_prompt_steps_through_actions() {
        let actions = crate::agent::default_actions();
        let mut m = Modal::prompt(
            "Ask an agent",
            "",
            PromptAction::Agent {
                actions: actions.clone(),
                selected: None,
            },
        );
        assert_eq!(
            m.handle_key(k("enter")),
            ModalOutcome::Open,
            "a custom message needs text"
        );
        m.handle_key(k("tab"));
        assert_eq!(
            m.handle_key(k("enter")),
            ModalOutcome::Agent(AgentRequest {
                action: Some(actions[0].clone()),
                message: String::new(),
            })
        );
        m.handle_key(k("shift+tab"));
        m.handle_key(k("shift+tab"));
        for c in "why".chars() {
            m.handle_key(KeyCombo::char(c));
        }
        assert_eq!(
            m.handle_key(k("enter")),
            ModalOutcome::Agent(AgentRequest {
                action: Some(actions[actions.len() - 1].clone()),
                message: "why".into(),
            }),
            "shift+tab goes round to the last action"
        );
        m.handle_key(k("tab"));
        assert_eq!(
            m.handle_key(k("enter")),
            ModalOutcome::Agent(AgentRequest {
                action: None,
                message: "why".into(),
            })
        );
    }

    #[test]
    fn discard_all_cycles_what_to_discard() {
        let counts = DiscardCounts {
            staged: 1,
            unstaged: 0,
            untracked: 2,
            nested: 0,
        };
        let mut m = Modal::discard_all(counts, None);
        assert_eq!(
            m.handle_key(k("y")),
            ModalOutcome::Run(GitOp::DiscardAll {
                scope: DiscardScope::Everything,
                folder: None
            })
        );
        m.handle_key(k("tab"));
        assert_eq!(
            m.handle_key(k("enter")),
            ModalOutcome::Open,
            "no unstaged changes to discard"
        );
        m.handle_key(k("tab"));
        assert_eq!(
            m.handle_key(k("y")),
            ModalOutcome::Run(GitOp::DiscardAll {
                scope: DiscardScope::UnstagedAndUntracked,
                folder: None
            })
        );
        for _ in 0..3 {
            m.handle_key(k("shift+tab"));
        }
        assert_eq!(
            m.handle_key(k("y")),
            ModalOutcome::Run(GitOp::DiscardAll {
                scope: DiscardScope::Untracked,
                folder: None
            }),
            "shift+tab goes round"
        );
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
