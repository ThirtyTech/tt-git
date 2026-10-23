//! The unified branch list: local and remote-tracking branches merged into
//! one entry per branch and ordered by most recent activity.
//!
//! Grouping rules:
//! 1. Every local branch is an entry.
//! 2. A remote branch joins the local branch that has it as its configured
//!    upstream (so local `x` tracking `origin/feature-x` is one entry). When
//!    several do, the one with the same name gets it.
//! 3. Otherwise it joins the local branch with the same name, after
//!    stripping `<remote>/`.
//! 4. Remaining remote branches are grouped by name into remote-only
//!    entries (`origin/bar` and `upstream/bar` → `bar`).
//!
//! Activity is the latest of: any member's tip commit date, any member's
//! last reflog update (commits and resets; for remote refs only pushes, see
//! [`Repo::ref_activity`](crate::Repo::ref_activity)), and the last time
//! HEAD was checked out to the local branch.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::model::{Checkout, GitRef, Head, RefKind};

/// Remote preferred when a remote-only branch exists on several remotes.
pub const PREFERRED_REMOTE: &str = "origin";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchEntry {
    /// Local branch name, or the remote branch name without its remote.
    pub name: String,
    pub local: Option<GitRef>,
    pub remotes: Vec<GitRef>,
    pub is_head: bool,
    /// Unix seconds of the most recent activity.
    pub activity: i64,
}

/// Summary of how an entry's local and remote sides relate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BranchState {
    /// Local branch with no remote counterpart.
    LocalOnly,
    /// Only exists on remote(s).
    RemoteOnly { remotes: Vec<String> },
    /// Tracks an upstream and matches it.
    InSync,
    /// Tracks an upstream and differs from it.
    Diverged { ahead: u32, behind: u32 },
    /// Tracks an upstream that was deleted.
    Gone,
    /// Has a same-named remote branch but no upstream configured.
    Untracked { remotes: Vec<String>, same_commit: bool },
}

/// What checking out an entry (or a commit) does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckoutPlan {
    /// `git switch <name>`
    Local(String),
    /// `git switch -c <name> --track <remote_ref>`; `remote_ref` is the short
    /// name like `origin/feature`.
    Track { name: String, remote_ref: String },
    /// `git switch --detach <oid>`
    Detach(String),
}

impl BranchEntry {
    pub fn state(&self) -> BranchState {
        let remote_names = || {
            let mut names: Vec<String> = self.remotes.iter().filter_map(remote_of).collect();
            names.dedup();
            names
        };
        let Some(local) = &self.local else {
            return BranchState::RemoteOnly {
                remotes: remote_names(),
            };
        };
        if local.upstream_gone {
            return BranchState::Gone;
        }
        // The upstream may be in another entry, when a same-named branch
        // also tracks it.
        if let Some(up) = &local.upstream
            && (up.starts_with("refs/remotes/") || self.remotes.iter().any(|r| &r.full_name == up))
        {
            return if local.ahead == 0 && local.behind == 0 {
                BranchState::InSync
            } else {
                BranchState::Diverged {
                    ahead: local.ahead,
                    behind: local.behind,
                }
            };
        }
        if self.remotes.is_empty() {
            BranchState::LocalOnly
        } else {
            BranchState::Untracked {
                same_commit: self.remotes.iter().any(|r| r.target == local.target),
                remotes: remote_names(),
            }
        }
    }

    /// The commit the entry points at: the local tip, else the preferred remote's.
    pub fn target(&self) -> Option<&str> {
        self.local
            .as_ref()
            .or_else(|| self.preferred_remote())
            .map(|r| r.target.as_str())
    }

    /// The remote ref chosen when several hold the branch: `origin` if
    /// one of them is `origin`, else the first.
    pub fn preferred_remote(&self) -> Option<&GitRef> {
        self.remotes
            .iter()
            .find(|r| remote_of(r).as_deref() == Some(PREFERRED_REMOTE))
            .or_else(|| self.remotes.first())
    }

    /// How to check this entry out. Remote-only entries get a local
    /// tracking branch created automatically.
    pub fn checkout_plan(&self) -> Option<CheckoutPlan> {
        if let Some(local) = &self.local {
            return Some(CheckoutPlan::Local(local.short_name.clone()));
        }
        let remote = self.preferred_remote()?;
        Some(CheckoutPlan::Track {
            name: self.name.clone(),
            remote_ref: remote.short_name.clone(),
        })
    }
}

fn remote_of(r: &GitRef) -> Option<String> {
    match &r.kind {
        RefKind::RemoteBranch { remote } => Some(remote.clone()),
        _ => None,
    }
}

/// Builds the unified list. `reflog_time` returns the last reflog update for
/// a full ref name (see [`Repo::ref_activity`](crate::Repo::ref_activity)).
pub fn unify(
    refs: &[GitRef],
    head: &Head,
    checkouts: &[Checkout],
    reflog_time: impl Fn(&str) -> Option<i64>,
) -> Vec<BranchEntry> {
    let mut entries: Vec<BranchEntry> = Vec::new();
    let mut by_name: HashMap<String, usize> = HashMap::new();
    let mut by_upstream: HashMap<String, usize> = HashMap::new();

    for r in refs.iter().filter(|r| r.kind == RefKind::LocalBranch) {
        let idx = entries.len();
        by_name.insert(r.short_name.clone(), idx);
        if let Some(up) = &r.upstream {
            by_upstream.insert(up.clone(), idx);
        }
        entries.push(BranchEntry {
            name: r.short_name.clone(),
            local: Some(r.clone()),
            remotes: Vec::new(),
            is_head: head.branch() == Some(r.short_name.as_str()),
            activity: 0,
        });
    }

    for r in refs
        .iter()
        .filter(|r| matches!(r.kind, RefKind::RemoteBranch { .. }) && r.symref.is_none())
    {
        let name = r.branch_name().to_string();
        let tracks =
            |idx: &&usize| entries[**idx].local.as_ref().and_then(|l| l.upstream.as_ref()) == Some(&r.full_name);
        let idx = match by_name
            .get(&name)
            .filter(tracks)
            .or_else(|| by_upstream.get(&r.full_name))
            .or_else(|| by_name.get(&name))
        {
            Some(&idx) => idx,
            None => {
                let idx = entries.len();
                by_name.insert(name.clone(), idx);
                entries.push(BranchEntry {
                    name,
                    local: None,
                    remotes: Vec::new(),
                    is_head: false,
                    activity: 0,
                });
                idx
            }
        };
        entries[idx].remotes.push(r.clone());
    }

    // Newest checkout per branch name (checkouts are newest first).
    let mut checked_out: HashMap<&str, i64> = HashMap::new();
    for c in checkouts {
        checked_out.entry(c.to.as_str()).or_insert(c.time);
    }

    for entry in &mut entries {
        let members = entry.local.iter().chain(entry.remotes.iter());
        let mut activity = 0;
        for r in members {
            activity = activity.max(r.date);
            if let Some(t) = reflog_time(&r.full_name) {
                activity = activity.max(t);
            }
        }
        if entry.local.is_some()
            && let Some(t) = checked_out.get(entry.name.as_str())
        {
            activity = activity.max(*t);
        }
        entry.activity = activity;
        entry.remotes.sort_by(|a, b| a.short_name.cmp(&b.short_name));
    }

    entries.sort_by(|a, b| b.activity.cmp(&a.activity).then_with(|| a.name.cmp(&b.name)));
    entries
}

/// Branch names tried, in order, when no remote has a `HEAD` to say which
/// branch is primary.
const PRIMARY_NAMES: &[&str] = &["main", "master", "development", "develop", "trunk"];

/// The project's primary branch: what `origin/HEAD` (or another remote's
/// `HEAD`) points at, else the first of `main`, `master`, `development`,
/// `develop`, `trunk` that exists on a remote, else as a local branch.
pub fn default_base(refs: &[GitRef]) -> Option<&GitRef> {
    let find = |full: &str| {
        refs.iter()
            .find(|r| r.full_name == full && r.symref.is_none() && is_branch(r))
    };
    let mut remote_heads: Vec<&GitRef> = refs
        .iter()
        .filter(|r| matches!(r.kind, RefKind::RemoteBranch { .. }) && r.branch_name() == "HEAD")
        .collect();
    remote_heads.sort_by_key(|r| remote_of(r).as_deref() != Some(PREFERRED_REMOTE));
    if let Some(target) = remote_heads.iter().find_map(|h| h.symref.as_deref().and_then(find)) {
        return Some(target);
    }
    let remote_branch = |name: &str| {
        refs.iter()
            .filter(|r| matches!(r.kind, RefKind::RemoteBranch { .. }) && r.symref.is_none())
            .filter(|r| r.branch_name() == name)
            .min_by_key(|r| remote_of(r).as_deref() != Some(PREFERRED_REMOTE))
    };
    PRIMARY_NAMES
        .iter()
        .find_map(|n| remote_branch(n))
        .or_else(|| PRIMARY_NAMES.iter().find_map(|n| find(&format!("refs/heads/{n}"))))
}

/// The base branch to compare against: `chosen` (a short name such as
/// `origin/development`) if it is still a branch, else [`default_base`].
pub fn resolve_base<'a>(refs: &'a [GitRef], chosen: Option<&str>) -> Option<&'a GitRef> {
    chosen
        .and_then(|name| {
            refs.iter()
                .find(|r| is_branch(r) && r.symref.is_none() && (r.short_name == name || r.full_name == name))
        })
        .or_else(|| default_base(refs))
}

fn is_branch(r: &GitRef) -> bool {
    matches!(r.kind, RefKind::LocalBranch | RefKind::RemoteBranch { .. })
}

/// Where branches compared with `base` come from: the base's remote
/// (`refs/remotes/origin/`), or local branches when the base is local.
pub fn base_scope(base: &GitRef) -> String {
    match &base.kind {
        RefKind::RemoteBranch { remote } => format!("refs/remotes/{remote}/"),
        _ => "refs/heads/".into(),
    }
}

/// Branches split by whether they are merged into a base branch, like
/// `git branch -r --merged origin/main` and `--no-merged`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BaseBranches {
    /// Short name of the base, e.g. `origin/main`.
    pub base: String,
    /// Its full ref name.
    pub base_ref: String,
    /// The base was picked by the user rather than detected.
    pub chosen: bool,
    /// What [`default_base`] picks, for going back to it.
    pub detected: Option<String>,
    /// Entries are narrowed to the compared ref (the one in
    /// [`base_scope`]); `activity` is that ref's commit date, and both lists
    /// are newest first.
    pub unmerged: Vec<BranchEntry>,
    pub merged: Vec<BranchEntry>,
    /// Author of each compared ref's tip, keyed by full ref name.
    pub authors: HashMap<String, String>,
    /// How far the [`TAGS_COMPARED`] newest tags are from the base, keyed
    /// by full ref name.
    pub tags: HashMap<String, AheadBehind>,
}

/// Commits a ref has that the base doesn't (`ahead`), and the base has
/// that it doesn't (`behind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AheadBehind {
    pub ahead: u32,
    pub behind: u32,
}

/// How many of the newest tags are compared with the base branch. Each one
/// is a history walk, which gets slower the older the tag is.
pub const TAGS_COMPARED: usize = 5;

/// The tags among `refs`, newest first (tagger date for annotated tags).
pub fn tags_newest_first(refs: &[GitRef]) -> Vec<GitRef> {
    let mut tags: Vec<GitRef> = refs.iter().filter(|r| r.kind == RefKind::Tag).cloned().collect();
    tags.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.short_name.cmp(&b.short_name)));
    tags
}

impl BaseBranches {
    /// The ref an entry from [`unmerged`](Self::unmerged) or
    /// [`merged`](Self::merged) was compared by.
    pub fn tip(entry: &BranchEntry) -> Option<&GitRef> {
        entry.remotes.first().or(entry.local.as_ref())
    }
}

/// Splits `entries` into unmerged and merged lists against `base`.
/// `merged` holds the full names of the refs in [`base_scope`] that are
/// merged into it. The base itself is left out, as are branches named
/// exactly one of `safe` (long-lived ones such as `main` or `production`)
/// and unmerged refs whose short name contains any of `exclude`
/// (`grep -v archive` style).
pub fn compare_with_base(
    entries: &[BranchEntry],
    base: &GitRef,
    merged: &HashSet<String>,
    exclude: &[String],
    safe: &[String],
) -> (Vec<BranchEntry>, Vec<BranchEntry>) {
    let scope = base_scope(base);
    let mut unmerged_list = Vec::new();
    let mut merged_list = Vec::new();
    for entry in entries {
        let narrowed = match &base.kind {
            RefKind::RemoteBranch { .. } => {
                let Some(tip) = entry.remotes.iter().find(|r| r.full_name.starts_with(&scope)) else {
                    continue;
                };
                BranchEntry {
                    remotes: vec![tip.clone()],
                    activity: tip.date,
                    ..entry.clone()
                }
            }
            _ => {
                let Some(local) = &entry.local else {
                    continue;
                };
                BranchEntry {
                    remotes: Vec::new(),
                    activity: local.date,
                    ..entry.clone()
                }
            }
        };
        let Some(tip) = BaseBranches::tip(&narrowed) else {
            continue;
        };
        if tip.full_name == base.full_name || safe.contains(&narrowed.name) {
            continue;
        }
        if merged.contains(&tip.full_name) {
            merged_list.push(narrowed);
        } else if !exclude
            .iter()
            .any(|x| !x.is_empty() && tip.short_name.contains(x.as_str()))
        {
            unmerged_list.push(narrowed);
        }
    }
    for list in [&mut unmerged_list, &mut merged_list] {
        list.sort_by(|a, b| b.activity.cmp(&a.activity).then_with(|| a.name.cmp(&b.name)));
    }
    (unmerged_list, merged_list)
}

/// Case-insensitive subsequence match, for filtering the list as you type.
pub fn matches_filter(name: &str, filter: &str) -> bool {
    let mut chars = name.chars().flat_map(char::to_lowercase);
    filter
        .chars()
        .flat_map(char::to_lowercase)
        .all(|f| chars.by_ref().any(|c| c == f))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(name: &str, target: &str, date: i64) -> GitRef {
        GitRef {
            full_name: format!("refs/heads/{name}"),
            short_name: name.into(),
            kind: RefKind::LocalBranch,
            target: target.into(),
            date,
            upstream: None,
            ahead: 0,
            behind: 0,
            upstream_gone: false,
            is_head: false,
            symref: None,
            subject: String::new(),
        }
    }

    fn remote(remote: &str, name: &str, target: &str, date: i64) -> GitRef {
        GitRef {
            full_name: format!("refs/remotes/{remote}/{name}"),
            short_name: format!("{remote}/{name}"),
            kind: RefKind::RemoteBranch { remote: remote.into() },
            ..local(name, target, date)
        }
    }

    fn tracking(mut r: GitRef, upstream: &str, ahead: u32, behind: u32) -> GitRef {
        r.upstream = Some(upstream.into());
        r.ahead = ahead;
        r.behind = behind;
        r
    }

    fn head(name: &str) -> Head {
        Head::Branch {
            name: name.into(),
            oid: "x".into(),
        }
    }

    fn names(entries: &[BranchEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn shared_upstream_goes_to_the_same_named_branch() {
        let up = "refs/remotes/origin/development";
        let development = tracking(local("development", "a", 100), up, 0, 0);
        let feature = tracking(local("feat/x", "b", 90), up, 1, 0);
        for locals in [[development.clone(), feature.clone()], [feature, development]] {
            let mut refs = locals.to_vec();
            refs.push(remote("origin", "development", "a", 100));
            let entries = unify(&refs, &head("development"), &[], |_| None);
            let dev = entries.iter().find(|e| e.name == "development").unwrap();
            assert_eq!(dev.remotes.len(), 1);
            assert_eq!(dev.state(), BranchState::InSync);
            let feat = entries.iter().find(|e| e.name == "feat/x").unwrap();
            assert!(feat.remotes.is_empty());
            assert_eq!(feat.state(), BranchState::Diverged { ahead: 1, behind: 0 });
        }
    }

    #[test]
    fn merges_local_and_remote_by_upstream_and_name() {
        let refs = vec![
            tracking(local("main", "a", 100), "refs/remotes/origin/main", 0, 0),
            tracking(local("x", "b", 90), "refs/remotes/origin/feature-x", 2, 1),
            local("solo", "c", 80),
            remote("origin", "main", "a", 100),
            remote("origin", "feature-x", "b0", 95),
            remote("origin", "solo", "c", 80),
            remote("origin", "remote-only", "d", 70),
            remote("upstream", "remote-only", "d2", 60),
        ];
        let entries = unify(&refs, &head("main"), &[], |_| None);
        assert_eq!(names(&entries), vec!["main", "x", "solo", "remote-only"]);

        let main = &entries[0];
        assert!(main.is_head);
        assert_eq!(main.state(), BranchState::InSync);

        let x = &entries[1];
        assert_eq!(x.remotes.len(), 1, "joined via upstream despite different name");
        assert_eq!(x.state(), BranchState::Diverged { ahead: 2, behind: 1 });
        assert_eq!(x.activity, 95, "remote tip is newer than local");

        assert_eq!(
            entries[2].state(),
            BranchState::Untracked {
                remotes: vec!["origin".into()],
                same_commit: true
            }
        );

        let ro = &entries[3];
        assert!(ro.local.is_none());
        assert_eq!(
            ro.state(),
            BranchState::RemoteOnly {
                remotes: vec!["origin".into(), "upstream".into()]
            }
        );
        assert_eq!(
            ro.checkout_plan(),
            Some(CheckoutPlan::Track {
                name: "remote-only".into(),
                remote_ref: "origin/remote-only".into()
            })
        );
        assert_eq!(ro.target(), Some("d"));
    }

    #[test]
    fn skips_symrefs_and_tags() {
        let mut origin_head = remote("origin", "HEAD", "a", 100);
        origin_head.symref = Some("refs/remotes/origin/main".into());
        let tag = GitRef {
            full_name: "refs/tags/v1".into(),
            kind: RefKind::Tag,
            ..local("v1", "a", 100)
        };
        let entries = unify(&[local("main", "a", 1), origin_head, tag], &head("main"), &[], |_| None);
        assert_eq!(names(&entries), vec!["main"]);
        assert_eq!(entries[0].state(), BranchState::LocalOnly);
    }

    #[test]
    fn gone_upstream() {
        let mut r = tracking(local("old", "a", 1), "refs/remotes/origin/old", 0, 0);
        r.upstream_gone = true;
        let entries = unify(&[r], &head("main"), &[], |_| None);
        assert_eq!(entries[0].state(), BranchState::Gone);
    }

    #[test]
    fn remote_only_without_origin_uses_first_remote() {
        let entries = unify(&[remote("fork", "topic", "a", 1)], &head("main"), &[], |_| None);
        assert_eq!(
            entries[0].checkout_plan(),
            Some(CheckoutPlan::Track {
                name: "topic".into(),
                remote_ref: "fork/topic".into()
            })
        );
    }

    #[test]
    fn activity_uses_reflog_and_checkouts() {
        let refs = vec![
            local("old-tip-recently-checked-out", "a", 10),
            local("old-tip-recently-reset", "b", 20),
            local("newest-commit", "c", 500),
            remote("origin", "pushed-recently", "d", 30),
        ];
        let checkouts = vec![
            Checkout {
                from: "x".into(),
                to: "old-tip-recently-checked-out".into(),
                time: 900,
            },
            Checkout {
                from: "x".into(),
                to: "old-tip-recently-checked-out".into(),
                time: 100,
            },
        ];
        let entries = unify(&refs, &head("newest-commit"), &checkouts, |r| match r {
            "refs/heads/old-tip-recently-reset" => Some(700),
            "refs/remotes/origin/pushed-recently" => Some(600),
            _ => None,
        });
        assert_eq!(
            names(&entries),
            vec![
                "old-tip-recently-checked-out",
                "old-tip-recently-reset",
                "pushed-recently",
                "newest-commit"
            ]
        );
        assert_eq!(entries[0].activity, 900);
    }

    #[test]
    fn ties_sort_by_name() {
        let entries = unify(&[local("b", "x", 5), local("a", "x", 5)], &head("a"), &[], |_| None);
        assert_eq!(names(&entries), vec!["a", "b"]);
    }

    fn symref(remote_name: &str, to: &str) -> GitRef {
        let mut r = remote(remote_name, "HEAD", "a", 0);
        r.symref = Some(format!("refs/remotes/{remote_name}/{to}"));
        r
    }

    #[test]
    fn default_base_follows_remote_head_then_common_names() {
        let refs = vec![
            local("main", "a", 1),
            remote("origin", "main", "a", 1),
            remote("origin", "development", "b", 1),
            symref("origin", "development"),
        ];
        assert_eq!(default_base(&refs).unwrap().short_name, "origin/development");

        let no_head = &refs[..3];
        assert_eq!(default_base(no_head).unwrap().short_name, "origin/main");

        let fork_first = vec![remote("fork", "master", "a", 1), remote("origin", "master", "a", 1)];
        assert_eq!(default_base(&fork_first).unwrap().short_name, "origin/master");

        let local_only = vec![local("topic", "a", 1), local("master", "a", 1)];
        assert_eq!(default_base(&local_only).unwrap().short_name, "master");
        assert!(default_base(&[local("topic", "a", 1)]).is_none());
    }

    #[test]
    fn resolve_base_prefers_choice_that_still_exists() {
        let refs = vec![remote("origin", "main", "a", 1), remote("origin", "release", "b", 1)];
        assert_eq!(
            resolve_base(&refs, Some("origin/release")).unwrap().short_name,
            "origin/release"
        );
        assert_eq!(
            resolve_base(&refs, Some("origin/gone")).unwrap().short_name,
            "origin/main"
        );
        assert_eq!(resolve_base(&refs, None).unwrap().short_name, "origin/main");
    }

    #[test]
    fn compares_remote_branches_with_base() {
        let refs = vec![
            tracking(local("main", "a", 100), "refs/remotes/origin/main", 0, 0),
            local("local-only", "x", 500),
            tracking(local("feat", "f", 300), "refs/remotes/origin/feat", 0, 0),
            remote("origin", "main", "a", 100),
            remote("origin", "feat", "f", 300),
            remote("origin", "old", "o", 50),
            remote("origin", "newer", "n", 400),
            remote("origin", "archive/x", "z", 450),
            remote("upstream", "elsewhere", "e", 600),
        ];
        let entries = unify(&refs, &head("main"), &[], |_| Some(999));
        let base = &refs[3];
        let merged_set: HashSet<String> = ["refs/remotes/origin/main", "refs/remotes/origin/old"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (unmerged, merged) = compare_with_base(&entries, base, &merged_set, &["archive".into()], &[]);
        assert_eq!(names(&unmerged), vec!["newer", "feat"], "newest commit first");
        assert_eq!(names(&merged), vec!["old"], "base itself left out");
        assert_eq!(unmerged[0].activity, 400, "commit date, not reflog activity");
        let feat = &unmerged[1];
        assert!(feat.local.is_some(), "keeps the local side");
        assert_eq!(BaseBranches::tip(feat).unwrap().short_name, "origin/feat");

        let (unmerged, _) = compare_with_base(&entries, base, &HashSet::new(), &[], &[]);
        assert!(names(&unmerged).contains(&"archive/x"), "nothing excluded");

        // Safe branches are left out of both lists, by exact name.
        let safe = ["newer".to_string(), "ol".to_string()];
        let (unmerged, merged) = compare_with_base(&entries, base, &merged_set, &[], &safe);
        assert_eq!(names(&unmerged), vec!["archive/x", "feat"]);
        assert_eq!(names(&merged), vec!["old"], "only exact names match");

        let local_base = &refs[0];
        let (unmerged, merged) = compare_with_base(&entries, local_base, &HashSet::new(), &[], &[]);
        assert_eq!(names(&unmerged), vec!["local-only", "feat"]);
        assert!(merged.is_empty());
        assert_eq!(BaseBranches::tip(&unmerged[1]).unwrap().short_name, "feat");
    }

    #[test]
    fn filter_is_case_insensitive_subsequence() {
        assert!(matches_filter("feature/Login-Page", "flp"));
        assert!(matches_filter("feature/Login-Page", "LOGIN"));
        assert!(matches_filter("main", ""));
        assert!(!matches_filter("main", "mx"));
        assert!(!matches_filter("ab", "ba"));
    }
}
