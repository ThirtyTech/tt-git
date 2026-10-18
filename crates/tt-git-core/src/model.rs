//! Plain data types produced by the parsers.

use serde::Serialize;

/// A commit as loaded for the graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Commit {
    /// Full hex object id.
    pub oid: String,
    /// Parent ids, first parent first.
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    /// Author time, unix seconds.
    pub author_time: i64,
    /// Committer time, unix seconds.
    pub commit_time: i64,
    pub summary: String,
}

impl Commit {
    pub fn short_oid(&self) -> &str {
        &self.oid[..self.oid.len().min(7)]
    }

    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}

/// One branch's own commits against a base branch, from where it branched
/// off to where it was merged back in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BranchHistory {
    /// Newest first: the base commit that merged the branch in (or, while
    /// unmerged, the base's tip), the branch's first-parent commits, then
    /// the base commit it branched off. Parents only name commits in the
    /// list, so the base's side is one edge from the merge (or tip) down to
    /// the fork.
    pub commits: Vec<Commit>,
    /// The base commit the branch branched off.
    pub fork: Option<String>,
    /// The base commit that merged the branch in.
    pub merge: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RefKind {
    LocalBranch,
    RemoteBranch { remote: String },
    Tag,
    Other,
}

/// A ref from `git for-each-ref`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GitRef {
    /// e.g. `refs/remotes/origin/main`
    pub full_name: String,
    /// e.g. `origin/main`
    pub short_name: String,
    pub kind: RefKind,
    /// The commit this ref points at (tags are peeled).
    pub target: String,
    /// Committer date of the tip (tagger date for annotated tags), unix seconds.
    pub date: i64,
    /// Full name of the configured upstream, if any.
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// The upstream is configured but no longer exists.
    pub upstream_gone: bool,
    /// This is the checked-out branch.
    pub is_head: bool,
    /// Set for symbolic refs such as `refs/remotes/origin/HEAD`.
    pub symref: Option<String>,
    pub subject: String,
}

impl GitRef {
    /// Branch name without the remote prefix (`origin/feature/x` → `feature/x`).
    pub fn branch_name(&self) -> &str {
        match &self.kind {
            RefKind::RemoteBranch { remote } => self
                .short_name
                .strip_prefix(remote.as_str())
                .and_then(|s| s.strip_prefix('/'))
                .unwrap_or(&self.short_name),
            _ => &self.short_name,
        }
    }
}

/// What HEAD points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Head {
    /// On a branch with at least one commit.
    Branch {
        name: String,
        oid: String,
    },
    /// On a branch with no commits yet.
    Unborn {
        name: String,
    },
    Detached {
        oid: String,
    },
}

impl Head {
    pub fn branch(&self) -> Option<&str> {
        match self {
            Head::Branch { name, .. } | Head::Unborn { name } => Some(name),
            Head::Detached { .. } => None,
        }
    }

    pub fn oid(&self) -> Option<&str> {
        match self {
            Head::Branch { oid, .. } | Head::Detached { oid } => Some(oid),
            Head::Unborn { .. } => None,
        }
    }
}

/// One side (index or worktree) of a status entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    Unmodified,
    Modified,
    TypeChanged,
    Added,
    Deleted,
    Renamed,
    Copied,
    Unmerged,
    Untracked,
    Ignored,
}

impl FileState {
    pub fn from_code(c: u8) -> Option<Self> {
        Some(match c {
            b'.' => FileState::Unmodified,
            b'M' => FileState::Modified,
            b'T' => FileState::TypeChanged,
            b'A' => FileState::Added,
            b'D' => FileState::Deleted,
            b'R' => FileState::Renamed,
            b'C' => FileState::Copied,
            b'U' => FileState::Unmerged,
            b'?' => FileState::Untracked,
            b'!' => FileState::Ignored,
            _ => return None,
        })
    }

    /// The short-status letter (`' '` for unmodified).
    pub fn code(self) -> char {
        match self {
            FileState::Unmodified => ' ',
            FileState::Modified => 'M',
            FileState::TypeChanged => 'T',
            FileState::Added => 'A',
            FileState::Deleted => 'D',
            FileState::Renamed => 'R',
            FileState::Copied => 'C',
            FileState::Unmerged => 'U',
            FileState::Untracked => '?',
            FileState::Ignored => '!',
        }
    }
}

/// A changed path from `git status --porcelain=v2`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusEntry {
    pub path: String,
    /// Source path of a rename or copy.
    pub orig_path: Option<String>,
    pub index: FileState,
    pub worktree: FileState,
}

impl StatusEntry {
    pub fn is_untracked(&self) -> bool {
        self.index == FileState::Untracked
    }

    pub fn is_conflicted(&self) -> bool {
        self.index == FileState::Unmerged || self.worktree == FileState::Unmerged
    }

    /// Has changes in the index.
    pub fn has_staged(&self) -> bool {
        !self.is_untracked() && !self.is_conflicted() && self.index != FileState::Unmodified
    }

    /// Has changes in the worktree not yet staged (including untracked).
    pub fn has_unstaged(&self) -> bool {
        self.is_untracked() || self.is_conflicted() || self.worktree != FileState::Unmodified
    }

    /// Two-letter short-status code, like `git status -s`.
    pub fn short_code(&self) -> String {
        if self.is_untracked() {
            return "??".into();
        }
        format!("{}{}", self.index.code(), self.worktree.code())
    }
}

/// The `# branch.*` headers of porcelain v2 status.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BranchStatus {
    /// `None` before the first commit.
    pub oid: Option<String>,
    /// `None` when detached.
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Status {
    pub branch: BranchStatus,
    pub entries: Vec<StatusEntry>,
    /// A merge, rebase, cherry-pick or revert waiting to be continued.
    pub in_progress: Option<InProgress>,
}

/// An operation git stopped in the middle of (usually on conflicts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InProgress {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl InProgress {
    pub fn label(self) -> &'static str {
        match self {
            InProgress::Merge => "MERGING",
            InProgress::Rebase => "REBASING",
            InProgress::CherryPick => "CHERRY-PICKING",
            InProgress::Revert => "REVERTING",
        }
    }

    /// The git subcommand that owns `--continue` / `--abort`.
    pub fn command(self) -> &'static str {
        match self {
            InProgress::Merge => "merge",
            InProgress::Rebase => "rebase",
            InProgress::CherryPick => "cherry-pick",
            InProgress::Revert => "revert",
        }
    }
}

impl Status {
    pub fn is_clean(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A path a commit changed, compared with its first parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitFile {
    pub state: FileState,
    pub path: String,
    /// Source path of a rename or copy.
    pub orig_path: Option<String>,
}

/// A `checkout: moving from A to B` entry from HEAD's reflog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Checkout {
    pub from: String,
    pub to: String,
    pub time: i64,
}
