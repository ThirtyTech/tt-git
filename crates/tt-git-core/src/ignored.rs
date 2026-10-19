//! What git ignores in the working tree, so the file watcher can skip
//! changes that can't show up in `git status` (build output,
//! `node_modules`).

use std::collections::{BTreeSet, HashMap};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::git::{GitError, Result};
use crate::repo::Repo;

/// Git's ignore rules for a working tree, as they were when loaded.
///
/// A path is ignored the way git decides it: each leading directory is
/// checked in turn, and once one is ignored so is everything under it (git
/// doesn't look inside). At each level the deepest `.gitignore` with a
/// matching pattern decides, then `info/exclude`, then `core.excludesFile`.
///
/// Rules can go stale. `.gitignore` files in the tree and the index are
/// the watcher's to notice (see [`affected_by`](Self::affected_by)); the
/// files outside the tree are checked by [`stale`](Self::stale). Config
/// files other than the repo's and the global one aren't checked.
#[derive(Debug)]
pub struct IgnoreRules {
    root: PathBuf,
    /// `.gitignore` files by the directory they're in, relative to the
    /// root (`""` for the root itself).
    dirs: HashMap<PathBuf, Gitignore>,
    /// `core.excludesFile` then `info/exclude`, so the latter wins.
    excludes: Gitignore,
    /// Tracked files that match a pattern, as [`key`](Self::key)s. Ignore
    /// patterns don't apply to them.
    tracked: BTreeSet<String>,
    ignore_case: bool,
    /// Files outside the tree that hold rules or say where they are, with
    /// their length and modified time (`None`: missing).
    stamps: Vec<(PathBuf, Option<(u64, SystemTime)>)>,
}

impl IgnoreRules {
    /// Loads the rules of `repo`'s working tree, matching paths under
    /// `root` (the root as the caller spells it, such as canonicalized).
    /// Fails if a rule can't be read or parsed, rather than guess.
    pub fn load(repo: &Repo, root: &Path) -> Result<Self> {
        let git = repo.git();
        let (mut excludes_file, mut ignore_case) = (None, false);
        let config = match git.run(&[
            "config",
            "-z",
            "--type=path",
            "--get-regexp",
            r"^core\.(excludesfile|ignorecase)$",
        ]) {
            Ok(out) => out.stdout,
            // Neither is set.
            Err(GitError::Failed { .. }) => Vec::new(),
            Err(e) => return Err(e),
        };
        for record in config.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let record = String::from_utf8_lossy(record);
            // No newline: a key with no value, which is true as a bool.
            let (key, value) = record.split_once('\n').unwrap_or((&record, "true"));
            match key {
                "core.excludesfile" => excludes_file = Some(root.join(value)),
                "core.ignorecase" => ignore_case = is_true(value),
                _ => {}
            }
        }
        let homes = homes(repo);
        let excludes_file = excludes_file.or_else(|| default_excludes_file(repo, &homes));
        let info_exclude = repo.common_dir().join("info").join("exclude");

        let builder = |dir: &Path| {
            let mut b = GitignoreBuilder::new(dir);
            b.case_insensitive(ignore_case).map_err(rule_error)?;
            Ok::<_, GitError>(b)
        };
        let mut b = builder(root)?;
        for file in excludes_file.iter().chain([&info_exclude]) {
            if file.exists() {
                add_file(&mut b, file)?;
            }
        }
        let excludes = b.build().map_err(rule_error)?;

        // The .gitignore files git reads: those it doesn't ignore, and
        // ignored ones outside ignored directories. With them, the tracked
        // files patterns match ("H"); --directory keeps ignored directories
        // from being listed file by file.
        let unignored = git.run(&[
            "ls-files",
            "-z",
            "-co",
            "--exclude-standard",
            "--",
            ":(glob)**/.gitignore",
        ])?;
        let ignored = git.run(&["ls-files", "-z", "-t", "-coi", "--exclude-standard", "--directory"])?;
        let mut gitignores: Vec<&str> = paths(&unignored.stdout).collect();
        let mut tracked = BTreeSet::new();
        for entry in paths(&ignored.stdout) {
            let (tag, rel) = entry.split_at_checked(2).unwrap_or(("", entry));
            match tag {
                "H " => {
                    tracked.insert(key(rel, ignore_case));
                }
                "? " if rel == ".gitignore" || rel.ends_with("/.gitignore") => gitignores.push(rel),
                _ => {}
            }
        }
        let mut dirs = HashMap::new();
        for rel in gitignores {
            let dir = Path::new(rel).parent().unwrap_or(Path::new("")).to_path_buf();
            if dirs.contains_key(&dir) {
                continue;
            }
            let mut b = builder(&root.join(&dir))?;
            add_file(&mut b, &root.join(rel))?;
            dirs.insert(dir, b.build().map_err(rule_error)?);
        }

        let mut outside = vec![info_exclude, repo.common_dir().join("config")];
        outside.extend(excludes_file);
        for home in &homes {
            outside.push(home.join(".gitconfig"));
        }
        if let Some(xdg) = env(repo, "XDG_CONFIG_HOME") {
            outside.push(PathBuf::from(xdg).join("git").join("config"));
        }
        let stamps = outside
            .into_iter()
            .map(|p| {
                let s = stamp(&p);
                (p, s)
            })
            .collect();

        Ok(Self {
            root: root.to_path_buf(),
            dirs,
            excludes,
            tracked,
            ignore_case,
            stamps,
        })
    }

    /// Whether a change at `path` (absolute) can't change `git status`:
    /// git ignores it, and nothing tracked is at or under it.
    pub fn hides(&self, path: &Path) -> bool {
        let Some(names) = self.names(path) else {
            return false;
        };
        // Git reads a .gitignore even if it's ignored itself, so a change
        // to one only hides behind an ignored directory.
        let levels = if names.last() == Some(&OsStr::new(".gitignore")) {
            names.len() - 1
        } else {
            names.len()
        };
        let mut prefix = PathBuf::new();
        for (i, name) in names.iter().take(levels).enumerate() {
            prefix.push(name);
            let is_dir = i + 1 < names.len() || std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir());
            if self.excluded(&prefix, is_dir) {
                let rel: PathBuf = names.iter().collect();
                return !self.tracked_at_or_under(&rel);
            }
        }
        false
    }

    /// Whether a change at `path` that wasn't hidden may change the rules:
    /// it's a `.gitignore`, a directory with a `.gitignore` the rules
    /// don't have (moved in), or holds one they have that's gone.
    pub fn affected_by(&self, path: &Path) -> bool {
        let Some(names) = self.names(path) else {
            return false;
        };
        if names.last() == Some(&OsStr::new(".gitignore")) {
            return true;
        }
        let rel: PathBuf = names.iter().collect();
        if !self.dirs.contains_key(&rel) && path.join(".gitignore").is_file() {
            return true;
        }
        self.dirs
            .keys()
            .any(|dir| dir.starts_with(&rel) && !self.root.join(dir).join(".gitignore").is_file())
    }

    /// Whether a file outside the tree that holds rules (or says where
    /// they are) changed since they were loaded.
    pub fn stale(&self) -> bool {
        self.stamps.iter().any(|(path, was)| stamp(path) != *was)
    }

    /// `path`'s names below the root, if it's under it.
    fn names<'p>(&self, path: &'p Path) -> Option<Vec<&'p OsStr>> {
        let rel = path.strip_prefix(&self.root).ok()?;
        let names = rel
            .components()
            .map(|c| match c {
                Component::Normal(name) => Some(name),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        (!names.is_empty()).then_some(names)
    }

    /// Whether `rel` matches as ignored, its parents aside.
    fn excluded(&self, rel: &Path, is_dir: bool) -> bool {
        let path = self.root.join(rel);
        let mut dir = rel.parent();
        while let Some(d) = dir {
            if let Some(rules) = self.dirs.get(d) {
                match rules.matched(&path, is_dir) {
                    Match::Ignore(_) => return true,
                    Match::Whitelist(_) => return false,
                    Match::None => {}
                }
            }
            dir = d.parent();
        }
        self.excludes.matched(&path, is_dir).is_ignore()
    }

    fn tracked_at_or_under(&self, rel: &Path) -> bool {
        let k = key(&rel.to_string_lossy(), self.ignore_case);
        if self.tracked.contains(&k) {
            return true;
        }
        let dir = format!("{k}/");
        self.tracked
            .range(dir.clone()..)
            .next()
            .is_some_and(|t| t.starts_with(&dir))
    }
}

/// A relative path as compared with git's: `/`-separated, and lowercase if
/// git ignores case.
fn key(path: &str, ignore_case: bool) -> String {
    let path = path.replace('\\', "/");
    if ignore_case { path.to_lowercase() } else { path }
}

/// The paths in `ls-files -z` output.
fn paths(out: &[u8]) -> impl Iterator<Item = &str> {
    out.split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .filter_map(|p| std::str::from_utf8(p).ok())
}

fn is_true(value: &str) -> bool {
    let v = value.trim();
    ["true", "yes", "on"].iter().any(|t| v.eq_ignore_ascii_case(t)) || v.parse::<i64>().is_ok_and(|n| n != 0)
}

fn env(repo: &Repo, name: &str) -> Option<std::ffi::OsString> {
    repo.git().env_var(name).filter(|v| !v.is_empty())
}

/// Where git takes `~` to be: `HOME`, or on Windows the profile.
fn homes(repo: &Repo) -> Vec<PathBuf> {
    if let Some(home) = env(repo, "HOME") {
        return vec![home.into()];
    }
    let mut homes = Vec::new();
    if let (Some(drive), Some(path)) = (env(repo, "HOMEDRIVE"), env(repo, "HOMEPATH")) {
        let mut home = drive;
        home.push(path);
        homes.push(home.into());
    }
    homes.extend(env(repo, "USERPROFILE").map(PathBuf::from));
    homes
}

/// `core.excludesFile` when it isn't set.
fn default_excludes_file(repo: &Repo, homes: &[PathBuf]) -> Option<PathBuf> {
    match env(repo, "XDG_CONFIG_HOME") {
        Some(xdg) => Some(PathBuf::from(xdg).join("git").join("ignore")),
        None => homes.first().map(|h| h.join(".config").join("git").join("ignore")),
    }
}

fn stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// Adds a rule file's patterns, refusing ones the matcher would read
/// differently from git.
fn add_file(b: &mut GitignoreBuilder, path: &Path) -> Result<()> {
    let text = std::fs::read_to_string(path).map_err(|e| GitError::Parse(format!("{}: {e}", path.display())))?;
    if let Some(line) = text.lines().find(|l| unclosed_bracket(l)) {
        return Err(GitError::Parse(format!("{}: unclosed [ in {line:?}", path.display())));
    }
    match b.add(path) {
        Some(e) => Err(rule_error(e)),
        None => Ok(()),
    }
}

/// Whether a pattern has a `[` with no `]` to close it, which git matches
/// against nothing and the matcher takes literally.
fn unclosed_bracket(line: &str) -> bool {
    if line.starts_with('#') {
        return false;
    }
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            // A `]` right after `[` (or `[!`, `[^`) is part of the class.
            '[' => {
                let rest = chars.as_str();
                let body = rest.strip_prefix(['!', '^']).unwrap_or(rest);
                match body.get(1..).and_then(|after| after.find(']')) {
                    Some(close) => chars = body[1 + close + 1..].chars(),
                    None => return true,
                }
            }
            _ => {}
        }
    }
    false
}

fn rule_error(e: ignore::Error) -> GitError {
    GitError::Parse(format!("ignore rules: {e}"))
}
