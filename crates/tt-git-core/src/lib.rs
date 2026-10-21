//! Git backend, data models and output parsers shared by every tt-git crate.
//!
//! All git access goes through the git CLI ([`git::GitCli`]) rather than a
//! library binding: it streams `--topo-order --all` logs fastest (using the
//! commit-graph), and fetch/push get the user's real credential helpers,
//! SSH config and hooks on every platform.

pub mod blame;
pub mod branches;
pub mod git;
pub mod ignored;
pub mod model;
mod oid;
pub mod ops;
pub mod parse;
pub mod patch;
pub mod paths;
pub mod repo;
pub mod time;
pub mod undo;

pub use blame::{Blame, BlameCommit, BlameLine};
pub use branches::{AheadBehind, BaseBranches, BranchEntry, BranchState, CheckoutPlan};
pub use git::{GitCli, GitError};
pub use ignored::IgnoreRules;
pub use model::*;
pub use ops::{DiscardKind, PatchOp, STAGED_ASIDE, StageSide, StashScope, UNDO_CHANGES_KEPT};
pub use repo::{LogFilter, LogScope, RemoteConfig, Repo};
pub use undo::{NoStep, UndoStep, UndoTarget};
