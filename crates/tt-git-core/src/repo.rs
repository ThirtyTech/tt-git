//! A discovered repository and its read operations.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::git::{GitCli, GitError, Result};
use crate::model::{BranchHistory, Checkout, Commit, CommitFile, GitRef, Head, Status};
use crate::parse;

/// Repo-local git config key holding the picked base branch.
pub const BASE_BRANCH_KEY: &str = "tt-git.baseBranch";

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

    /// The base branch picked for this repo (`tt-git.baseBranch`), if any.
    pub fn base_branch_setting(&self) -> Option<String> {
        let out = self.git.run(&["config", "--get", BASE_BRANCH_KEY]).ok()?;
        let name = out.stdout_str().trim().to_string();
        (!name.is_empty()).then_some(name)
    }

    /// Full names of the refs under `prefix` (e.g. `refs/remotes/origin/`)
    /// that are merged into `base`.
    pub fn merged_refs(&self, base: &str, prefix: &str) -> Result<HashSet<String>> {
        let merged = format!("--merged={base}");
        let out = self
            .git
            .run(&["for-each-ref", "--format=%(refname)", &merged, prefix])?;
        Ok(out
            .stdout_str()
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect())
    }

    /// Author of each tip under `prefix`, keyed by full ref name, with names
    /// mapped through `.mailmap`.
    pub fn ref_authors(&self, prefix: &str) -> Result<HashMap<String, String>> {
        let out = self.git.run(&[
            "for-each-ref",
            "--format=%(refname)%00%(authorname)%00%(authoremail)",
            prefix,
        ])?;
        let text = out.stdout_str();
        let rows: Vec<(&str, String)> = text
            .lines()
            .filter_map(|l| {
                let mut f = l.split('\0');
                let (name, author, email) = (f.next()?, f.next()?, f.next().unwrap_or(""));
                let email = if email.starts_with('<') {
                    email.to_string()
                } else {
                    format!("<{email}>")
                };
                Some((name, format!("{author} {email}")))
            })
            .collect();
        let mut contacts: Vec<&str> = rows.iter().map(|(_, c)| c.as_str()).collect();
        contacts.sort_unstable();
        contacts.dedup();
        let mapped = self.mailmap(&contacts);
        Ok(rows
            .iter()
            .map(|(name, contact)| {
                let contact = mapped.get(contact.as_str()).map_or(contact.as_str(), String::as_str);
                let author = contact.rsplit_once(" <").map_or(contact, |(n, _)| n);
                (name.to_string(), author.to_string())
            })
            .collect())
    }

    /// `git check-mailmap` for each `Name <email>`; contacts git can't map
    /// are left out.
    fn mailmap(&self, contacts: &[&str]) -> HashMap<String, String> {
        let mut mapped = HashMap::new();
        // Batches keep the command line short (Windows caps it at 32K).
        for chunk in contacts.chunks(100) {
            let mut args = vec!["check-mailmap"];
            args.extend(chunk);
            let Ok(out) = self.git.run(&args) else {
                continue;
            };
            for (from, to) in chunk.iter().zip(out.stdout_str().lines()) {
                mapped.insert(from.to_string(), to.to_string());
            }
        }
        mapped
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
    pub fn stream_commits(&self, scope: &LogScope, limit: usize, on_commit: impl FnMut(Commit) -> bool) -> Result<()> {
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
        self.stream_log(&args, on_commit)
    }

    /// Runs `args` (a `git log -z` with [`parse::LOG_FORMAT`]), handing each
    /// commit to `on_commit` until it returns `false`.
    fn stream_log(&self, args: &[String], mut on_commit: impl FnMut(Commit) -> bool) -> Result<()> {
        let mut error = None;
        self.git.stream(args, 0, |record| {
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

    /// `git log` of `revs` with `options`, collected.
    fn log(&self, options: &[&str], revs: &[&str]) -> Result<Vec<Commit>> {
        let mut args: Vec<String> = vec!["log".into(), "-z".into(), parse::LOG_FORMAT.into()];
        args.extend(options.iter().map(|o| o.to_string()));
        args.extend(revs.iter().map(|r| r.to_string()));
        args.push("--".into());
        let mut commits = Vec::new();
        self.stream_log(&args, |c| {
            commits.push(c);
            true
        })?;
        Ok(commits)
    }

    /// Whether `ancestor` is reachable from `rev`.
    pub fn is_ancestor(&self, ancestor: &str, rev: &str) -> Result<bool> {
        match self.git.run(&["merge-base", "--is-ancestor", ancestor, rev]) {
            Ok(_) => Ok(true),
            Err(GitError::Failed { code, .. }) if code == "1" => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// The commits `tip` made on its own against `base`: up to `limit` of its
    /// first-parent commits that aren't on the base, where it branched off,
    /// and the base commit that merged it in (or the base's tip, if it isn't
    /// merged). A branch fast-forwarded into the base, so that no commit of
    /// its own is left to show, is just its tip.
    pub fn branch_history(&self, tip: &str, base: &str, limit: usize) -> Result<BranchHistory> {
        let tip_commit = self
            .log(&["--no-walk"], &[tip])?
            .pop()
            .ok_or_else(|| GitError::Parse(format!("{tip} is not a commit")))?;
        let merge = if self.is_ancestor(tip, base)? {
            // The oldest commit on the base's first-parent line that has the
            // tip in it.
            let range = format!("{tip}..{base}");
            let out = self
                .git
                .run(&["rev-list", "--first-parent", "--ancestry-path", &range, "--"])?;
            let text = out.stdout_str();
            let merge = match text.lines().rfind(|l| !l.is_empty()) {
                Some(oid) => self.log(&["--no-walk"], &[oid])?.pop(),
                None => None,
            };
            match merge.filter(|m| m.parents.len() > 1 && m.parents[0] != tip_commit.oid) {
                Some(m) => Some(m),
                None => {
                    let mut only = tip_commit;
                    only.parents.clear();
                    return Ok(BranchHistory {
                        commits: vec![only],
                        ..BranchHistory::default()
                    });
                }
            }
        } else {
            None
        };
        let exclude = format!("^{}", merge.as_ref().map_or(base, |m| m.parents[0].as_str()));
        let max = format!("--max-count={limit}");
        let mut commits = self.log(&["--first-parent", "--topo-order", &max], &[tip, &exclude])?;
        let fork = match commits.last() {
            Some(last) if commits.len() < limit => last.parents.first().cloned(),
            _ => None,
        };
        let mut extras = Vec::new();
        extras.extend(fork.as_deref());
        if merge.is_none() {
            extras.push(base);
        }
        let mut extras = self.log(&["--no-walk=unsorted"], &extras)?.into_iter();
        let fork_commit = fork.as_ref().and_then(|_| extras.next());
        let shown: HashSet<String> = commits.iter().map(|c| c.oid.clone()).chain(fork.clone()).collect();
        for c in &mut commits {
            c.parents.retain(|p| shown.contains(p));
        }
        let merge_oid = merge.as_ref().map(|m| m.oid.clone());
        match merge {
            Some(mut m) => {
                m.parents = fork.iter().cloned().chain([tip_commit.oid]).collect();
                commits.insert(0, m);
            }
            // While unmerged, the base's tip shows where the base is now,
            // unless it hasn't moved since the fork.
            None => {
                if let Some(mut b) = extras.next().filter(|b| Some(&b.oid) != fork.as_ref()) {
                    b.parents = fork.iter().cloned().collect();
                    let at = commits
                        .iter()
                        .position(|c| c.commit_time < b.commit_time)
                        .unwrap_or(commits.len());
                    commits.insert(at, b);
                }
            }
        }
        if let Some(mut f) = fork_commit {
            f.parents.clear();
            commits.push(f);
        }
        Ok(BranchHistory {
            commits,
            fork,
            merge: merge_oid,
        })
    }

    /// Paths `oid` changed, compared with its first parent (renames detected).
    pub fn commit_files(&self, oid: &str) -> Result<Vec<CommitFile>> {
        let out = self.git.run(&[
            "show",
            "--format=",
            "-z",
            "--name-status",
            "-M",
            "--diff-merges=first-parent",
            oid,
            "--",
        ])?;
        Ok(parse::parse_name_status(&out.stdout))
    }

    /// The patch `oid` made to one of its [`commit_files`](Self::commit_files).
    pub fn commit_file_diff(&self, oid: &str, file: &CommitFile) -> Result<String> {
        let mut args = vec![
            "show",
            "--format=",
            "--patch",
            "-M",
            "--diff-merges=first-parent",
            oid,
            "--",
        ];
        args.push(&file.path);
        if let Some(orig) = &file.orig_path {
            args.push(orig);
        }
        Ok(self.git.run(&args)?.stdout_str())
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
