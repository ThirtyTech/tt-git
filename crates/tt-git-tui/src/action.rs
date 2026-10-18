//! Messages that drive [`App::update`](crate::App::update).

use tt_git_core::{
    BaseBranches, BranchEntry, BranchHistory, Checkout, Commit, CommitFile, GitRef, Head, Status, StatusEntry,
};
use tt_git_plugin::runtime::CommandLog;
use tt_git_plugin::socket::Call;

use crate::filter::{Choice, FilterTab};
use crate::keymap::Command;
use crate::modal::{GitOp, Modal};
use crate::panes::{BranchFocus, DetailsSubject};

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
    /// The history for graph load `generation` of a focused branch.
    BranchHistoryLoaded {
        generation: u64,
        history: BranchHistory,
    },
    RefsLoaded(Result<RefsSnapshot, String>),
    StatusLoaded(Result<Status, String>),
    /// The graph selection moved to this commit.
    CommitSelected(String),
    /// The status pane selection moved to this file.
    FileSelected(StatusEntry),
    /// The file watcher saw changes.
    RepoChanged {
        refs: bool,
        status: bool,
    },
    /// The branch list selection moved to a branch whose tip is this commit.
    BranchSelected(String),
    /// The branch the graph should show on its own, or `None` for the
    /// whole graph.
    BranchFocused(Option<BranchFocus>),
    /// Run a git write operation in the background.
    Run(GitOp),
    /// A write operation finished; `result` is git's output or error text.
    /// `head` is HEAD's commit afterwards.
    OpDone {
        op: GitOp,
        result: Result<String, String>,
        head: Option<String>,
    },
    /// A request arrived on the API socket.
    Socket(Call),
    /// A plugin command finished.
    PluginFinished(CommandLog),
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
    },
    /// The authors or files for a list in the graph's filter panel.
    FilterChoicesLoaded {
        tab: FilterTab,
        choices: Result<Vec<Choice>, String>,
    },
}
