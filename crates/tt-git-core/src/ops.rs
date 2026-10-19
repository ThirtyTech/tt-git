//! Write operations. Each returns git's combined output on success so the
//! UI can show it, or a [`GitError`] carrying stderr.

use crate::branches::CheckoutPlan;
use crate::git::{GitError, GitOutput, Result};
use crate::repo::{BASE_BRANCH_KEY, Repo, WORKTREE_DIR};

fn text(out: GitOutput) -> String {
    let mut s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let err = String::from_utf8_lossy(&out.stderr);
    let err = err.trim();
    if !err.is_empty() {
        if !s.is_empty() {
            s.push('\n');
        }
        s.push_str(err);
    }
    s
}

impl Repo {
    /// Checks out a branch entry. Remote-only branches get a local tracking
    /// branch created without asking.
    pub fn checkout(&self, plan: &CheckoutPlan) -> Result<String> {
        let out = match plan {
            CheckoutPlan::Local(name) => self.git().run(&["switch", name.as_str()])?,
            CheckoutPlan::Track { name, remote_ref } => {
                self.git()
                    .run(&["switch", "-c", name.as_str(), "--track", remote_ref.as_str()])?
            }
        };
        Ok(text(out))
    }

    /// Creates `name` at `start` (a commit or ref) and switches to it.
    pub fn create_branch(&self, name: &str, start: &str) -> Result<String> {
        self.git().run(&["switch", "-c", name, start]).map(text)
    }

    /// Creates `name` at `start` checked out in a new worktree at
    /// [`worktree_path`](Self::worktree_path), leaving this one as it is.
    /// The worktrees folder is added to the repo's `info/exclude` so it
    /// doesn't show up as untracked.
    pub fn create_worktree(&self, name: &str, start: &str) -> Result<String> {
        self.exclude_worktrees().map_err(GitError::Spawn)?;
        let path = self.worktree_path(name).to_string_lossy().into_owned();
        self.git()
            .run(&["worktree", "add", "-b", name, path.as_str(), start])
            .map(text)
    }

    /// Adds `/.worktrees/` to `info/exclude` unless it's already there.
    fn exclude_worktrees(&self) -> std::io::Result<()> {
        let pattern = format!("/{WORKTREE_DIR}/");
        let path = self.common_dir().join("info").join("exclude");
        let current = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        if current.lines().any(|l| l.trim() == pattern) {
            return Ok(());
        }
        let mut updated = current;
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push_str(&pattern);
        updated.push('\n');
        std::fs::create_dir_all(path.parent().expect("info dir"))?;
        std::fs::write(&path, updated)
    }

    /// Deletes a local branch; without `force` git refuses unmerged ones.
    pub fn delete_branch(&self, name: &str, force: bool) -> Result<String> {
        let flag = if force { "-D" } else { "-d" };
        self.git().run(&["branch", flag, name]).map(text)
    }

    /// Deletes `branch` on `remote` (`git push <remote> --delete <branch>`).
    pub fn delete_remote_branch(&self, remote: &str, branch: &str) -> Result<String> {
        self.git().run(&["push", remote, "--delete", branch]).map(text)
    }

    /// Deletes a local tag (`git tag -d`).
    pub fn delete_tag(&self, name: &str) -> Result<String> {
        self.git().run(&["tag", "-d", name]).map(text)
    }

    pub fn rename_branch(&self, old: &str, new: &str) -> Result<String> {
        self.git().run(&["branch", "-m", old, new]).map(text)
    }

    /// `git fetch --all --prune`
    pub fn fetch_all(&self) -> Result<String> {
        self.git().run(&["fetch", "--all", "--prune"]).map(text)
    }

    /// Saves the base branch for this repo in its local git config, or with
    /// `None` goes back to detecting it.
    pub fn set_base_branch(&self, base: Option<&str>) -> Result<String> {
        match base {
            Some(name) => self.git().run(&["config", "--local", BASE_BRANCH_KEY, name]).map(text),
            None => match self.git().run(&["config", "--local", "--unset", BASE_BRANCH_KEY]) {
                Ok(out) => Ok(text(out)),
                // Exit 5: it wasn't set.
                Err(GitError::Failed { code, .. }) if code == "5" => Ok(String::new()),
                Err(e) => Err(e),
            },
        }
    }
}

/// What `discard` has to undo for a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscardKind {
    /// Not tracked: delete it from disk.
    Untracked,
    /// Newly added to the index (not in HEAD): remove from index and disk.
    Added,
    /// Renamed in the index: restore the original and drop the new path.
    Renamed { orig: String },
    /// Tracked: restore index and worktree from HEAD.
    Tracked,
}

impl DiscardKind {
    pub fn of(entry: &crate::model::StatusEntry) -> Self {
        use crate::model::FileState;
        if entry.is_untracked() {
            DiscardKind::Untracked
        } else if entry.index == FileState::Added {
            DiscardKind::Added
        } else if let (FileState::Renamed, Some(orig)) = (entry.index, &entry.orig_path) {
            DiscardKind::Renamed { orig: orig.clone() }
        } else {
            DiscardKind::Tracked
        }
    }
}

impl Repo {
    fn head_is_unborn(&self) -> bool {
        self.git()
            .run(&["rev-parse", "-q", "--verify", "HEAD^{commit}"])
            .is_err()
    }

    pub fn stage(&self, paths: &[String]) -> Result<String> {
        let mut args = vec!["add", "--"];
        args.extend(paths.iter().map(String::as_str));
        self.git().run(&args).map(text)
    }

    pub fn unstage(&self, paths: &[String]) -> Result<String> {
        let mut args = if self.head_is_unborn() {
            vec!["rm", "--cached", "-r", "-q", "--"]
        } else {
            vec!["restore", "--staged", "--"]
        };
        args.extend(paths.iter().map(String::as_str));
        self.git().run(&args).map(text)
    }

    pub fn stage_all(&self) -> Result<String> {
        self.git().run(&["add", "-A"]).map(text)
    }

    pub fn unstage_all(&self) -> Result<String> {
        if self.head_is_unborn() {
            self.git().run(&["rm", "--cached", "-r", "-q", "."]).map(text)
        } else {
            self.git().run(&["reset", "-q"]).map(text)
        }
    }

    /// Throws away all changes (staged and unstaged) to `path`.
    pub fn discard(&self, path: &str, kind: &DiscardKind) -> Result<String> {
        match kind {
            DiscardKind::Untracked => {
                let full = self.root().join(path);
                let result = if full.is_dir() {
                    std::fs::remove_dir_all(&full)
                } else {
                    std::fs::remove_file(&full)
                };
                result.map_err(crate::git::GitError::Spawn)?;
                Ok(String::new())
            }
            DiscardKind::Added => self.git().run(&["rm", "-f", "-q", "--", path]).map(text),
            DiscardKind::Renamed { orig } => {
                self.git().run(&["rm", "-f", "-q", "--", path])?;
                self.git()
                    .run(&["restore", "--source=HEAD", "--staged", "--worktree", "--", orig])
                    .map(text)
            }
            DiscardKind::Tracked => self
                .git()
                .run(&["restore", "--source=HEAD", "--staged", "--worktree", "--", path])
                .map(text),
        }
    }

    /// Commits the index with `message`; with `amend` and no message, amends
    /// keeping the previous message.
    pub fn commit(&self, message: Option<&str>, amend: bool) -> Result<String> {
        let mut args = vec!["commit"];
        if amend {
            args.push("--amend");
        }
        match message {
            Some(msg) => {
                args.extend(["-F", "-"]);
                self.git().run_with_stdin(&args, Some(msg.as_bytes())).map(text)
            }
            None => {
                args.push("--no-edit");
                self.git().run(&args).map(text)
            }
        }
    }

    /// Pushes the current branch, setting an upstream on first push.
    pub fn push(&self) -> Result<String> {
        let has_upstream = self
            .git()
            .run(&["rev-parse", "--abbrev-ref", "--verify", "-q", "@{upstream}"])
            .is_ok();
        if has_upstream {
            return self.git().run(&["push"]).map(text);
        }
        let remotes = self.remotes()?;
        let remote = remotes
            .iter()
            .find(|r| *r == crate::branches::PREFERRED_REMOTE)
            .or(remotes.first())
            .ok_or_else(|| crate::git::GitError::Parse("no remote configured to push to".into()))?;
        self.git().run(&["push", "-u", remote.as_str(), "HEAD"]).map(text)
    }

    /// `git pull --no-edit` (merge or rebase per the user's config).
    pub fn pull(&self) -> Result<String> {
        self.git().run(&["pull", "--no-edit"]).map(text)
    }

    /// Diff of one file for the details pane: staged and unstaged changes,
    /// or the whole content of an untracked file.
    pub fn file_diff(&self, entry: &crate::model::StatusEntry, max_bytes: usize) -> Result<String> {
        if entry.is_untracked() {
            let full = self.root().join(&entry.path);
            if full.is_dir() {
                return Ok(format!("{} is an untracked directory", entry.path));
            }
            let bytes = std::fs::read(&full).map_err(crate::git::GitError::Spawn)?;
            if bytes.contains(&0) {
                return Ok(format!("{} (untracked binary file, {} bytes)", entry.path, bytes.len()));
            }
            let content = String::from_utf8_lossy(&bytes[..bytes.len().min(max_bytes)]);
            let mut out = format!("new file {}\n", entry.path);
            for line in content.lines() {
                out.push('+');
                out.push_str(line);
                out.push('\n');
            }
            return Ok(out);
        }
        let mut out = String::new();
        let mut paths = vec![entry.path.as_str()];
        if let Some(orig) = &entry.orig_path {
            paths.push(orig);
        }
        if entry.has_staged() {
            let mut args = vec!["diff", "--cached", "-M", "--"];
            args.extend(&paths);
            out.push_str("── staged ──\n");
            out.push_str(&String::from_utf8_lossy(&self.git().run(&args)?.stdout));
        }
        if entry.has_unstaged() {
            let mut args = vec!["diff", "--"];
            args.extend(&paths);
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str("── unstaged ──\n");
            out.push_str(&String::from_utf8_lossy(&self.git().run(&args)?.stdout));
        }
        Ok(out)
    }
}

impl Repo {
    /// `git merge --no-edit <target>` into the current branch.
    pub fn merge(&self, target: &str) -> Result<String> {
        self.git().run(&["merge", "--no-edit", target]).map(text)
    }

    /// Rebases the current branch onto `onto` (non-interactive).
    pub fn rebase(&self, onto: &str) -> Result<String> {
        self.git().run(&["-c", "core.editor=:", "rebase", onto]).map(text)
    }

    pub fn cherry_pick(&self, oid: &str) -> Result<String> {
        self.git().run(&["cherry-pick", oid]).map(text)
    }

    /// Continues the operation in progress after conflicts were resolved
    /// and staged, keeping git's prepared commit messages.
    pub fn continue_operation(&self, op: crate::model::InProgress) -> Result<String> {
        use crate::model::InProgress;
        let out = match op {
            // A merge is concluded by committing.
            InProgress::Merge => self.git().run(&["-c", "core.editor=:", "commit", "--no-edit"])?,
            _ => self.git().run(&["-c", "core.editor=:", op.command(), "--continue"])?,
        };
        Ok(text(out))
    }

    pub fn abort_operation(&self, op: crate::model::InProgress) -> Result<String> {
        self.git().run(&[op.command(), "--abort"]).map(text)
    }
}
