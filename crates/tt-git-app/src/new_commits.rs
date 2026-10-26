//! New commits on the remote-tracking branches: what a fetch or pull (from
//! tt-git, a plugin or a `git fetch` outside it) brought in, for the
//! desktop app's notifications (`[ui] notifications`).
//!
//! Each refs load's tips are kept as [`Tips`]. When a later load finds a
//! remote-tracking branch moved or new, [`count`] works out how many of its
//! commits no ref had before, and the app hands the front end a
//! [`NewCommits`] through [`App::take_new_commits`](crate::App::take_new_commits).

use tt_git_core::{GitRef, RefKind, Repo};

use crate::action::Action;

/// The ref tips one refs load saw.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tips {
    /// The remote-tracking branches: full name, short name, tip.
    remote: Vec<(String, String, String)>,
    /// Every ref's tip, remote or not.
    all: Vec<String>,
    /// The full name of HEAD's upstream, when it has one.
    head_upstream: Option<String>,
}

impl Tips {
    pub fn of(refs: &[GitRef]) -> Self {
        let remote = refs
            .iter()
            .filter(|r| matches!(r.kind, RefKind::RemoteBranch { .. }) && r.symref.is_none())
            .map(|r| (r.full_name.clone(), r.short_name.clone(), r.target.clone()))
            .collect();
        let mut all: Vec<String> = refs.iter().map(|r| r.target.clone()).collect();
        all.sort();
        all.dedup();
        let head_upstream = refs.iter().find(|r| r.is_head).and_then(|r| r.upstream.clone());
        Self {
            remote,
            all,
            head_upstream,
        }
    }

    /// The remote-tracking branches that moved or appeared since `before`.
    pub fn moved_since(&self, before: &Tips) -> Vec<Moved> {
        self.remote
            .iter()
            .filter(|(full, _, tip)| !before.remote.iter().any(|(f, _, t)| f == full && t == tip))
            .map(|(full, short, tip)| Moved {
                branch: short.clone(),
                tip: tip.clone(),
                head_upstream: self.head_upstream.as_ref() == Some(full),
            })
            .collect()
    }

    /// Every ref's tip.
    pub fn all(&self) -> &[String] {
        &self.all
    }
}

/// A remote-tracking branch that moved or appeared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    /// Its short name (`origin/main`).
    pub branch: String,
    pub tip: String,
    /// It's what the checked-out branch tracks, so a pull would bring the
    /// commits in.
    pub head_upstream: bool,
}

/// The commits that arrived on one remote-tracking branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchCommits {
    /// Its short name (`origin/main`).
    pub branch: String,
    pub count: usize,
    /// See [`Moved::head_upstream`].
    pub head_upstream: bool,
}

/// The commits that arrived since the front end last took them, by branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewCommits {
    pub branches: Vec<BranchCommits>,
}

impl NewCommits {
    /// The commits on all the branches together. A commit on two of them
    /// counts twice.
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
                }
                None => self.branches.push(b),
            }
        }
    }

    /// `"3 new commits on origin/main"`, or with more than one branch
    /// `"5 new commits on origin/main and 1 other branch"`.
    pub fn summary(&self) -> String {
        let total = self.total();
        let commits = if total == 1 { "commit" } else { "commits" };
        match self.branches.as_slice() {
            [] => format!("{total} new {commits}"),
            [one] => format!("{total} new {commits} on {}", one.branch),
            [first, rest @ ..] => {
                let others = if rest.len() == 1 { "branch" } else { "branches" };
                format!(
                    "{total} new {commits} on {} and {} other {others}",
                    first.branch,
                    rest.len()
                )
            }
        }
    }
}

/// Counts the commits on each of `moved` that none of `before` (every ref's
/// tip at the last load) reaches: new to this repository, so a push of
/// your own commits counts none. Branches with none are left out.
pub fn count(repo: &Repo, moved: &[Moved], before: &[String]) -> NewCommits {
    let mut stdin = String::new();
    for tip in before {
        stdin.push('^');
        stdin.push_str(tip);
        stdin.push('\n');
    }
    let branches = moved
        .iter()
        .filter_map(|m| {
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
            (count > 0).then(|| BranchCommits {
                branch: m.branch.clone(),
                count,
                head_upstream: m.head_upstream,
            })
        })
        .collect();
    NewCommits { branches }
}

/// [`count`] as a job.
pub fn count_job(repo: Repo, moved: Vec<Moved>, before: Vec<String>) -> Action {
    Action::NewCommits(count(&repo, &moved, &before))
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

    #[test]
    fn finds_moved_and_new_remote_branches() {
        let before = Tips::of(&[
            remote("origin/main", "a"),
            remote("origin/old", "b"),
            local("main", "a"),
        ]);
        let mut head = local("main", "a");
        head.is_head = true;
        head.upstream = Some("refs/remotes/origin/main".into());
        let after = Tips::of(&[
            remote("origin/main", "c"),
            remote("origin/old", "b"),
            remote("origin/new", "d"),
            head,
        ]);
        assert_eq!(
            after.moved_since(&before),
            vec![
                Moved {
                    branch: "origin/main".into(),
                    tip: "c".into(),
                    head_upstream: true
                },
                Moved {
                    branch: "origin/new".into(),
                    tip: "d".into(),
                    head_upstream: false
                },
            ]
        );
        assert!(after.moved_since(&after).is_empty());
    }

    #[test]
    fn leaves_out_local_branches_and_origin_head() {
        let before = Tips::of(&[local("main", "a")]);
        let mut origin_head = remote("origin/HEAD", "b");
        origin_head.symref = Some("refs/remotes/origin/main".into());
        let after = Tips::of(&[local("main", "b"), origin_head]);
        assert!(after.moved_since(&before).is_empty());
        assert_eq!(after.all(), ["b"]);
    }

    #[test]
    fn adds_up_and_summarizes() {
        let one = |branch: &str, count| BranchCommits {
            branch: branch.into(),
            count,
            head_upstream: false,
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
    }

    #[test]
    fn counts_only_commits_no_ref_had() {
        let sb = Sandbox::new();
        sb.commit("first");
        let url = sb.add_remote("origin");
        let repo = Repo::discover(sb.path(), &GitCli::new(sb.path())).unwrap();
        let tips = |repo: &Repo| Tips::of(&repo.refs_with_authors(&["origin".into()]).unwrap().0);
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
        let moved = after.moved_since(&before);
        assert_eq!(
            count(&repo, &moved, before.all()).branches,
            vec![BranchCommits {
                branch: "origin/main".into(),
                count: 2,
                head_upstream: false,
            }]
        );

        // Pushing your own commit brings none.
        sb.git(&["merge", "--ff-only", "origin/main"]);
        let before = tips(&repo);
        sb.commit("mine");
        let before_push = tips(&repo);
        sb.git(&["push", "origin", "main"]);
        let after = tips(&repo);
        let moved = after.moved_since(&before_push);
        assert_eq!(moved.len(), 1);
        assert!(count(&repo, &moved, before_push.all()).branches.is_empty());
        assert_eq!(
            count(&repo, &moved, before.all()).total(),
            1,
            "against the tips before the commit"
        );
    }
}
