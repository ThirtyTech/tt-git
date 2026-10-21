use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tt_git_core::time::{SharedClock, clock_time, relative};
use tt_git_core::{BranchHistory, CheckoutPlan, Commit, GitRef, Head, LogFilter, Oid, RefKind};
use tt_git_graph::{GraphBuilder, NodeId, RowRef, Rows};
use unicode_width::UnicodeWidthStr;

use super::{DrawnDates, PaneId, pane_block};
use crate::action::{Action, RefsSnapshot};
use crate::component::{Component, EventState};
use crate::filter::author_name;
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

/// How the graph's date column reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateStyle {
    /// Compact age: `5m`, `3h`, `2d`.
    #[default]
    Relative,
    /// Local date and 12-hour time: `2026-10-17 01:30 PM`.
    Clock,
}

impl DateStyle {
    pub fn toggled(self) -> Self {
        match self {
            DateStyle::Relative => DateStyle::Clock,
            DateStyle::Clock => DateStyle::Relative,
        }
    }

    pub(crate) fn width(self) -> usize {
        match self {
            DateStyle::Relative => 4,
            DateStyle::Clock => 19,
        }
    }
}

/// A branch shown on its own, from the branches pane's merged and unmerged
/// tabs: its commits between where it branched off the base and where it
/// was merged back in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchFocus {
    /// Short name, e.g. `origin/feat`.
    pub branch: String,
    /// Its tip commit, selected when the view opens.
    pub tip: String,
    /// Short name of the base branch, e.g. `origin/main`.
    pub base: String,
    /// The base's full ref name.
    pub base_ref: String,
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

/// One graph load, as handed to its background job: batches carry its
/// `generation`, and the job can ask whether a newer load replaced it.
#[derive(Debug, Clone)]
pub struct LoadToken {
    generation: u64,
    current: Arc<AtomicU64>,
    tips: SharedTips,
}

/// What the load with this generation walked from, as
/// [`loaders::graph_tips`](crate::loaders::graph_tips) describes it.
type SharedTips = Arc<Mutex<Option<(u64, Vec<u8>)>>>;

impl LoadToken {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// False once a newer load started; its results would be dropped.
    pub fn is_current(&self) -> bool {
        self.current.load(Ordering::Relaxed) == self.generation
    }

    /// Records what this load walks from, taken before it started reading
    /// refs, or `None` when that's unknown (it failed).
    pub fn set_tips(&self, tips: Option<Vec<u8>>) {
        let mut slot = self.tips.lock().unwrap_or_else(|e| e.into_inner());
        *slot = tips.map(|t| (self.generation, t));
    }
}

const AUTHOR_WIDTH: usize = 16;
const SHA_WIDTH: usize = 7;
/// Load more commits when the selection gets this close to the end.
const LOAD_MORE_MARGIN: usize = 200;

/// The primary view: commit graph of all (or the current) branches.
#[derive(Debug)]
pub struct GraphPane {
    pub scope: GraphScope,
    /// Shows only this branch's history instead of `scope`.
    pub focus: Option<BranchFocus>,
    /// Narrows `scope` to some commits (not used with a `focus`).
    pub filter: LogFilter,
    /// Maximum number of lanes drawn before collapsing into `…`.
    pub max_lanes: usize,
    pub dates: DateStyle,
    /// Commit limit of the current load; reaching it means more may exist.
    pub limit: usize,
    clock: SharedClock,
    commits: Vec<Commit>,
    /// One per commit, keeping only the columns drawn (`max_lanes`, as it
    /// was when the load began).
    rows: Rows,
    builder: GraphBuilder,
    /// Node ids of the commits (and parents) the running load has pushed
    /// into `builder`; emptied once it's done, since only loads use it.
    ids: HashMap<Oid, NodeId>,
    labels: HashMap<String, Vec<RefLabel>>,
    /// Notes on the focused branch's fork and merge commits.
    notes: HashMap<Oid, String>,
    cursor: Cursor,
    generation: u64,
    /// `generation`, shared with the running load's job.
    current: Arc<AtomicU64>,
    /// Set by the load's job; see [`tips`](Self::tips).
    tips: SharedTips,
    loading: bool,
    /// The rows shown are from the previous load and will be replaced by the
    /// first batch of the current one (so reloads don't flash empty).
    stale: bool,
    /// Commit to reselect once a reload finishes.
    reselect: Option<String>,
    /// Last commit announced through `CommitSelected`.
    announced: Option<Oid>,
    drawn_dates: DrawnDates,
}

impl GraphPane {
    pub fn new(clock: SharedClock) -> Self {
        Self {
            scope: GraphScope::All,
            focus: None,
            filter: LogFilter::default(),
            max_lanes: 16,
            dates: DateStyle::Relative,
            limit: usize::MAX,
            clock,
            commits: Vec::new(),
            rows: Rows::new(16),
            builder: GraphBuilder::new(),
            ids: HashMap::new(),
            labels: HashMap::new(),
            notes: HashMap::new(),
            cursor: Cursor::default(),
            generation: 0,
            current: Arc::new(AtomicU64::new(0)),
            tips: SharedTips::default(),
            loading: false,
            stale: false,
            reselect: None,
            announced: None,
            drawn_dates: DrawnDates::default(),
        }
    }

    pub fn commits(&self) -> &[Commit] {
        &self.commits
    }

    pub fn rows(&self) -> &Rows {
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

    /// Whether the `row`-th body line drawn last frame is a commit.
    pub fn has_commit_at_row(&self, row: usize) -> bool {
        self.cursor.offset + row < self.commits.len()
    }

    /// Starts a new load and returns its token, whose generation incoming
    /// batches must carry. The current rows stay visible until the first
    /// batch arrives.
    pub fn begin_load(&mut self) -> LoadToken {
        self.generation += 1;
        self.current.store(self.generation, Ordering::Relaxed);
        if self.reselect.is_none() {
            self.reselect = self.selected().map(|c| c.oid.to_string());
        }
        self.stale = true;
        self.loading = true;
        LoadToken {
            generation: self.generation,
            current: self.current.clone(),
            tips: self.tips.clone(),
        }
    }

    /// Starts a load that continues the current one past its limit,
    /// keeping its rows, if it can: the current load has finished, listed
    /// the whole graph unfiltered up to its limit and noted what it walked
    /// from. Returns the token, how many commits are shown, and those tips;
    /// the job must check they still hold (see
    /// [`loaders::load_graph`](crate::loaders::load_graph)), or else send
    /// [`Action::GraphRestarted`] and list every commit. The rows end up as
    /// a fresh load with the larger limit would leave them, since a log with
    /// a smaller limit lists the first of the same commits.
    pub fn begin_more(&mut self) -> Option<(LoadToken, usize, Vec<u8>)> {
        if self.loading
            || self.stale
            || self.focus.is_some()
            || !self.filter.is_empty()
            || self.commits.len() != self.limit
            || self.rows.len() != self.commits.len()
        {
            return None;
        }
        let tips = {
            let slot = self.tips.lock().unwrap_or_else(|e| e.into_inner());
            match &*slot {
                Some((generation, tips)) if *generation == self.generation => tips.clone(),
                _ => return None,
            }
        };
        // The node ids the load gave, so the builder's open lanes lead to
        // the commits still to come.
        self.ids = HashMap::with_capacity(self.commits.len() + self.commits.len() / 4);
        for commit in &self.commits {
            intern(&mut self.ids, commit.oid);
            for parent in commit.parents.iter() {
                intern(&mut self.ids, *parent);
            }
        }
        let skip = self.commits.len();
        let load = self.begin_load();
        self.stale = false;
        Some((load, skip, tips))
    }

    /// Load `generation`, begun by [`begin_more`](Self::begin_more), lists
    /// every commit after all: its first batch replaces the rows.
    pub fn restart(&mut self, generation: u64) {
        if generation == self.generation && self.loading {
            self.stale = true;
        }
    }

    /// The latest load's generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether the current load's job has recorded what it walks from.
    pub fn has_tips(&self) -> bool {
        self.recorded_tips(|_| true)
    }

    /// Whether `tips` is what the current load walks from: if so, a reload
    /// would list the same commits.
    pub fn same_tips(&self, tips: &[u8]) -> bool {
        self.recorded_tips(|t| t == tips)
    }

    fn recorded_tips(&self, check: impl FnOnce(&[u8]) -> bool) -> bool {
        let slot = self.tips.lock().unwrap_or_else(|e| e.into_inner());
        matches!(&*slot, Some((generation, tips)) if *generation == self.generation && check(tips))
    }

    /// Does what a reload that found the same commits would: reselect
    /// (normally the same commit) and centre on it. Not while a load is
    /// running, since its end does this anyway.
    pub fn reload_unchanged(&mut self, ctx: &Ctx) {
        if self.loading {
            return;
        }
        let reselect = self
            .reselect
            .take()
            .or_else(|| self.selected().map(|c| c.oid.to_string()));
        self.cursor.selected = reselect.and_then(|oid| self.position(&oid)).unwrap_or(0);
        self.cursor.offset = self.cursor.selected.saturating_sub(self.cursor.height / 2);
        self.cursor.clamp(self.commits.len());
        self.announce(ctx);
    }

    /// Whether a relative date drawn last frame would read differently at `now`.
    pub fn dates_stale(&self, now: i64) -> bool {
        self.drawn_dates.stale(now)
    }

    fn clear(&mut self) {
        self.commits.clear();
        // Drawing never shows more columns than this.
        self.rows.reset(self.max_lanes.max(2));
        self.ids.clear();
        self.notes.clear();
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
        let mut parents: Vec<NodeId> = Vec::new();
        for commit in commits {
            let id = intern(&mut self.ids, commit.oid);
            parents.clear();
            parents.extend(commit.parents.iter().map(|p| intern(&mut self.ids, *p)));
            self.rows.push(&self.builder.push(id, &parents));
            self.commits.push(commit);
        }
        if done {
            self.loading = false;
            self.ids = HashMap::new();
            self.cursor.selected = self.reselect.take().and_then(|oid| self.position(&oid)).unwrap_or(0);
            self.cursor.offset = self.cursor.selected.saturating_sub(self.cursor.height / 2);
        }
        self.cursor.clamp(self.commits.len());
        // Only announce once the selection is final, so details don't churn
        // through intermediate commits while batches stream in.
        if done {
            self.announce(ctx);
        }
    }

    /// Shows a focused branch's history from load `generation`.
    pub fn apply_history(&mut self, generation: u64, history: BranchHistory, ctx: &Ctx) {
        if generation != self.generation {
            return;
        }
        self.apply_batch(generation, history.commits, true, ctx);
        if let Some(focus) = &self.focus {
            if let Some(fork) = history.fork {
                self.notes.insert(fork, format!("branched off {}", focus.base));
            }
            if let Some(merge) = history.merge {
                self.notes.insert(merge, format!("merged into {}", focus.base));
            }
        }
    }

    /// What the graph shows, for the title and plugins: `all`, `current`,
    /// or the focused branch.
    pub fn scope_label(&self) -> &str {
        match &self.focus {
            Some(focus) => &focus.branch,
            None => self.scope.label(),
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

    /// Where the commit whose id reads `oid` is in the list.
    fn position(&self, oid: &str) -> Option<usize> {
        // Only canonical text parses, so this matches exactly the commits
        // whose id reads `oid`.
        let oid = Oid::from_hex(oid)?;
        self.commits.iter().position(|c| c.oid == oid)
    }

    /// Moves the selection to `oid` if it is loaded.
    pub fn select_oid(&mut self, oid: &str, ctx: &Ctx) -> bool {
        match self.position(oid) {
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
        let branch = self.labels.get(c.oid.hex().as_str()).and_then(|ls| {
            ls.iter()
                .find(|l| matches!(l.kind, LabelKind::Local | LabelKind::Head | LabelKind::Remote))
                .map(|l| l.name.clone())
        });
        Some(branch.unwrap_or_else(|| c.oid.to_string()))
    }

    /// Checks out the selected commit: the local branch pointing at it if
    /// there is one, else the commit itself with HEAD detached.
    fn checkout(&self, ctx: &Ctx) {
        let Some(c) = self.selected() else {
            return;
        };
        let labels = self.labels.get(c.oid.hex().as_str()).map_or(&[][..], Vec::as_slice);
        let of_kind = |kind| labels.iter().find(|l| l.kind == kind);
        if let Some(head) = of_kind(LabelKind::Head) {
            ctx.emit(Action::Notify(format!("already on {}", head.name)));
        } else if let Some(local) = of_kind(LabelKind::Local) {
            ctx.emit(Action::Checkout(CheckoutPlan::Local(local.name.clone())));
        } else if of_kind(LabelKind::DetachedHead).is_some() {
            ctx.emit(Action::Notify(format!("HEAD is already at {}", c.short_oid())));
        } else {
            ctx.emit(Action::Checkout(CheckoutPlan::Detach(c.oid.to_string())));
        }
    }

    fn announce(&mut self, ctx: &Ctx) {
        let current = self.selected().map(|c| c.oid);
        if current != self.announced {
            self.announced = current;
            if let Some(oid) = current {
                ctx.emit(Action::CommitSelected(oid.to_string()));
            }
        }
    }

    /// Whether the graph is narrowed by its filter.
    pub fn is_filtered(&self) -> bool {
        self.focus.is_none() && !self.filter.is_empty()
    }

    fn title(&self) -> String {
        let mut t = format!("Graph [{}", self.scope_label());
        if self.is_filtered() {
            if let Some(author) = &self.filter.author {
                t.push_str(&format!(" · author {}", author_name(author)));
            }
            if let Some(path) = &self.filter.path {
                t.push_str(&format!(" · file {path}"));
            }
        }
        t.push(']');
        if !self.commits.is_empty() {
            t.push_str(&format!(" {}/{}", self.cursor.selected + 1, self.commits.len()));
        }
        if self.loading {
            t.push_str(" loading…");
        }
        t
    }

    fn render_row<'a>(&'a self, idx: usize, lanes: usize, width: usize, now: i64, theme: &Theme) -> Line<'a> {
        let commit = &self.commits[idx];
        let row: RowRef = self.rows.get(idx).expect("a row per commit");
        let mut spans: Vec<Span<'a>> = Vec::new();

        // Graph columns.
        let node_glyph = if row.is_merge() { "◉" } else { "●" };
        let overflow = row.width() > lanes;
        let shown = if overflow { lanes.saturating_sub(1) } else { row.width() };
        for i in 0..shown {
            let cell = row.cell(i);
            spans.push(Span::styled(cell.glyph_str(node_glyph), theme.lane(cell.color)));
            let filler = match row.filler(i) {
                Some(c) if i + 1 < shown || overflow => Span::styled("─", theme.lane(c)),
                _ => Span::raw(" "),
            };
            spans.push(filler);
        }
        if overflow {
            let glyph = match (row.node_lane() >= shown, row.is_merge()) {
                (true, true) => "◉ ",
                (true, false) => "● ",
                (false, _) => "… ",
            };
            spans.push(Span::styled(glyph, theme.lane(row.node_color())));
        }
        let graph_used = (shown + usize::from(overflow)) * 2;
        spans.push(Span::raw(spaces((lanes * 2).saturating_sub(graph_used) + 1)));

        // Metadata columns, dropped from the right when narrow.
        let graph_cols = lanes * 2 + 1;
        let date_width = self.dates.width();
        let meta_full = 1 + AUTHOR_WIDTH + 1 + date_width + 1 + SHA_WIDTH;
        let meta_small = 1 + date_width + 1 + SHA_WIDTH;
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
        if let Some(labels) = self.labels.get(commit.oid.hex().as_str()) {
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
        if let Some(note) = self.notes.get(&commit.oid) {
            let text = format!("({note})");
            let w = text.width() + 1;
            if w <= text_width {
                text_width -= w;
                spans.push(Span::styled(text, theme.muted));
                spans.push(Span::raw(" "));
            }
        }
        push_fit(&mut spans, &commit.summary, text_width, Style::default());

        if meta > 0 {
            if show_author {
                spans.push(Span::raw(" "));
                push_fit(&mut spans, &commit.author_name, AUTHOR_WIDTH, theme.author);
            }
            spans.push(Span::raw(" "));
            let date = match self.dates {
                DateStyle::Relative => relative(now, commit.commit_time),
                DateStyle::Clock => clock_time(commit.commit_time, self.clock.utc_offset(commit.commit_time)),
            };
            spans.push(Span::styled(fit_right(&date, date_width), theme.date));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(commit.short_oid().to_string(), theme.sha));
        }
        Line::from(spans)
    }
}

/// `n` spaces, borrowed when there are few.
fn spaces(n: usize) -> std::borrow::Cow<'static, str> {
    const SPACES: &str = "                                                                ";
    match SPACES.get(..n) {
        Some(s) => s.into(),
        None => " ".repeat(n).into(),
    }
}

/// Pushes what [`fit`] makes of `s`, borrowing `s` when it needn't be cut.
fn push_fit<'a>(spans: &mut Vec<Span<'a>>, s: &'a str, width: usize, style: Style) {
    let used = s.width();
    if used > width {
        spans.push(Span::styled(fit(s, width), style));
        return;
    }
    spans.push(Span::styled(s, style));
    if used < width {
        spans.push(Span::styled(spaces(width - used), style));
    }
}

fn intern(ids: &mut HashMap<Oid, NodeId>, oid: Oid) -> NodeId {
    let next = ids.len() as NodeId;
    *ids.entry(oid).or_insert(next)
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
                // Leaving a focused branch goes back to the scope it covered.
                if self.focus.take().is_none() {
                    self.scope = self.scope.toggled();
                }
                ctx.emit(Action::ReloadGraph);
                EventState::Consumed
            }
            Command::ToggleDates => {
                self.dates = self.dates.toggled();
                EventState::Consumed
            }
            Command::Checkout => {
                self.checkout(ctx);
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
                            oid: c.oid.to_string(),
                            summary: c.summary.to_string(),
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
                        PromptAction::CreateBranch {
                            start: c.oid.to_string(),
                            worktree: false,
                        },
                    )));
                }
                EventState::Consumed
            }
            Command::NewTag => {
                if let Some(c) = self.selected() {
                    ctx.emit(Action::OpenModal(Modal::new_tag(&c.short_oid(), c.oid.to_string())));
                }
                EventState::Consumed
            }
            Command::Copy => {
                if let Some(c) = self.selected() {
                    ctx.emit(Action::Copy(c.oid.to_string()));
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
            Command::ExpandDetails,
            Command::ToggleDetails,
            Command::ToggleScope,
            Command::ToggleDates,
            Command::Checkout,
            Command::NewBranch,
            Command::NewTag,
            Command::Merge,
            Command::Rebase,
            Command::CherryPick,
            Command::Filter,
            Command::Copy,
        ]
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
        let block = pane_block(PaneId::Graph, self.title(), focused, theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let now = self.clock.now();
        self.drawn_dates = DrawnDates::new(now);

        if self.commits.is_empty() {
            let msg = match () {
                _ if self.loading => "loading…",
                _ if self.is_filtered() => "no commits match the filter (f to change it, esc to clear it)",
                _ => "no commits",
            };
            frame.render_widget(Paragraph::new(msg).style(theme.muted), inner);
            return;
        }

        let height = inner.height as usize;
        self.cursor.scroll_into_view(height);
        let start = self.cursor.offset.min(self.commits.len());
        let end = (start + height).min(self.commits.len());
        let lanes = (start..end)
            .filter_map(|i| self.rows.get(i))
            .map(|r| r.width())
            .max()
            .unwrap_or(1)
            // Past the cap the rows don't keep their columns.
            .clamp(1, self.max_lanes.max(2).min(self.rows.cap()));
        let width = inner.width as usize;

        let lines: Vec<Line> = (start..end)
            .map(|i| {
                let line = self.render_row(i, lanes, width, now, theme);
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
        // Whether or not the date column fits; extra redraws are harmless.
        // Clock times don't change as time passes.
        if self.dates == DateStyle::Relative {
            for commit in &self.commits[start..end] {
                self.drawn_dates.add(commit.commit_time);
            }
        }
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

    fn commit(oid: &str, parents: &[&str]) -> Commit {
        Commit {
            oid: oid.parse().unwrap(),
            parents: parents.iter().map(|p| p.parse().unwrap()).collect(),
            author_name: "Ann".into(),
            author_email: "ann@example.com".into(),
            author_time: 0,
            commit_time: 0,
            summary: "a commit".into(),
        }
    }

    #[test]
    fn batches_share_node_ids_until_the_load_is_done() {
        let (a, b, c) = ("a".repeat(40), "b".repeat(40), "c".repeat(40));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = Ctx::new(tx);
        let mut graph = GraphPane::new(Arc::new(tt_git_core::time::FixedClock(0)));
        let load = graph.begin_load();
        // b's parent arrives in a later batch: it must get the node id b
        // already gave it, so the lane continues.
        graph.apply_batch(
            load.generation(),
            vec![commit(&a, &[&b]), commit(&b, &[&c])],
            false,
            &ctx,
        );
        assert_eq!(graph.ids.len(), 3);
        graph.apply_batch(load.generation(), vec![commit(&c, &[])], true, &ctx);
        assert!(graph.ids.is_empty() && graph.ids.capacity() == 0, "released once done");
        let lanes: Vec<usize> = graph.rows().iter().map(|r| r.node_lane()).collect();
        assert_eq!(lanes, [0, 0, 0], "one straight line");

        graph.labels = build_labels(&Head::Detached { oid: b.clone() }, &[r("refs/heads/feat", &a)]);
        assert_eq!(graph.selected_ref().as_deref(), Some("feat"));
        assert!(graph.select_oid(&b, &ctx));
        assert_eq!(graph.selected_ref(), Some(b.clone()), "HEAD isn't a branch label");
        assert!(!graph.select_oid(&b.to_uppercase(), &ctx));
        assert!(!graph.select_oid("bbb", &ctx));
        let announced: Vec<String> = std::iter::from_fn(|| rx.try_recv().ok())
            .filter_map(|a| match a {
                Action::CommitSelected(oid) => Some(oid),
                _ => None,
            })
            .collect();
        assert_eq!(announced, [a.clone(), b.clone()]);
    }

    /// The graph columns as drawn before rows were packed: from the whole
    /// row as laid out.
    fn unpacked_graph_spans(row: &tt_git_graph::Row, lanes: usize, theme: &Theme) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
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
        spans
    }

    #[test]
    fn wide_graphs_draw_as_from_whole_rows() {
        // 30 tips taking turns, merging each other in now and then, so rows
        // run far past the lanes drawn.
        let n = 400;
        let oid = |i: usize| format!("{i:040x}");
        let commits: Vec<Commit> = (0..n)
            .map(|i| {
                let mut parents = Vec::new();
                if i + 30 < n {
                    parents.push(oid(i + 30));
                }
                if i % 7 == 0 && i + 41 < n {
                    parents.push(oid(i + 41));
                }
                let parents: Vec<&str> = parents.iter().map(String::as_str).collect();
                commit(&oid(i), &parents)
            })
            .collect();
        let mut ids = HashMap::new();
        let mut builder = GraphBuilder::new();
        let whole: Vec<tt_git_graph::Row> = commits
            .iter()
            .map(|c| {
                let id = intern(&mut ids, c.oid);
                let parents: Vec<NodeId> = c.parents.iter().map(|p| intern(&mut ids, *p)).collect();
                builder.push(id, &parents)
            })
            .collect();
        assert!(whole.iter().any(|r| r.width() > 30), "wide enough");

        let theme = Theme::default();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = Ctx::new(tx);
        for max_lanes in [2, 5, 16, 64] {
            let mut graph = GraphPane::new(Arc::new(tt_git_core::time::FixedClock(0)));
            graph.max_lanes = max_lanes;
            let load = graph.begin_load();
            graph.apply_batch(load.generation(), commits.clone(), true, &ctx);
            for lanes in 1..=max_lanes.max(2) {
                for (i, row) in whole.iter().enumerate() {
                    let want = unpacked_graph_spans(row, lanes, &theme);
                    let line = graph.render_row(i, lanes, 200, 0, &theme);
                    assert_eq!(
                        line.spans[..want.len()],
                        want[..],
                        "max {max_lanes}, {lanes} lanes, row {i}"
                    );
                }
            }
        }
    }

    #[test]
    fn fitted_text_draws_as_fit_does() {
        let theme = Theme::default();
        for s in ["", "abc", "abcdef", "日本語", "a\u{301}b"] {
            for width in 0..8 {
                let mut spans = Vec::new();
                push_fit(&mut spans, s, width, theme.author);
                let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
                assert_eq!(text, fit(s, width), "{s:?} in {width}");
                assert!(spans.iter().all(|s| s.style == theme.author));
            }
        }
        assert_eq!(spaces(3), "   ");
        assert_eq!(spaces(100), " ".repeat(100));
    }

    #[test]
    fn detached_head_label() {
        let labels = build_labels(&Head::Detached { oid: "c".into() }, &[]);
        assert_eq!(labels["c"][0].kind, LabelKind::DetachedHead);
    }
}
