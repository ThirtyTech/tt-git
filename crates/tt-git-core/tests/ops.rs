use tt_git_core::{DiscardKind, FileState, GitCli, Repo, StatusEntry};
use tt_git_testkit::Sandbox;

fn open(sb: &Sandbox) -> Repo {
    Repo::discover(sb.path(), &GitCli::new(".").with_env(sb.env())).expect("discover")
}

fn entry(repo: &Repo, path: &str) -> Option<StatusEntry> {
    repo.status().unwrap().entries.into_iter().find(|e| e.path == path)
}

fn paths(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn stage_and_unstage() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "1\n", "base");
    sb.write("a.txt", "2\n").write("b.txt", "new\n");
    let repo = open(&sb);

    repo.stage(&paths(&["a.txt", "b.txt"])).unwrap();
    assert_eq!(entry(&repo, "a.txt").unwrap().index, FileState::Modified);
    assert_eq!(entry(&repo, "b.txt").unwrap().index, FileState::Added);

    repo.unstage(&paths(&["a.txt"])).unwrap();
    let a = entry(&repo, "a.txt").unwrap();
    assert_eq!((a.index, a.worktree), (FileState::Unmodified, FileState::Modified));

    repo.unstage_all().unwrap();
    assert!(entry(&repo, "b.txt").unwrap().is_untracked());
    repo.stage_all().unwrap();
    assert!(repo.status().unwrap().entries.iter().all(|e| e.has_staged()));
}

#[test]
fn stage_deleted_file() {
    let sb = Sandbox::new();
    sb.commit_file("gone.txt", "x\n", "base");
    std::fs::remove_file(sb.path().join("gone.txt")).unwrap();
    let repo = open(&sb);
    repo.stage(&paths(&["gone.txt"])).unwrap();
    assert_eq!(entry(&repo, "gone.txt").unwrap().index, FileState::Deleted);
}

#[test]
fn unstage_on_unborn_branch() {
    let sb = Sandbox::new();
    sb.write("first.txt", "x");
    let repo = open(&sb);
    repo.stage(&paths(&["first.txt"])).unwrap();
    repo.unstage(&paths(&["first.txt"])).unwrap();
    assert!(entry(&repo, "first.txt").unwrap().is_untracked());
    repo.stage_all().unwrap();
    repo.unstage_all().unwrap();
    assert!(entry(&repo, "first.txt").unwrap().is_untracked());
}

#[test]
fn discard_each_kind() {
    let sb = Sandbox::new();
    sb.commit_file("tracked.txt", "orig\n", "base");
    sb.commit_file("to-rename.txt", "rename me please\n", "base2");
    sb.write("tracked.txt", "changed\n");
    sb.write("untracked.txt", "u");
    sb.write("added.txt", "a");
    sb.git(&["add", "added.txt"]);
    sb.git(&["add", "tracked.txt"]);
    sb.write("tracked.txt", "changed again\n");
    sb.git(&["mv", "to-rename.txt", "renamed.txt"]);
    let repo = open(&sb);

    for path in ["tracked.txt", "untracked.txt", "added.txt", "renamed.txt"] {
        let e = entry(&repo, path).unwrap_or_else(|| panic!("{path}"));
        repo.discard(&e.path, &DiscardKind::of(&e)).unwrap();
    }
    assert!(repo.status().unwrap().is_clean(), "{:#?}", repo.status().unwrap());
    assert_eq!(
        std::fs::read_to_string(sb.path().join("tracked.txt")).unwrap(),
        "orig\n"
    );
    assert!(sb.path().join("to-rename.txt").exists());
    assert!(!sb.path().join("untracked.txt").exists());
}

#[test]
fn commit_and_amend() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.write("x.txt", "x");
    let repo = open(&sb);
    repo.stage_all().unwrap();
    repo.commit(Some("Add x\n\nWith a body."), false).unwrap();
    assert_eq!(sb.git(&["log", "-1", "--format=%B"]), "Add x\n\nWith a body.");

    sb.write("y.txt", "y");
    repo.stage_all().unwrap();
    repo.commit(None, true).unwrap();
    assert_eq!(sb.git(&["log", "-1", "--format=%s"]), "Add x");
    assert_eq!(sb.git(&["rev-list", "--count", "HEAD"]), "2");
    assert!(sb.git(&["show", "--name-only", "--format=", "HEAD"]).contains("y.txt"));
}

#[test]
fn commit_with_nothing_staged_fails() {
    let sb = Sandbox::new();
    sb.commit("base");
    let err = open(&sb).commit(Some("empty"), false).unwrap_err();
    assert!(err.to_string().contains("nothing"), "{err}");
}

#[test]
fn push_sets_upstream_then_pull_gets_remote_work() {
    let sb = Sandbox::new();
    sb.commit("base");
    let remote = sb.add_remote("origin");
    sb.checkout_new("topic");
    sb.commit("topic work");
    let repo = open(&sb);

    repo.push().expect("first push sets upstream");
    assert_eq!(
        sb.git(&["rev-parse", "--abbrev-ref", "topic@{upstream}"]),
        "origin/topic"
    );

    let other = sb.clone_to(&remote, "other");
    sb.git_in(&other, &["switch", "-q", "topic"]);
    sb.git_in(&other, &["commit", "-q", "--allow-empty", "-m", "from elsewhere"]);
    sb.git_in(&other, &["push", "-q"]);

    repo.pull().unwrap();
    assert_eq!(sb.git(&["log", "-1", "--format=%s"]), "from elsewhere");
}

#[test]
fn push_without_remote_errors() {
    let sb = Sandbox::new();
    sb.commit("base");
    let err = open(&sb).push().unwrap_err();
    assert!(err.to_string().contains("no remote"), "{err}");
}

#[test]
fn file_diffs() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "one\n", "base");
    sb.write("f.txt", "two\n");
    sb.git(&["add", "f.txt"]);
    sb.write("f.txt", "three\n");
    sb.write("new.txt", "hello\nworld\n");
    let repo = open(&sb);

    let both = repo.file_diff(&entry(&repo, "f.txt").unwrap(), 1 << 20).unwrap();
    assert!(both.contains("── staged ──") && both.contains("+two"), "{both}");
    assert!(both.contains("── unstaged ──") && both.contains("+three"), "{both}");

    let new = repo.file_diff(&entry(&repo, "new.txt").unwrap(), 1 << 20).unwrap();
    assert_eq!(new, "new file new.txt\n+hello\n+world\n");
}

fn conflicting_branches(sb: &Sandbox) {
    sb.commit_file("f.txt", "base\n", "base");
    sb.checkout_new("other");
    sb.commit_file("f.txt", "theirs\n", "theirs");
    sb.checkout("main");
    sb.commit_file("f.txt", "ours\n", "ours");
}

#[test]
fn merge_conflict_then_continue() {
    let sb = Sandbox::new();
    conflicting_branches(&sb);
    let repo = open(&sb);
    let err = repo.merge("other").unwrap_err();
    assert!(err.stderr().unwrap_or_default().contains("CONFLICT"), "{err}");
    let st = repo.status().unwrap();
    assert_eq!(st.in_progress, Some(tt_git_core::InProgress::Merge));
    assert!(st.entries[0].is_conflicted());

    sb.write("f.txt", "resolved\n");
    repo.stage(&paths(&["f.txt"])).unwrap();
    repo.continue_operation(tt_git_core::InProgress::Merge).unwrap();
    let st = repo.status().unwrap();
    assert_eq!(st.in_progress, None);
    assert!(st.is_clean());
    assert_eq!(
        sb.git(&["rev-list", "--parents", "-n1", "HEAD"]).split(' ').count(),
        3,
        "merge commit"
    );
}

#[test]
fn rebase_conflict_then_abort() {
    let sb = Sandbox::new();
    conflicting_branches(&sb);
    let repo = open(&sb);
    let before = sb.rev_parse("HEAD");
    assert!(repo.rebase("other").is_err());
    assert_eq!(
        repo.status().unwrap().in_progress,
        Some(tt_git_core::InProgress::Rebase)
    );
    repo.abort_operation(tt_git_core::InProgress::Rebase).unwrap();
    assert_eq!(repo.status().unwrap().in_progress, None);
    assert_eq!(sb.rev_parse("HEAD"), before);
}

#[test]
fn clean_rebase_and_cherry_pick() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("topic");
    let pick = sb.commit("topic work");
    sb.checkout("main");
    sb.commit("main work");
    let repo = open(&sb);
    repo.cherry_pick(&pick).unwrap();
    assert_eq!(sb.git(&["log", "-1", "--format=%s"]), "topic work");

    sb.checkout("topic");
    repo.rebase("main").unwrap();
    // topic's commit was already applied on main, so the rebase drops it.
    assert_eq!(sb.rev_parse("topic"), sb.rev_parse("main"));
}

#[test]
fn create_branch_in_a_worktree() {
    let sb = Sandbox::new();
    let base = sb.commit("base");
    let repo = open(&sb);
    repo.create_worktree("feature/x", "main").unwrap();

    let path = sb.path().join(".worktrees").join("feature").join("x");
    assert_eq!(
        repo.worktree_path("feature/x"),
        repo.root().join(".worktrees/feature/x")
    );
    assert!(path.join("base.txt").is_file(), "checked out at {}", path.display());
    assert_eq!(sb.current_branch(), "main", "this worktree stays where it was");
    assert_eq!(sb.git_in(&path, &["branch", "--show-current"]), "feature/x");
    assert_eq!(sb.rev_parse("feature/x"), base);
    assert!(
        repo.status().unwrap().entries.is_empty(),
        "the worktrees folder is excluded"
    );

    // From inside the new worktree, more go next to it rather than in it.
    let inner = Repo::discover(&path, &GitCli::new(".").with_env(sb.env())).unwrap();
    assert_eq!(
        std::fs::canonicalize(inner.main_root()).unwrap(),
        std::fs::canonicalize(sb.path()).unwrap()
    );
    inner.create_worktree("y", "main").unwrap();
    assert!(sb.path().join(".worktrees/y/base.txt").is_file());
    let exclude = std::fs::read_to_string(repo.common_dir().join("info/exclude")).unwrap();
    assert_eq!(exclude.matches("/.worktrees/").count(), 1, "{exclude}");

    let err = repo.create_worktree("y", "main").unwrap_err();
    assert!(err.stderr().unwrap_or_default().contains("already exists"), "{err}");
}
