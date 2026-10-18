//! The unified branch list: local and remote-tracking branches merged into
//! one entry per branch and ordered by most recent activity.
//!
//! Grouping rules:
//! 1. Every local branch is an entry.
//! 2. A remote branch joins the local branch that has it as its configured
//!    upstream (so local `x` tracking `origin/feature-x` is one entry).
//! 3. Otherwise it joins the local branch with the same name, after
//!    stripping `<remote>/`.
//! 4. Remaining remote branches are grouped by name into remote-only
//!    entries (`origin/bar` and `upstream/bar` → `bar`).
//!
//! Activity is the latest of: any member's tip commit date, any member's
//! last reflog update (commits, resets, fetches), and the last time HEAD was
//! checked out to the local branch.

use std::collections::HashMap;

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

/// What checking out an entry does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckoutPlan {
    /// `git switch <name>`
    Local(String),
    /// `git switch -c <name> --track <remote_ref>`; `remote_ref` is the short
    /// name like `origin/feature`.
    Track { name: String, remote_ref: String },
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
        if let Some(up) = &local.upstream
            && self.remotes.iter().any(|r| &r.full_name == up)
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

    fn preferred_remote(&self) -> Option<&GitRef> {
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
        let idx = match by_upstream.get(&r.full_name).or_else(|| by_name.get(&name)) {
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
            remote("origin", "fetched-recently", "d", 30),
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
            "refs/remotes/origin/fetched-recently" => Some(600),
            _ => None,
        });
        assert_eq!(
            names(&entries),
            vec![
                "old-tip-recently-checked-out",
                "old-tip-recently-reset",
                "fetched-recently",
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

    #[test]
    fn filter_is_case_insensitive_subsequence() {
        assert!(matches_filter("feature/Login-Page", "flp"));
        assert!(matches_filter("feature/Login-Page", "LOGIN"));
        assert!(matches_filter("main", ""));
        assert!(!matches_filter("main", "mx"));
        assert!(!matches_filter("ab", "ba"));
    }
}
