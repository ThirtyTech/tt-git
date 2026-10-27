//! New commits on the branches: what a fetch or pull (from tt-git, a
//! plugin or a `git fetch` outside it) brought to the remote-tracking
//! branches, and what was committed to the local branches from outside this
//! window (another worktree, a terminal, an agent), for the desktop app's
//! notifications (`[ui] notifications` and `[ui.alerts]`).
//!
//! Each refs load's tips are kept as [`Tips`]. When a later load finds a
//! branch moved or new, [`count`] works out how many of its commits no ref
//! had before, and the app hands the front end a [`NewCommits`] through
//! [`App::take_new_commits`](crate::App::take_new_commits), a busy local
//! branch at most once a minute ([`Pending`]).

use std::collections::HashMap;
use std::path::Path;

use tt_git_core::{GitRef, RefKind, Repo, Worktree};

use crate::action::Action;

/// One branch's tip in a refs load.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BranchTip {
    full: String,
    short: String,
    tip: String,
    local: bool,
}

/// The ref tips one refs load saw.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tips {
    /// The local and remote-tracking branches.
    branches: Vec<BranchTip>,
    /// Every ref's tip, branch or not.
    all: Vec<String>,
    /// The full name of HEAD's upstream, when it has one.
    head_upstream: Option<String>,
    /// The full name of the branch checked out in this worktree.
    head_branch: Option<String>,
    /// The local branches checked out in other worktrees, by short name,
    /// with the worktree as [`worktree_label`] names it.
    worktrees: Vec<(String, String)>,
}

/// Which branches [`Tips::moved_since`] looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Watch {
    pub remote: bool,
    pub local: bool,
    /// Leave out the branch checked out in this worktree: this window moved
    /// it (a commit, rebase or reset of its own).
    pub skip_head: bool,
}

impl Tips {
    /// The tips of `refs`, with `worktrees` saying where each local branch
    /// is checked out.
    pub fn of(refs: &[GitRef], worktrees: &[Worktree]) -> Self {
        let branches = refs
            .iter()
            .filter(|r| r.symref.is_none())
            .filter_map(|r| {
                let local = match r.kind {
                    RefKind::LocalBranch => true,
                    RefKind::RemoteBranch { .. } => false,
                    _ => return None,
                };
                Some(BranchTip {
                    full: r.full_name.clone(),
                    short: r.short_name.clone(),
                    tip: r.target.clone(),
                    local,
                })
            })
            .collect();
        let mut all: Vec<String> = refs.iter().map(|r| r.target.clone()).collect();
        all.sort();
        all.dedup();
        let head = refs.iter().find(|r| r.is_head);
        let main = worktrees.iter().find(|w| w.main).map(|w| w.path.as_path());
        let worktrees = worktrees
            .iter()
            .filter(|w| !w.current && !w.bare)
            .filter_map(|w| Some((w.branch.clone()?, worktree_label(&w.path, main))))
            .collect();
        Self {
            branches,
            all,
            head_upstream: head.and_then(|r| r.upstream.clone()),
            head_branch: head.map(|r| r.full_name.clone()),
            worktrees,
        }
    }

    /// The branches `watch` looks at that moved or appeared since
    /// `before`, the local ones first.
    pub fn moved_since(&self, before: &Tips, watch: Watch) -> Vec<Moved> {
        let mut moved: Vec<Moved> = self
            .branches
            .iter()
            .filter(|b| if b.local { watch.local } else { watch.remote })
            .filter(|b| !(watch.skip_head && b.local && self.head_branch.as_ref() == Some(&b.full)))
            .filter(|b| !before.branches.iter().any(|o| o.full == b.full && o.tip == b.tip))
            .map(|b| Moved {
                branch: b.short.clone(),
                tip: b.tip.clone(),
                head_upstream: self.head_upstream.as_ref() == Some(&b.full),
                local: b.local,
                worktree: b
                    .local
                    .then(|| self.worktrees.iter().find(|(name, _)| *name == b.short))
                    .flatten()
                    .map(|(_, w)| w.clone()),
            })
            .collect();
        moved.sort_by_key(|m| !m.local);
        moved
    }

    /// Every ref's tip.
    pub fn all(&self) -> &[String] {
        &self.all
    }
}

/// How a notification names the worktree at `path`: relative to the main
/// working tree `main` when it's inside it (`.worktrees/feature`), else its
/// folder's name.
pub fn worktree_label(path: &Path, main: Option<&Path>) -> String {
    if let Some(rel) = main.and_then(|m| path.strip_prefix(m).ok())
        && !rel.as_os_str().is_empty()
    {
        return rel.to_string_lossy().replace('\\', "/");
    }
    path.file_name()
        .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// A branch that moved or appeared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    /// Its short name (`origin/main`, `feature`).
    pub branch: String,
    pub tip: String,
    /// It's what the checked-out branch tracks, so a pull would bring the
    /// commits in.
    pub head_upstream: bool,
    /// A local branch rather than a remote-tracking one.
    pub local: bool,
    /// The other worktree it's checked out in, if any.
    pub worktree: Option<String>,
}

/// The commits that arrived on one branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchCommits {
    /// Its short name (`origin/main`, `feature`).
    pub branch: String,
    pub count: usize,
    /// See [`Moved::head_upstream`].
    pub head_upstream: bool,
    /// See [`Moved::local`].
    pub local: bool,
    /// See [`Moved::worktree`].
    pub worktree: Option<String>,
}

/// The commits that arrived since the front end last took them, by branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewCommits {
    pub branches: Vec<BranchCommits>,
}

impl NewCommits {
    /// The commits on all the branches together.
    pub fn total(&self) -> usize {
        self.branches.iter().map(|b| b.count).sum()
    }

    /// Adds `more` to these, a branch already here getting its count added.
    pub fn add(&mut self, more: NewCommits) {
        for b in more.branches {
            match self.branches.iter_mut().find(|have| have.branch == b.branch) {
                Some(have) => {
                    have.count += b.count;
                    have.head_upstream = b.head_upstream;
                    have.worktree = b.worktree;
                }
                None => self.branches.push(b),
            }
        }
    }

    /// `"3 new commits on origin/main"`, `"2 new commits on feature
    /// (worktree .worktrees/feature)"`, or with more than one branch
    /// `"5 new commits on origin/main and 1 other branch"`.
    pub fn summary(&self) -> String {
        let total = self.total();
        let commits = if total == 1 { "commit" } else { "commits" };
        let on = |b: &BranchCommits| match &b.worktree {
            Some(w) => format!("{} (worktree {w})", b.branch),
            None => b.branch.clone(),
        };
        match self.branches.as_slice() {
            [] => format!("{total} new {commits}"),
            [one] => format!("{total} new {commits} on {}", on(one)),
            [first, rest @ ..] => {
                let others = if rest.len() == 1 { "branch" } else { "branches" };
                format!(
                    "{total} new {commits} on {} and {} other {others}",
                    on(first),
                    rest.len()
                )
            }
        }
    }
}

/// Counts the commits on each of `moved` that none of `before` (every ref's
/// tip at the last load) reaches: new to this repository, so a push of
/// your own commits counts none. A commit is counted on the first of
/// `moved` that has it only, so one committed and pushed between two loads
/// counts once. Branches with none are left out.
pub fn count(repo: &Repo, moved: &[Moved], before: &[String]) -> NewCommits {
    let mut stdin = String::new();
    for tip in before {
        stdin.push('^');
        stdin.push_str(tip);
        stdin.push('\n');
    }
    let mut branches = Vec::new();
    for m in moved {
        let input = format!("{}\n{stdin}", m.tip);
        let out = repo.git().run_with_stdin(
            &["rev-list", "--count", "--ignore-missing", "--stdin"],
            Some(input.as_bytes()),
        );
        let count = match out {
            Ok(out) => out.stdout_str().trim().parse().unwrap_or(0),
            Err(e) => {
                tracing::warn!("counting the new commits on {} failed: {e}", m.branch);
                0
            }
        };
        stdin.push('^');
        stdin.push_str(&m.tip);
        stdin.push('\n');
        if count > 0 {
            branches.push(BranchCommits {
                branch: m.branch.clone(),
                count,
                head_upstream: m.head_upstream,
                local: m.local,
                worktree: m.worktree.clone(),
            });
        }
    }
    NewCommits { branches }
}

/// [`count`] as a job.
pub fn count_job(repo: Repo, moved: Vec<Moved>, before: Vec<String>) -> Action {
    Action::NewCommits(count(&repo, &moved, &before))
}

/// The new commits waiting for the front end. An agent can commit to a
/// branch every few seconds, so a local branch that was handed over less
/// than `hold` ago keeps its next commits back until `hold` has passed and
/// hands them over together. Remote-tracking branches only move when
/// something fetches, so they're never held.
#[derive(Debug, Default)]
pub struct Pending {
    ready: Option<NewCommits>,
    held: NewCommits,
    /// When each local branch was last handed over, in ticks.
    last: HashMap<String, u64>,
}

impl Pending {
    /// `new` arrived at tick `now`.
    pub fn arrived(&mut self, new: NewCommits, now: u64, hold: u64) {
        for b in new.branches {
            let recent = self
                .last
                .get(&b.branch)
                .is_some_and(|at| now.saturating_sub(*at) < hold);
            if b.local && recent {
                self.held.add(NewCommits { branches: vec![b] });
                continue;
            }
            if b.local {
                self.last.insert(b.branch.clone(), now);
            }
            self.ready.get_or_insert_default().add(NewCommits { branches: vec![b] });
        }
    }

    /// At tick `now`: the held branches whose `hold` is up are ready.
    pub fn release(&mut self, now: u64, hold: u64) {
        if self.held.branches.is_empty() {
            self.last.retain(|_, at| now.saturating_sub(*at) < hold);
            return;
        }
        let (due, held): (Vec<_>, Vec<_>) = std::mem::take(&mut self.held.branches).into_iter().partition(|b| {
            self.last
                .get(&b.branch)
                .is_none_or(|at| now.saturating_sub(*at) >= hold)
        });
        self.held.branches = held;
        for b in &due {
            self.last.insert(b.branch.clone(), now);
        }
        if !due.is_empty() {
            self.ready.get_or_insert_default().add(NewCommits { branches: due });
        }
    }

    /// The commits ready to hand over.
    pub fn take(&mut self) -> Option<NewCommits> {
        self.ready.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_git_core::GitCli;
    use tt_git_testkit::Sandbox;

    fn remote(short: &str, target: &str) -> GitRef {
        let (remote, _) = short.split_once('/').unwrap();
        GitRef {
            full_name: format!("refs/remotes/{short}"),
            short_name: short.into(),
            kind: RefKind::RemoteBranch { remote: remote.into() },
            target: target.into(),
            ..local("x", target)
        }
    }

    fn local(name: &str, target: &str) -> GitRef {
        GitRef {
            full_name: format!("refs/heads/{name}"),
            short_name: name.into(),
            kind: RefKind::LocalBranch,
            target: target.into(),
            date: 0,
            upstream: None,
            ahead: 0,
            behind: 0,
            upstream_gone: false,
            is_head: false,
            symref: None,
            subject: String::new(),
        }
    }

    const ALL: Watch = Watch {
        remote: true,
        local: true,
        skip_head: false,
    };
    const REMOTE: Watch = Watch {
        remote: true,
        local: false,
        skip_head: false,
    };

    fn remote_moved(branch: &str, tip: &str, head_upstream: bool) -> Moved {
        Moved {
            branch: branch.into(),
            tip: tip.into(),
            head_upstream,
            local: false,
            worktree: None,
        }
    }

    fn worktree(path: &str, branch: &str, main: bool, current: bool) -> Worktree {
        Worktree {
            path: path.into(),
            head: None,
            branch: Some(branch.into()),
            main,
            bare: false,
            locked: None,
            prunable: None,
            current,
        }
    }

    #[test]
    fn finds_moved_and_new_remote_branches() {
        let before = Tips::of(
            &[
                remote("origin/main", "a"),
                remote("origin/old", "b"),
                local("main", "a"),
            ],
            &[],
        );
        let mut head = local("main", "a");
        head.is_head = true;
        head.upstream = Some("refs/remotes/origin/main".into());
        let after = Tips::of(
            &[
                remote("origin/main", "c"),
                remote("origin/old", "b"),
                remote("origin/new", "d"),
                head,
            ],
            &[],
        );
        assert_eq!(
            after.moved_since(&before, REMOTE),
            vec![
                remote_moved("origin/main", "c", true),
                remote_moved("origin/new", "d", false)
            ]
        );
        assert!(after.moved_since(&after, ALL).is_empty());
    }

    #[test]
    fn leaves_out_origin_head() {
        let before = Tips::of(&[local("main", "a")], &[]);
        let mut origin_head = remote("origin/HEAD", "b");
        origin_head.symref = Some("refs/remotes/origin/main".into());
        let after = Tips::of(&[local("main", "a"), origin_head], &[]);
        assert!(after.moved_since(&before, ALL).is_empty());
        assert_eq!(after.all(), ["a", "b"]);
    }

    #[test]
    fn finds_local_branches_first_naming_their_worktree() {
        let worktrees = [
            worktree("/src/web", "main", true, true),
            worktree("/src/web/.worktrees/feature", "feature", false, false),
            worktree("/elsewhere/fix", "fix", false, false),
        ];
        let before = Tips::of(
            &[remote("origin/main", "a"), local("main", "a"), local("feature", "a")],
            &worktrees,
        );
        let mut head = local("main", "b");
        head.is_head = true;
        let after = Tips::of(
            &[
                remote("origin/main", "c"),
                head,
                local("feature", "d"),
                local("fix", "e"),
            ],
            &worktrees,
        );
        let moved = after.moved_since(&before, ALL);
        let names: Vec<(&str, Option<&str>)> = moved
            .iter()
            .map(|m| (m.branch.as_str(), m.worktree.as_deref()))
            .collect();
        assert_eq!(
            names,
            [
                ("main", None),
                ("feature", Some(".worktrees/feature")),
                ("fix", Some("fix")),
                ("origin/main", None)
            ]
        );
        assert!(moved[..3].iter().all(|m| m.local));

        let own = Watch { skip_head: true, ..ALL };
        let moved = after.moved_since(&before, own);
        assert!(!moved.iter().any(|m| m.branch == "main"), "this window moved it");
        let local_only = Watch { remote: false, ..ALL };
        assert!(after.moved_since(&before, local_only).iter().all(|m| m.local));
    }

    #[test]
    fn adds_up_and_summarizes() {
        let one = |branch: &str, count| BranchCommits {
            branch: branch.into(),
            count,
            head_upstream: false,
            local: false,
            worktree: None,
        };
        let mut new = NewCommits {
            branches: vec![one("origin/main", 1)],
        };
        assert_eq!(new.summary(), "1 new commit on origin/main");
        new.add(NewCommits {
            branches: vec![one("origin/main", 2)],
        });
        assert_eq!(new.summary(), "3 new commits on origin/main");
        new.add(NewCommits {
            branches: vec![one("origin/a", 1), one("origin/b", 1)],
        });
        assert_eq!(new.total(), 5);
        assert_eq!(new.summary(), "5 new commits on origin/main and 2 other branches");

        let in_worktree = NewCommits {
            branches: vec![BranchCommits {
                local: true,
                worktree: Some(".worktrees/feature".into()),
                ..one("feature", 2)
            }],
        };
        assert_eq!(
            in_worktree.summary(),
            "2 new commits on feature (worktree .worktrees/feature)"
        );
    }

    #[test]
    fn holds_a_busy_local_branch_for_a_while() {
        let local = |branch: &str, count| NewCommits {
            branches: vec![BranchCommits {
                branch: branch.into(),
                count,
                head_upstream: false,
                local: true,
                worktree: None,
            }],
        };
        let remote = |count| NewCommits {
            branches: vec![BranchCommits {
                local: false,
                ..local("origin/main", count).branches.remove(0)
            }],
        };
        let mut pending = Pending::default();
        pending.arrived(local("agent", 1), 0, 10);
        assert_eq!(pending.take().map(|n| n.total()), Some(1), "the first goes at once");
        pending.arrived(local("agent", 1), 3, 10);
        pending.arrived(local("agent", 2), 6, 10);
        pending.arrived(remote(4), 6, 10);
        pending.arrived(local("other", 1), 6, 10);
        let ready = pending.take().unwrap();
        assert_eq!(ready.summary(), "5 new commits on origin/main and 1 other branch");
        pending.release(9, 10);
        assert_eq!(pending.take(), None, "still held");
        pending.release(10, 10);
        assert_eq!(pending.take(), Some(local("agent", 3)), "together once the hold is up");
        pending.arrived(local("agent", 1), 12, 10);
        assert_eq!(pending.take(), None, "held again");
        pending.release(30, 10);
        pending.release(31, 10);
        assert_eq!(pending.take(), Some(local("agent", 1)));
    }

    #[test]
    fn counts_only_commits_no_ref_had() {
        let sb = Sandbox::new();
        sb.commit("first");
        let url = sb.add_remote("origin");
        let repo = Repo::discover(sb.path(), &GitCli::new(sb.path())).unwrap();
        let tips = |repo: &Repo| Tips::of(&repo.refs_with_authors(&["origin".into()]).unwrap().0, &[]);
        let before = tips(&repo);

        // Someone else pushes two commits; fetching brings them in.
        let other = sb.clone_to(&url, "other");
        for msg in ["second", "third"] {
            std::fs::write(other.join(msg), msg).unwrap();
            sb.git_in(&other, &["add", "."]);
            sb.git_in(&other, &["commit", "-m", msg]);
        }
        sb.git_in(&other, &["push", "origin", "main"]);
        sb.git(&["fetch", "origin"]);
        let after = tips(&repo);
        let moved = after.moved_since(&before, ALL);
        assert_eq!(
            count(&repo, &moved, before.all()).branches,
            vec![BranchCommits {
                branch: "origin/main".into(),
                count: 2,
                head_upstream: false,
                local: false,
                worktree: None,
            }]
        );

        // Pushing your own commit brings none.
        sb.git(&["merge", "--ff-only", "origin/main"]);
        let before = tips(&repo);
        sb.commit("mine");
        let before_push = tips(&repo);
        sb.git(&["push", "origin", "main"]);
        let after = tips(&repo);
        let moved = after.moved_since(&before_push, ALL);
        assert_eq!(moved.len(), 1);
        assert!(count(&repo, &moved, before_push.all()).branches.is_empty());

        // Committed and pushed between two loads, it counts once, on the
        // local branch.
        let moved = after.moved_since(&before, ALL);
        assert_eq!(moved.len(), 2);
        let new = count(&repo, &moved, before.all());
        assert_eq!(new.total(), 1, "{new:?}");
        assert_eq!(new.branches[0].branch, "main");
    }
}
