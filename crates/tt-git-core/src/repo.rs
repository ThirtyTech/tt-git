//! A discovered repository and its read operations.

use std::path::{Path, PathBuf};

use crate::git::{GitCli, GitError, Result};
use crate::model::{Checkout, Commit, GitRef, Head, Status};
use crate::parse;

/// Which commits the graph loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogScope {
    /// Every ref (`--all`), minus the stash.
    All,
    /// HEAD plus, when given, its upstream (full ref name) so incoming
    /// commits are visible.
    Head { upstream: Option<String> },
}

#[derive(Debug, Clone)]
pub struct Repo {
    git: GitCli,
    root: PathBuf,
    git_dir: PathBuf,
    common_dir: PathBuf,
}

impl Repo {
    /// Finds the repository containing `path`. `git` supplies the runner
    /// configuration (env, shared log); its directory is replaced.
    pub fn discover(path: &Path, git: &GitCli) -> Result<Self> {
        let probe = git.in_dir(path);
        let out = probe
            .run(&["rev-parse", "--show-toplevel", "--absolute-git-dir", "--git-common-dir"])
            .map_err(|e| match e {
                GitError::Failed { ref stderr, .. } if stderr.contains("not a git repository") => {
                    GitError::NotARepo(path.to_path_buf())
                }
                other => other,
            })?;
        let text = out.stdout_str();
        let mut lines = text.lines();
        let (Some(root), Some(git_dir), Some(common)) = (lines.next(), lines.next(), lines.next()) else {
            // A bare repo prints no toplevel; we need a working tree.
            return Err(GitError::NotARepo(path.to_path_buf()));
        };
        let root = PathBuf::from(root);
        let git_dir = PathBuf::from(git_dir);
        let common = PathBuf::from(common);
        let common_dir = if common.is_absolute() {
            common
        } else {
            path.join(common)
        };
        Ok(Self {
            git: git.in_dir(&root),
            root,
            git_dir,
            common_dir,
        })
    }

    pub fn git(&self) -> &GitCli {
        &self.git
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory name of the working tree.
    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string())
    }

    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// The shared `.git` directory (differs from `git_dir` in linked worktrees).
    pub fn common_dir(&self) -> &Path {
        &self.common_dir
    }

    pub fn head(&self) -> Result<Head> {
        let branch = match self.git.run(&["symbolic-ref", "-q", "--short", "HEAD"]) {
            Ok(out) => Some(out.stdout_str().trim().to_string()),
            Err(GitError::Failed { .. }) => None,
            Err(e) => return Err(e),
        };
        let oid = match self.git.run(&["rev-parse", "-q", "--verify", "HEAD^{commit}"]) {
            Ok(out) => Some(out.stdout_str().trim().to_string()),
            Err(GitError::Failed { .. }) => None,
            Err(e) => return Err(e),
        };
        match (branch, oid) {
            (Some(name), Some(oid)) => Ok(Head::Branch { name, oid }),
            (Some(name), None) => Ok(Head::Unborn { name }),
            (None, Some(oid)) => Ok(Head::Detached { oid }),
            (None, None) => Err(GitError::Parse("HEAD is neither a branch nor a commit".into())),
        }
    }

    pub fn remotes(&self) -> Result<Vec<String>> {
        let out = self.git.run(&["remote"])?;
        Ok(out
            .stdout_str()
            .lines()
            .map(str::to_string)
            .filter(|s| !s.is_empty())
            .collect())
    }

    /// Local branches, remote-tracking branches and tags.
    pub fn refs(&self) -> Result<Vec<GitRef>> {
        let remotes = self.remotes()?;
        let out = self.git.run(&[
            "for-each-ref",
            parse::REF_FORMAT,
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ])?;
        parse::parse_refs(&out.stdout, &remotes)
    }

    pub fn status(&self) -> Result<Status> {
        let out = self
            .git
            .run(&["status", "--porcelain=v2", "-z", "--branch", "--untracked-files=all"])?;
        let mut status = parse::parse_status(&out.stdout)?;
        status.in_progress = self.in_progress();
        Ok(status)
    }

    /// Which multi-step operation, if any, is waiting to be continued.
    pub fn in_progress(&self) -> Option<crate::model::InProgress> {
        use crate::model::InProgress;
        let g = &self.git_dir;
        if g.join("rebase-merge").is_dir() || g.join("rebase-apply").is_dir() {
            Some(InProgress::Rebase)
        } else if g.join("MERGE_HEAD").is_file() {
            Some(InProgress::Merge)
        } else if g.join("CHERRY_PICK_HEAD").is_file() {
            Some(InProgress::CherryPick)
        } else if g.join("REVERT_HEAD").is_file() {
            Some(InProgress::Revert)
        } else {
            None
        }
    }

    /// Recent `checkout: moving from A to B` entries from HEAD's reflog,
    /// newest first. Empty when there is no reflog yet.
    pub fn checkouts(&self, limit: usize) -> Result<Vec<Checkout>> {
        let max = format!("--max-count={limit}");
        match self
            .git
            .run(&["log", "-g", "--date=unix", parse::REFLOG_FORMAT, &max, "HEAD", "--"])
        {
            Ok(out) => Ok(parse::parse_checkouts(&out.stdout)),
            Err(GitError::Failed { .. }) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Time of the most recent update to `full_ref` according to its reflog
    /// file, if it has one.
    pub fn ref_activity(&self, full_ref: &str) -> Option<i64> {
        let rel: PathBuf = full_ref.split('/').collect();
        let path = self.common_dir.join("logs").join(rel);
        std::fs::read(path).ok().and_then(|c| parse::last_reflog_time(&c))
    }

    /// Streams commits in topological order, newest first, stopping after
    /// `limit` or when `on_commit` returns `false`.
    pub fn stream_commits(
        &self,
        scope: &LogScope,
        limit: usize,
        mut on_commit: impl FnMut(Commit) -> bool,
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "log".into(),
            "--topo-order".into(),
            "-z".into(),
            parse::LOG_FORMAT.into(),
            format!("--max-count={limit}"),
        ];
        match scope {
            LogScope::All => {
                args.push("--exclude=refs/stash".into());
                // --all includes HEAD (even when detached) and, unlike an
                // explicit HEAD argument, does not fail on an unborn branch.
                args.push("--all".into());
            }
            LogScope::Head { upstream } => {
                if self.head()?.oid().is_none() {
                    return Ok(());
                }
                args.push("HEAD".into());
                if let Some(up) = upstream {
                    args.push(up.clone());
                }
            }
        }
        args.push("--".into());

        let mut error = None;
        self.git.stream(&args, 0, |record| {
            if record.iter().all(|b| b.is_ascii_whitespace()) {
                return true;
            }
            match parse::parse_commit(record) {
                Ok(c) => on_commit(c),
                Err(e) => {
                    error = Some(e);
                    false
                }
            }
        })?;
        match error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Loads up to `limit` commits into a Vec.
    pub fn commits(&self, scope: &LogScope, limit: usize) -> Result<Vec<Commit>> {
        let mut commits = Vec::new();
        self.stream_commits(scope, limit, |c| {
            commits.push(c);
            true
        })?;
        Ok(commits)
    }
}
