use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tt_git_core::time::SharedClock;
use tt_git_core::{BranchHistory, CheckoutPlan, Commit, GitRef, Head, LogFilter, Oid, RefKind};
use tt_git_graph::{GraphBuilder, NodeId, Rows};

use super::DrawnDates;
use crate::action::{Action, RefsSnapshot};
use crate::component::{Component, EventState};
use crate::filter::author_name;
use crate::jobs::Ctx;
use crate::keymap::{Command, Context};
use crate::modal::{GitOp, Modal, PaletteItem, PaletteTarget, PromptAction};
use crate::widgets::Cursor;

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

    /// The columns a date takes.
    pub fn width(self) -> usize {
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
    /// Draws the branch lanes left of each commit.
    pub lanes: bool,
    /// Lists merge commits; hiding them joins each line through a merge to
    /// its first parent (see [`LogFilter::no_merges`]).
    pub merges: bool,
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
    /// The commit a range selection started at, and where it is in the
    /// list (`None` while a reload hasn't reached it yet).
    range: Option<(Oid, Option<usize>)>,
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
            lanes: true,
            merges: true,
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
            range: None,
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

    /// The ref labels (badges) on `commit`.
    pub fn labels(&self, commit: &Commit) -> &[RefLabel] {
        self.labels.get(commit.oid.hex().as_str()).map_or(&[], Vec::as_slice)
    }

    /// A note on a focused branch's fork or merge commit.
    pub fn note(&self, commit: &Commit) -> Option<&str> {
        self.notes.get(&commit.oid).map(String::as_str)
    }

    /// The clock dates are relative to.
    pub fn clock(&self) -> &SharedClock {
        &self.clock
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
            || !self.merges
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

    /// The front end shows `rows` lines of commits: keeps the selection
    /// among them, and pages by them. Call before drawing.
    pub fn set_viewport(&mut self, rows: usize) {
        if !self.commits.is_empty() {
            self.cursor.scroll_into_view(rows);
        }
    }

    /// The commits a viewport of `rows` lines shows: from the scroll
    /// offset on.
    pub fn visible_range(&self, rows: usize) -> std::ops::Range<usize> {
        let start = self.cursor.offset.min(self.commits.len());
        start..(start + rows).min(self.commits.len())
    }

    /// The front end drew the commits in `range` at `now`: notes their
    /// dates, so a tick can tell when they'd read differently.
    pub fn note_drawn(&mut self, now: i64, range: std::ops::Range<usize>) {
        self.drawn_dates = DrawnDates::new(now);
        // Whether or not the date column fits; extra redraws are harmless.
        // Clock times don't change as time passes.
        if self.dates == DateStyle::Relative {
            for commit in &self.commits[range] {
                self.drawn_dates.add(commit.commit_time);
            }
        }
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
        // A filtered load lists every match at once, and its parents may name
        // commits it left out; lanes to those would never end.
        let listed: Option<HashSet<Oid>> = (self.is_filtered() || self.hides_merges())
            .then(|| self.commits.iter().chain(&commits).map(|c| c.oid).collect());
        let mut parents: Vec<NodeId> = Vec::new();
        for commit in commits {
            let id = intern(&mut self.ids, commit.oid);
            parents.clear();
            parents.extend(
                commit
                    .parents
                    .iter()
                    .filter(|p| listed.as_ref().is_none_or(|l| l.contains(p)))
                    .map(|p| intern(&mut self.ids, *p)),
            );
            self.rows.push(&self.builder.push(id, &parents));
            self.commits.push(commit);
        }
        if let Some((oid, at)) = &mut self.range {
            *at = self.commits.iter().position(|c| c.oid == *oid);
        }
        if done {
            self.loading = false;
            self.ids = HashMap::new();
            self.cursor.selected = self.reselect.take().and_then(|oid| self.position(&oid)).unwrap_or(0);
            self.cursor.offset = self.cursor.selected.saturating_sub(self.cursor.height / 2);
            // The commit the range started at is gone.
            if matches!(self.range, Some((_, None))) {
                self.range = None;
            }
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

    /// Asks before reverting `c`. A merge commit asks which parent's side
    /// to keep instead, one entry per parent.
    fn revert_modal(&self, c: &Commit) -> Modal {
        let op = |mainline| GitOp::Revert {
            oid: c.oid.to_string(),
            summary: c.summary.to_string(),
            mainline,
        };
        if !c.is_merge() {
            return Modal::confirm(
                "Revert",
                format!(
                    "Revert {} \"{}\"? This makes a new commit that undoes it.",
                    c.short_oid(),
                    c.summary
                ),
                op(None),
            );
        }
        let items = c
            .parents
            .iter()
            .enumerate()
            .map(|(i, parent)| {
                let short = &parent.hex()[..7];
                let label = match self.commits.iter().find(|p| p.oid == *parent) {
                    Some(p) => format!("Keep parent {}: {short} \"{}\"", i + 1, p.summary),
                    None => format!("Keep parent {}: {short}", i + 1),
                };
                let hint = match i {
                    0 => "-m 1, the branch merged into".to_string(),
                    _ => format!("-m {}", i + 1),
                };
                PaletteItem {
                    label,
                    hint,
                    target: PaletteTarget::Op(op(Some(i + 1))),
                }
            })
            .collect();
        Modal::picker(format!("Revert merge {}: keep which side?", c.short_oid()), items, 0)
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

    /// The first and last index of the range selection, if one is open.
    pub fn selected_range(&self) -> Option<(usize, usize)> {
        let (_, Some(anchor)) = self.range? else {
            return None;
        };
        let at = self.cursor.selected;
        Some((anchor.min(at), anchor.max(at)))
    }

    /// Whether a range selection is open (`v`).
    pub fn has_range(&self) -> bool {
        self.range.is_some()
    }

    /// Ends the range selection.
    pub fn clear_range(&mut self) {
        self.range = None;
    }

    /// Asks to cherry-pick the selected commit, or the range's, oldest first.
    fn cherry_pick(&self, ctx: &Ctx) {
        let picked: Vec<&Commit> = match self.selected_range() {
            Some((first, last)) => self.commits[first..=last].iter().rev().collect(),
            None => self.selected().into_iter().collect(),
        };
        let (title, msg) = match &picked[..] {
            [] => return,
            [c] => (
                "Cherry-pick",
                format!("Apply {} \"{}\" onto the current branch?", c.short_oid(), c.summary),
            ),
            [oldest, .., newest] => {
                if let Some(merge) = picked.iter().find(|c| c.parents.len() > 1) {
                    ctx.emit(Action::Notify(format!(
                        "the range includes merge commit {}; cherry-pick can't apply merges",
                        merge.short_oid()
                    )));
                    return;
                }
                (
                    "Cherry-pick range",
                    format!(
                        "Apply {} commits, {} \"{}\" through {} \"{}\", onto the current branch?",
                        picked.len(),
                        oldest.short_oid(),
                        oldest.summary,
                        newest.short_oid(),
                        newest.summary
                    ),
                )
            }
        };
        let summary = picked.last().map(|c| c.summary.to_string()).unwrap_or_default();
        ctx.emit(Action::OpenModal(Modal::confirm(
            title,
            msg,
            GitOp::CherryPick {
                oids: picked.iter().map(|c| c.oid.to_string()).collect(),
                summary,
            },
        )));
    }

    /// Whether the graph is narrowed by its filter.
    pub fn is_filtered(&self) -> bool {
        self.focus.is_none() && !self.filter.is_empty()
    }

    /// Whether merge commits are left out (a focused branch shows them all).
    pub fn hides_merges(&self) -> bool {
        self.focus.is_none() && !self.merges
    }

    /// What a load of the graph narrows it to: [`filter`](Self::filter),
    /// and whether to leave out merges.
    pub fn log_filter(&self) -> LogFilter {
        LogFilter {
            no_merges: !self.merges,
            ..self.filter.clone()
        }
    }

    /// The pane's title: what it shows, the selection, and loading.
    pub fn title(&self) -> String {
        let mut t = format!("Graph [{}", self.scope_label());
        if self.is_filtered() {
            if let Some(author) = &self.filter.author {
                t.push_str(&format!(" · author {}", author_name(author)));
            }
            if let Some(path) = &self.filter.path {
                let what = if path.ends_with('/') { "folder" } else { "file" };
                t.push_str(&format!(" · {what} {path}"));
            }
            if let Some(message) = &self.filter.message {
                t.push_str(&format!(" · {} /{message}", self.filter.search_in.label()));
            }
        }
        if self.hides_merges() {
            t.push_str(" · no merges");
        }
        t.push(']');
        if !self.commits.is_empty() {
            t.push_str(&format!(" {}/{}", self.cursor.selected + 1, self.commits.len()));
        }
        if let Some((first, last)) = self.selected_range() {
            t.push_str(&format!(" · {} selected", last + 1 - first));
        }
        if self.loading {
            t.push_str(" loading…");
        }
        t
    }
}

fn intern(ids: &mut HashMap<Oid, NodeId>, oid: Oid) -> NodeId {
    let next = ids.len() as NodeId;
    *ids.entry(oid).or_insert(next)
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
            Command::ToggleLanes => {
                self.lanes = !self.lanes;
                EventState::Consumed
            }
            Command::ToggleMerges => {
                self.merges = !self.merges;
                ctx.emit(Action::ReloadGraph);
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
                self.cherry_pick(ctx);
                EventState::Consumed
            }
            Command::SelectRange => {
                self.range = match (&self.range, self.selected()) {
                    (None, Some(c)) => Some((c.oid, Some(self.cursor.selected))),
                    _ => None,
                };
                EventState::Consumed
            }
            Command::Revert => {
                if let Some(modal) = self.selected().map(|c| self.revert_modal(c)) {
                    ctx.emit(Action::OpenModal(modal));
                }
                EventState::Consumed
            }
            Command::Reset => {
                if let Some(c) = self.selected() {
                    ctx.emit(Action::Reset {
                        target: c.oid.to_string(),
                        display: c.short_oid().to_string(),
                    });
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

    /// As enter does: the commit fullscreen, files beside the diff.
    fn double_click(&self, row: usize) -> Option<Command> {
        self.has_commit_at_row(row).then_some(Command::ExpandDetails)
    }

    fn commands(&self) -> &[Command] {
        &[
            Command::ExpandDetails,
            Command::ToggleDetails,
            Command::ToggleScope,
            Command::ToggleDates,
            Command::ToggleLanes,
            Command::ToggleMerges,
            Command::Checkout,
            Command::NewBranch,
            Command::NewTag,
            Command::Merge,
            Command::Rebase,
            Command::CherryPick,
            Command::Revert,
            Command::Reset,
            Command::SelectRange,
            Command::Filter,
            Command::FilterMessage,
            Command::FilterFolder,
            Command::Copy,
            Command::Agent,
            Command::NextTab,
            Command::PrevTab,
            Command::DetailsPageDown,
            Command::DetailsPageUp,
        ]
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

    #[test]
    fn detached_head_label() {
        let labels = build_labels(&Head::Detached { oid: "c".into() }, &[]);
        assert_eq!(labels["c"][0].kind, LabelKind::DetachedHead);
    }
}
