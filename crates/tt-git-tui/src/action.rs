//! Messages that drive [`App::update`](crate::App::update).

use tt_git_core::{
    BaseBranches, BranchDeletes, BranchEntry, BranchHistory, Checkout, CheckoutPlan, Commit, CommitFile, GitRef,
    GoneBranches, Head, NoStep, ResetPlan, StageSide, Stash, Status, StatusEntry, UndoStep, Worktree,
};
use tt_git_plugin::runtime::CommandLog;
use tt_git_plugin::socket::Call;

use crate::editor::{EditTarget, Snapshot};
use crate::filter::{Choice, FilterTab};
use crate::keymap::Command;
use crate::modal::{GitOp, Modal};
use crate::panes::{BranchFocus, DetailsSubject};
use crate::syntax::{HunkTokens, LineTokens};

/// Refs and HEAD, loaded together so labels and branch lists agree.
#[derive(Debug, Clone)]
pub struct RefsSnapshot {
    pub head: Head,
    pub refs: Vec<GitRef>,
    /// Recent checkouts from HEAD's reflog, newest first.
    pub checkouts: Vec<Checkout>,
    /// The unified, recency-sorted branch list.
    pub branches: Vec<BranchEntry>,
    /// Branches merged and not merged into the base branch, when there is one.
    pub base: Option<BaseBranches>,
    /// The stash list, newest first.
    pub stashes: Vec<Stash>,
    /// Every working tree, the main one first.
    pub worktrees: Vec<Worktree>,
}

/// Everything that can change app state. Input, timers, finished background
/// jobs and (later) plugin socket requests are all funnelled into one channel
/// of these.
#[derive(Debug, Clone)]
pub enum Action {
    /// A bound command, from a key press or the command palette.
    Command(Command),
    /// Periodic tick for spinners and polling.
    Tick,
    /// Show a transient message in the help bar.
    Notify(String),
    /// The latest tt-git release's tag, or why it couldn't be found.
    UpdateChecked(Result<String, String>),
    /// Show an error popup.
    Error(String),
    /// Put this text on the clipboard.
    Copy(String),
    /// Reload refs, status and the graph.
    Refresh,
    /// Reload only the graph (e.g. after changing its scope).
    ReloadGraph,
    /// A chunk of commits for graph load `generation`.
    GraphBatch {
        generation: u64,
        commits: Vec<Commit>,
        done: bool,
    },
    GraphFailed {
        generation: u64,
        error: String,
    },
    /// Graph load `generation`, started to continue the one before it,
    /// found the refs changed: it lists every commit instead, so its first
    /// batch replaces the rows shown.
    GraphRestarted {
        generation: u64,
    },
    /// The history for graph load `generation` of a focused branch.
    BranchHistoryLoaded {
        generation: u64,
        history: BranchHistory,
    },
    /// What a whole-graph load would walk from now (`None` if that couldn't
    /// be listed), checked after refs changed while load `generation` was
    /// the graph's latest.
    GraphTipsChecked {
        generation: u64,
        tips: Option<Vec<u8>>,
    },
    /// Boxed, being several times the size of any other action.
    RefsLoaded(Result<Box<RefsSnapshot>, String>),
    StatusLoaded(Result<Status, String>),
    /// Refs or status load `generation` finished with `result`, its
    /// `RefsLoaded` or `StatusLoaded`. These loads run one at a time each,
    /// and only the newest result is applied.
    Reloaded {
        generation: u64,
        result: Box<Action>,
    },
    /// The graph selection moved to this commit.
    CommitSelected(String),
    /// The status pane selection moved to this file.
    FileSelected(StatusEntry),
    /// The status pane selection moved to this folder in its tree, holding
    /// `entries`.
    FolderSelected {
        folder: String,
        entries: Vec<StatusEntry>,
    },
    /// The file watcher saw changes.
    RepoChanged {
        refs: bool,
        status: bool,
    },
    /// The branch list selection moved to this stash.
    StashSelected(Stash),
    /// The branch list selection moved to a branch whose tip is this commit.
    BranchSelected(String),
    /// The worktrees tab is showing: count each worktree's changes.
    CountWorktreeChanges,
    /// How many files have changes in each worktree (by path), or why it
    /// couldn't be told, and how many commits its HEAD has that the base
    /// branch doesn't (`None` with no base, or when git couldn't tell).
    WorktreeChanges(Vec<(std::path::PathBuf, Result<usize, String>, Option<u32>)>),
    /// The bytes the folder of the worktree at this path takes, or why it
    /// couldn't be read.
    WorktreeSize(std::path::PathBuf, Result<u64, String>),
    /// What merging each of `tips` (commits) into the base branch's commit
    /// `base` would leave in conflict: by tip, the conflicted files, none
    /// when it merges cleanly. `None` when git is too old to tell.
    MergesChecked {
        base: String,
        tips: Vec<String>,
        result: Result<Option<std::collections::HashMap<String, Vec<String>>>, String>,
    },
    /// Close this repository and open the worktree at this path instead.
    OpenWorktree(std::path::PathBuf),
    /// The branch the graph should show on its own, or `None` for the
    /// whole graph.
    BranchFocused(Option<BranchFocus>),
    /// Run a git write operation in the background.
    Run(GitOp),
    /// Check out `plan`, offering to stash first if there are uncommitted
    /// changes.
    Checkout(CheckoutPlan),
    /// Reset the current branch to `target` (a commit, branch or tag),
    /// shown as `display`: works out what it would lose, then asks how.
    Reset {
        target: String,
        display: String,
    },
    /// What resetting to `target` would lose was worked out (or the error).
    ResetPlanned {
        target: String,
        display: String,
        plan: Result<ResetPlan, String>,
    },
    /// The next undo step was worked out (or why there's none,
    /// or the error reading the reflog).
    UndoPlanned(Result<Result<UndoStep, NoStep>, String>),
    /// A write operation finished; `result` is git's output or error text.
    /// `head` is HEAD's commit afterwards.
    OpDone {
        op: GitOp,
        result: Result<String, String>,
        head: Option<String>,
    },
    /// [`GitOp::PruneGone`] fetched, and found these branches.
    GoneFound(Result<GoneBranches, String>),
    /// [`GitOp::DeleteBranches`] finished.
    BranchesDeleted {
        names: Vec<String>,
        force: bool,
        result: BranchDeletes,
    },
    /// Open this file in the user's editor.
    OpenInEditor(EditTarget),
    /// A copy of a file that isn't in the working tree, written to open in
    /// the editor, or why it couldn't be.
    EditSnapshot(Result<Snapshot, String>),
    /// The editor from `OpenInEditor` exited (`Ok` with its exit code, when
    /// it has one) or, for one with a window of its own, started. `Err` says
    /// why it couldn't start.
    EditorFinished(Result<Option<i32>, String>),
    /// The agent was started in herdr, or is ready to start in a pane
    /// (`Err` says why it couldn't be).
    AgentStarted(Result<crate::agent::AgentStart, String>),
    /// A request arrived on the API socket.
    Socket(Call),
    /// A plugin command finished.
    PluginFinished(std::sync::Arc<CommandLog>),
    /// A plugin pane produced output (redraw).
    PaneOutput(u64),
    /// A plugin pane's process ended.
    PaneExited {
        id: u64,
        status: String,
    },
    OpenModal(Modal),
    /// Fetch more commits than the current graph limit.
    LoadMoreCommits,
    /// Text for the details pane (`git show` or a file diff).
    DetailsLoaded {
        subject: DetailsSubject,
        text: String,
        /// The code in `text`'s diffs, line by line.
        syntax: LineTokens,
    },
    /// A commit's header and changed files, for the details files view.
    CommitFilesLoaded {
        oid: String,
        header: String,
        files: Result<Vec<CommitFile>, String>,
    },
    /// The details files view selected a file whose diff isn't loaded yet.
    CommitFileSelected {
        oid: String,
        file: CommitFile,
    },
    /// One file's diff in a commit.
    CommitFileDiffLoaded {
        oid: String,
        path: String,
        text: String,
        syntax: LineTokens,
    },
    /// Both sides of a file's changes, for the staging view, with the code
    /// in each side's hunks.
    StageDiffsLoaded {
        entry: StatusEntry,
        unstaged: StageSide,
        staged: StageSide,
        syntax: (HunkTokens, HunkTokens),
    },
    /// Blame `path` as of `rev` (the working tree when `None`) for blame
    /// load `id`.
    LoadBlame {
        id: u64,
        rev: Option<String>,
        path: String,
    },
    /// Blame load `id` finished.
    BlameLoaded {
        id: u64,
        result: Result<tt_git_core::Blame, String>,
        /// The code, line by line.
        syntax: LineTokens,
        /// When the file last changed in a commit, as of the version blamed.
        changed: Option<i64>,
        /// How the version blamed stands against `HEAD`'s.
        recency: Option<tt_git_core::Recency>,
    },
    /// Find the version of `path` before the one at `rev` (the working tree
    /// when `None`), or with `newest`, after it on the way to the version at
    /// that rev and path, for blame load `id`.
    StepBlame {
        id: u64,
        rev: Option<String>,
        path: String,
        newest: Option<(Option<String>, String)>,
    },
    /// The version (rev and path) blame load `id` stepped to; `None` when
    /// there's none that way.
    BlameStepped {
        id: u64,
        version: Result<Option<(String, String)>, String>,
    },
    /// Load a commit's full message, for the blame view's commit details.
    LoadCommitMessage(String),
    /// A commit's message below its subject line (empty when there's none).
    CommitMessageLoaded {
        oid: String,
        body: String,
    },
    /// The authors or files for a list in the graph's filter panel.
    FilterChoicesLoaded {
        tab: FilterTab,
        choices: Result<Vec<Choice>, String>,
    },
}
