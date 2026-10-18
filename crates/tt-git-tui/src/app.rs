//! Top-level application state, update and draw.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tt_git_core::time::SharedClock;
use tt_git_core::{LogFilter, Repo};

use crate::action::{Action, RefsSnapshot};
use crate::component::{Component, EventState};
use crate::config::Config;
use crate::filter::FilterPanel;
use crate::jobs::{Ctx, JobTracker};
use crate::keymap::{Command, Context, KeyCombo, Keymap, KeymapError};
use crate::layout::View;
use crate::loaders;
use crate::modal::{GitOp, Modal, ModalOutcome};
use crate::panes::{BranchFocus, BranchesPane, DetailsPane, DetailsSubject, GraphPane, PaneId, StatusPane};
use crate::theme::Theme;
use crate::watcher::RepoWatcher;

mod panes;
mod plugins;
pub use panes::PaneRequest;
pub use plugins::PluginOptions;

/// Commands always listed in the help bar after the focused pane's own.
const GLOBAL_HELP: &[Command] = &[Command::FocusNext, Command::Help, Command::Quit];

/// Global commands listed in the help popup.
const ALL_GLOBAL: &[Command] = &[
    Command::FocusStatus,
    Command::FocusBranches,
    Command::FocusGraph,
    Command::FocusDetails,
    Command::FocusNext,
    Command::FocusPrev,
    Command::FocusLeft,
    Command::FocusDown,
    Command::FocusUp,
    Command::FocusRight,
    Command::Zoom,
    Command::MoveUp,
    Command::MoveDown,
    Command::PageUp,
    Command::PageDown,
    Command::Top,
    Command::Bottom,
    Command::Refresh,
    Command::Help,
    Command::Quit,
];

/// How much of the screen the graph and details panes take. `+` steps
/// through the levels and wraps back to [`Zoom::Off`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Zoom {
    #[default]
    Off,
    /// Only graph and details (if it's open), at full width.
    Wide,
    /// One pane fills the screen.
    Full(PaneId),
}

pub struct App {
    ctx: Ctx,
    config: Config,
    keymap: Keymap,
    theme: Theme,
    repo: Repo,
    focus: PaneId,
    zoom: Zoom,
    /// Details is open in place of the layout's `details = true` node.
    details_open: bool,
    status: StatusPane,
    branches: BranchesPane,
    graph: GraphPane,
    details: DetailsPane,
    refs: Option<RefsSnapshot>,
    graph_limit: usize,
    modal: Option<Modal>,
    /// Labels of write operations in flight.
    busy: Vec<String>,
    /// Select HEAD in the graph once refs reload (after commit/checkout).
    select_head: bool,
    watcher: Option<RepoWatcher>,
    plugins: Option<plugins::PluginState>,
    panes: Vec<crate::pty_pane::PtyPane>,
    next_pane_id: u64,
    release_keys: Vec<KeyCombo>,
    /// (main, details) areas from the last frame, for sizing new panes.
    layout_rects: (Rect, Rect),
    show_help: bool,
    error: Option<String>,
    notice: Option<String>,
    /// Text waiting for the runner to put on the clipboard.
    clipboard: Option<String>,
    should_quit: bool,
    pane_areas: Vec<(PaneId, Rect)>,
}

impl App {
    /// Creates the app and the receiving end of its action channel. Call
    /// [`start`](Self::start) inside a tokio runtime to begin loading.
    pub fn new(
        config: Config,
        repo: Repo,
        clock: SharedClock,
    ) -> Result<(Self, UnboundedReceiver<Action>), KeymapError> {
        let (tx, rx) = mpsc::unbounded_channel();
        let keymap = config.keymap()?;
        let release_keys = config
            .plugins
            .pane_release_keys
            .iter()
            .filter_map(|k| k.parse().ok())
            .collect();
        let mut graph = GraphPane::new(clock.clone());
        graph.max_lanes = config.graph.max_lanes;
        let app = Self {
            ctx: Ctx::new(tx),
            graph_limit: config.graph.max_commits.max(1),
            config,
            keymap,
            theme: Theme::default(),
            status: StatusPane::new(repo.name()),
            repo,
            focus: PaneId::Graph,
            zoom: Zoom::Off,
            details_open: false,
            branches: BranchesPane::new(clock),
            graph,
            details: DetailsPane::default(),
            refs: None,
            modal: None,
            busy: Vec::new(),
            select_head: false,
            watcher: None,
            plugins: None,
            panes: Vec::new(),
            next_pane_id: 1,
            release_keys,
            layout_rects: (Rect::new(0, 0, 120, 40), Rect::new(0, 0, 80, 15)),
            show_help: false,
            error: None,
            notice: None,
            clipboard: None,
            should_quit: false,
            pane_areas: Vec::new(),
        };
        Ok((app, rx))
    }

    /// Kicks off the initial loads and the file watcher.
    pub fn start(&mut self) {
        self.update(Action::Refresh);
        if self.config.watch && self.watcher.is_none() {
            match crate::watcher::watch(self.repo.root(), self.repo.git_dir(), self.ctx.clone()) {
                Ok(w) => self.watcher = Some(w),
                Err(e) => tracing::warn!("file watching disabled: {e}"),
            }
        }
        self.start_plugins();
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn focus(&self) -> PaneId {
        self.focus
    }

    pub fn zoom(&self) -> Zoom {
        self.zoom
    }

    /// Whether `pane` is on screen with the current layout, zoom and details state.
    pub fn shows(&self, pane: PaneId) -> bool {
        match self.zoom {
            Zoom::Full(full) => pane == full,
            _ => self.config.layout.shows(pane, self.view()),
        }
    }

    fn view(&self) -> View {
        View {
            details: self.details_open,
            wide: self.zoom == Zoom::Wide,
        }
    }

    pub fn repo(&self) -> &Repo {
        &self.repo
    }

    pub fn graph(&self) -> &GraphPane {
        &self.graph
    }

    pub fn details(&self) -> &DetailsPane {
        &self.details
    }

    pub fn status(&self) -> &StatusPane {
        &self.status
    }

    pub fn branches(&self) -> &BranchesPane {
        &self.branches
    }

    pub fn modal(&self) -> Option<&Modal> {
        self.modal.as_ref()
    }

    pub fn refs(&self) -> Option<&RefsSnapshot> {
        self.refs.as_ref()
    }

    pub fn jobs(&self) -> &JobTracker {
        self.ctx.jobs()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Takes the text a command copied, for the runner to send to the terminal.
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clipboard.take()
    }

    /// Where each pane was drawn in the last frame.
    pub fn pane_area(&self, pane: PaneId) -> Option<Rect> {
        self.pane_areas.iter().find(|(p, _)| *p == pane).map(|(_, r)| *r)
    }

    /// Handles a terminal event.
    pub fn handle_event(&mut self, event: Event) {
        if let Event::Key(key) = &event {
            tracing::debug!(code = ?key.code, modifiers = ?key.modifiers, kind = ?key.kind, "key");
        }
        match event {
            // Windows reports releases too; only act on presses (and repeats).
            Event::Key(key) if key.kind != KeyEventKind::Release => self.handle_key(key),
            Event::Mouse(m) if self.config.mouse => self.handle_mouse(m),
            _ => {}
        }
    }

    /// Whether the app wants terminal mouse capture.
    pub fn wants_mouse(&self) -> bool {
        self.config.mouse
    }

    pub fn handle_mouse(&mut self, m: MouseEvent) {
        // Popups and floating plugin panes own the screen.
        if self.modal.is_some() || self.error.is_some() || self.show_help || self.panes.iter().any(|p| p.is_modal()) {
            return;
        }
        let Some((pane, area)) = self
            .pane_areas
            .iter()
            .copied()
            .find(|(_, r)| r.contains(ratatui::layout::Position::new(m.column, m.row)))
        else {
            return;
        };
        if pane == PaneId::Details && self.has_split_pane() {
            if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.focus = PaneId::Details;
            }
            return;
        }
        let body_row =
            (m.row > area.y && m.row < area.bottom().saturating_sub(1)).then(|| (m.row - area.y - 1) as usize);
        let ctx = self.ctx.clone();
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if pane == PaneId::Details && m.modifiers.contains(KeyModifiers::CONTROL) {
                    if let Some(row) = body_row {
                        self.ctrl_click_details(row);
                    }
                    return;
                }
                let before = self.focus;
                self.focus = pane;
                if let Some(row) = body_row {
                    self.pane_mut(pane).on_click(row, &ctx);
                }
                if self.focus != before {
                    self.refresh_details(false);
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let cmd = if m.kind == MouseEventKind::ScrollDown {
                    Command::MoveDown
                } else {
                    Command::MoveUp
                };
                for _ in 0..3 {
                    self.pane_mut(pane).on_command(cmd, &ctx);
                }
            }
            _ => {}
        }
    }

    fn pane_mut(&mut self, pane: PaneId) -> &mut dyn Component {
        match pane {
            PaneId::Status => &mut self.status,
            PaneId::Branches => &mut self.branches,
            PaneId::Graph => &mut self.graph,
            PaneId::Details => &mut self.details,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        let combo = KeyCombo::from(key);
        if self.error.is_none() && self.modal.is_none() && self.pane_key(combo) {
            return;
        }
        if self.error.is_some() {
            if matches!(combo.code, KeyCode::Esc | KeyCode::Enter) || combo == KeyCombo::char('q') {
                self.error = None;
            }
            return;
        }
        if let Some(modal) = &mut self.modal {
            match modal.handle_key(combo) {
                ModalOutcome::Open => {}
                ModalOutcome::Cancelled => self.modal = None,
                ModalOutcome::Run(op) => {
                    self.modal = None;
                    self.update(Action::Run(op));
                }
                ModalOutcome::Command(cmd) => {
                    self.modal = None;
                    self.run_command(cmd);
                }
                ModalOutcome::Filter(filter) => {
                    self.modal = None;
                    self.set_graph_filter(filter);
                }
                ModalOutcome::PluginAction(action) => {
                    self.modal = None;
                    let _ = self.invoke_plugin_action(&action, "palette");
                }
                ModalOutcome::PluginPane(plugin, entry) => {
                    self.modal = None;
                    let req = PaneRequest {
                        focus: true,
                        ..Default::default()
                    };
                    if let Err(e) = self.open_pane(&plugin, &entry, req) {
                        self.error = Some(e);
                    }
                }
            }
            return;
        }
        if self.show_help {
            match self.keymap.lookup(Context::Global, combo) {
                Some(Command::Quit) => self.should_quit = true,
                Some(Command::Help) => self.show_help = false,
                _ if combo.code == KeyCode::Esc => self.show_help = false,
                _ => {}
            }
            return;
        }
        if self.focused().wants_text_input() {
            let ctx = self.ctx.clone();
            self.focused_mut().on_text_key(combo, &ctx);
            return;
        }
        let context = self.focused().context();
        if let Some(custom) = self.keymap.lookup_custom(context, combo) {
            let action = custom.action.clone();
            let _ = self.invoke_plugin_action(&action, "keybinding");
            return;
        }
        if let Some(cmd) = self.keymap.lookup(context, combo) {
            self.update(Action::Command(cmd));
        }
    }

    pub fn update(&mut self, action: Action) {
        match action {
            Action::Command(cmd) => self.run_command(cmd),
            Action::Tick => self.poll_plugins(),
            Action::Socket(call) => self.handle_call(call),
            Action::PluginFinished(log) => self.plugin_finished(log),
            Action::PaneOutput(_) => {}
            Action::PaneExited { id, status } => self.pane_exited(id, status),
            Action::Notify(msg) => self.notice = Some(msg),
            Action::Error(msg) => self.error = Some(msg),
            Action::Copy(text) => {
                self.notice = Some(format!("copied {text}"));
                self.clipboard = Some(text);
            }
            Action::Refresh => {
                self.load_refs();
                self.load_status();
                self.load_graph();
            }
            Action::GraphBatch {
                generation,
                commits,
                done,
            } => self.graph.apply_batch(generation, commits, done, &self.ctx),
            Action::BranchHistoryLoaded { generation, history } => {
                self.graph.apply_history(generation, history, &self.ctx)
            }
            Action::GraphFailed { generation, error } => {
                self.graph.load_failed(generation);
                self.error = Some(error);
            }
            Action::ReloadGraph => self.load_graph(),
            Action::LoadMoreCommits => {
                self.graph_limit = self.graph_limit.saturating_add(self.config.graph.max_commits.max(1));
                self.load_graph();
            }
            Action::RefsLoaded(Ok(snapshot)) => {
                if std::mem::take(&mut self.select_head)
                    && let Some(oid) = snapshot.head.oid()
                {
                    self.graph.select_when_loaded(oid.to_string(), &self.ctx);
                }
                self.graph.set_refs(&snapshot);
                self.branches
                    .set_entries(snapshot.branches.clone(), snapshot.base.clone());
                self.refs = Some(snapshot);
                self.track_head();
                // Follow the focused branch if the list changed under it.
                if self.graph.focus.is_some() {
                    self.focus_branch(self.branches.focus());
                }
            }
            Action::BranchSelected(oid) => self.graph.select_when_loaded(oid, &self.ctx),
            Action::BranchFocused(focus) => self.focus_branch(focus),
            Action::OpenModal(modal) => self.modal = Some(modal),
            Action::Run(op) => self.run_op(op),
            Action::OpDone { op, result, head } => self.op_done(op, result, head),
            Action::StatusLoaded(Ok(status)) => {
                self.status.set_status(status);
                if self.focus == PaneId::Status {
                    self.refresh_details(true);
                }
            }
            Action::FileSelected(entry) => self.show_file(entry, false),
            Action::RepoChanged { refs, status } => {
                if refs {
                    self.load_refs();
                    self.load_graph();
                }
                if status {
                    self.load_status();
                }
            }
            Action::RefsLoaded(Err(e)) | Action::StatusLoaded(Err(e)) => self.error = Some(e),
            Action::CommitSelected(oid) => {
                if matches!(self.focus, PaneId::Graph | PaneId::Branches) {
                    self.show_commit(oid);
                }
            }
            Action::DetailsLoaded { subject, text } => self.details.set_content(&subject, &text),
            Action::CommitFilesLoaded { oid, header, files } => match files {
                Ok(files) => self.details.set_commit_files(&oid, &header, files, &self.ctx),
                Err(e) => self
                    .details
                    .set_content(&DetailsSubject::CommitFiles(oid), &format!("{header}\n{e}")),
            },
            Action::CommitFileSelected { oid, file } => {
                let repo = self.repo.clone();
                self.ctx.spawn(move || loaders::load_commit_file_diff(repo, oid, file));
            }
            Action::CommitFileDiffLoaded { oid, path, text } => self.details.set_file_diff(&oid, &path, &text),
            Action::FilterChoicesLoaded { tab, choices } => {
                if let Some(Modal::Filter(panel)) = &mut self.modal {
                    panel.set_choices(tab, choices);
                }
            }
        }
    }

    fn load_refs(&self) {
        let repo = self.repo.clone();
        let config = self.config.branches.clone();
        self.ctx.spawn(move || loaders::load_refs(repo, config));
    }

    fn load_status(&self) {
        let repo = self.repo.clone();
        self.ctx.spawn(move || loaders::load_status(repo));
    }

    fn load_graph(&mut self) {
        let generation = self.graph.begin_load();
        self.graph.limit = self.graph_limit;
        let repo = self.repo.clone();
        let scope = self.graph.scope;
        let focus = self.graph.focus.clone();
        let filter = self.graph.filter.clone();
        let limit = self.graph_limit;
        self.ctx
            .spawn_streaming(move |ctx| loaders::load_graph(repo, scope, focus, filter, generation, limit, ctx));
    }

    /// Opens the graph's filter panel and loads the authors and files it lists.
    fn open_graph_filter(&mut self) {
        self.modal = Some(Modal::Filter(Box::new(FilterPanel::new(self.graph.filter.clone()))));
        let repo = self.repo.clone();
        self.ctx.spawn(move || loaders::load_filter_authors(repo));
        let repo = self.repo.clone();
        self.ctx.spawn(move || loaders::load_filter_files(repo));
    }

    /// Narrows the graph to `filter`'s commits (leaving a focused branch,
    /// which the filter doesn't apply to), or shows them all again.
    fn set_graph_filter(&mut self, filter: LogFilter) {
        if self.graph.filter == filter && self.graph.focus.is_none() {
            return;
        }
        self.graph.filter = filter;
        self.graph.focus = None;
        self.load_graph();
    }

    /// Shows only `focus`'s history in the graph (with its tip selected), or
    /// the whole graph again.
    fn focus_branch(&mut self, focus: Option<BranchFocus>) {
        if self.graph.focus == focus {
            return;
        }
        let tip = focus.as_ref().map(|f| f.tip.clone());
        self.graph.focus = focus;
        self.load_graph();
        if let Some(tip) = tip {
            self.graph.select_when_loaded(tip, &self.ctx);
        }
    }

    fn run_op(&mut self, op: GitOp) {
        let label = op.label();
        self.notice = Some(format!("{label}…"));
        self.busy.push(label);
        let repo = self.repo.clone();
        self.ctx.spawn(move || loaders::run_op(repo, op));
    }

    fn op_done(&mut self, op: GitOp, result: Result<String, String>, head: Option<String>) {
        let label = op.label();
        if let Some(pos) = self.busy.iter().position(|b| *b == label) {
            self.busy.remove(pos);
        }
        if result.is_ok()
            && matches!(
                op,
                GitOp::Commit { .. }
                    | GitOp::Checkout(_)
                    | GitOp::CreateBranch { worktree: false, .. }
                    | GitOp::Pull
                    | GitOp::Merge { .. }
                    | GitOp::Rebase { .. }
                    | GitOp::CherryPick { .. }
                    | GitOp::Continue(_)
                    | GitOp::Abort(_)
            )
        {
            self.select_head = true;
        }
        if result.is_ok() {
            self.op_event(&op, head.as_deref());
        }
        match result {
            Ok(_) => self.notice = Some(format!("✓ {label}")),
            Err(err) => match op {
                GitOp::DeleteBranch { name, force: false } if err.contains("not fully merged") => {
                    self.notice = None;
                    self.modal = Some(Modal::confirm(
                        "Force delete?",
                        format!("{name} is not fully merged. Delete it anyway (git branch -D)?"),
                        GitOp::DeleteBranch { name, force: true },
                    ));
                }
                _ => {
                    self.notice = None;
                    self.error = Some(format!("{label} failed:\n{err}"));
                }
            },
        }
        self.update(Action::Refresh);
    }

    /// Points the details pane at whatever the focused pane has selected.
    fn refresh_details(&mut self, force: bool) {
        match self.focus {
            PaneId::Status => {
                if let Some(entry) = self.status.selected().cloned() {
                    self.show_file(entry, force);
                } else if force {
                    self.details.clear();
                }
            }
            PaneId::Graph | PaneId::Branches => {
                if let Some(oid) = self.graph.selected().map(|c| c.oid.clone()) {
                    self.show_commit(oid);
                }
            }
            PaneId::Details => {}
        }
    }

    fn show_file(&mut self, entry: tt_git_core::StatusEntry, force: bool) {
        let subject = DetailsSubject::File(entry.path.clone());
        if !force && self.details.subject() == Some(&subject) {
            return;
        }
        self.details.request(subject, format!("Diff {}", entry.path));
        let repo = self.repo.clone();
        self.ctx.spawn(move || loaders::load_file_diff(repo, entry));
    }

    /// Shows `oid` in details, staying in the files view if that's open.
    fn show_commit(&mut self, oid: String) {
        if matches!(self.details.subject(), Some(DetailsSubject::CommitFiles(_))) {
            self.show_commit_files(oid);
        } else {
            self.show_commit_plain(oid);
        }
    }

    fn show_commit_plain(&mut self, oid: String) {
        let subject = DetailsSubject::Commit(oid.clone());
        if self.details.subject() == Some(&subject) {
            return;
        }
        let short = oid.chars().take(7).collect::<String>();
        self.details.request(subject, format!("Commit {short}"));
        let repo = self.repo.clone();
        self.ctx.spawn(move || loaders::load_commit_details(repo, oid));
    }

    fn show_commit_files(&mut self, oid: String) {
        let subject = DetailsSubject::CommitFiles(oid.clone());
        if self.details.subject() == Some(&subject) {
            return;
        }
        let short = oid.chars().take(7).collect::<String>();
        self.details.request(subject, format!("Commit {short} files"));
        let repo = self.repo.clone();
        self.ctx.spawn(move || loaders::load_commit_files(repo, oid));
    }

    /// Opens the selected commit fullscreen as changed files beside a diff.
    fn expand_details(&mut self) {
        let Some(oid) = self.graph.selected().map(|c| c.oid.clone()) else {
            return;
        };
        self.show_commit_files(oid);
        self.details_open = true;
        self.zoom = Zoom::Full(PaneId::Details);
        self.focus = PaneId::Details;
    }

    fn run_command(&mut self, cmd: Command) {
        let before = self.focus;
        self.dispatch(cmd);
        if self.focus != before {
            self.settle_focus(before);
            self.refresh_details(false);
        }
    }

    fn step_zoom(&mut self) {
        self.zoom = match self.zoom {
            Zoom::Off => {
                if !matches!(self.focus, PaneId::Graph | PaneId::Details) {
                    self.focus = PaneId::Graph;
                }
                Zoom::Wide
            }
            // The graph alone already fills the screen; fullscreen would look the same.
            Zoom::Wide if !self.shows(PaneId::Details) => Zoom::Off,
            Zoom::Wide => Zoom::Full(self.focus),
            Zoom::Full(_) => Zoom::Off,
        };
    }

    /// Makes the newly focused pane visible: unzoom, then open details (to
    /// focus it) or close it (to focus a pane it covers). Falls back to
    /// `before` if neither helps.
    fn settle_focus(&mut self, before: PaneId) {
        if self.shows(self.focus) {
            return;
        }
        self.zoom = Zoom::Off;
        if self.shows(self.focus) {
            return;
        }
        self.details_open = self.focus == PaneId::Details;
        if !self.shows(self.focus) {
            self.focus = before;
        }
    }

    fn toggle_details(&mut self) {
        if self.shows(PaneId::Details) {
            self.close_details();
        } else {
            self.details_open = true;
            // A fullscreen graph has no room for details; drop back to wide.
            if self.zoom == Zoom::Full(PaneId::Graph) {
                self.zoom = Zoom::Wide;
            }
        }
    }

    fn close_details(&mut self) {
        // Leave the files view: next time details opens it's the plain commit.
        if let Some(DetailsSubject::CommitFiles(oid)) = self.details.subject().cloned() {
            self.show_commit_plain(oid);
        }
        self.details_open = false;
        if self.zoom == Zoom::Full(PaneId::Details) {
            self.zoom = Zoom::Off;
        }
        if self.focus == PaneId::Details {
            self.focus = PaneId::Graph;
        }
    }

    fn dispatch(&mut self, cmd: Command) {
        match cmd {
            Command::Quit => self.should_quit = true,
            Command::Help => self.show_help = !self.show_help,
            Command::FocusNext => self.cycle_focus(1),
            Command::FocusPrev => self.cycle_focus(PaneId::ALL.len() - 1),
            Command::FocusLeft | Command::FocusDown | Command::FocusUp | Command::FocusRight => self.focus_toward(cmd),
            Command::FocusStatus => self.focus = PaneId::Status,
            Command::FocusBranches => self.focus = PaneId::Branches,
            Command::FocusGraph => self.focus = PaneId::Graph,
            Command::FocusDetails => self.focus = PaneId::Details,
            Command::Zoom => self.step_zoom(),
            Command::ToggleDetails if self.focus == PaneId::Graph => self.toggle_details(),
            Command::ExpandDetails if self.focus == PaneId::Graph => self.expand_details(),
            // With details already closed, esc in the graph drops its filter.
            Command::CloseDetails
                if self.focus == PaneId::Graph && !self.shows(PaneId::Details) && self.graph.is_filtered() =>
            {
                self.set_graph_filter(LogFilter::default())
            }
            Command::CloseDetails if matches!(self.focus, PaneId::Graph | PaneId::Details) => self.close_details(),
            Command::Filter if self.focus == PaneId::Graph => self.open_graph_filter(),
            Command::Refresh => self.update(Action::Refresh),
            Command::Palette => self.modal = Some(Modal::palette(self.palette_items())),
            Command::PluginLogs => self.show_plugin_logs(),
            Command::OpenLink if self.focus == PaneId::Details => self.open_link(),
            Command::Continue | Command::Abort if self.focus != PaneId::Status => {
                let ctx = self.ctx.clone();
                self.status.on_command(cmd, &ctx);
            }
            Command::Push => self.update(Action::Run(GitOp::Push)),
            Command::Pull => self.update(Action::Run(GitOp::Pull)),
            _ => {
                let ctx = self.ctx.clone();
                if self.focused_mut().on_command(cmd, &ctx) == EventState::NotConsumed {
                    tracing::debug!(?cmd, pane = ?self.focus, "command not handled");
                }
            }
        }
    }

    /// Moves focus to the nearest pane on screen in the direction of `cmd`
    /// (one of the `Focus{Left,Down,Up,Right}` commands), vim-window style.
    /// Panes that overlap the focused one across that direction win; ties go
    /// to the one whose middle is closest.
    fn focus_toward(&mut self, cmd: Command) {
        let Some(from) = self.pane_area(self.focus) else {
            return;
        };
        let sideways = matches!(cmd, Command::FocusLeft | Command::FocusRight);
        // A rect's extent across the direction of travel, as (start, end).
        let across = |r: Rect| if sideways { (r.y, r.bottom()) } else { (r.x, r.right()) };
        // Sorts best first: overlapping across, then nearest, then most centred.
        let score = |to: Rect| -> Option<(bool, u16, u32)> {
            let gap = match cmd {
                Command::FocusLeft => from.x.checked_sub(to.right())?,
                Command::FocusRight => to.x.checked_sub(from.right())?,
                Command::FocusUp => from.y.checked_sub(to.bottom())?,
                Command::FocusDown => to.y.checked_sub(from.bottom())?,
                _ => return None,
            };
            let ((a0, a1), (b0, b1)) = (across(from), across(to));
            let overlaps = a0 < b1 && b0 < a1;
            let off_centre = (u32::from(a0) + u32::from(a1)).abs_diff(u32::from(b0) + u32::from(b1));
            Some((!overlaps, gap, off_centre))
        };
        let next = self
            .pane_areas
            .iter()
            .filter(|(p, _)| *p != self.focus)
            .filter_map(|(p, r)| score(*r).map(|s| (s, *p)))
            .min_by_key(|(s, _)| *s);
        if let Some((_, pane)) = next {
            self.focus = pane;
        }
    }

    /// Moves focus `step` places round [`PaneId::ALL`], skipping hidden panes.
    fn cycle_focus(&mut self, step: usize) {
        let idx = PaneId::ALL.iter().position(|p| *p == self.focus).unwrap_or(0);
        let n = PaneId::ALL.len();
        if let Some(next) = (1..n)
            .map(|i| PaneId::ALL[(idx + i * step) % n])
            .find(|p| self.shows(*p))
        {
            self.focus = next;
        }
    }

    fn focused(&self) -> &dyn Component {
        match self.focus {
            PaneId::Status => &self.status,
            PaneId::Branches => &self.branches,
            PaneId::Graph => &self.graph,
            PaneId::Details => &self.details,
        }
    }

    fn focused_mut(&mut self) -> &mut dyn Component {
        match self.focus {
            PaneId::Status => &mut self.status,
            PaneId::Branches => &mut self.branches,
            PaneId::Graph => &mut self.graph,
            PaneId::Details => &mut self.details,
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let [main, help] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());
        self.pane_areas = match self.zoom {
            Zoom::Full(pane) => vec![(pane, main)],
            _ => self.config.layout.place(main, self.view()),
        };
        // Where details would open, for sizing split plugin panes while it's closed.
        let details_area = self.pane_area(PaneId::Details).unwrap_or_else(|| {
            let open = View {
                details: true,
                ..self.view()
            };
            let placed = self.config.layout.place(main, open);
            placed
                .into_iter()
                .find(|(p, _)| *p == PaneId::Details)
                .map_or(main, |(_, r)| r)
        });
        self.layout_rects = (main, details_area);
        let focus = self.focus;
        let theme = &self.theme;
        for (pane, area) in self.pane_areas.clone() {
            match pane {
                PaneId::Status => self.status.draw(frame, area, focus == pane, theme),
                PaneId::Branches => self.branches.draw(frame, area, focus == pane, theme),
                PaneId::Graph => self.graph.draw(frame, area, focus == pane, theme),
                PaneId::Details => {}
            }
        }
        if self.pane_area(PaneId::Details).is_some() && !self.draw_split_pane(frame, details_area) {
            let theme = &self.theme;
            self.details.draw(frame, details_area, focus == PaneId::Details, theme);
        }

        self.draw_help_bar(frame, help);
        if self.show_help {
            self.draw_help_popup(frame, main);
        }
        self.draw_modal_panes(frame, main, details_area);
        if let Some(modal) = &self.modal {
            modal.draw(frame, main, &self.theme);
        }
        if let Some(err) = &self.error {
            draw_error_popup(frame, main, err, &self.theme);
        }
    }

    fn draw_help_bar(&self, frame: &mut Frame, area: Rect) {
        use unicode_width::UnicodeWidthStr;

        // The notice gets its own right-aligned slot (up to half the width)
        // so a long list of key hints can't push it off screen.
        let mut keys_area = area;
        if let Some(notice) = &self.notice {
            let text = format!(" │ {notice}");
            let width = (text.width() as u16).min(area.width / 2);
            let [left, right] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(width)]).areas(area);
            keys_area = left;
            let shown = crate::widgets::truncate(&text, width as usize);
            frame.render_widget(Paragraph::new(Span::styled(shown, self.theme.muted)), right);
        }

        // Global hints always show; pane hints fill what's left, in order,
        // skipping any that don't fit (the full list is under `?`).
        let context = self.focused().context();
        let hint = |cmd: &Command| {
            self.keymap
                .display_key(context, *cmd)
                .map(|key| (key.to_string(), cmd.label()))
        };
        let globals: Vec<_> = GLOBAL_HELP.iter().filter_map(hint).collect();
        let width_of = |(k, l): &(String, &str)| k.width() + 1 + l.width() + 2;
        let mut budget = (keys_area.width as usize).saturating_sub(globals.iter().map(width_of).sum());
        let mut items = Vec::new();
        for item in self.focused().commands().iter().filter_map(hint) {
            if width_of(&item) <= budget {
                budget -= width_of(&item);
                items.push(item);
            }
        }
        items.extend(globals);
        let mut spans = Vec::new();
        for (key, label) in items {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(key, self.theme.help_key));
            spans.push(Span::styled(format!(" {label}"), self.theme.help_text));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), keys_area);
    }

    fn draw_help_popup(&self, frame: &mut Frame, area: Rect) {
        let context = self.focused().context();
        let sections: [(Context, &[Command]); 2] =
            [(context, self.focused().commands()), (Context::Global, ALL_GLOBAL)];
        let mut lines = Vec::new();
        for (ctx, cmds) in sections {
            for cmd in cmds {
                let keys = self.keymap.keys_for(ctx, *cmd);
                if keys.is_empty() {
                    continue;
                }
                let keys = keys.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
                lines.push(Line::from(vec![
                    Span::styled(format!("{keys:>16}  "), self.theme.help_key),
                    Span::styled(cmd.label(), self.theme.help_text),
                ]));
            }
        }

        let popup = centered(area, 50, lines.len() as u16 + 2);
        frame.render_widget(Clear, popup);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(self.theme.border_focused)
            .title(Span::styled(" Keybindings ", self.theme.title_focused));
        frame.render_widget(Paragraph::new(lines).block(block), popup);
    }
}

fn draw_error_popup(frame: &mut Frame, area: Rect, msg: &str, theme: &Theme) {
    let width = 70.min(area.width);
    let inner_width = width.saturating_sub(2).max(1) as usize;
    let text_lines: usize = msg
        .lines()
        .map(|l| l.chars().count().div_ceil(inner_width).max(1))
        .sum();
    let popup = centered(area, width, text_lines as u16 + 2);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.error)
        .title(Span::styled(" Error ", theme.error));
    frame.render_widget(Paragraph::new(msg).wrap(Wrap { trim: false }).block(block), popup);
}

pub(crate) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}
