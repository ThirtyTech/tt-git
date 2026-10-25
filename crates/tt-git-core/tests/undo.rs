//! Undo against real git: the reflog markers, and each kind of step.

use tt_git_core::{GitCli, NoStep, Repo, ResetMode, UNDO_CHANGES_KEPT, UndoStep, UndoTarget};
use tt_git_testkit::Sandbox;

fn open(sb: &Sandbox) -> Repo {
    Repo::discover(sb.path(), &GitCli::new(".").with_env(sb.env())).expect("discover")
}

fn step(repo: &Repo) -> Result<UndoStep, NoStep> {
    repo.undo_step().expect("reading the reflog")
}

/// Works out and takes the next step, with a clean tree.
fn take(repo: &Repo) -> UndoStep {
    let s = step(repo).unwrap_or_else(|e| panic!("no step: {e:?}"));
    repo.apply_undo(&s, false)
        .unwrap_or_else(|e| panic!("{}: {e}", s.label()));
    s
}

fn last_reflog_subject(sb: &Sandbox) -> String {
    sb.git(&["log", "-g", "-1", "--format=%gs", "HEAD"])
}

/// Brings in the commit `change` makes on a branch as a fast-forward merge
/// into main: a step undone with `reset --keep`, unlike your own commits.
fn merged(sb: &Sandbox, change: impl FnOnce(&Sandbox)) {
    sb.checkout_new("incoming");
    change(sb);
    sb.checkout("main");
    sb.git(&["merge", "-q", "--ff-only", "incoming"]);
}

fn read(sb: &Sandbox, path: &str) -> Option<String> {
    std::fs::read_to_string(sb.path().join(path)).ok()
}

#[test]
fn undoing_a_commit_keeps_its_changes() {
    let sb = Sandbox::new();
    let one = sb.commit_file("a.txt", "1\n", "one");
    sb.write("a.txt", "2\n").write("b.txt", "new\n");
    sb.git(&["add", "a.txt", "b.txt"]);
    sb.commit_staged("two");
    // Work in progress that has nothing to do with the commit.
    sb.write("wip.txt", "wip\n");
    let repo = open(&sb);

    let s = take(&repo);
    assert_eq!(s.target, UndoTarget::SoftReset(one.clone()));
    assert_eq!(sb.rev_parse("HEAD"), one);
    // The commit's changes are back, staged, as they were before committing.
    assert_eq!(sb.git(&["diff", "--cached", "--name-only"]), "a.txt\nb.txt");
    assert_eq!(read(&sb, "a.txt").as_deref(), Some("2\n"));
    assert_eq!(read(&sb, "wip.txt").as_deref(), Some("wip\n"));
}

#[test]
fn undo_commits() {
    let sb = Sandbox::new();
    let one = sb.commit("one");
    let two = sb.commit("two");
    sb.commit("three");
    let repo = open(&sb);

    let s = take(&repo);
    assert_eq!(s.label(), "undo commit: three");
    assert_eq!(sb.rev_parse("HEAD"), two);
    assert_eq!(sb.current_branch(), "main");
    assert_eq!(last_reflog_subject(&sb), "[tt-git undo]: updating HEAD");
    assert_eq!(sb.git(&["diff", "--cached", "--name-only"]), "three.txt");
    take(&repo);
    assert_eq!(sb.rev_parse("HEAD"), one);
    assert_eq!(sb.git(&["diff", "--cached", "--name-only"]), "three.txt\ntwo.txt");
    assert_eq!(step(&repo), Err(NoStep::FirstCommit));
}

#[test]
fn a_new_commit_after_undo_is_undone_first() {
    let sb = Sandbox::new();
    sb.commit("one");
    sb.commit("two");
    let repo = open(&sb);
    take(&repo);
    sb.git(&["commit", "-q", "-m", "other"]);
    assert_eq!(take(&repo).label(), "undo commit: other");
    // "two" is already undone.
    assert_eq!(step(&repo), Err(NoStep::FirstCommit));
}

#[test]
fn undo_checkouts() {
    let sb = Sandbox::new();
    sb.commit("one");
    sb.checkout_new("feat");
    sb.commit("on feat");
    let repo = open(&sb);

    take(&repo);
    assert_eq!(sb.current_branch(), "feat");
    let s = take(&repo);
    assert_eq!(s.target, UndoTarget::Branch("main".into()));
    assert_eq!(sb.current_branch(), "main");
    assert_eq!(
        last_reflog_subject(&sb),
        "[tt-git undo] checkout: moving from feat to main"
    );
    // Still a checkout as far as branch recency is concerned.
    let checkouts = repo.checkouts(5).unwrap();
    assert_eq!((checkouts[0].from.as_str(), checkouts[0].to.as_str()), ("feat", "main"));
}

#[test]
fn undo_a_detached_checkout() {
    let sb = Sandbox::new();
    sb.commit("one");
    let two = sb.commit("two");
    sb.git(&["switch", "-q", "--detach", "HEAD~1"]);
    let repo = open(&sb);

    let s = take(&repo);
    assert_eq!(s.target, UndoTarget::Branch("main".into()));
    assert_eq!(sb.current_branch(), "main");
    assert_eq!(sb.rev_parse("HEAD"), two);
}

#[test]
fn undo_a_rebase() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("feat");
    let mine = sb.commit("mine");
    sb.checkout("main");
    sb.commit("theirs");
    sb.checkout("feat");
    let repo = open(&sb);
    repo.rebase("main").unwrap();
    let rebased = sb.rev_parse("HEAD");
    assert_ne!(rebased, mine);

    let s = take(&repo);
    assert_eq!(s.what, "rebase onto main");
    assert_eq!((sb.current_branch(), sb.rev_parse("HEAD")), ("feat".into(), mine));
}

#[test]
fn undo_a_rebase_started_on_another_branch() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("feat");
    let mine = sb.commit("mine");
    sb.checkout("main");
    let theirs = sb.commit("theirs");
    // Run on main, it rebases feat and leaves HEAD there.
    sb.git(&["rebase", "-q", "main", "feat"]);
    assert_eq!(sb.current_branch(), "feat");
    let repo = open(&sb);

    let s = take(&repo);
    assert_eq!(s.what, "rebase of feat onto main");
    assert_eq!(sb.current_branch(), "main");
    assert_eq!(sb.rev_parse("HEAD"), theirs);
    assert_eq!(sb.rev_parse("feat"), mine, "feat is back where it was");
    assert_eq!(
        last_reflog_subject(&sb),
        "[tt-git undo] checkout: moving from feat to main"
    );
    assert!(repo.status().unwrap().is_clean());
}

#[test]
fn undo_a_rebase_after_renaming_the_branch() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("old");
    let mine = sb.commit("mine");
    sb.checkout("main");
    sb.commit("theirs");
    sb.checkout("old");
    sb.git(&["branch", "-m", "old", "new"]);
    let repo = open(&sb);
    repo.rebase("main").unwrap();

    let s = take(&repo);
    assert_eq!(s.what, "rebase onto main");
    assert_eq!((sb.current_branch(), sb.rev_parse("HEAD")), ("new".into(), mine));
}

#[test]
fn a_rebased_branch_that_moved_since_is_left_alone() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("feat");
    sb.commit("mine");
    sb.checkout("main");
    sb.commit("theirs");
    sb.git(&["rebase", "-q", "main", "feat"]);
    let repo = open(&sb);
    let s = step(&repo).unwrap();
    // feat (checked out, so HEAD with it) moves after the step is worked out.
    sb.git(&["update-ref", "refs/heads/feat", "main"]);
    let err = repo.apply_undo(&s, false).unwrap_err().to_string();
    assert!(err.contains("moved after the undo was worked out. Try again."), "{err}");
    assert_eq!(sb.current_branch(), "feat");
    assert_eq!(sb.rev_parse("feat"), sb.rev_parse("main"));
}

#[test]
fn undo_a_merge_and_an_amend() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("feat");
    sb.commit("feature");
    sb.checkout("main");
    let before = sb.commit("main work");
    let repo = open(&sb);

    sb.merge("feat");
    take(&repo);
    assert_eq!(sb.rev_parse("HEAD"), before);

    sb.git(&["commit", "-q", "--amend", "-m", "reworded"]);
    let s = take(&repo);
    assert_eq!(s.what, "commit (amend): reworded");
    assert_eq!(sb.rev_parse("HEAD"), before);
}

#[test]
fn stashed_changes_come_back_after_a_reset() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "1\n", "one");
    merged(&sb, |sb| {
        sb.commit_file("b.txt", "1\n", "two");
    });
    sb.write("a.txt", "changed\n").write("new.txt", "untracked\n");
    let repo = open(&sb);

    let s = step(&repo).unwrap();
    assert!(s.target.touches_worktree());
    repo.apply_undo(&s, true).unwrap();
    assert!(!sb.path().join("b.txt").exists(), "merge undone");
    assert_eq!(std::fs::read_to_string(sb.path().join("a.txt")).unwrap(), "changed\n");
    assert!(sb.path().join("new.txt").exists());
    assert!(repo.stashes().unwrap().is_empty());
    // The stash's own reflog entry doesn't count as a step: the checkout
    // before the merge is next.
    assert_eq!(step(&repo).unwrap().label(), "undo checkout of main");
}

#[test]
fn stashed_changes_that_conflict_stay_stashed() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "1\n", "one");
    merged(&sb, |sb| {
        sb.commit_file("a.txt", "2\n", "two");
    });
    sb.write("a.txt", "3\n");
    let repo = open(&sb);

    let s = step(&repo).unwrap();
    let err = repo.apply_undo(&s, true).unwrap_err().to_string();
    assert!(err.contains(UNDO_CHANGES_KEPT), "{err}");
    assert_eq!(repo.stashes().unwrap().len(), 1);
    assert_eq!(sb.git(&["log", "-1", "--format=%s"]), "one");
}

#[test]
fn stashed_changes_stay_stashed_after_a_switch() {
    let sb = Sandbox::new();
    sb.commit("one");
    sb.checkout_new("feat");
    sb.write("wip.txt", "wip\n");
    let repo = open(&sb);

    let s = step(&repo).unwrap();
    assert!(s.target.is_switch());
    repo.apply_undo(&s, true).unwrap();
    assert_eq!(sb.current_branch(), "main");
    assert!(!sb.path().join("wip.txt").exists());
    assert_eq!(repo.stashes().unwrap().len(), 1);
}

#[test]
fn a_reset_refuses_to_overwrite_changes() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "1\n", "one");
    merged(&sb, |sb| {
        sb.commit_file("a.txt", "2\n", "two");
    });
    sb.write("a.txt", "mine\n");
    let repo = open(&sb);

    let s = step(&repo).unwrap();
    assert!(repo.apply_undo(&s, false).is_err());
    assert_eq!(std::fs::read_to_string(sb.path().join("a.txt")).unwrap(), "mine\n");
    assert_eq!(sb.git(&["log", "-1", "--format=%s"]), "two");
}

#[test]
fn a_reset_refuses_to_overwrite_an_untracked_file() {
    let sb = Sandbox::new();
    sb.commit_file("x.txt", "old\n", "one");
    merged(&sb, |sb| {
        sb.git(&["rm", "-q", "x.txt"]);
        sb.commit_staged("delete x");
    });
    // Untracked now, but undoing the delete would bring back x.txt.
    sb.write("x.txt", "mine\n");
    let repo = open(&sb);

    let s = step(&repo).unwrap();
    assert!(repo.apply_undo(&s, false).is_err());
    assert_eq!(std::fs::read_to_string(sb.path().join("x.txt")).unwrap(), "mine\n");
    assert_eq!(sb.git(&["log", "-1", "--format=%s"]), "delete x");
}

#[test]
fn refuses_when_head_moved() {
    let sb = Sandbox::new();
    sb.commit("one");
    sb.commit("two");
    let repo = open(&sb);
    let s = step(&repo).unwrap();
    let three = sb.commit("three");
    assert!(repo.apply_undo(&s, false).is_err());
    assert_eq!(sb.rev_parse("HEAD"), three);
}

#[test]
fn refuses_during_a_merge_and_before_the_first_commit() {
    let sb = Sandbox::new();
    assert_eq!(step(&open(&sb)), Err(NoStep::Unborn));

    sb.commit_file("a.txt", "base\n", "base");
    sb.checkout_new("feat");
    sb.commit_file("a.txt", "feat\n", "feat");
    sb.checkout("main");
    sb.commit_file("a.txt", "main\n", "main");
    let repo = open(&sb);
    assert!(repo.merge("feat").is_err());
    assert_eq!(step(&repo), Err(NoStep::InProgress(tt_git_core::InProgress::Merge)));
}

#[test]
fn undoing_a_soft_reset_leaves_nothing_staged() {
    let sb = Sandbox::new();
    let one = sb.commit_file("a.txt", "1\n", "one");
    let two = sb.commit_file("a.txt", "2\n", "two");
    let repo = open(&sb);

    repo.reset(&one, ResetMode::Soft).expect("reset");
    assert_eq!(sb.rev_parse("HEAD"), one);
    assert_eq!(sb.git(&["diff", "--cached", "--name-only"]), "a.txt");

    let s = take(&repo);
    assert_eq!(s.label(), format!("undo reset --soft {}", &one[..7]));
    assert_eq!(s.target, UndoTarget::SoftReset(two.clone()));
    assert_eq!(sb.rev_parse("HEAD"), two);
    assert_eq!(sb.git(&["status", "--porcelain"]), "");
}

#[test]
fn undoing_a_mixed_reset_leaves_a_clean_tree() {
    let sb = Sandbox::new();
    let one = sb.commit_file("a.txt", "1\n", "one");
    let two = sb.commit_file("a.txt", "2\n", "two");
    let repo = open(&sb);

    repo.reset("main~1", ResetMode::Mixed).expect("reset");
    assert_eq!(sb.rev_parse("HEAD"), one);
    assert_eq!(sb.git(&["diff", "--name-only"]), "a.txt");
    assert_eq!(sb.git(&["diff", "--cached", "--name-only"]), "");
    assert_eq!(last_reflog_subject(&sb), "reset --mixed main~1: updating HEAD");

    let s = step(&repo).expect("a step");
    assert_eq!(s.target, UndoTarget::MixedReset(two.clone()));
    // Its own changes are the ones in the tree: nothing to stash.
    assert!(!s.target.touches_worktree());
    repo.apply_undo(&s, false).expect("undo");
    assert_eq!(sb.rev_parse("HEAD"), two);
    assert_eq!(sb.git(&["status", "--porcelain"]), "");
}

#[test]
fn a_hard_reset_says_what_it_loses_and_undo_brings_the_commits_back() {
    let sb = Sandbox::new();
    let one = sb.commit_file("a.txt", "1\n", "one");
    sb.commit_file("a.txt", "2\n", "two");
    let three = sb.commit_file("b.txt", "3\n", "three");
    sb.write("a.txt", "dirty\n").write("new.txt", "untracked\n");
    let repo = open(&sb);

    let plan = repo.reset_plan(&one).expect("plan");
    assert_eq!(plan.branch.as_deref(), Some("main"));
    assert_eq!(plan.commits, 2);
    // Untracked files aren't touched by a hard reset.
    assert_eq!(plan.changed, 1);

    repo.reset(&one, ResetMode::Hard).expect("reset");
    assert_eq!(sb.rev_parse("HEAD"), one);
    assert_eq!(read(&sb, "a.txt").as_deref(), Some("1\n"));
    assert_eq!(read(&sb, "new.txt").as_deref(), Some("untracked\n"));
    assert_eq!(read(&sb, "b.txt"), None);

    let s = step(&repo).expect("a step");
    assert_eq!(s.target, UndoTarget::Reset(three.clone()));
    assert_eq!(s.label(), format!("undo reset --hard {}", &one[..7]));
    repo.apply_undo(&s, false).expect("undo");
    assert_eq!(sb.rev_parse("HEAD"), three);
    assert_eq!(read(&sb, "b.txt").as_deref(), Some("3\n"));
}

#[test]
fn nothing_to_reset_without_commits() {
    let sb = Sandbox::new();
    let repo = open(&sb);
    assert!(repo.reset_plan("HEAD").is_err());
}
