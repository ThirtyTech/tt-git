//! A discovered repository and its read operations.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::branches::AheadBehind;
use crate::git::{GitCli, GitError, Result};
use crate::model::{BranchHistory, Checkout, Commit, CommitFile, GitRef, Head, Oid, Stash, Status};
use crate::parse;

/// Repo-local git config key holding the picked base branch.
pub const BASE_BRANCH_KEY: &str = "tt-git.baseBranch";

/// Folder, in the main working tree, that new worktrees are created in.
pub const WORKTREE_DIR: &str = ".worktrees";

/// Which commits the graph loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogScope {
    /// Every ref (`--all`), minus the stash.
    All,
    /// HEAD plus, when given, its upstream (full ref name) so incoming
    /// commits are visible.
    Head { upstream: Option<String> },
}

/// Narrows a log to some of its commits. Both parts must match.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogFilter {
    /// `Name <email>` as [`Repo::authors`] lists it (after `.mailmap`).
    pub author: Option<String>,
    /// A path or pathspec; commits that didn't change it are left out.
    pub path: Option<String>,
}

impl LogFilter {
    pub fn is_empty(&self) -> bool {
        self.author.is_none() && self.path.is_none()
    }
}

#[derive(Debug, Clone)]
pub struct Repo {
    git: GitCli,
    root: PathBuf,
    git_dir: PathBuf,
    common_dir: PathBuf,
    /// git turned out too old for something tried in one command, so it's
    /// done the old way from then on.
    old_git: Arc<OldGit>,
}

/// What [`Repo::old_git`] found missing.
#[derive(Debug, Default)]
struct OldGit {
    /// `for-each-ref`'s `%(ahead-behind:…)` (git 2.41).
    ahead_behind: AtomicBool,
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
            old_git: Arc::default(),
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

    /// The main working tree's root: the same as [`root`](Self::root)
    /// except inside a linked worktree.
    pub fn main_root(&self) -> &Path {
        match self.common_dir.parent() {
            Some(parent) if self.common_dir.file_name().is_some_and(|n| n == ".git") => parent,
            _ => &self.root,
        }
    }

    /// Where [`create_worktree`](Self::create_worktree) puts branch `name`:
    /// `.worktrees/<name>` in the main working tree.
    pub fn worktree_path(&self, name: &str) -> PathBuf {
        self.main_root().join(WORKTREE_DIR).join(name)
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

    /// What [`remotes`](Self::remotes) and
    /// [`base_branch_setting`](Self::base_branch_setting) return, read with
    /// one `git config` instead of two commands.
    pub fn remote_config(&self) -> Result<RemoteConfig> {
        // Keys come out with the section and variable names lowercased.
        let pattern = "^(remote\\..*|tt-git\\.basebranch)$";
        match self.git.run(&["config", "-z", "--get-regexp", pattern]) {
            Ok(out) => {
                let (remotes, base_branch) = parse::parse_remote_config(&out.stdout);
                Ok(RemoteConfig { remotes, base_branch })
            }
            // Nothing matched.
            Err(GitError::Failed { code, .. }) if code == "1" => Ok(RemoteConfig::default()),
            Err(e) => Err(e),
        }
    }

    /// Local branches, remote-tracking branches and tags.
    pub fn refs(&self) -> Result<Vec<GitRef>> {
        let remotes = self.remotes()?;
        Ok(self.refs_with_authors(&remotes)?.0)
    }

    /// [`refs`](Self::refs) given the remote names, plus the author of each
    /// ref's commit as `Name <email>` (before `.mailmap`), by full ref name,
    /// for [`ref_authors_of`](Self::ref_authors_of).
    pub fn refs_with_authors(&self, remotes: &[String]) -> Result<(Vec<GitRef>, parse::RefAuthors)> {
        let out = self.git.run(&[
            "for-each-ref",
            parse::REF_FORMAT,
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ])?;
        parse::parse_refs_with_authors(&out.stdout, remotes)
    }

    /// The base branch picked for this repo (`tt-git.baseBranch`), if any.
    pub fn base_branch_setting(&self) -> Option<String> {
        let out = self.git.run(&["config", "--get", BASE_BRANCH_KEY]).ok()?;
        let name = out.stdout_str().trim().to_string();
        (!name.is_empty()).then_some(name)
    }

    /// git's `core.editor`, if set.
    pub fn core_editor(&self) -> Option<String> {
        let out = self.git.run(&["config", "--get", "core.editor"]).ok()?;
        let cmd = out.stdout_str().trim().to_string();
        (!cmd.is_empty()).then_some(cmd)
    }

    /// `path`'s contents as of `rev`, and `rev`'s short commit id.
    pub fn file_at(&self, rev: &str, path: &str) -> Result<(Vec<u8>, String)> {
        let commit = format!("{rev}^{{commit}}");
        let short = self.git.run(&["rev-parse", "--short", &commit])?.stdout_str();
        let blob = self.git.run(&["cat-file", "blob", &format!("{rev}:{path}")])?;
        Ok((blob.stdout, short.trim().to_string()))
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

    /// How far `rev` is from `base` (`git rev-list --left-right --count base...rev`).
    pub fn ahead_behind(&self, base: &str, rev: &str) -> Result<AheadBehind> {
        let range = format!("{base}...{rev}");
        let out = self.git.run(&["rev-list", "--left-right", "--count", &range, "--"])?;
        let text = out.stdout_str();
        let mut counts = text.split_whitespace().map(str::parse::<u32>);
        match (counts.next(), counts.next()) {
            (Some(Ok(behind)), Some(Ok(ahead))) => Ok(AheadBehind { ahead, behind }),
            _ => Err(GitError::Parse(format!("bad rev-list count {:?}", text.trim()))),
        }
    }

    /// [`ahead_behind`](Self::ahead_behind) for several refs in one
    /// `for-each-ref`, given as (full name, target commit) pairs, keyed by
    /// full name. Refs left out are ones it couldn't compare (or that no
    /// longer point at the given target), for asking `ahead_behind` about;
    /// `None` when it can't be done in one go (git before 2.41, or a ref
    /// that isn't a commit).
    pub fn ahead_behind_many(&self, base: &str, refs: &[(&str, &str)]) -> Option<HashMap<String, AheadBehind>> {
        // The atom's argument ends at the first ')'.
        if self.old_git.ahead_behind.load(Ordering::Relaxed) || base.contains(['(', ')', '%']) {
            return None;
        }
        let format = format!("--format=%(refname)%00%(objectname)%00%(*objectname)%00%(ahead-behind:{base})");
        let mut args = vec!["for-each-ref", &format];
        args.extend(refs.iter().map(|(name, _)| *name));
        let out = match self.git.run(&args) {
            Ok(out) => out,
            Err(e) => {
                if e.stderr().is_some_and(|s| s.contains("unknown field name")) {
                    self.old_git.ahead_behind.store(true, Ordering::Relaxed);
                }
                return None;
            }
        };
        let text = out.stdout_str();
        let mut found = HashMap::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\0').collect();
            let [name, oid, peeled, counts] = f.as_slice() else {
                continue;
            };
            let target = if peeled.is_empty() { oid } else { peeled };
            // A pattern also matches refs under it, so only exact names count.
            if !refs.iter().any(|(n, t)| n == name && t == target) {
                continue;
            }
            let mut counts = counts.split_whitespace().map(str::parse::<u32>);
            if let (Some(Ok(ahead)), Some(Ok(behind)), None) = (counts.next(), counts.next(), counts.next()) {
                found.insert(name.to_string(), AheadBehind { ahead, behind });
            }
        }
        Some(found)
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
                Some((name, parse::contact(author, email)))
            })
            .collect();
        Ok(self.map_ref_authors(&rows))
    }

    /// [`ref_authors`](Self::ref_authors) from the authors
    /// [`refs_with_authors`](Self::refs_with_authors) read, which must
    /// include every ref under `prefix`; only `.mailmap` needs git.
    pub fn ref_authors_of(&self, authors: &[(String, String)], prefix: &str) -> Result<HashMap<String, String>> {
        if prefix.contains(['*', '?', '[', '\\']) {
            // A pattern for-each-ref would glob-match.
            return self.ref_authors(prefix);
        }
        let rows: Vec<(&str, String)> = authors
            .iter()
            .filter(|(name, _)| ref_pattern_matches(prefix, name))
            .map(|(name, contact)| (name.as_str(), contact.clone()))
            .collect();
        Ok(self.map_ref_authors(&rows))
    }

    /// (full name, `Name <email>`) rows to (full name, mapped name).
    fn map_ref_authors(&self, rows: &[(&str, String)]) -> HashMap<String, String> {
        let mut contacts: Vec<&str> = rows.iter().map(|(_, c)| c.as_str()).collect();
        contacts.sort_unstable();
        contacts.dedup();
        let mapped = self.mailmap(&contacts);
        rows.iter()
            .map(|(name, contact)| {
                let contact = mapped.get(contact.as_str()).map_or(contact.as_str(), String::as_str);
                let author = contact.rsplit_once(" <").map_or(contact, |(n, _)| n);
                (name.to_string(), author.to_string())
            })
            .collect()
    }

    /// `git check-mailmap` for each `Name <email>`; contacts git can't map
    /// are left out.
    fn mailmap(&self, contacts: &[&str]) -> HashMap<String, String> {
        if contacts.is_empty() {
            return HashMap::new();
        }
        // All at once on stdin. As arguments, a contact starting with '-'
        // reads as an option and fails its batch, and one git can't parse
        // fails its batch too; batches keep those results.
        if !contacts.iter().any(|c| c.starts_with('-')) {
            let mut input = Vec::new();
            for contact in contacts {
                input.extend_from_slice(contact.as_bytes());
                input.push(b'\n');
            }
            if let Ok(out) = self.git.run_with_stdin(&["check-mailmap", "--stdin"], Some(&input)) {
                return contacts
                    .iter()
                    .zip(out.stdout_str().lines())
                    .map(|(from, to)| (from.to_string(), to.to_string()))
                    .collect();
            }
        }
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

    /// The stash list, newest first. Without a stash reflog there are none,
    /// and no git process is run to find that out.
    pub fn stashes(&self) -> Result<Vec<Stash>> {
        let dir = &self.common_dir;
        let reflog = dir.join("logs").join("refs").join("stash");
        if !dir.join("reftable").exists() && std::fs::metadata(&reflog).map_or(true, |m| m.len() == 0) {
            return Ok(Vec::new());
        }
        let out = self.git.run(&["stash", "list", parse::STASH_FORMAT])?;
        Ok(parse::parse_stashes(&out.stdout))
    }

    /// What `stash` holds, for the details pane: its message, then the
    /// changes to tracked files and any untracked files it saved, each as a
    /// diffstat and patch.
    pub fn stash_details(&self, stash: &Stash) -> Result<String> {
        let mut out = format!(
            "{}  {}

",
            stash.name(),
            stash.message
        );
        let show = |rev: &str| -> Result<String> {
            Ok(self
                .git
                .run(&[
                    "show",
                    "--format=",
                    "--stat",
                    "--patch",
                    "--diff-merges=first-parent",
                    rev,
                    "--",
                ])?
                .into_stdout_string())
        };
        out.push_str(&show(&stash.oid)?);
        if let Some(untracked) = &stash.untracked {
            out.push_str(
                "
── untracked ──
",
            );
            out.push_str(&show(untracked)?);
        }
        Ok(out)
    }

    /// Time of the most recent update to `full_ref` according to its reflog
    /// file, if it has one.
    pub fn ref_activity(&self, full_ref: &str) -> Option<i64> {
        let rel: PathBuf = full_ref.split('/').collect();
        let path = self.common_dir.join("logs").join(rel);
        // Only the last line counts, so read just the end of the file.
        const WINDOW: u64 = 4096;
        let tail = std::fs::File::open(&path).and_then(|mut file| {
            let len = file.metadata()?.len();
            parse::tail_reflog_time(&mut file, len, WINDOW)
        });
        match tail {
            Ok(time) => time,
            // E.g. rewritten shorter (`reflog expire`) while being read.
            Err(_) => std::fs::read(path).ok().and_then(|c| parse::last_reflog_time(&c)),
        }
    }

    /// Streams commits in topological order, newest first, stopping after
    /// `limit` or when `on_commit` returns `false`.
    pub fn stream_commits(&self, scope: &LogScope, limit: usize, on_commit: impl FnMut(Commit) -> bool) -> Result<()> {
        self.stream_commits_from(scope, 0, limit, on_commit)
    }

    /// Like [`stream_commits`](Self::stream_commits), leaving out the first
    /// `skip` of the `limit` commits: with the same refs, the commits a
    /// load with a smaller limit stopped before.
    pub fn stream_commits_from(
        &self,
        scope: &LogScope,
        skip: usize,
        limit: usize,
        on_commit: impl FnMut(Commit) -> bool,
    ) -> Result<()> {
        let mut args = topo_log_args(limit.saturating_sub(skip));
        if skip > 0 {
            // git counts --max-count after skipping.
            args.push(format!("--skip={skip}"));
        }
        if !self.push_scope(&mut args, scope)? {
            return Ok(());
        }
        args.push("--".into());
        self.stream_log(&args, on_commit)
    }

    /// The stored state of everything a log's starting points and history
    /// are read from: every ref (loose and packed, the stash included),
    /// every worktree's HEAD and per-worktree refs, and the shallow and
    /// graft lists that cut history short. Equal results mean a log of HEAD,
    /// of any ref or of `--all` would list the same commits. Reflogs and
    /// pseudorefs such as FETCH_HEAD are left out.
    ///
    /// Reads files only, with no git process, so it's cheap enough to take
    /// before every log. `None` when it can't tell: refs stored as a
    /// reftable, or a file that couldn't be read.
    pub fn ref_fingerprint(&self) -> Option<Vec<u8>> {
        let dir = &self.common_dir;
        if dir.join("reftable").exists() {
            return None;
        }
        let mut out = Vec::new();
        for name in ["HEAD", "packed-refs", "shallow", "info/grafts"] {
            fingerprint_file(&mut out, dir, Path::new(name)).ok()?;
        }
        fingerprint_tree(&mut out, dir, Path::new("refs")).ok()?;
        // Linked worktrees' HEADs and their own refs (bisect and the like).
        for id in sorted_entries(&dir.join("worktrees")).ok()? {
            if !id.file_type().ok()?.is_dir() {
                continue;
            }
            let rel = Path::new("worktrees").join(id.file_name());
            fingerprint_file(&mut out, dir, &rel.join("HEAD")).ok()?;
            fingerprint_tree(&mut out, dir, &rel.join("refs")).ok()?;
        }
        Some(out)
    }

    /// Up to `limit` commits of `scope` that match `filter`, in topological
    /// order. Parents name only commits in the result: a path filter
    /// rewrites them to the nearest commit that changed the path, and
    /// parents an author filter leaves out are dropped.
    pub fn filtered_commits(&self, scope: &LogScope, filter: &LogFilter, limit: usize) -> Result<Vec<Commit>> {
        let mut args = topo_log_args(limit);
        if let Some(author) = &filter.author {
            // --use-mailmap so the match is against the mapped identity
            // that %aN/%aE (and `authors`) show.
            args.extend([
                "--use-mailmap".into(),
                "--fixed-strings".into(),
                format!("--author={author}"),
            ]);
        }
        if filter.path.is_some() {
            // Without it %P shows the real parents, which mostly aren't listed.
            args.push("--parents".into());
        }
        if !self.push_scope(&mut args, scope)? {
            return Ok(Vec::new());
        }
        args.push("--".into());
        args.extend(filter.path.clone());
        let mut commits = Vec::new();
        self.stream_log(&args, |c| {
            commits.push(c);
            true
        })?;
        let shown: HashSet<Oid> = commits.iter().map(|c| c.oid).collect();
        for c in &mut commits {
            c.parents.retain(|p| shown.contains(p));
        }
        Ok(commits)
    }

    /// Adds the revisions for `scope` to a log command line. `false` when
    /// there's nothing to log (HEAD is unborn).
    fn push_scope(&self, args: &mut Vec<String>, scope: &LogScope) -> Result<bool> {
        match scope {
            LogScope::All => {
                args.push("--exclude=refs/stash".into());
                // --all includes HEAD (even when detached) and, unlike an
                // explicit HEAD argument, does not fail on an unborn branch.
                args.push("--all".into());
            }
            LogScope::Head { upstream } => {
                if self.head()?.oid().is_none() {
                    return Ok(false);
                }
                args.push("HEAD".into());
                if let Some(up) = upstream {
                    args.push(up.clone());
                }
            }
        }
        Ok(true)
    }

    /// Everyone who authored a commit on any ref, as `Name <email>` after
    /// `.mailmap`, with their commit counts: most commits first.
    pub fn authors(&self) -> Result<Vec<(String, usize)>> {
        let args = [
            "log",
            "-z",
            "--use-mailmap",
            "--format=%aN <%aE>",
            "--exclude=refs/stash",
            "--all",
            "--",
        ];
        let mut counts: HashMap<String, usize> = HashMap::new();
        self.git.stream(&args, 0, |record| {
            let ident = String::from_utf8_lossy(record);
            let ident = ident.trim();
            if let Some(n) = counts.get_mut(ident) {
                *n += 1;
            } else if !ident.is_empty() {
                counts.insert(ident.to_string(), 1);
            }
            true
        })?;
        let mut authors: Vec<(String, usize)> = counts.into_iter().collect();
        authors.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Ok(authors)
    }

    /// Paths of the files in the index, relative to the root.
    pub fn tracked_files(&self) -> Result<Vec<String>> {
        let out = self.git.run(&["ls-files", "-z"])?;
        Ok(out
            .stdout
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect())
    }

    /// Runs `args` (a `git log -z` with [`parse::LOG_FORMAT`]), handing each
    /// commit to `on_commit` until it returns `false`.
    fn stream_log(&self, args: &[String], mut on_commit: impl FnMut(Commit) -> bool) -> Result<()> {
        let mut error = None;
        let mut idents = parse::Idents::default();
        self.git.stream(args, 0, |record| {
            if record.iter().all(|b| b.is_ascii_whitespace()) {
                return true;
            }
            match parse::parse_commit_interned(record, &mut idents) {
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
        let exclude = match &merge {
            Some(m) => format!("^{}", m.parents[0]),
            None => format!("^{base}"),
        };
        let max = format!("--max-count={limit}");
        let mut commits = self.log(&["--first-parent", "--topo-order", &max], &[tip, &exclude])?;
        let fork = match commits.last() {
            Some(last) if commits.len() < limit => last.parents.first().copied(),
            _ => None,
        };
        let fork_hex = fork.map(|f| f.hex());
        let mut extras = Vec::new();
        extras.extend(fork_hex.as_deref());
        if merge.is_none() {
            extras.push(base);
        }
        let mut extras = self.log(&["--no-walk=unsorted"], &extras)?.into_iter();
        let fork_commit = fork.as_ref().and_then(|_| extras.next());
        let shown: HashSet<Oid> = commits.iter().map(|c| c.oid).chain(fork).collect();
        for c in &mut commits {
            c.parents.retain(|p| shown.contains(p));
        }
        let merge_oid = merge.as_ref().map(|m| m.oid);
        match merge {
            Some(mut m) => {
                m.parents = fork.into_iter().chain([tip_commit.oid]).collect();
                commits.insert(0, m);
            }
            // While unmerged, the base's tip shows where the base is now,
            // unless it hasn't moved since the fork.
            None => {
                if let Some(mut b) = extras.next().filter(|b| Some(b.oid) != fork) {
                    b.parents = fork.into_iter().collect();
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
        Ok(self.git.run(&args)?.into_stdout_string())
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

/// What [`Repo::remote_config`] reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteConfig {
    /// Remote names, as `git remote` lists them.
    pub remotes: Vec<String>,
    /// As [`Repo::base_branch_setting`] gives it.
    pub base_branch: Option<String>,
}

/// Whether for-each-ref's `pattern` (with no glob characters) selects
/// `refname`: the same name, or one under it.
fn ref_pattern_matches(pattern: &str, refname: &str) -> bool {
    refname
        .strip_prefix(pattern)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/') || pattern.ends_with('/'))
}

/// `git log` in topological order with [`parse::LOG_FORMAT`], up to `limit`.
fn topo_log_args(limit: usize) -> Vec<String> {
    vec![
        "log".into(),
        "--topo-order".into(),
        "-z".into(),
        parse::LOG_FORMAT.into(),
        format!("--max-count={limit}"),
    ]
}

/// Appends `dir/rel` to a [`Repo::ref_fingerprint`]: its path and length
/// before its content, so no two different sets of files run together the
/// same, or a mark that it's missing.
fn fingerprint_file(out: &mut Vec<u8>, dir: &Path, rel: &Path) -> std::io::Result<()> {
    out.extend_from_slice(rel.as_os_str().as_encoded_bytes());
    out.push(0);
    match std::fs::read(dir.join(rel)) {
        Ok(content) => {
            out.push(b'+');
            out.extend_from_slice(&(content.len() as u64).to_le_bytes());
            out.extend_from_slice(&content);
        }
        // Not there (or deleted while listed): an absent ref.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => out.push(b'-'),
        Err(e) => return Err(e),
    }
    Ok(())
}

/// Appends every file under `dir/rel` to a [`Repo::ref_fingerprint`], in
/// name order, leaving out `.lock` files (updates not yet made).
fn fingerprint_tree(out: &mut Vec<u8>, dir: &Path, rel: &Path) -> std::io::Result<()> {
    for entry in sorted_entries(&dir.join(rel))? {
        let rel = rel.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            fingerprint_tree(out, dir, &rel)?;
        } else if !entry.file_name().as_encoded_bytes().ends_with(b".lock") {
            fingerprint_file(out, dir, &rel)?;
        }
    }
    Ok(())
}

/// `dir`'s entries by name; none if it doesn't exist.
fn sorted_entries(dir: &Path) -> std::io::Result<Vec<std::fs::DirEntry>> {
    let mut entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries.collect::<std::io::Result<Vec<_>>>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    entries.sort_by_key(|e| e.file_name());
    Ok(entries)
}
