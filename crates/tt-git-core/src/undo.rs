//! Undo: stepping back through HEAD's reflog.
//!
//! Each step tt-git takes is written to the reflog with [`UNDO_MARK`]
//! through `GIT_REFLOG_ACTION`, so walking the reflog newest first can count
//! how many of the user's own actions are already undone. The counting is
//! lazygit's: an undo marker adds one, and every action passed takes one
//! away.

use crate::model::InProgress;

/// Starts the reflog message of every undo step.
pub const UNDO_MARK: &str = "[tt-git undo]";

/// How many reflog entries are read looking for a step.
pub const REFLOG_LIMIT: usize = 500;

/// One entry of HEAD's reflog: the commit HEAD moved to and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflogEntry {
    pub oid: String,
    pub subject: String,
}

/// Where a step puts HEAD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoTarget {
    /// Switch to this local branch.
    Branch(String),
    /// Detach HEAD at this commit.
    Detach(String),
    /// Move the current branch (or detached HEAD) here with `reset --keep`.
    Reset(String),
    /// Move the current branch (or detached HEAD) here with `reset --soft`,
    /// leaving the index and working tree alone: undoing a commit puts its
    /// changes back, staged.
    SoftReset(String),
    /// Undo a rebase of `branch` started while HEAD was elsewhere: switch
    /// back to `back` (a [`Branch`](Self::Branch) or [`Detach`](Self::Detach)),
    /// then move `branch` from `rebased` back to `before`. With `before`
    /// unknown, `branch` is left where it is.
    LeaveRebase {
        branch: String,
        before: Option<String>,
        rebased: String,
        back: Box<UndoTarget>,
    },
}

impl UndoTarget {
    /// Whether the step switches branches rather than moving the current one.
    pub fn is_switch(&self) -> bool {
        !matches!(self, UndoTarget::Reset(_) | UndoTarget::SoftReset(_))
    }

    /// Whether the step rewrites files in the working tree, and so has to
    /// ask about uncommitted changes first. A soft reset never does.
    pub fn touches_worktree(&self) -> bool {
        !matches!(self, UndoTarget::SoftReset(_))
    }

    /// The target for messages: a branch name or a short commit.
    pub fn display(&self) -> String {
        match self {
            UndoTarget::Branch(name) => name.clone(),
            UndoTarget::Detach(oid) | UndoTarget::Reset(oid) | UndoTarget::SoftReset(oid) => short(oid).to_string(),
            UndoTarget::LeaveRebase { back, .. } => back.display(),
        }
    }
}

/// What working out a step needs to know about the repository as it is now.
pub trait RepoRefs {
    /// Whether `name` is a local branch.
    fn is_branch(&self, name: &str) -> bool;
    /// Where `branch` was before the rebase that left it at `rebased`,
    /// from the branch's own reflog; `None` if that can't be found.
    fn before_rebase(&self, branch: &str, rebased: &str) -> Option<String>;
}

/// An undo worked out from the reflog, ready to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoStep {
    /// What is undone, e.g. `commit: fix typo` or
    /// `checkout of main`.
    pub what: String,
    pub target: UndoTarget,
    /// HEAD's commit when the step was worked out. The step isn't taken if
    /// HEAD has moved since.
    pub head: String,
    /// HEAD's branch, or its commit when detached, for the reflog message
    /// of a switch.
    pub current: String,
}

impl UndoStep {
    /// `undo commit: fix typo`
    pub fn label(&self) -> String {
        format!("undo {}", self.what)
    }
}

/// Why there's no step to take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoStep {
    /// Nothing left to undo.
    Nothing,
    /// A merge, rebase, cherry-pick or revert is in progress.
    InProgress(InProgress),
    /// The step would undo the repository's first commit.
    FirstCommit,
    /// There's no commit yet.
    Unborn,
}

impl NoStep {
    pub fn message(&self) -> String {
        match self {
            NoStep::Nothing => "Nothing to undo".into(),
            NoStep::InProgress(op) => format!(
                "A {} is in progress. Continue or abort it first (C / X in the status pane).",
                op.command()
            ),
            NoStep::FirstCommit => "Can't undo the repository's first commit".into(),
            NoStep::Unborn => "Nothing to undo: there are no commits yet".into(),
        }
    }

    /// Whether this is worth only a passing notice rather than an error.
    pub fn is_notice(&self) -> bool {
        matches!(self, NoStep::Nothing | NoStep::Unborn)
    }
}

/// What a reflog subject says happened.
#[derive(Debug, PartialEq, Eq)]
enum Kind<'a> {
    Undo,
    /// `rebase (finish)` or `(abort)`, also from `pull --rebase`, with the
    /// branch it returned to (`returning to refs/heads/<branch>`).
    RebaseEnd {
        branch: Option<&'a str>,
    },
    /// `rebase (start): checkout <onto>`.
    RebaseStart {
        onto: Option<&'a str>,
    },
    /// Any other `rebase (…)` entry: a pick, a continue.
    RebaseStep,
    Checkout {
        from: &'a str,
        to: &'a str,
    },
    /// HEAD's branch moved: commit, reset, merge, pull, cherry-pick, revert.
    Move,
    Other,
}

fn classify(subject: &str) -> Kind<'_> {
    if subject.starts_with(UNDO_MARK) {
        return Kind::Undo;
    }
    classify_action(subject)
}

/// [`classify`] without looking for the undo marker.
fn classify_action(subject: &str) -> Kind<'_> {
    let (head, rest) = subject.split_once(": ").unwrap_or((subject, ""));
    let rebasing = head.starts_with("rebase") || head.starts_with("pull");
    if rebasing && let Some(step) = head.strip_suffix(')').and_then(|h| h.rsplit_once(" (")).map(|(_, s)| s) {
        return match step {
            "finish" | "abort" => Kind::RebaseEnd {
                branch: rest.strip_prefix("returning to refs/heads/").filter(|s| !s.is_empty()),
            },
            "start" => Kind::RebaseStart {
                onto: rest.strip_prefix("checkout ").filter(|s| !s.is_empty()),
            },
            _ => Kind::RebaseStep,
        };
    }
    if head == "checkout"
        && let Some((from, to)) = rest.strip_prefix("moving from ").and_then(|r| r.split_once(" to "))
    {
        return Kind::Checkout { from, to };
    }
    let verb = head.split([' ', '(']).next().unwrap_or("");
    match verb {
        "commit" | "reset" | "merge" | "pull" | "cherry-pick" | "revert" => Kind::Move,
        _ => Kind::Other,
    }
}

/// The branch HEAD was on at `entries[at]` (newest first), going by the
/// newest checkout, finished rebase or rename of the current branch at or
/// before it. A detached HEAD gives the commit or revision it was checked
/// out by; `None` if the reflog doesn't say.
fn branch_at(entries: &[ReflogEntry], at: usize) -> Option<&str> {
    entries.iter().skip(at).find_map(|entry| {
        // Undo writes its checkouts behind the marker.
        let subject = entry
            .subject
            .strip_prefix(UNDO_MARK)
            .and_then(|s| s.strip_prefix(' '))
            .unwrap_or(&entry.subject);
        // `git branch -m` writes this to HEAD's reflog only for the
        // checked-out branch.
        if let Some(rest) = subject.strip_prefix("Branch: renamed refs/heads/") {
            return rest.split_once(" to refs/heads/").map(|(_, new)| new);
        }
        match classify_action(subject) {
            Kind::Checkout { to, .. } => Some(to),
            Kind::RebaseEnd { branch: Some(branch) } => Some(branch),
            _ => None,
        }
    })
}

/// One of the user's actions found in the reflog.
struct Action<'a> {
    /// HEAD's commit before it, if the reflog goes back that far.
    from: Option<&'a str>,
    /// HEAD's commit after it.
    to: &'a str,
    change: Change<'a>,
}

#[derive(Clone, Copy)]
enum Change<'a> {
    Checkout {
        from: &'a str,
        to: &'a str,
    },
    /// `subject` is the reflog subject.
    Move {
        subject: &'a str,
    },
    /// `branch` is the branch rebased, and `started_on` the branch (or
    /// commit) HEAD was on when the rebase started, when the reflog says.
    Rebase {
        onto: Option<&'a str>,
        branch: Option<&'a str>,
        started_on: Option<&'a str>,
    },
}

/// A rebase of a branch HEAD wasn't on when it started
/// (`git rebase <upstream> <branch>`).
struct Elsewhere<'a> {
    branch: &'a str,
    /// The branch (or commit) HEAD was on, if the reflog says.
    started_on: Option<&'a str>,
    /// Where `branch` was before the rebase, if its reflog says.
    before: Option<String>,
}

impl<'a> Action<'a> {
    /// Whether this is a rebase of a branch HEAD wasn't on. The branch's
    /// own reflog decides when it can: HEAD was on the branch if the branch
    /// started where HEAD did. Otherwise it's where HEAD's reflog last saw
    /// it.
    fn elsewhere(&self, refs: &impl RepoRefs) -> Option<Elsewhere<'a>> {
        let Change::Rebase {
            branch: Some(branch),
            started_on,
            ..
        } = self.change
        else {
            return None;
        };
        if started_on == Some(branch) {
            return None;
        }
        let before = refs.before_rebase(branch, self.to);
        if before.is_some() && before.as_deref() == self.from {
            return None;
        }
        Some(Elsewhere {
            branch,
            started_on,
            before,
        })
    }

    fn what(&self, elsewhere: bool) -> String {
        match &self.change {
            Change::Checkout { to, .. } => format!("checkout of {}", short_name(to)),
            Change::Move { subject } => subject.to_string(),
            Change::Rebase { onto, branch, .. } => {
                let mut what = "rebase".to_string();
                if let Some(branch) = branch.filter(|_| elsewhere) {
                    what.push_str(" of ");
                    what.push_str(branch);
                }
                if let Some(onto) = onto {
                    what.push_str(" onto ");
                    what.push_str(short_name(onto));
                }
                what
            }
        }
    }
}

/// The next undo step, from HEAD's reflog `entries` (newest first). A
/// checkout goes back to a branch by name if it's still a local branch
/// (`refs`), else to the commit it was on, detached. `head` is HEAD's
/// commit and `current` its branch (or commit when detached).
pub fn plan(entries: &[ReflogEntry], refs: &impl RepoRefs, head: &str, current: &str) -> Result<UndoStep, NoStep> {
    let mut counter: i64 = 0;
    // The commit and branch a rebase finished on, while looking for where
    // it started.
    let mut rebase_end: Option<(&str, Option<&str>)> = None;
    for (i, entry) in entries.iter().enumerate() {
        let from = entries.get(i + 1).map(|e| e.oid.as_str());
        let kind = classify(&entry.subject);
        let change = match (rebase_end, kind) {
            (Some((_, branch)), Kind::RebaseStart { onto }) => Change::Rebase {
                onto,
                branch,
                started_on: branch_at(entries, i + 1),
            },
            (Some(_), _) => continue,
            (None, Kind::Undo) => {
                counter += 1;
                continue;
            }
            (None, Kind::RebaseEnd { branch }) => {
                rebase_end = Some((&entry.oid, branch));
                continue;
            }
            (None, Kind::Checkout { from, to }) => Change::Checkout { from, to },
            (None, Kind::Move) => Change::Move {
                subject: &entry.subject,
            },
            // A start with no finish is a rebase that was never finished
            // or aborted; the repo isn't rebasing now, or we wouldn't be here.
            (None, Kind::RebaseStart { .. } | Kind::RebaseStep | Kind::Other) => continue,
        };
        let to = rebase_end.take().map_or(entry.oid.as_str(), |(oid, _)| oid);
        let action = Action { from, to, change };
        // A checkout counts if it changed branch, even to the same commit.
        let no_op = match action.change {
            Change::Checkout { from, to } => from == to,
            _ => action.from == Some(action.to),
        };
        if no_op {
            continue;
        }
        if counter == 0 {
            let elsewhere = action.elsewhere(refs);
            let what = action.what(elsewhere.is_some());
            let target = undo_target(&action, elsewhere, refs).ok_or_else(|| {
                if entry.subject.starts_with("commit (initial)") {
                    NoStep::FirstCommit
                } else {
                    NoStep::Nothing
                }
            })?;
            return Ok(UndoStep {
                what,
                target,
                head: head.to_string(),
                current: current.to_string(),
            });
        }
        counter -= 1;
    }
    Err(NoStep::Nothing)
}

/// Where undoing `action` puts HEAD; `None` when the reflog doesn't go
/// back far enough to say.
fn undo_target(action: &Action<'_>, elsewhere: Option<Elsewhere<'_>>, refs: &impl RepoRefs) -> Option<UndoTarget> {
    let back = |name: &str, oid: Option<&str>| {
        if refs.is_branch(name) {
            Some(UndoTarget::Branch(name.to_string()))
        } else {
            oid.map(|oid| UndoTarget::Detach(oid.to_string()))
        }
    };
    if let Some(Elsewhere {
        branch,
        started_on,
        before,
    }) = elsewhere
    {
        let back = match started_on {
            Some(name) => back(name, action.from)?,
            None => UndoTarget::Detach(action.from?.to_string()),
        };
        return Some(UndoTarget::LeaveRebase {
            branch: branch.to_string(),
            before,
            rebased: action.to.to_string(),
            back: Box::new(back),
        });
    }
    match action.change {
        Change::Checkout { from: name, .. } => back(name, action.from),
        Change::Move { subject } if is_own_commit(subject) => {
            action.from.map(|from| UndoTarget::SoftReset(from.to_string()))
        }
        _ => action.from.map(|from| UndoTarget::Reset(from.to_string())),
    }
}

/// A commit or amend you made, as opposed to one a merge, pull,
/// cherry-pick or revert made: undoing it gives its changes back.
fn is_own_commit(subject: &str) -> bool {
    subject.starts_with("commit: ") || subject.starts_with("commit (amend): ")
}

fn short(oid: &str) -> &str {
    &oid[..oid.len().min(7)]
}

/// A checkout's from/to for display: full commit ids shortened.
fn short_name(name: &str) -> &str {
    if name.len() == 40 && name.bytes().all(|b| b.is_ascii_hexdigit()) {
        short(name)
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reflog entries from `(oid, subject)` pairs, written oldest first
    /// like a story and returned newest first like git prints them.
    fn log(story: &[(&str, &str)]) -> Vec<ReflogEntry> {
        story
            .iter()
            .rev()
            .map(|(oid, subject)| ReflogEntry {
                oid: oid.to_string(),
                subject: subject.to_string(),
            })
            .collect()
    }

    /// `main` and `feat` are branches, and the branch reflogs know where
    /// two rebases started.
    struct Refs;

    impl RepoRefs for Refs {
        fn is_branch(&self, name: &str) -> bool {
            matches!(name, "main" | "feat")
        }

        fn before_rebase(&self, branch: &str, rebased: &str) -> Option<String> {
            match (branch, rebased) {
                ("feat", "h8") => Some("f6".into()),
                ("main", "y0") => Some("c3".into()),
                _ => None,
            }
        }
    }

    fn step(entries: &[ReflogEntry]) -> Result<UndoTarget, NoStep> {
        plan(entries, &Refs, &entries[0].oid, "main").map(|s| s.target)
    }

    fn reset(oid: &str) -> Result<UndoTarget, NoStep> {
        Ok(UndoTarget::Reset(oid.into()))
    }

    /// A commit is undone softly, keeping its changes.
    fn soft(oid: &str) -> Result<UndoTarget, NoStep> {
        Ok(UndoTarget::SoftReset(oid.into()))
    }

    const BASE: &[(&str, &str)] = &[
        ("a1", "commit (initial): one"),
        ("b2", "commit: two"),
        ("c3", "commit: three"),
    ];

    #[test]
    fn undo_steps_back_through_commits() {
        let mut story = BASE.to_vec();
        assert_eq!(step(&log(&story)), soft("b2"));
        story.push(("b2", "[tt-git undo]: updating HEAD"));
        assert_eq!(step(&log(&story)), soft("a1"));
        story.push(("a1", "[tt-git undo]: updating HEAD"));
        assert_eq!(step(&log(&story)), Err(NoStep::FirstCommit));
    }

    #[test]
    fn a_new_action_after_an_undo_is_undone_first() {
        let mut story = BASE.to_vec();
        story.push(("b2", "[tt-git undo]: updating HEAD"));
        story.push(("d4", "commit: four"));
        assert_eq!(step(&log(&story)), soft("b2"));
        story.push(("b2", "[tt-git undo]: updating HEAD"));
        assert_eq!(step(&log(&story)), soft("a1"));
    }

    #[test]
    fn checkouts_go_back_by_name_or_commit() {
        let mut story = BASE.to_vec();
        story.push(("e5", "checkout: moving from main to feat"));
        let entries = log(&story);
        let undo = plan(&entries, &Refs, "e5", "feat").unwrap();
        assert_eq!(undo.target, UndoTarget::Branch("main".into()));
        assert_eq!(undo.label(), "undo checkout of feat");

        // A branch that's gone since: back to the commit it was on.
        story.push(("c3", "checkout: moving from feat to gone"));
        assert_eq!(step(&log(&story)), Ok(UndoTarget::Branch("feat".into())));
        story.pop();
        story.push(("c3", "checkout: moving from gone to main"));
        assert_eq!(step(&log(&story)), Ok(UndoTarget::Detach("e5".into())));

        // An undone checkout is passed over.
        let mut story = BASE.to_vec();
        story.push(("b2", "checkout: moving from main to HEAD~1"));
        story.push(("c3", "[tt-git undo] checkout: moving from b2b2 to main"));
        assert_eq!(step(&log(&story)), soft("b2"));
    }

    #[test]
    fn a_rebase_is_one_step() {
        let mut story = BASE.to_vec();
        story.push(("f6", "checkout: moving from main to feat"));
        story.push(("c3", "rebase (start): checkout main"));
        story.push(("g7", "rebase (pick): seven"));
        story.push(("h8", "rebase (finish): returning to refs/heads/feat"));
        let entries = log(&story);
        let undo = plan(&entries, &Refs, "h8", "feat").unwrap();
        assert_eq!(undo.target, UndoTarget::Reset("f6".into()));
        assert_eq!(undo.what, "rebase onto main");

        story.push(("f6", "[tt-git undo]: updating HEAD"));
        assert_eq!(step(&log(&story)), Ok(UndoTarget::Branch("main".into())));
    }

    #[test]
    fn a_rebase_started_on_another_branch_goes_back_there() {
        let mut story = BASE.to_vec();
        story.push(("f6", "checkout: moving from main to feat"));
        story.push(("c3", "checkout: moving from feat to main"));
        // `git rebase main feat`, run on main.
        story.push(("c3", "rebase (start): checkout main"));
        story.push(("h8", "rebase (pick): mine"));
        story.push(("h8", "rebase (finish): returning to refs/heads/feat"));
        let entries = log(&story);
        let undo = plan(&entries, &Refs, "h8", "feat").unwrap();
        assert_eq!(undo.what, "rebase of feat onto main");
        assert_eq!(
            undo.target,
            UndoTarget::LeaveRebase {
                branch: "feat".into(),
                before: Some("f6".into()),
                rebased: "h8".into(),
                back: Box::new(UndoTarget::Branch("main".into())),
            }
        );

        // The checkout before it is next.
        story.push(("c3", "[tt-git undo] checkout: moving from feat to main"));
        assert_eq!(step(&log(&story)), Ok(UndoTarget::Branch("feat".into())));
    }

    #[test]
    fn a_rebase_after_renaming_the_branch_stays_on_it() {
        let mut story = BASE.to_vec();
        story.push(("f6", "checkout: moving from main to old"));
        story.push(("f6", "Branch: renamed refs/heads/old to refs/heads/new"));
        story.push(("c3", "rebase (start): checkout main"));
        story.push(("k1", "rebase (finish): returning to refs/heads/new"));
        let entries = log(&story);
        // The branch reflog doesn't know this rebase: the rename decides.
        let undo = plan(&entries, &Refs, "k1", "new").unwrap();
        assert_eq!(undo.target, UndoTarget::Reset("f6".into()));
        assert_eq!(undo.what, "rebase onto main");
    }

    #[test]
    fn the_branch_reflog_overrules_a_stale_checkout() {
        let mut story = BASE.to_vec();
        // HEAD's reflog last saw it switch to "other", but feat's reflog
        // says feat started where HEAD was.
        story.push(("f6", "checkout: moving from main to other"));
        story.push(("c3", "rebase (start): checkout main"));
        story.push(("h8", "rebase (finish): returning to refs/heads/feat"));
        let entries = log(&story);
        let undo = plan(&entries, &Refs, "h8", "feat").unwrap();
        assert_eq!(undo.target, UndoTarget::Reset("f6".into()));
    }

    #[test]
    fn a_rebase_with_no_record_of_where_it_started_never_moves_the_branch() {
        let mut story = BASE.to_vec();
        story.push(("b2", "rebase (start): checkout b2"));
        story.push(("x9", "rebase (finish): returning to refs/heads/other"));
        let entries = log(&story);
        let undo = plan(&entries, &Refs, "x9", "other").unwrap();
        assert_eq!(
            undo.target,
            UndoTarget::LeaveRebase {
                branch: "other".into(),
                before: None,
                rebased: "x9".into(),
                back: Box::new(UndoTarget::Detach("c3".into())),
            }
        );
    }

    #[test]
    fn pull_rebase_is_a_rebase() {
        let mut story = BASE.to_vec();
        story.push(("x9", "pull --rebase origin main (start): checkout x9"));
        story.push(("y0", "pull --rebase origin main (pick): mine"));
        story.push(("y0", "pull --rebase origin main (finish): returning to refs/heads/main"));
        assert_eq!(step(&log(&story)), reset("c3"));
    }

    #[test]
    fn no_op_entries_are_skipped() {
        let mut story = BASE.to_vec();
        // `git stash` resets to HEAD.
        story.push(("c3", "reset: moving to HEAD"));
        // An aborted rebase returns where it started.
        story.push(("a1", "rebase (start): checkout a1"));
        story.push(("c3", "rebase (abort): returning to refs/heads/main"));
        story.push(("c3", "clone: from somewhere"));
        assert_eq!(step(&log(&story)), soft("b2"));
    }

    #[test]
    fn other_moves_are_undone() {
        let own = |oid: &str| UndoTarget::SoftReset(oid.into());
        let other = |oid: &str| UndoTarget::Reset(oid.into());
        for (subject, kind) in [
            ("commit (amend): three again", own as fn(&str) -> UndoTarget),
            ("commit (merge): Merge branch 'feat'", other),
            ("merge feat: Fast-forward", other),
            ("pull: Fast-forward", other),
            ("cherry-pick: picked", other),
            ("revert: Revert \"three\"", other),
            ("reset: moving to HEAD~2", other),
        ] {
            let mut story = BASE.to_vec();
            story.push(("z9", subject));
            let entries = log(&story);
            let undo = plan(&entries, &Refs, "z9", "main").unwrap();
            assert_eq!(undo.target, kind("c3"), "{subject}");
            assert_eq!(undo.what, subject);
        }
    }

    #[test]
    fn running_out_of_reflog() {
        let entries = log(&[("b2", "commit: two"), ("c3", "commit: three")]);
        assert_eq!(step(&entries), soft("b2"));
        let entries = log(&[("c3", "commit: three")]);
        assert_eq!(step(&entries), Err(NoStep::Nothing));
        assert_eq!(plan(&[], &Refs, "c3", "main"), Err(NoStep::Nothing));
    }
}
