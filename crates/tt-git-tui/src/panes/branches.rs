use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tt_git_core::branches::{matches_filter, tags_newest_first};
use tt_git_core::time::{SharedClock, relative};
use tt_git_core::{BaseBranches, BranchEntry, BranchState, GitRef, RefKind};
use unicode_width::UnicodeWidthStr;

use super::{BranchFocus, PaneId, pane_block};
use crate::action::Action;
use crate::component::{Component, EventState};
use crate::jobs::Ctx;
use crate::keymap::{Command, Context, KeyCombo};
use crate::modal::{GitOp, Modal, PaletteItem, PaletteTarget, PromptAction};
use crate::theme::Theme;
use crate::widgets::{Cursor, fit, fit_right};

const STATE_WIDTH: usize = 9;
const DATE_WIDTH: usize = 4;
const AUTHOR_WIDTH: usize = 14;
/// Narrowest commit subject column worth showing in tag and merged/unmerged rows.
const SUBJECT_MIN_WIDTH: usize = 12;

/// The pane's lists, switched lazygit-style with `[` and `]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BranchTab {
    /// Every local and remote branch, by recent activity.
    #[default]
    All,
    /// Tags, newest first.
    Tags,
    /// Branches on the base's remote that aren't merged into it.
    Unmerged,
    /// Branches on the base's remote that are merged into it.
    Merged,
}

impl BranchTab {
    pub const ALL: [BranchTab; 4] = [BranchTab::All, BranchTab::Tags, BranchTab::Unmerged, BranchTab::Merged];

    pub fn label(self) -> &'static str {
        match self {
            BranchTab::All => "Branches",
            BranchTab::Tags => "Tags",
            BranchTab::Unmerged => "Unmerged",
            BranchTab::Merged => "Merged",
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    /// The tabs that compare branches with the base branch.
    fn compares(self) -> bool {
        matches!(self, BranchTab::Unmerged | BranchTab::Merged)
    }
}

/// Unified local + remote branch list sorted by recent activity, the tags,
/// and the branches merged and not merged into the base branch.
#[derive(Debug)]
pub struct BranchesPane {
    clock: SharedClock,
    entries: Vec<BranchEntry>,
    base: Option<BaseBranches>,
    /// Newest first.
    tags: Vec<GitRef>,
    tab: BranchTab,
    /// Indices into the current tab's list that pass the filter.
    visible: Vec<usize>,
    filter: String,
    editing_filter: bool,
    /// One per tab, so switching back returns to the same row.
    cursors: [Cursor; BranchTab::ALL.len()],
}

impl BranchesPane {
    pub fn new(clock: SharedClock) -> Self {
        Self {
            clock,
            entries: Vec::new(),
            base: None,
            tags: Vec::new(),
            tab: BranchTab::default(),
            visible: Vec::new(),
            filter: String::new(),
            editing_filter: false,
            cursors: [Cursor::default(); BranchTab::ALL.len()],
        }
    }

    /// Replaces the lists, keeping the selection on the same row if it
    /// still exists. `refs` may hold any refs; only its tags are listed.
    pub fn set_entries(&mut self, entries: Vec<BranchEntry>, base: Option<BaseBranches>, refs: &[GitRef]) {
        let keep = self.selected_name().map(str::to_string);
        self.entries = entries;
        self.base = base;
        self.tags = tags_newest_first(refs);
        self.refilter();
        let pos = keep.and_then(|name| (0..self.visible.len()).position(|pos| self.name_at(pos) == name));
        if let Some(pos) = pos {
            self.cursor_mut().selected = pos;
        }
    }

    /// The unified list of every branch.
    pub fn entries(&self) -> &[BranchEntry] {
        &self.entries
    }

    pub fn base(&self) -> Option<&BaseBranches> {
        self.base.as_ref()
    }

    pub fn tab(&self) -> BranchTab {
        self.tab
    }

    /// The current tab's branches passing the filter, in display order
    /// (none on the tags tab).
    pub fn visible(&self) -> impl Iterator<Item = &BranchEntry> {
        let list = self.list(self.tab);
        self.visible.iter().filter_map(move |&i| list.get(i))
    }

    pub fn selected(&self) -> Option<&BranchEntry> {
        self.visible
            .get(self.cursor().selected)
            .and_then(|&i| self.list(self.tab).get(i))
    }

    /// The tags passing the filter, in display order, on the tags tab.
    pub fn visible_tags(&self) -> impl Iterator<Item = &GitRef> {
        let tags = self.tag_list();
        self.visible.iter().filter_map(move |&i| tags.get(i))
    }

    pub fn selected_tag(&self) -> Option<&GitRef> {
        self.visible
            .get(self.cursor().selected)
            .and_then(|&i| self.tag_list().get(i))
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    fn list(&self, tab: BranchTab) -> &[BranchEntry] {
        match (tab, &self.base) {
            (BranchTab::All, _) => &self.entries,
            (BranchTab::Unmerged, Some(base)) => &base.unmerged,
            (BranchTab::Merged, Some(base)) => &base.merged,
            (BranchTab::Tags, _) | (_, None) => &[],
        }
    }

    fn tag_list(&self) -> &[GitRef] {
        if self.tab == BranchTab::Tags { &self.tags } else { &[] }
    }

    /// How many rows `tab` has before filtering.
    fn len(&self, tab: BranchTab) -> usize {
        match tab {
            BranchTab::Tags => self.tags.len(),
            _ => self.list(tab).len(),
        }
    }

    /// Name of the current tab's row `i` (an index into its unfiltered list).
    fn row_name(&self, i: usize) -> &str {
        match self.tab {
            BranchTab::Tags => &self.tags[i].short_name,
            _ => &self.list(self.tab)[i].name,
        }
    }

    /// Name of the visible row at `pos`.
    fn name_at(&self, pos: usize) -> &str {
        self.row_name(self.visible[pos])
    }

    fn selected_name(&self) -> Option<&str> {
        (self.cursor().selected < self.visible.len()).then(|| self.name_at(self.cursor().selected))
    }

    fn cursor(&self) -> &Cursor {
        &self.cursors[self.tab.index()]
    }

    fn cursor_mut(&mut self) -> &mut Cursor {
        &mut self.cursors[self.tab.index()]
    }

    fn refilter(&mut self) {
        self.visible = (0..self.len(self.tab))
            .filter(|&i| matches_filter(self.row_name(i), &self.filter))
            .collect();
        let len = self.visible.len();
        self.cursor_mut().clamp(len);
    }

    /// Moves `step` tabs along, wrapping round.
    fn switch_tab(&mut self, step: usize, ctx: &Ctx) {
        let n = BranchTab::ALL.len();
        self.tab = BranchTab::ALL[(self.tab.index() + step) % n];
        self.refilter();
        self.announce(ctx);
    }

    /// On the merged and unmerged tabs, the selected branch for the graph
    /// to show on its own.
    pub fn focus(&self) -> Option<BranchFocus> {
        if !self.tab.compares() {
            return None;
        }
        let base = self.base.as_ref()?;
        let tip = BaseBranches::tip(self.selected()?)?;
        Some(BranchFocus {
            branch: tip.short_name.clone(),
            tip: tip.target.clone(),
            base: base.base.clone(),
            base_ref: base.base_ref.clone(),
        })
    }

    fn announce(&self, ctx: &Ctx) {
        ctx.emit(Action::BranchFocused(self.focus()));
        let oid = match self.tab {
            BranchTab::All => self.selected().and_then(BranchEntry::target),
            BranchTab::Tags => self.selected_tag().map(|t| t.target.as_str()),
            _ => None,
        };
        if let Some(oid) = oid {
            ctx.emit(Action::BranchSelected(oid.to_string()));
        }
    }

    /// The base branch picker: detect automatically, then every remote
    /// branch, then every local one.
    fn base_picker(&self) -> Modal {
        let current = self.base.as_ref().map(|b| b.base.as_str());
        let detected = self.base.as_ref().and_then(|b| b.detected.clone());
        let mut items = vec![PaletteItem {
            label: "Detect automatically".into(),
            hint: detected.unwrap_or_else(|| "none found".into()),
            target: PaletteTarget::Op(GitOp::SetBaseBranch(None)),
        }];
        let remotes = self.entries.iter().flat_map(|e| e.remotes.iter());
        let locals = self.entries.iter().filter_map(|e| e.local.as_ref());
        for r in remotes.chain(locals) {
            let hint = if Some(r.short_name.as_str()) == current {
                "current"
            } else {
                ""
            };
            items.push(PaletteItem {
                label: r.short_name.clone(),
                hint: hint.into(),
                target: PaletteTarget::Op(GitOp::SetBaseBranch(Some(r.short_name.clone()))),
            });
        }
        let selected = match &self.base {
            Some(b) if b.chosen => items.iter().position(|i| i.label == b.base).unwrap_or(0),
            _ => 0,
        };
        Modal::picker("Base branch", items, selected)
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

    /// Commands that act on the selected tag.
    fn on_tag_command(&self, cmd: Command, ctx: &Ctx) -> EventState {
        let Some(tag) = self.selected_tag() else {
            return EventState::NotConsumed;
        };
        let name = tag.short_name.clone();
        match cmd {
            Command::Checkout => ctx.emit(Action::Notify(format!("n makes a branch from {name}"))),
            Command::NewBranch => ctx.emit(Action::OpenModal(Modal::prompt(
                format!("New branch from {name}"),
                "",
                PromptAction::CreateBranch {
                    start: name,
                    worktree: false,
                },
            ))),
            Command::Merge => ctx.emit(Action::OpenModal(Modal::confirm(
                "Merge",
                format!("Merge {name} into the current branch?"),
                GitOp::Merge { target: name },
            ))),
            Command::Rebase => ctx.emit(Action::OpenModal(Modal::confirm(
                "Rebase",
                format!("Rebase the current branch onto {name}?"),
                GitOp::Rebase { onto: name },
            ))),
            Command::DeleteBranch => ctx.emit(Action::OpenModal(Modal::confirm(
                "Delete tag",
                format!("Delete local tag {name}?"),
                GitOp::DeleteTag { name },
            ))),
            Command::RenameBranch => ctx.emit(Action::Notify("only local branches can be renamed".into())),
            Command::Copy => ctx.emit(Action::Copy(name)),
            _ => return EventState::NotConsumed,
        }
        EventState::Consumed
    }

    /// Commands that act on the selected branch.
    fn on_entry_command(&self, cmd: Command, ctx: &Ctx) -> EventState {
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
                    worktree: false,
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
            Command::Copy => ctx.emit(Action::Copy(entry.name.clone())),
            _ => return EventState::NotConsumed,
        }
        EventState::Consumed
    }

    fn name_style(entry: &BranchEntry, theme: &Theme) -> Style {
        if entry.is_head {
            theme.label_local
        } else if entry.local.is_none() {
            theme.label_remote
        } else {
            Style::new()
        }
    }

    fn marker(entry: &BranchEntry, theme: &Theme) -> Span<'static> {
        if entry.is_head {
            Span::styled("* ", theme.label_local)
        } else {
            Span::raw("  ")
        }
    }

    fn render_entry<'a>(&self, entry: &'a BranchEntry, width: usize, theme: &Theme) -> Line<'a> {
        let (state, state_style) = state_text(&entry.state(), theme);
        let name_width = width.saturating_sub(2 + 1 + STATE_WIDTH + 1 + DATE_WIDTH);
        Line::from(vec![
            Self::marker(entry, theme),
            Span::styled(fit(&entry.name, name_width), Self::name_style(entry, theme)),
            Span::raw(" "),
            Span::styled(fit(&state, STATE_WIDTH), state_style),
            Span::raw(" "),
            Span::styled(
                fit_right(&relative(self.clock.now(), entry.activity), DATE_WIDTH),
                theme.date,
            ),
        ])
    }

    /// A merged/unmerged row: name, tip subject, author and commit date,
    /// like a `git branch -r --no-merged` alias prints them.
    fn render_compared<'a>(
        &'a self,
        entry: &'a BranchEntry,
        name_width: usize,
        width: usize,
        theme: &Theme,
    ) -> Line<'a> {
        let tip = BaseBranches::tip(entry);
        let author = tip
            .and_then(|t| self.base.as_ref()?.authors.get(&t.full_name))
            .map_or("", String::as_str);
        let mut spans = vec![
            Self::marker(entry, theme),
            Span::styled(fit(&entry.name, name_width), Self::name_style(entry, theme)),
        ];
        let rest = width.saturating_sub(2 + name_width + 1 + DATE_WIDTH);
        let mut author_width = AUTHOR_WIDTH.min(rest.saturating_sub(1));
        let subject_width = rest.saturating_sub(author_width + 2);
        if subject_width < SUBJECT_MIN_WIDTH {
            author_width = rest.saturating_sub(1);
        } else {
            spans.push(Span::raw(" "));
            spans.push(Span::raw(fit(tip.map_or("", |t| t.subject.as_str()), subject_width)));
        }
        if author_width > 0 {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(fit(author, author_width), theme.muted));
        }
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            fit_right(&relative(self.clock.now(), entry.activity), DATE_WIDTH),
            theme.date,
        ));
        Line::from(spans)
    }

    /// How far a tag is from the base: `↓12` when the base has moved on 12
    /// commits, `↑3` for commits the base doesn't have, `✓` at its tip.
    /// Only the newest tags are compared.
    fn tag_distance(&self, tag: &GitRef, theme: &Theme) -> Option<(String, Style)> {
        let d = self.base.as_ref()?.tags.get(&tag.full_name)?;
        let state = if d.ahead == 0 && d.behind == 0 {
            BranchState::InSync
        } else {
            BranchState::Diverged {
                ahead: d.ahead,
                behind: d.behind,
            }
        };
        Some(state_text(&state, theme))
    }

    /// Width of the tag distance column: the widest of the compared tags.
    fn tag_distance_width(&self, theme: &Theme) -> usize {
        self.tags
            .iter()
            .filter_map(|t| self.tag_distance(t, theme))
            .map(|(text, _)| text.width())
            .max()
            .unwrap_or(0)
    }

    /// A tag row: name, distance from the base, subject (the tag message's
    /// for annotated tags, else the commit's) and date.
    fn render_tag<'a>(
        &self,
        tag: &'a GitRef,
        name_width: usize,
        distance_width: usize,
        width: usize,
        theme: &Theme,
    ) -> Line<'a> {
        let mut spans = vec![
            Span::raw("  "),
            Span::styled(fit(&tag.short_name, name_width), theme.label_tag),
        ];
        let mut used = 2 + name_width;
        if distance_width > 0 {
            let (text, style) = self.tag_distance(tag, theme).unwrap_or_default();
            spans.push(Span::raw(" "));
            spans.push(Span::styled(fit(&text, distance_width), style));
            used += 1 + distance_width;
        }
        let subject_width = width.saturating_sub(used + 1 + 1 + DATE_WIDTH);
        if subject_width > 0 {
            spans.push(Span::raw(" "));
            spans.push(Span::raw(fit(&tag.subject, subject_width)));
        }
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            fit_right(&relative(self.clock.now(), tag.date), DATE_WIDTH),
            theme.date,
        ));
        Line::from(spans)
    }

    /// Name column width for tag and merged/unmerged rows: the longest name on
    /// screen, up to half the row if that leaves room for subjects, else
    /// everything but the author column.
    fn compared_name_width(&self, rows: &[usize], width: usize) -> usize {
        let room = width.saturating_sub(2 + 1 + DATE_WIDTH);
        let longest = rows.iter().map(|&i| self.row_name(i).width()).max().unwrap_or(0).max(8);
        let with_subject = longest.min(room / 2);
        if room.saturating_sub(with_subject + 1 + AUTHOR_WIDTH + 1) >= SUBJECT_MIN_WIDTH {
            with_subject
        } else {
            longest.min(room.saturating_sub(AUTHOR_WIDTH + 1).max(room / 2))
        }
    }

    /// `Branches 12 · Tags 3 · Unmerged 4 · Merged 9` with the current tab
    /// underlined, then the base everywhere but the full list.
    fn title(&self, theme: &Theme) -> Line<'static> {
        let mut spans = Vec::new();
        for tab in BranchTab::ALL {
            if !spans.is_empty() {
                spans.push(Span::styled(" · ", theme.muted));
            }
            let count = if tab.compares() && self.base.is_none() {
                String::new()
            } else {
                format!(" {}", self.len(tab))
            };
            let style = if tab == self.tab {
                Style::new().add_modifier(Modifier::UNDERLINED)
            } else {
                theme.muted
            };
            spans.push(Span::styled(format!("{}{count}", tab.label()), style));
        }
        if self.tab != BranchTab::All
            && let Some(base) = &self.base
        {
            spans.push(Span::styled(format!(" ({})", base.base), theme.muted));
        }
        if self.editing_filter || !self.filter.is_empty() {
            let cursor = if self.editing_filter { "█" } else { "" };
            spans.push(Span::raw(format!(" /{}{cursor}", self.filter)));
        }
        Line::from(spans)
    }

    fn empty_message(&self) -> String {
        let base = self.base.as_ref().map(|b| b.base.as_str());
        match (self.tab, base) {
            _ if self.len(self.tab) > 0 => "no matches".into(),
            (BranchTab::All, _) => "no branches".into(),
            (BranchTab::Tags, _) => "no tags".into(),
            (_, None) => "no base branch found; pick one".into(),
            (BranchTab::Unmerged, Some(base)) => format!("everything is merged into {base}"),
            (BranchTab::Merged, Some(base)) => format!("nothing is merged into {base} yet"),
        }
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
        let len = self.visible.len();
        if self.cursor_mut().apply(cmd, len) {
            self.announce(ctx);
            return EventState::Consumed;
        }
        match cmd {
            Command::Filter => self.editing_filter = true,
            Command::Fetch => ctx.emit(Action::Run(GitOp::FetchAll)),
            Command::NextTab => self.switch_tab(1, ctx),
            Command::PrevTab => self.switch_tab(BranchTab::ALL.len() - 1, ctx),
            Command::PickBase => ctx.emit(Action::OpenModal(self.base_picker())),
            _ if self.tab == BranchTab::Tags => return self.on_tag_command(cmd, ctx),
            _ => return self.on_entry_command(cmd, ctx),
        }
        EventState::Consumed
    }

    fn commands(&self) -> &[Command] {
        &[
            Command::Checkout,
            Command::NextTab,
            Command::PrevTab,
            Command::PickBase,
            Command::NewBranch,
            Command::DeleteBranch,
            Command::RenameBranch,
            Command::Fetch,
            Command::Filter,
            Command::Merge,
            Command::Rebase,
            Command::Copy,
        ]
    }

    fn on_click(&mut self, row: usize, ctx: &Ctx) {
        let len = self.visible.len();
        if self.cursor_mut().click(row, len) {
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
                let len = self.visible.len();
                self.cursor_mut().apply(cmd, len);
                self.announce(ctx);
            }
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.cursor_mut().selected = 0;
                self.refilter();
            }
            _ => {}
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
        let block = pane_block(PaneId::Branches, self.title(theme), focused, theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if self.visible.is_empty() {
            frame.render_widget(Paragraph::new(self.empty_message()).style(theme.muted), inner);
            return;
        }
        let height = inner.height as usize;
        self.cursor_mut().scroll_into_view(height);
        let cursor = *self.cursor();
        let start = cursor.offset.min(self.visible.len());
        let end = (start + height).min(self.visible.len());
        let width = inner.width as usize;
        let list = self.list(self.tab);
        let name_width = self.compared_name_width(&self.visible[start..end], width);
        let distance_width = self.tag_distance_width(theme);
        let lines: Vec<Line> = (start..end)
            .map(|pos| {
                let i = self.visible[pos];
                let line = match self.tab {
                    BranchTab::All => self.render_entry(&list[i], width, theme),
                    BranchTab::Tags => self.render_tag(&self.tags[i], name_width, distance_width, width, theme),
                    _ => self.render_compared(&list[i], name_width, width, theme),
                };
                if pos == cursor.selected {
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
