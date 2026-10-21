//! Plain data types produced by the parsers.

use std::sync::Arc;

use serde::Serialize;
use smallvec::SmallVec;

pub use crate::oid::{Oid, OidHex, ParseOidError};

/// A commit's parent ids, first parent first. Up to two (every commit but
/// an octopus merge) are stored inline.
pub type Parents = SmallVec<[Oid; 2]>;

/// A commit as loaded for the graph.
///
/// Kept compact, since the graph holds tens of thousands: ids are bytes,
/// and a log shares one allocation per distinct author name and email.
/// It serializes exactly as it did with `String` fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Commit {
    /// Full object id.
    pub oid: Oid,
    /// Parent ids, first parent first.
    pub parents: Parents,
    pub author_name: Arc<str>,
    pub author_email: Arc<str>,
    /// Author time, unix seconds.
    pub author_time: i64,
    /// Committer time, unix seconds.
    pub commit_time: i64,
    pub summary: Box<str>,
}

impl Commit {
    /// The first 7 hex digits of the id.
    pub fn short_oid(&self) -> ShortOid {
        ShortOid(self.oid.hex())
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
    pub fork: Option<Oid>,
    /// The base commit that merged the branch in.
    pub merge: Option<Oid>,
}

/// [`Commit::short_oid`]: up to 7 hex digits, on the stack.
#[derive(Clone, Copy)]
pub struct ShortOid(OidHex);

impl ShortOid {
    pub fn as_str(&self) -> &str {
        let hex = self.0.as_str();
        &hex[..hex.len().min(7)]
    }
}

impl std::ops::Deref for ShortOid {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for ShortOid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self.as_str(), f)
    }
}

impl std::fmt::Debug for ShortOid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.as_str(), f)
    }
}

impl PartialEq<&str> for ShortOid {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
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

/// One working tree of the repository, as `git worktree list --porcelain`
/// lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Worktree {
    /// As git prints it (forward slashes on Windows too).
    pub path: std::path::PathBuf,
    /// The commit checked out; `None` for a bare repository or an unborn branch.
    pub head: Option<String>,
    /// The branch checked out, short name; `None` when detached (or bare).
    pub branch: Option<String>,
    /// The main working tree (git lists it first). It can't be removed.
    pub main: bool,
    /// The bare repository itself, which has no working tree to open.
    pub bare: bool,
    /// `git worktree lock`ed, with the reason given (empty if none).
    pub locked: Option<String>,
    /// Its folder is gone; `git worktree prune` would drop it. Why, as git says.
    pub prunable: Option<String>,
    /// The working tree this [`Repo`](crate::Repo) was opened in.
    pub current: bool,
}

/// One entry of `git stash list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Stash {
    /// Its place in the list, newest first: `stash@{index}`.
    pub index: usize,
    pub oid: String,
    /// The commit holding the untracked files stashed with it, if any.
    pub untracked: Option<String>,
    /// When it was made.
    pub date: i64,
    /// `On main: message`, or `WIP on main: <commit> <subject>` without one.
    pub message: String,
}

impl Stash {
    /// `stash@{n}`, how git commands name it.
    pub fn name(&self) -> String {
        format!("stash@{{{}}}", self.index)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::parse::{FS, Idents, parse_commit_interned};

    /// The `String`-based layout `Commit` had before it was made compact.
    mod before {
        #[derive(Debug, serde::Serialize)]
        pub struct Commit {
            pub oid: String,
            pub parents: Vec<String>,
            pub author_name: String,
            pub author_email: String,
            pub author_time: i64,
            pub commit_time: i64,
            pub summary: String,
        }
    }

    const A: &str = "0123456789abcdef0123456789abcdef01234567";
    const B: &str = "89abcdef0123456789abcdef0123456789abcdef";
    const C: &str = "fedcba9876543210fedcba9876543210fedcba98";

    fn parse(oid: &str, parents: &str, idents: &mut Idents) -> super::Commit {
        let rec = format!("{oid}{FS}{parents}{FS}Ann \"A\" Author{FS}ann@example.com{FS}100{FS}200{FS}Say \"hi\" ✓");
        parse_commit_interned(rec.as_bytes(), idents).unwrap()
    }

    fn before(c: &super::Commit) -> before::Commit {
        before::Commit {
            oid: c.oid.to_string(),
            parents: c.parents.iter().map(|p| p.to_string()).collect(),
            author_name: c.author_name.to_string(),
            author_email: c.author_email.to_string(),
            author_time: c.author_time,
            commit_time: c.commit_time,
            summary: c.summary.to_string(),
        }
    }

    #[test]
    fn serializes_and_debugs_like_the_string_layout() {
        let mut idents = Idents::default();
        for parents in [String::new(), B.to_string(), format!("{B} {C}"), format!("{A} {B} {C}")] {
            let c = parse(A, &parents, &mut idents);
            let old = before(&c);
            assert_eq!(old.parents.join(" "), parents);
            assert_eq!(serde_json::to_string(&c).unwrap(), serde_json::to_string(&old).unwrap());
            assert_eq!(format!("{c:?}"), format!("{old:?}"));
            assert_eq!(format!("{c:#?}"), format!("{old:#?}"));
        }
    }

    #[test]
    fn short_oid_is_the_first_seven_digits() {
        let c = parse(A, "", &mut Idents::default());
        assert_eq!(c.short_oid(), "0123456");
        assert_eq!(format!("[{}]", c.short_oid()), "[0123456]");
    }

    #[test]
    fn one_log_shares_author_strings() {
        let mut idents = Idents::default();
        let a = parse(A, "", &mut idents);
        let b = parse(B, A, &mut idents);
        assert!(Arc::ptr_eq(&a.author_name, &b.author_name));
        assert!(Arc::ptr_eq(&a.author_email, &b.author_email));
    }

    #[test]
    fn is_compact() {
        // Was 136 bytes plus 6 or more heap blocks; now one (the summary).
        assert!(
            std::mem::size_of::<super::Commit>() <= 192,
            "{}",
            std::mem::size_of::<super::Commit>()
        );
    }
}
