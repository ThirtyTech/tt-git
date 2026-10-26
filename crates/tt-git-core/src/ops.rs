//! Write operations. Each returns git's combined output on success so the
//! UI can show it, or a [`GitError`] carrying stderr.

use std::collections::HashSet;

use crate::branches::CheckoutPlan;
use crate::git::{GitError, GitOutput, Result};
use crate::model::RefKind;
use crate::parse;
use crate::patch::{FileDiff, PatchDirection, untracked_diff};
use crate::repo::{BASE_BRANCH_KEY, Repo, WORKTREE_DIR};
use crate::undo::{self, NoStep, RepoRefs, UNDO_MARK, UndoStep, UndoTarget};

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
    /// Checks out a branch entry or a commit. Remote-only branches get a
    /// local tracking branch created without asking.
    pub fn checkout(&self, plan: &CheckoutPlan) -> Result<String> {
        let out = match plan {
            CheckoutPlan::Local(name) => self.git().run(&["switch", name.as_str()])?,
            CheckoutPlan::Track { name, remote_ref } => {
                self.git()
                    .run(&["switch", "-c", name.as_str(), "--track", remote_ref.as_str()])?
            }
            CheckoutPlan::Detach(oid) => self.git().run(&["switch", "--detach", oid.as_str()])?,
        };
        Ok(text(out))
    }

    /// Stashes every change, untracked files included, then checks out
    /// `plan`. If the checkout fails, the changes are popped back.
    pub fn stash_and_checkout(&self, plan: &CheckoutPlan) -> Result<String> {
        let before = self.stash_tip();
        let stashed = self.stash_push(None, &StashScope::All)?;
        match self.checkout(plan) {
            Ok(out) => Ok(format!("{stashed}\n{out}")),
            Err(e) => {
                // `stash push` with nothing to save makes no stash, and
                // stash@{0} is then someone else's.
                if self.stash_tip() != before
                    && let Err(pop) = self.stash_apply(0, true)
                {
                    tracing::warn!("restoring the stash after a failed checkout failed: {pop}");
                }
                Err(e)
            }
        }
    }

    /// What `refs/stash` points at, if there is a stash.
    fn stash_tip(&self) -> Option<String> {
        let out = self.git().run(&["rev-parse", "-q", "--verify", "refs/stash"]).ok()?;
        Some(out.into_stdout_string())
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

    /// `git worktree remove` the linked worktree at `path`: deletes its
    /// folder (ignored files too) and git's record of it; its branch stays.
    /// Without `force` git refuses one with uncommitted or untracked
    /// changes; it refuses a locked one either way. tt-git refuses the
    /// main worktree and the one it's running in.
    pub fn remove_worktree(&self, path: &std::path::Path, force: bool) -> Result<String> {
        if crate::repo::same_path(path, self.root()) {
            return Err(GitError::Refused(
                "tt-git is running in that worktree; open another one first".into(),
            ));
        }
        if crate::repo::same_path(path, self.main_root()) {
            return Err(GitError::Refused("the main worktree can't be removed".into()));
        }
        let path = path.to_string_lossy();
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(&path);
        self.git().run(&args).map(text)
    }

    /// Deletes a local branch; without `force` git refuses unmerged ones.
    pub fn delete_branch(&self, name: &str, force: bool) -> Result<String> {
        let flag = if force { "-D" } else { "-d" };
        self.git().run(&["branch", flag, name]).map(text)
    }

    /// Local branches whose upstream is configured but no longer exists,
    /// as `git fetch --prune` leaves them. Ones checked out here or in
    /// another worktree are listed apart, since git won't delete them.
    pub fn gone_branches(&self) -> Result<GoneBranches> {
        let checked_out: HashSet<String> = self.worktrees()?.into_iter().filter_map(|wt| wt.branch).collect();
        let mut gone = GoneBranches::default();
        for r in self.refs()? {
            if r.kind != RefKind::LocalBranch || !r.upstream_gone {
                continue;
            }
            if r.is_head || checked_out.contains(&r.short_name) {
                gone.checked_out.push(r.short_name);
            } else {
                gone.deletable.push(r.short_name);
            }
        }
        Ok(gone)
    }

    /// Deletes local branches one by one (`git branch -d`, or `-D` with
    /// `force`), carrying on past the ones git refuses.
    pub fn delete_branches(&self, names: &[String], force: bool) -> BranchDeletes {
        let mut out = BranchDeletes::default();
        for name in names {
            match self.delete_branch(name, force) {
                Ok(_) => out.deleted.push(name.clone()),
                Err(e) => {
                    let err = e.stderr().map(str::to_string).unwrap_or_else(|| e.to_string());
                    if !force && err.contains("not fully merged") {
                        out.unmerged.push(name.clone());
                    } else {
                        out.failed.push((name.clone(), err));
                    }
                }
            }
        }
        out
    }

    /// Deletes `branch` on `remote` (`git push <remote> --delete <branch>`).
    pub fn delete_remote_branch(&self, remote: &str, branch: &str) -> Result<String> {
        self.git().run(&["push", remote, "--delete", branch]).map(text)
    }

    /// Deletes a local tag (`git tag -d`).
    pub fn delete_tag(&self, name: &str) -> Result<String> {
        self.git().run(&["tag", "-d", name]).map(text)
    }

    /// Deletes tag `name` on `remote` (`git push <remote> --delete
    /// refs/tags/<name>`), leaving the local one.
    pub fn delete_remote_tag(&self, remote: &str, name: &str) -> Result<String> {
        let refspec = format!("refs/tags/{name}");
        self.git()
            .run(&["push", remote, "--delete", refspec.as_str()])
            .map(text)
    }

    /// Tags `target` as `name`: a lightweight tag, or with `message` an
    /// annotated one (`git tag -a -m`).
    pub fn create_tag(&self, name: &str, target: &str, message: Option<&str>) -> Result<String> {
        match message {
            Some(message) => self.git().run(&["tag", "-a", name, "-m", message, target]).map(text),
            None => self.git().run(&["tag", name, target]).map(text),
        }
    }

    /// Pushes the tag `name` to the remote [`push`](Self::push) would pick
    /// for a new branch (`git push <remote> refs/tags/<name>`).
    pub fn push_tag(&self, name: &str) -> Result<String> {
        let remote = self.push_remote()?;
        let refspec = format!("refs/tags/{name}");
        self.git().run(&["push", remote.as_str(), refspec.as_str()]).map(text)
    }

    /// `origin` if there is one, otherwise the first remote.
    fn push_remote(&self) -> Result<String> {
        let mut remotes = self.remotes()?;
        if remotes.is_empty() {
            return Err(GitError::Parse("no remote configured to push to".into()));
        }
        let at = remotes
            .iter()
            .position(|r| r == crate::branches::PREFERRED_REMOTE)
            .unwrap_or(0);
        Ok(remotes.swap_remove(at))
    }

    pub fn rename_branch(&self, old: &str, new: &str) -> Result<String> {
        self.git().run(&["branch", "-m", old, new]).map(text)
    }

    /// `git fetch --all --prune`
    pub fn fetch_all(&self) -> Result<String> {
        self.git().run(&["fetch", "--all", "--prune"]).map(text)
    }

    /// Fast-forwards local branch `name` to its upstream without checking
    /// it out: `git fetch <remote> <merge>:refs/heads/<name>`, where
    /// `<remote>` and `<merge>` are the branch's `branch.<name>.*` config
    /// (a local upstream has remote `.`). Refuses when there's no upstream,
    /// when the branch is checked out in any worktree, and when it has
    /// diverged from its upstream. Returns [`FAST_FORWARD_UP_TO_DATE`] when
    /// the branch was already there.
    pub fn fast_forward_branch(&self, name: &str) -> Result<String> {
        let Some((remote, merge)) = self.upstream_config(name) else {
            return Err(GitError::Refused(format!("{name} has no upstream to fast-forward to")));
        };
        if let Some(wt) = self.worktrees()?.iter().find(|w| w.branch.as_deref() == Some(name)) {
            return Err(GitError::Refused(if wt.current {
                format!("{name} is checked out here; pull it instead")
            } else {
                format!(
                    "{name} is checked out in the worktree at {}; pull it there instead",
                    wt.path.display()
                )
            }));
        }
        let local = format!("refs/heads/{name}");
        let tip = || {
            self.git()
                .run(&["rev-parse", "--verify", "-q", &local])
                .map(|out| out.stdout_str().trim().to_string())
        };
        let before = tip()?;
        let refspec = format!("{merge}:{local}");
        let out = match self.git().run(&["fetch", remote.as_str(), refspec.as_str()]) {
            Ok(out) => out,
            Err(GitError::Failed { stderr, .. }) if stderr.contains("non-fast-forward") => {
                return Err(GitError::Refused(format!(
                    "{name} has diverged from its upstream, so it can't be fast-forwarded; \
                     check it out and merge or rebase instead"
                )));
            }
            Err(e) => return Err(e),
        };
        if tip()? == before {
            return Ok(FAST_FORWARD_UP_TO_DATE.to_string());
        }
        Ok(text(out))
    }

    /// Local branch `name`'s `branch.<name>.remote` and `branch.<name>.merge`,
    /// when both are set.
    fn upstream_config(&self, name: &str) -> Option<(String, String)> {
        let config = |key: &str| {
            self.git()
                .run(&["config", "--get", &format!("branch.{name}.{key}")])
                .ok()
                .map(|out| out.stdout_str().trim().to_string())
                .filter(|v| !v.is_empty())
        };
        Some((config("remote")?, config("merge")?))
    }

    /// Pushes local branch `name` without checking it out: to its upstream
    /// (`git push <remote> refs/heads/<name>:<merge>`), or when it has none
    /// to the remote [`push`](Self::push) would pick, setting the upstream
    /// (`git push -u <remote> refs/heads/<name>`). Refuses a branch whose
    /// upstream is another local branch.
    pub fn push_branch(&self, name: &str) -> Result<String> {
        let local = format!("refs/heads/{name}");
        match self.upstream_config(name) {
            Some((remote, merge)) if remote == "." => {
                let other = merge.strip_prefix("refs/heads/").unwrap_or(&merge);
                Err(GitError::Refused(format!(
                    "{name} tracks local branch {other}; there's no remote to push it to"
                )))
            }
            Some((remote, merge)) => {
                let refspec = format!("{local}:{merge}");
                self.git().run(&["push", remote.as_str(), refspec.as_str()]).map(text)
            }
            None => {
                let remote = self.push_remote()?;
                self.git()
                    .run(&["push", "-u", remote.as_str(), local.as_str()])
                    .map(text)
            }
        }
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

/// What `discard_all` throws away. Ignored files and nested repositories
/// are always kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardScope {
    /// Staged, unstaged and untracked changes.
    Everything,
    /// Unstaged changes to tracked files; staged ones and untracked files stay.
    Unstaged,
    /// Unstaged changes and untracked files; staged ones stay.
    UnstagedAndUntracked,
    /// Untracked files only.
    Untracked,
}

impl DiscardScope {
    pub const ALL: [DiscardScope; 4] = [
        DiscardScope::Everything,
        DiscardScope::Unstaged,
        DiscardScope::UnstagedAndUntracked,
        DiscardScope::Untracked,
    ];

    /// The name plugins see in `files.discarded`.
    pub fn name(self) -> &'static str {
        match self {
            DiscardScope::Everything => "everything",
            DiscardScope::Unstaged => "unstaged",
            DiscardScope::UnstagedAndUntracked => "unstaged_and_untracked",
            DiscardScope::Untracked => "untracked",
        }
    }

    pub fn drops_staged(self) -> bool {
        self == DiscardScope::Everything
    }

    pub fn drops_unstaged(self) -> bool {
        self != DiscardScope::Untracked
    }

    pub fn drops_untracked(self) -> bool {
        self != DiscardScope::Unstaged
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

    /// Stages every change, or with `folder` only those under it.
    pub fn stage_all(&self, folder: Option<&str>) -> Result<String> {
        match folder {
            Some(folder) => self.git().run(&["add", "-A", "--", &literal(folder)]).map(text),
            None => self.git().run(&["add", "-A"]).map(text),
        }
    }

    /// Unstages everything, or with `folder` only what's under it.
    pub fn unstage_all(&self, folder: Option<&str>) -> Result<String> {
        match (folder.map(literal), self.head_is_unborn()) {
            (None, true) => self.git().run(&["rm", "--cached", "-r", "-q", "."]).map(text),
            (None, false) => self.git().run(&["reset", "-q"]).map(text),
            // --ignore-unmatch: a folder with nothing staged isn't an error.
            (Some(spec), true) => self
                .git()
                .run(&["rm", "--cached", "-r", "-q", "--ignore-unmatch", "--", &spec])
                .map(text),
            (Some(spec), false) => self.git().run(&["reset", "-q", "--", &spec]).map(text),
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

    /// Throws away every change `scope` covers: `reset --hard` for
    /// everything, `restore --worktree` for unstaged changes, `clean -fd` for
    /// untracked files. `clean` runs without `-x` (ignored files stay) and
    /// without `-ff` (nested repositories stay). Refused while a merge,
    /// rebase, cherry-pick or revert is in progress.
    ///
    /// With `folder`, only changes under it go: `restore --staged
    /// --worktree --source=HEAD` stands in for `reset --hard`, and the
    /// `restore` and `clean` are limited to it.
    pub fn discard_all(&self, scope: DiscardScope, folder: Option<&str>) -> Result<String> {
        if let Some(op) = self.in_progress() {
            return Err(GitError::Refused(format!(
                "a {} is in progress; finish or abort it first",
                op.command()
            )));
        }
        let spec = literal(folder.unwrap_or("."));
        let mut out = Vec::new();
        if scope.drops_staged() {
            if folder.is_none() {
                out.push(self.git().run(&["reset", "--hard", "-q"]).map(text)?);
            } else if self.head_is_unborn() {
                // Nothing to restore to: unstaged, the files are untracked
                // and the clean below removes them.
                self.git()
                    .run(&["rm", "--cached", "-r", "-q", "--ignore-unmatch", "--", &spec])?;
            } else if self.git().run(&["diff", "HEAD", "--quiet", "--", &spec]).is_err() {
                out.push(
                    self.git()
                        .run(&["restore", "--source=HEAD", "--staged", "--worktree", "--", &spec])
                        .map(text)?,
                );
            }
        } else if scope.drops_unstaged() && self.git().run(&["diff", "--quiet", "--", &spec]).is_err() {
            // `restore` errors when nothing is tracked, so only run it when
            // there's something to restore.
            out.push(self.git().run(&["restore", "--worktree", "--", &spec]).map(text)?);
        }
        if scope.drops_untracked() {
            match folder {
                Some(_) => out.push(self.git().run(&["clean", "-fd", "-q", "--", &spec]).map(text)?),
                None => out.push(self.git().run(&["clean", "-fd", "-q"]).map(text)?),
            }
        }
        out.retain(|s| !s.is_empty());
        Ok(out.join("\n"))
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
        let remote = self.push_remote()?;
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

    /// Diff of a folder for the details pane: the staged and unstaged
    /// changes under `folder` (ending in `/`), then the untracked files
    /// among `entries` (its changed files) by name.
    pub fn folder_diff(&self, folder: &str, entries: &[crate::model::StatusEntry]) -> Result<String> {
        let spec = literal(folder);
        let mut out = String::new();
        let section = |out: &mut String, title: &str, body: &str| {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("── {title} ──\n"));
            out.push_str(body);
        };
        if entries.iter().any(|e| e.has_staged()) {
            let diff = self.git().run(&["diff", "--cached", "-M", "--", &spec])?;
            section(&mut out, "staged", &String::from_utf8_lossy(&diff.stdout));
        }
        if entries.iter().any(|e| e.has_unstaged() && !e.is_untracked()) {
            let diff = self.git().run(&["diff", "--", &spec])?;
            section(&mut out, "unstaged", &String::from_utf8_lossy(&diff.stdout));
        }
        let untracked: String = entries
            .iter()
            .filter(|e| e.is_untracked())
            .map(|e| format!("{}\n", e.path))
            .collect();
        if !untracked.is_empty() {
            section(&mut out, "untracked", &untracked);
        }
        Ok(out)
    }
}

/// What applying a partial patch of one file does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchOp {
    /// Adds part of the unstaged changes to the index.
    Stage,
    /// Takes part of the staged changes back out of the index.
    Unstage,
    /// Throws part of the unstaged changes away from the working tree.
    Discard,
}

impl PatchOp {
    /// Which way to build the patch for this operation.
    pub fn direction(self) -> PatchDirection {
        match self {
            PatchOp::Stage => PatchDirection::Forward,
            PatchOp::Unstage | PatchOp::Discard => PatchDirection::Reverse,
        }
    }
}

/// One side of a file's changes, for staging by hunk: its diff (empty for
/// no changes), or why it can't be split into hunks.
pub type StageSide = std::result::Result<FileDiff, String>;

impl Repo {
    /// The staged or unstaged changes to `entry` as hunks. An untracked file
    /// comes back as one hunk adding its content, from
    /// [`untracked_diff`](crate::patch::untracked_diff).
    pub fn stage_diff(&self, entry: &crate::model::StatusEntry, staged: bool, max_bytes: usize) -> Result<StageSide> {
        if entry.is_conflicted() {
            return Ok(Err("conflicted: resolve it, then stage the whole file".into()));
        }
        let changed = if staged {
            entry.has_staged()
        } else {
            entry.has_unstaged()
        };
        if !changed {
            return Ok(Ok(FileDiff::default()));
        }
        if entry.is_untracked() {
            let full = self.root().join(&entry.path);
            if full.is_dir() {
                return Ok(Err(format!("{} is an untracked directory", entry.path)));
            }
            let bytes = std::fs::read(&full).map_err(GitError::Spawn)?;
            if bytes.len() > max_bytes {
                return Ok(Err(format!("too large to split ({} bytes)", bytes.len())));
            }
            return Ok(untracked_diff(&entry.path, &bytes)
                .and_then(|text| FileDiff::parse(&text))
                .ok_or_else(|| "binary or not UTF-8: stage the whole file".to_string()));
        }
        // Pin everything that changes the shape of the output, so patches
        // built from it apply with `git apply`'s defaults.
        let mut args = vec![
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--no-renames",
            "--no-relative",
            "--src-prefix=a/",
            "--dst-prefix=b/",
        ];
        if staged {
            args.push("--cached");
        }
        args.extend(["--", entry.path.as_str()]);
        let out = self.git().run(&args)?;
        let Ok(text) = String::from_utf8(out.stdout) else {
            return Ok(Err("not UTF-8: stage the whole file".into()));
        };
        Ok(FileDiff::parse(&text).ok_or_else(|| "binary: stage the whole file".to_string()))
    }

    /// Applies `patch` (built from [`stage_diff`](Self::stage_diff) in
    /// `op`'s [direction](PatchOp::direction)) to the index or working tree.
    /// Staging part of an untracked file first adds it with `git add -N`,
    /// taking that back if the patch doesn't apply.
    pub fn apply_patch(&self, path: &str, patch: &str, op: PatchOp, untracked: bool) -> Result<String> {
        let intent = untracked && op == PatchOp::Stage;
        if intent {
            self.git().run(&["add", "-N", "--", path])?;
        }
        let mut args = vec!["apply", "--whitespace=nowarn"];
        match op {
            PatchOp::Stage => args.push("--cached"),
            PatchOp::Unstage => args.extend(["--cached", "-R"]),
            PatchOp::Discard => args.push("-R"),
        }
        args.push("-");
        let result = self.git().run_with_stdin(&args, Some(patch.as_bytes())).map(text);
        if result.is_err() && intent {
            let _ = self.git().run(&["rm", "--cached", "-f", "-q", "--", path]);
        }
        result
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

    /// Applies `oids` onto HEAD in the order given, in one `git cherry-pick`
    /// so a conflict part way leaves git's sequencer to continue the rest.
    pub fn cherry_pick<S: AsRef<str>>(&self, oids: &[S]) -> Result<String> {
        let mut args = vec!["cherry-pick"];
        args.extend(oids.iter().map(AsRef::as_ref));
        self.git().run(&args).map(text)
    }

    /// `git revert --no-edit <oid>`: a new commit that undoes `oid`. A merge
    /// commit needs `mainline`, the parent (from 1) whose side is kept.
    pub fn revert(&self, oid: &str, mainline: Option<usize>) -> Result<String> {
        let parent = mainline.map(|n| n.to_string());
        let mut args = vec!["revert", "--no-edit"];
        if let Some(n) = &parent {
            args.extend(["-m", n.as_str()]);
        }
        args.push(oid);
        self.git().run(&args).map(text)
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

/// How much of the index and working tree a reset matches to the commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetMode {
    /// Only the branch moves; the changes it leaves behind are staged.
    Soft,
    /// The index matches the commit; the changes are left unstaged.
    Mixed,
    /// The index and working tree match the commit; uncommitted changes
    /// to tracked files are thrown away.
    Hard,
}

impl ResetMode {
    /// `soft`, `mixed` or `hard`.
    pub fn name(self) -> &'static str {
        match self {
            ResetMode::Soft => "soft",
            ResetMode::Mixed => "mixed",
            ResetMode::Hard => "hard",
        }
    }
}

/// What resetting to a commit would leave behind, for asking first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetPlan {
    /// HEAD's branch, or `None` when detached.
    pub branch: Option<String>,
    /// Commits on HEAD that the target doesn't have: they leave the branch.
    pub commits: usize,
    /// Tracked files with uncommitted changes, staged or not: a hard reset
    /// throws those changes away.
    pub changed: usize,
}

impl Repo {
    /// What resetting the current branch to `target` would leave behind.
    pub fn reset_plan(&self, target: &str) -> Result<ResetPlan> {
        let head = self.head()?;
        if head.oid().is_none() {
            return Err(GitError::Refused("Nothing to reset: there are no commits yet".into()));
        }
        let range = format!("{target}..HEAD");
        let commits = self.git().run(&["rev-list", "--count", range.as_str(), "--"])?;
        let changed = self.git().run(&["diff", "--name-only", "-z", "HEAD", "--"])?;
        Ok(ResetPlan {
            branch: head.branch().map(str::to_string),
            commits: commits.stdout_str().trim().parse().unwrap_or(0),
            changed: changed.stdout.split(|&b| b == 0).filter(|p| !p.is_empty()).count(),
        })
    }

    /// `git reset --<mode> <target>`. The reflog entry names the mode
    /// (`reset --hard main: updating HEAD`) so undo can reset back the
    /// same way.
    pub fn reset(&self, target: &str, mode: ResetMode) -> Result<String> {
        let flag = format!("--{}", mode.name());
        let action = format!("reset {flag} {}", undo::short_name(target));
        self.git()
            .clone()
            .with_env([("GIT_REFLOG_ACTION", action)])
            .run(&["reset", "-q", flag.as_str(), target, "--"])
            .map(text)
    }
}

/// Starts the error [`Repo::apply_undo`] returns when the step was taken but
/// the changes stashed for it didn't go back cleanly.
pub const UNDO_CHANGES_KEPT: &str = "Putting your changes back didn't go cleanly";

impl Repo {
    /// The next undo step, worked out from HEAD's reflog, or why there
    /// isn't one.
    pub fn undo_step(&self) -> Result<std::result::Result<UndoStep, NoStep>> {
        if let Some(op) = self.in_progress() {
            return Ok(Err(NoStep::InProgress(op)));
        }
        let head = self.head()?;
        let Some(oid) = head.oid() else {
            return Ok(Err(NoStep::Unborn));
        };
        let max = format!("--max-count={}", undo::REFLOG_LIMIT);
        let entries = match self
            .git()
            .run(&["log", "-g", parse::UNDO_LOG_FORMAT, &max, "HEAD", "--"])
        {
            Ok(out) => parse::parse_reflog(&out.stdout),
            // No reflog.
            Err(GitError::Failed { .. }) => Vec::new(),
            Err(e) => return Err(e),
        };
        let out = self
            .git()
            .run(&["for-each-ref", "--format=%(refname)", "refs/heads/"])?;
        let refs = LiveRefs {
            repo: self,
            branches: out
                .stdout_str()
                .lines()
                .filter_map(|r| r.strip_prefix("refs/heads/"))
                .map(str::to_string)
                .collect(),
        };
        let current = head.branch().unwrap_or(oid);
        Ok(undo::plan(&entries, &refs, oid, current))
    }

    /// Takes `step`, unless HEAD has moved since it was worked out. Each git
    /// command is marked in the reflog through `GIT_REFLOG_ACTION`, so later
    /// steps know what's already undone.
    ///
    /// With `stash`, every change is stashed first. After a switch the
    /// stash is left for the user, as [`stash_and_checkout`](Self::stash_and_checkout)
    /// does; after a reset it's popped back, and if that doesn't go cleanly
    /// the error starts with [`UNDO_CHANGES_KEPT`]. If the step itself
    /// fails, the stash is popped back. A step that doesn't
    /// [touch the working tree](UndoTarget::touches_worktree) never stashes.
    pub fn apply_undo(&self, step: &UndoStep, stash: bool) -> Result<String> {
        let stash = stash && step.target.touches_worktree();
        if self.commit_of("HEAD").as_deref() != Some(step.head.as_str()) {
            return Err(GitError::Refused(
                "HEAD moved after the undo was worked out. Try again.".into(),
            ));
        }
        // A rebased branch moves too: it must still be where the step expects.
        if let UndoTarget::LeaveRebase { branch, rebased, .. } = &step.target
            && self.commit_of(&format!("refs/heads/{branch}")).as_ref() != Some(rebased)
        {
            return Err(GitError::Refused(format!(
                "{branch} moved after the undo was worked out. Try again."
            )));
        }
        let mut out = String::new();
        let before = self.stash_tip();
        if stash {
            out = self.stash_push(None, &StashScope::All)?;
        }
        let stashed = stash && self.stash_tip() != before;
        let done = match self.move_for_undo(step, &step.target) {
            Ok(done) => done,
            Err(e) => {
                if stashed && let Err(pop) = self.stash_apply(0, true) {
                    tracing::warn!("restoring the stash after a failed undo failed: {pop}");
                }
                return Err(e);
            }
        };
        push_line(&mut out, &done);
        if stashed && !step.target.is_switch() {
            match self.stash_apply(0, true) {
                Ok(popped) => push_line(&mut out, &popped),
                Err(e) => {
                    return Err(GitError::Refused(format!(
                        "{UNDO_CHANGES_KEPT}, so they're still in stash@{{0}}.\n\
                         Sort out the files git mentions below, then drop the stash.\n\n{e}"
                    )));
                }
            }
        }
        Ok(out)
    }
}

impl Repo {
    /// The commit `rev` names, if it names one.
    fn commit_of(&self, rev: &str) -> Option<String> {
        let rev = format!("{rev}^{{commit}}");
        let out = self.git().run(&["rev-parse", "-q", "--verify", rev.as_str()]).ok()?;
        Some(out.stdout_str().trim().to_string())
    }

    /// Runs the git commands that take `step` to `target`. Only the command
    /// that moves HEAD is marked, so each step leaves one marked entry in
    /// HEAD's reflog.
    fn move_for_undo(&self, step: &UndoStep, target: &UndoTarget) -> Result<String> {
        let marked = |action: String| self.git().clone().with_env([("GIT_REFLOG_ACTION", action)]);
        match target {
            UndoTarget::Reset(oid) => marked(UNDO_MARK.to_string())
                .run(&["reset", "--keep", oid.as_str()])
                .map(text),
            UndoTarget::SoftReset(oid) => marked(UNDO_MARK.to_string())
                .run(&["reset", "--soft", oid.as_str()])
                .map(text),
            UndoTarget::MixedReset(oid) => marked(UNDO_MARK.to_string())
                .run(&["reset", "-q", "--mixed", oid.as_str()])
                .map(text),
            UndoTarget::Branch(name) => marked(switch_message(step, name))
                .run(&["switch", name.as_str()])
                .map(text),
            UndoTarget::Detach(oid) => marked(switch_message(step, oid))
                .run(&["switch", "--detach", oid.as_str()])
                .map(text),
            UndoTarget::LeaveRebase {
                branch,
                before,
                rebased,
                back,
            } => {
                // Off the branch first: git won't move the checked-out one.
                let out = self.move_for_undo(step, back)?;
                if let Some(before) = before {
                    let full = format!("refs/heads/{branch}");
                    let message = format!("{UNDO_MARK}: back to before the rebase");
                    self.git()
                        .run(&[
                            "update-ref",
                            "-m",
                            message.as_str(),
                            full.as_str(),
                            before.as_str(),
                            rebased.as_str(),
                        ])
                        .map_err(|e| {
                            GitError::Refused(format!(
                                "Switched to {}, but moving {branch} back to before the rebase failed:\n{e}",
                                back.display()
                            ))
                        })?;
                }
                Ok(out)
            }
        }
    }
}

/// [`RepoRefs`] for [`undo::plan`], read from the repository.
struct LiveRefs<'a> {
    repo: &'a Repo,
    branches: HashSet<String>,
}

impl RepoRefs for LiveRefs<'_> {
    fn is_branch(&self, name: &str) -> bool {
        self.branches.contains(name)
    }

    /// The entry before the rebase's `(finish)` in the branch's reflog.
    fn before_rebase(&self, branch: &str, rebased: &str) -> Option<String> {
        let max = format!("--max-count={}", undo::REFLOG_LIMIT);
        let full = format!("refs/heads/{branch}");
        let out = self
            .repo
            .git()
            .run(&["log", "-g", parse::UNDO_LOG_FORMAT, max.as_str(), full.as_str(), "--"])
            .ok()?;
        let entries = parse::parse_reflog(&out.stdout);
        let at = entries
            .iter()
            .position(|e| e.oid == rebased && e.subject.contains("(finish)"))?;
        Some(entries.get(at + 1)?.oid.clone())
    }
}

/// The reflog message for a switch made by `step`: git uses
/// `GIT_REFLOG_ACTION` as the whole message, so it's written the way git
/// writes a checkout, behind the marker.
fn switch_message(step: &UndoStep, to: &str) -> String {
    format!("{UNDO_MARK} checkout: moving from {} to {to}", step.current)
}

fn push_line(out: &mut String, line: &str) {
    if line.is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(line);
}

/// Which changes `git stash push` saves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StashScope {
    /// Staged, unstaged and untracked changes.
    All,
    /// Only what's staged (`--staged`, git 2.35).
    Staged,
    /// Only unstaged and untracked changes; the staged ones stay staged
    /// and out of the stash. git has no flag for this, see
    /// [`Repo::stash_push`].
    Unstaged,
    /// Everything, leaving the staged changes in place (`--keep-index`).
    KeepStaged,
    /// Changes to these paths, untracked ones included.
    Paths(Vec<String>),
}

/// The message on the stash [`StashScope::Unstaged`] parks the staged
/// changes in while it stashes the rest. It's popped straight back, so
/// it's only ever seen if that fails.
pub const STAGED_ASIDE: &str = "tt-git: staged changes set aside";

/// `path` as a pathspec that matches only that path, so a file called
/// `a[b].txt` doesn't also stash `ab.txt`.
fn literal(path: &str) -> String {
    format!(":(literal){path}")
}

impl Repo {
    /// Stashes `scope`'s changes, with `message` if given.
    ///
    /// [`StashScope::Unstaged`] runs `git stash push --staged` to set the
    /// staged changes aside, `git stash push --include-untracked` for the
    /// rest, then `git stash pop --index` on the first stash to put the
    /// staged changes back. With nothing staged it's a plain stash.
    pub fn stash_push(&self, message: Option<&str>, scope: &StashScope) -> Result<String> {
        match scope {
            StashScope::All => self.stash_cmd(&["--include-untracked"], message, &[]).map(text),
            StashScope::KeepStaged => self
                .stash_cmd(&["--include-untracked", "--keep-index"], message, &[])
                .map(text),
            StashScope::Staged => self.stash_staged(message),
            StashScope::Unstaged => self.stash_unstaged(message),
            StashScope::Paths(paths) => self.stash_paths(message, paths),
        }
    }

    fn stash_cmd(&self, flags: &[&str], message: Option<&str>, paths: &[String]) -> Result<GitOutput> {
        let mut args: Vec<String> = ["stash", "push"].iter().chain(flags).map(|s| s.to_string()).collect();
        if let Some(message) = message {
            args.extend(["-m".to_string(), message.to_string()]);
        }
        if !paths.is_empty() {
            args.push("--".into());
            args.extend(paths.iter().map(|p| literal(p)));
        }
        self.git().run(&args)
    }

    fn stash_staged(&self, message: Option<&str>) -> Result<String> {
        let before = self.stash_tip();
        self.stash_cmd(&["--staged"], message, &[])
            .map(text)
            .map_err(|e| self.staged_failure(e, before.as_deref()))
    }

    /// Explains a failed `git stash push --staged`. When the staged and
    /// unstaged changes touch the same lines, git can't take the staged
    /// ones out of the working tree: it says "Cannot remove worktree
    /// changes" and leaves the files alone, but it has already saved a
    /// stash by then. That stash only duplicates what's still in the
    /// working tree, so it's dropped.
    fn staged_failure(&self, e: GitError, before: Option<&str>) -> GitError {
        let stderr = e.stderr().unwrap_or_default();
        if stderr.contains("unknown option") && stderr.contains("staged") {
            return GitError::Refused(
                "stashing staged or unstaged changes on their own needs git 2.35 or newer".into(),
            );
        }
        if stderr.contains("Cannot remove worktree changes") {
            let tip = self.stash_tip();
            if tip.is_some()
                && tip.as_deref() != before
                && let Err(drop) = self.stash_drop(0)
            {
                tracing::warn!("dropping the stash a failed --staged push left behind failed: {drop}");
            }
            return GitError::Refused(
                "the staged and unstaged changes overlap, so git can't stash them apart; \
                 stage or unstage the rest of the file, or stash everything"
                    .into(),
            );
        }
        e
    }

    fn stash_unstaged(&self, message: Option<&str>) -> Result<String> {
        let entries = self.status()?.entries;
        if !entries.iter().any(|e| e.has_unstaged()) {
            return Err(GitError::Refused("no unstaged changes to stash".into()));
        }
        if !entries.iter().any(|e| e.has_staged()) {
            return self.stash_push(message, &StashScope::All);
        }
        let before = self.stash_tip();
        self.stash_cmd(&["--staged"], Some(STAGED_ASIDE), &[])
            .map_err(|e| self.staged_failure(e, before.as_deref()))?;
        let Some(aside) = self.stash_tip().filter(|tip| Some(tip) != before.as_ref()) else {
            return Err(GitError::Refused("git didn't stash the staged changes".into()));
        };
        let pushed = self.stash_cmd(&["--include-untracked"], message, &[]);
        // The staged changes are stash@{1} under the new stash, or still
        // stash@{0} if that push failed or saved nothing.
        let at = if self.stash_tip().as_deref() == Some(aside.as_str()) {
            0
        } else {
            1
        };
        let name = format!("stash@{{{at}}}");
        let restored = match self.git().run(&["rev-parse", "-q", "--verify", name.as_str()]) {
            Ok(out) if out.stdout_str() == aside => {
                self.git().run(&["stash", "pop", "--index", name.as_str()]).map(|_| ())
            }
            _ => Err(GitError::Refused(format!("{name} isn't the stash tt-git just made"))),
        };
        if let Err(e) = restored {
            let why = match e.stderr().map(str::trim) {
                Some(stderr) if !stderr.is_empty() => stderr.to_string(),
                _ => e.to_string(),
            };
            return Err(GitError::Refused(format!(
                "couldn't put the staged changes back; they're on the Stash tab as \"{STAGED_ASIDE}\": {why}"
            )));
        }
        pushed.map(text)
    }

    /// git can't stash a staged deletion by path (the old side of a
    /// staged rename is one): it saves the stash, then fails to find the
    /// file and leaves everything in place. Unstaging the deletion first
    /// makes it an unstaged one, which stashes fine; popping the stash
    /// brings the file back deleted but not staged.
    fn stash_paths(&self, message: Option<&str>, paths: &[String]) -> Result<String> {
        if !self.head_is_unborn() {
            let specs: Vec<String> = paths.iter().map(|p| literal(p)).collect();
            let mut args = vec![
                "diff",
                "--cached",
                "--no-renames",
                "--name-only",
                "-z",
                "--diff-filter=D",
                "--",
            ];
            args.extend(specs.iter().map(String::as_str));
            let out = self.git().run(&args)?;
            let deleted: Vec<String> = out
                .stdout_str()
                .split('\0')
                .filter(|p| !p.is_empty())
                .map(literal)
                .collect();
            if !deleted.is_empty() {
                let mut args = vec!["restore", "--staged", "--"];
                args.extend(deleted.iter().map(String::as_str));
                self.git().run(&args)?;
            }
        }
        self.stash_cmd(&["--include-untracked"], message, paths).map(text)
    }

    /// Applies `stash@{index}` to the working tree, dropping it afterwards
    /// with `pop` (git keeps it if the changes conflict).
    pub fn stash_apply(&self, index: usize, pop: bool) -> Result<String> {
        let name = format!("stash@{{{index}}}");
        let verb = if pop { "pop" } else { "apply" };
        self.git().run(&["stash", verb, name.as_str()]).map(text)
    }

    pub fn stash_drop(&self, index: usize) -> Result<String> {
        let name = format!("stash@{{{index}}}");
        self.git().run(&["stash", "drop", name.as_str()]).map(text)
    }

    /// Creates branch `name` at the commit `stash@{index}` was made on,
    /// switches to it and pops the stash there.
    pub fn stash_branch(&self, name: &str, index: usize) -> Result<String> {
        let stash = format!("stash@{{{index}}}");
        self.git().run(&["stash", "branch", name, stash.as_str()]).map(text)
    }
}

/// Which file [`Repo::ignore`] adds a pattern to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoreTarget {
    /// The `.gitignore` at the root of the working tree, which is committed
    /// and shared.
    Gitignore,
    /// `.git/info/exclude`, which stays on this machine.
    Exclude,
}

impl IgnoreTarget {
    /// The file as the UI names it.
    pub fn label(self) -> &'static str {
        match self {
            IgnoreTarget::Gitignore => ".gitignore",
            IgnoreTarget::Exclude => ".git/info/exclude",
        }
    }
}

/// Starts the error [`Repo::ignore`] returns when the path is tracked and
/// it wasn't asked to stop tracking it.
pub const IGNORE_TRACKED: &str = "is tracked, so ignoring it wouldn't hide it";

/// What [`Repo::gone_branches`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoneBranches {
    /// Not checked out anywhere, so they can be deleted.
    pub deletable: Vec<String>,
    /// Checked out here or in another worktree.
    pub checked_out: Vec<String>,
}

/// What [`Repo::delete_branches`] did with each branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchDeletes {
    pub deleted: Vec<String>,
    /// Refused by `git branch -d` as not fully merged.
    pub unmerged: Vec<String>,
    /// Refused for any other reason, with git's message.
    pub failed: Vec<(String, String)>,
}

/// What [`Repo::fast_forward_branch`] returns when the branch was already
/// at its upstream.
pub const FAST_FORWARD_UP_TO_DATE: &str = "already up to date";

/// The ignore pattern for `path` (relative to the root, `/`-separated):
/// anchored to the root, ending in `/` for a directory, with the
/// characters gitignore treats specially escaped.
pub fn ignore_pattern(path: &str, dir: bool) -> String {
    let path = path.trim_end_matches('/');
    let mut pattern = String::with_capacity(path.len() + 2);
    pattern.push('/');
    for c in path.chars() {
        if matches!(c, '\\' | '*' | '?' | '[') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    // Trailing spaces are dropped unless escaped.
    let spaces = pattern.len() - pattern.trim_end_matches(' ').len();
    if spaces > 0 {
        pattern.truncate(pattern.len() - spaces);
        pattern.push_str(&"\\ ".repeat(spaces));
    }
    if dir {
        pattern.push('/');
    }
    pattern
}

impl Repo {
    /// Where `target`'s patterns live. In a linked worktree `info/exclude`
    /// is the common git dir's, as `git rev-parse --git-path` says.
    pub fn ignore_file(&self, target: IgnoreTarget) -> Result<std::path::PathBuf> {
        match target {
            IgnoreTarget::Gitignore => Ok(self.root().join(".gitignore")),
            IgnoreTarget::Exclude => {
                let out = self.git().run(&["rev-parse", "--git-path", "info/exclude"])?;
                let path = std::path::PathBuf::from(out.stdout_str().trim());
                Ok(if path.is_absolute() {
                    path
                } else {
                    self.root().join(path)
                })
            }
        }
    }

    /// Whether anything at or under `path` is in the index.
    pub fn is_tracked(&self, path: &str) -> Result<bool> {
        let spec = literal(path.trim_end_matches('/'));
        let out = self.git().run(&["ls-files", "-z", "--", spec.as_str()])?;
        Ok(!out.stdout.is_empty())
    }

    /// Adds `path` (a file, or a directory with a trailing `/`) to
    /// `target` as a pattern anchored to the root, creating the file if it's
    /// missing and leaving it alone if the pattern is already there.
    ///
    /// Ignoring doesn't hide a tracked path, so if anything at or under
    /// `path` is tracked this refuses with [`IGNORE_TRACKED`] and writes
    /// nothing, unless `untrack` is set: then it also runs
    /// `git rm --cached -r`, which keeps the files on disk.
    pub fn ignore(&self, path: &str, target: IgnoreTarget, untrack: bool) -> Result<String> {
        let tracked = self.is_tracked(path)?;
        if tracked && !untrack {
            return Err(GitError::Refused(format!("{path} {IGNORE_TRACKED}")));
        }
        let dir = path.ends_with('/') || self.root().join(path).is_dir();
        let pattern = ignore_pattern(path, dir);
        let file = self.ignore_file(target)?;
        let failed = |e: std::io::Error| GitError::Refused(format!("could not write {}: {e}", file.display()));
        let existing = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(failed(e)),
        };
        let mut out = String::new();
        // git drops trailing spaces that aren't escaped.
        if existing
            .lines()
            .any(|line| line == pattern || line.trim_end() == pattern)
        {
            push_line(&mut out, &format!("{pattern} is already in {}", target.label()));
        } else {
            let newline = if existing.contains("\r\n") { "\r\n" } else { "\n" };
            let mut add = String::new();
            if !existing.is_empty() && !existing.ends_with('\n') {
                add.push_str(newline);
            }
            add.push_str(&pattern);
            add.push_str(newline);
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent).map_err(failed)?;
            }
            use std::io::Write;
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&file)
                .and_then(|mut f| f.write_all(add.as_bytes()))
                .map_err(failed)?;
            push_line(&mut out, &format!("added {pattern} to {}", target.label()));
        }
        if tracked {
            let spec = literal(path.trim_end_matches('/'));
            self.git().run(&["rm", "--cached", "-r", "-q", "--", spec.as_str()])?;
            push_line(&mut out, &format!("stopped tracking {path}"));
        }
        Ok(out)
    }
}
