//! Git backend, data models and output parsers shared by every tt-git crate.
//!
//! All git access goes through the git CLI ([`git::GitCli`]) rather than a
//! library binding: it streams `--topo-order --all` logs fastest (using the
//! commit-graph), and fetch/push get the user's real credential helpers,
//! SSH config and hooks on every platform.

pub mod branches;
pub mod git;
pub mod model;
pub mod ops;
pub mod parse;
pub mod paths;
pub mod repo;
pub mod time;

pub use branches::{BaseBranches, BranchEntry, BranchState, CheckoutPlan};
pub use git::{GitCli, GitError};
pub use model::*;
pub use ops::DiscardKind;
pub use repo::{LogFilter, LogScope, Repo};
