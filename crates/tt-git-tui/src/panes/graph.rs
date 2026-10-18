use std::collections::HashMap;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tt_git_core::time::{SharedClock, relative};
use tt_git_core::{Commit, GitRef, Head, RefKind};
use tt_git_graph::{GraphBuilder, NodeId, Row};
use unicode_width::UnicodeWidthStr;

use super::{PaneId, pane_block};
use crate::action::{Action, RefsSnapshot};
use crate::component::{Component, EventState};
use crate::jobs::Ctx;
use crate::keymap::{Command, Context};
use crate::modal::{GitOp, Modal, PromptAction};
use crate::theme::Theme;
use crate::widgets::{Cursor, fit, fit_right};

/// Which refs the graph shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GraphScope {
    /// `git log --all`
    #[default]
    All,
    /// Only the checked-out branch (and its upstream).
    Current,
}

impl GraphScope {
    pub fn label(self) -> &'static str {
        match self {
            GraphScope::All => "all",
            GraphScope::Current => "current",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            GraphScope::All => GraphScope::Current,
            GraphScope::Current => GraphScope::All,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelKind {
    /// The checked-out branch.
    Head,
    /// HEAD when detached.
    DetachedHead,
    Local,
    Remote,
    Tag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefLabel {
    pub name: String,
    pub kind: LabelKind,
}

const AUTHOR_WIDTH: usize = 16;
const DATE_WIDTH: usize = 4;
const SHA_WIDTH: usize = 7;
/// Load more commits when the selection gets this close to the end.
const LOAD_MORE_MARGIN: usize = 200;

/// The primary view: commit graph of all (or the current) branches.
#[derive(Debug)]
pub struct GraphPane {
    pub scope: GraphScope,
    /// Maximum number of lanes drawn before collapsing into `…`.
    pub max_lanes: usize,
    /// Commit limit of the current load; reaching it means more may exist.
    pub limit: usize,
    clock: SharedClock,
    commits: Vec<Commit>,
    rows: Vec<Row>,
    builder: GraphBuilder,
    ids: HashMap<String, NodeId>,
    labels: HashMap<String, Vec<RefLabel>>,
    cursor: Cursor,
    generation: u64,
    loading: bool,
    /// The rows shown are from the previous load and will be replaced by the
    /// first batch of the current one (so reloads don't flash empty).
    stale: bool,
    /// Commit to reselect once a reload finishes.
    reselect: Option<String>,
    /// Last commit announced through `CommitSelected`.
    announced: Option<String>,
}

impl GraphPane {
    pub fn new(clock: SharedClock) -> Self {
        Self {
            scope: GraphScope::All,
            max_lanes: 16,
            limit: usize::MAX,
            clock,
            commits: Vec::new(),
            rows: Vec::new(),
            builder: GraphBuilder::new(),
            ids: HashMap::new(),
            labels: HashMap::new(),
            cursor: Cursor::default(),
            generation: 0,
            loading: false,
            stale: false,
            reselect: None,
            announced: None,
        }
    }

    pub fn commits(&self) -> &[Commit] {
        &self.commits
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn selected(&self) -> Option<&Commit> {
        self.commits.get(self.cursor.selected)
    }

    pub fn selected_index(&self) -> usize {
        self.cursor.selected
    }

    /// Starts a new load and returns the generation incoming batches must
    /// carry. The current rows stay visible until the first batch arrives.
    pub fn begin_load(&mut self) -> u64 {
        self.generation += 1;
        if self.reselect.is_none() {
            self.reselect = self.selected().map(|c| c.oid.clone());
        }
        self.stale = true;
        self.loading = true;
        self.generation
    }

    fn clear(&mut self) {
        self.commits.clear();
        self.rows.clear();
        self.ids.clear();
        self.builder = GraphBuilder::new();
    }

    /// Selects `oid` now if it is loaded, otherwise once the running load
    /// finishes.
    pub fn select_when_loaded(&mut self, oid: String, ctx: &Ctx) {
        if self.loading || !self.select_oid(&oid, ctx) {
            self.reselect = Some(oid);
        }
    }

    /// Appends a batch from load `generation`; stale batches are ignored.
    pub fn apply_batch(&mut self, generation: u64, commits: Vec<Commit>, done: bool, ctx: &Ctx) {
        if generation != self.generation {
            return;
        }
        if std::mem::take(&mut self.stale) {
            self.clear();
        }
        for commit in commits {
            let id = intern(&mut self.ids, &commit.oid);
            let parents: Vec<NodeId> = commit.parents.iter().map(|p| intern(&mut self.ids, p)).collect();
            self.rows.push(self.builder.push(id, &parents));
            self.commits.push(commit);
        }
        if done {
            self.loading = false;
            self.cursor.selected = self
                .reselect
                .take()
                .and_then(|oid| self.commits.iter().position(|c| c.oid == oid))
                .unwrap_or(0);
            self.cursor.offset = self.cursor.selected.saturating_sub(self.cursor.height / 2);
        }
        self.cursor.clamp(self.commits.len());
        // Only announce once the selection is final, so details don't churn
        // through intermediate commits while batches stream in.
        if done {
            self.announce(ctx);
        }
    }

    pub fn load_failed(&mut self, generation: u64) {
        if generation == self.generation {
            self.loading = false;
            self.stale = false;
        }
    }

    /// Rebuilds ref labels (badges) from a refs snapshot.
    pub fn set_refs(&mut self, snapshot: &RefsSnapshot) {
        self.labels = build_labels(&snapshot.head, &snapshot.refs);
    }

    /// Moves the selection to `oid` if it is loaded.
    pub fn select_oid(&mut self, oid: &str, ctx: &Ctx) -> bool {
        match self.commits.iter().position(|c| c.oid == oid) {
            Some(i) => {
                self.cursor.selected = i;
                self.cursor.offset = i.saturating_sub(self.cursor.height / 3);
                self.announce(ctx);
                true
            }
            None => false,
        }
    }

    /// A name for the selected commit: its first branch label, else the hash.
    pub fn selected_ref(&self) -> Option<String> {
        let c = self.selected()?;
        let branch = self.labels.get(&c.oid).and_then(|ls| {
            ls.iter()
                .find(|l| matches!(l.kind, LabelKind::Local | LabelKind::Head | LabelKind::Remote))
                .map(|l| l.name.clone())
        });
        Some(branch.unwrap_or_else(|| c.oid.clone()))
    }

    fn announce(&mut self, ctx: &Ctx) {
        let current = self.selected().map(|c| c.oid.clone());
        if current != self.announced {
            self.announced = current.clone();
            if let Some(oid) = current {
                ctx.emit(Action::CommitSelected(oid));
            }
        }
    }

    fn title(&self) -> String {
        let mut t = format!("Graph [{}]", self.scope.label());
        if !self.commits.is_empty() {
            t.push_str(&format!(" {}/{}", self.cursor.selected + 1, self.commits.len()));
        }
        if self.loading {
            t.push_str(" loading…");
        }
        t
    }

    fn render_row<'a>(&self, idx: usize, lanes: usize, width: usize, theme: &Theme) -> Line<'a> {
        let commit = &self.commits[idx];
        let row = &self.rows[idx];
        let mut spans: Vec<Span<'a>> = Vec::new();

        // Graph columns.
        let node_glyph = if row.is_merge { '◉' } else { '●' };
        let overflow = row.cells.len() > lanes;
        let shown = if overflow {
            lanes.saturating_sub(1)
        } else {
            row.cells.len()
        };
        for i in 0..shown {
            let cell = row.cells[i];
            spans.push(Span::styled(cell.glyph(node_glyph).to_string(), theme.lane(cell.color)));
            let filler = match row.fillers[i] {
                Some(c) if i + 1 < shown || overflow => Span::styled("─", theme.lane(c)),
                _ => Span::raw(" "),
            };
            spans.push(filler);
        }
        if overflow {
            let glyph = if row.node_lane >= shown { node_glyph } else { '…' };
            spans.push(Span::styled(format!("{glyph} "), theme.lane(row.node_color)));
        }
        let graph_used = (shown + usize::from(overflow)) * 2;
        spans.push(Span::raw(" ".repeat((lanes * 2).saturating_sub(graph_used) + 1)));

        // Metadata columns, dropped from the right when narrow.
        let graph_cols = lanes * 2 + 1;
        let meta_full = 1 + AUTHOR_WIDTH + 1 + DATE_WIDTH + 1 + SHA_WIDTH;
        let meta_small = 1 + DATE_WIDTH + 1 + SHA_WIDTH;
        let rest = width.saturating_sub(graph_cols);
        let (show_author, meta) = if rest >= meta_full + 20 {
            (true, meta_full)
        } else if rest >= meta_small + 12 {
            (false, meta_small)
        } else {
            (false, 0)
        };
        let mut text_width = rest.saturating_sub(meta);

        // Ref labels, then the summary.
        if let Some(labels) = self.labels.get(&commit.oid) {
            for label in labels {
                let (text, style) = label_span(label, theme);
                let w = text.width() + 1;
                if w > text_width {
                    break;
                }
                text_width -= w;
                spans.push(Span::styled(text, style));
                spans.push(Span::raw(" "));
            }
        }
        spans.push(Span::raw(fit(&commit.summary, text_width)));

        if meta > 0 {
            if show_author {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(fit(&commit.author_name, AUTHOR_WIDTH), theme.author));
            }
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                fit_right(&relative(self.clock.now(), commit.commit_time), DATE_WIDTH),
                theme.date,
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(commit.short_oid().to_string(), theme.sha));
        }
        Line::from(spans)
    }
}

fn intern(ids: &mut HashMap<String, NodeId>, oid: &str) -> NodeId {
    if let Some(id) = ids.get(oid) {
        return *id;
    }
    let id = ids.len() as NodeId;
    ids.insert(oid.to_string(), id);
    id
}

fn label_span(label: &RefLabel, theme: &Theme) -> (String, Style) {
    match label.kind {
        LabelKind::Head => (format!(" {} ", label.name), theme.label_head),
        LabelKind::DetachedHead => (" HEAD ".to_string(), theme.label_detached),
        LabelKind::Local => (format!("[{}]", label.name), theme.label_local),
        LabelKind::Remote => (format!("[{}]", label.name), theme.label_remote),
        LabelKind::Tag => (format!("<{}>", label.name), theme.label_tag),
    }
}

/// Labels per commit: HEAD first, then local branches, remotes and tags,
/// each alphabetical.
pub fn build_labels(head: &Head, refs: &[GitRef]) -> HashMap<String, Vec<RefLabel>> {
    let mut labels: HashMap<String, Vec<RefLabel>> = HashMap::new();
    if let Head::Detached { oid } = head {
        labels.entry(oid.clone()).or_default().push(RefLabel {
            name: "HEAD".into(),
            kind: LabelKind::DetachedHead,
        });
    }
    let head_branch = head.branch();
    for r in refs.iter().filter(|r| r.symref.is_none()) {
        let kind = match &r.kind {
            RefKind::LocalBranch if Some(r.short_name.as_str()) == head_branch => LabelKind::Head,
            RefKind::LocalBranch => LabelKind::Local,
            RefKind::RemoteBranch { .. } => LabelKind::Remote,
            RefKind::Tag => LabelKind::Tag,
            RefKind::Other => continue,
        };
        labels.entry(r.target.clone()).or_default().push(RefLabel {
            name: r.short_name.clone(),
            kind,
        });
    }
    for list in labels.values_mut() {
        list.sort_by(|a, b| rank(a.kind).cmp(&rank(b.kind)).then_with(|| a.name.cmp(&b.name)));
    }
    labels
}

fn rank(kind: LabelKind) -> u8 {
    match kind {
        LabelKind::DetachedHead | LabelKind::Head => 0,
        LabelKind::Local => 1,
        LabelKind::Remote => 2,
        LabelKind::Tag => 3,
    }
}

impl Component for GraphPane {
    fn context(&self) -> Context {
        Context::Graph
    }

    fn on_command(&mut self, cmd: Command, ctx: &Ctx) -> EventState {
        if self.cursor.apply(cmd, self.commits.len()) {
            self.announce(ctx);
            let near_end = self.cursor.selected + LOAD_MORE_MARGIN >= self.commits.len();
            if near_end && !self.loading && self.commits.len() >= self.limit {
                ctx.emit(Action::LoadMoreCommits);
            }
            return EventState::Consumed;
        }
        match cmd {
            Command::ToggleScope => {
                self.scope = self.scope.toggled();
                ctx.emit(Action::ReloadGraph);
                EventState::Consumed
            }
            Command::Merge | Command::Rebase => {
                if let Some(target) = self.selected_ref() {
                    let short = if target.len() == 40 {
                        target[..7].to_string()
                    } else {
                        target.clone()
                    };
                    let (title, msg, op) = if cmd == Command::Merge {
                        (
                            "Merge",
                            format!("Merge {short} into the current branch?"),
                            GitOp::Merge { target },
                        )
                    } else {
                        (
                            "Rebase",
                            format!("Rebase the current branch onto {short}?"),
                            GitOp::Rebase { onto: target },
                        )
                    };
                    ctx.emit(Action::OpenModal(Modal::confirm(title, msg, op)));
                }
                EventState::Consumed
            }
            Command::CherryPick => {
                if let Some(c) = self.selected() {
                    ctx.emit(Action::OpenModal(Modal::confirm(
                        "Cherry-pick",
                        format!("Apply {} \"{}\" onto the current branch?", c.short_oid(), c.summary),
                        GitOp::CherryPick {
                            oid: c.oid.clone(),
                            summary: c.summary.clone(),
                        },
                    )));
                }
                EventState::Consumed
            }
            Command::NewBranch => {
                if let Some(c) = self.selected() {
                    ctx.emit(Action::OpenModal(Modal::prompt(
                        format!("New branch at {}", c.short_oid()),
                        "",
                        PromptAction::CreateBranch { start: c.oid.clone() },
                    )));
                }
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn on_click(&mut self, row: usize, ctx: &Ctx) {
        if self.cursor.click(row, self.commits.len()) {
            self.announce(ctx);
        }
    }

    fn commands(&self) -> &[Command] {
        &[
            Command::ToggleDetails,
            Command::ExpandDetails,
            Command::ToggleScope,
            Command::NewBranch,
            Command::Merge,
            Command::Rebase,
            Command::CherryPick,
        ]
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
        let block = pane_block(PaneId::Graph, self.title(), focused, theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.commits.is_empty() {
            let msg = if self.loading { "loading…" } else { "no commits" };
            frame.render_widget(Paragraph::new(msg).style(theme.muted), inner);
            return;
        }

        let height = inner.height as usize;
        self.cursor.scroll_into_view(height);
        let start = self.cursor.offset.min(self.commits.len());
        let end = (start + height).min(self.commits.len());
        let lanes = self.rows[start..end]
            .iter()
            .map(Row::width)
            .max()
            .unwrap_or(1)
            .clamp(1, self.max_lanes.max(2));
        let width = inner.width as usize;

        let lines: Vec<Line> = (start..end)
            .map(|i| {
                let line = self.render_row(i, lanes, width, theme);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn r(full: &str, target: &str) -> GitRef {
        let (kind, short) = if let Some(s) = full.strip_prefix("refs/heads/") {
            (RefKind::LocalBranch, s)
        } else if let Some(s) = full.strip_prefix("refs/remotes/") {
            (
                RefKind::RemoteBranch {
                    remote: "origin".into(),
                },
                s,
            )
        } else {
            (RefKind::Tag, full.strip_prefix("refs/tags/").unwrap())
        };
        GitRef {
            full_name: full.into(),
            short_name: short.into(),
            kind,
            target: target.into(),
            date: 0,
            upstream: None,
            ahead: 0,
            behind: 0,
            upstream_gone: false,
            is_head: false,
            symref: None,
            subject: String::new(),
        }
    }

    #[test]
    fn labels_are_grouped_and_ordered() {
        let head = Head::Branch {
            name: "main".into(),
            oid: "a".into(),
        };
        let mut origin_head = r("refs/remotes/origin/HEAD", "a");
        origin_head.symref = Some("refs/remotes/origin/main".into());
        let refs = vec![
            r("refs/tags/v1", "a"),
            r("refs/remotes/origin/main", "a"),
            origin_head,
            r("refs/heads/zeta", "a"),
            r("refs/heads/main", "a"),
            r("refs/heads/other", "b"),
        ];
        let labels = build_labels(&head, &refs);
        let a: Vec<(&str, LabelKind)> = labels["a"].iter().map(|l| (l.name.as_str(), l.kind)).collect();
        assert_eq!(
            a,
            vec![
                ("main", LabelKind::Head),
                ("zeta", LabelKind::Local),
                ("origin/main", LabelKind::Remote),
                ("v1", LabelKind::Tag),
            ]
        );
        assert_eq!(labels["b"][0].kind, LabelKind::Local);
    }

    #[test]
    fn detached_head_label() {
        let labels = build_labels(&Head::Detached { oid: "c".into() }, &[]);
        assert_eq!(labels["c"][0].kind, LabelKind::DetachedHead);
    }
}
