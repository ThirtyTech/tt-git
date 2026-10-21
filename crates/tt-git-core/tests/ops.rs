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
fn create_lightweight_and_annotated_tags_then_push_one() {
    let sb = Sandbox::new();
    let first = sb.commit("first");
    let second = sb.commit("second");
    sb.add_remote("upstream");
    let remote = sb.add_remote("origin");
    let repo = open(&sb);

    repo.create_tag("v1", &first, None).unwrap();
    assert_eq!(sb.git(&["cat-file", "-t", "v1"]), "commit", "lightweight");
    assert_eq!(sb.rev_parse("v1"), first);

    repo.create_tag("v2", &second, Some("Release 2\n\nNotes")).unwrap();
    assert_eq!(sb.git(&["cat-file", "-t", "v2"]), "tag", "annotated");
    assert_eq!(sb.rev_parse("v2^{commit}"), second);
    assert_eq!(
        sb.git(&["tag", "-l", "--format=%(contents)", "v2"]),
        "Release 2\n\nNotes"
    );

    let err = repo.create_tag("v1", &second, None).unwrap_err();
    assert!(err.to_string().contains("already exists"), "{err}");

    repo.push_tag("v2").expect("push goes to origin");
    let pushed = sb.git_in(&remote, &["tag", "-l"]);
    assert_eq!(pushed, "v2", "only the one tag is pushed");

    repo.delete_remote_tag("origin", "v2").unwrap();
    assert_eq!(sb.git_in(&remote, &["tag", "-l"]), "");
    assert_eq!(sb.git(&["tag", "-l", "v2"]), "v2", "the local tag stays");
}

#[test]
fn push_tag_without_remote_errors() {
    let sb = Sandbox::new();
    let head = sb.commit("base");
    let repo = open(&sb);
    repo.create_tag("v1", &head, None).unwrap();
    let err = repo.push_tag("v1").unwrap_err();
    assert!(err.to_string().contains("no remote"), "{err}");
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

#[test]
fn lists_worktrees_and_their_changes() {
    let sb = Sandbox::new();
    let base = sb.commit("base");
    let repo = open(&sb);
    repo.create_worktree("side", "main").unwrap();
    let detached = sb.path().join(".worktrees/review");
    sb.git(&["worktree", "add", "-q", "--detach", detached.to_str().unwrap(), "main"]);
    sb.git(&["worktree", "lock", "--reason", "on usb", detached.to_str().unwrap()]);

    let wts = repo.worktrees().unwrap();
    assert_eq!(wts.len(), 3, "{wts:#?}");
    assert!(wts[0].main && wts[0].current, "{:?}", wts[0]);
    assert_eq!(wts[0].branch.as_deref(), Some("main"));
    let side = wts.iter().find(|w| w.branch.as_deref() == Some("side")).unwrap();
    assert!(!side.main && !side.current && side.locked.is_none());
    assert_eq!(side.head.as_deref(), Some(base.as_str()));
    let review = wts.iter().find(|w| w.branch.is_none()).unwrap();
    assert_eq!(review.locked.as_deref(), Some("on usb"));

    // Seen from inside a linked worktree, that one is current.
    let inner = Repo::discover(&side.path, &GitCli::new(".").with_env(sb.env())).unwrap();
    let from_inner = inner.worktrees().unwrap();
    assert!(
        from_inner
            .iter()
            .any(|w| w.current && w.branch.as_deref() == Some("side"))
    );
    assert!(!from_inner[0].current);

    assert_eq!(repo.worktree_changes(&side.path).unwrap(), 0);
    std::fs::write(
        side.path.join("base.txt"),
        "changed
",
    )
    .unwrap();
    std::fs::write(
        side.path.join("new.txt"),
        "new
",
    )
    .unwrap();
    assert_eq!(repo.worktree_changes(&side.path).unwrap(), 2);
    assert!(repo.status().unwrap().entries.is_empty(), "this worktree is untouched");
}

#[test]
fn remove_worktree_refuses_changes_until_forced() {
    let sb = Sandbox::new();
    sb.commit("base");
    let repo = open(&sb);
    repo.create_worktree("clean", "main").unwrap();
    repo.create_worktree("dirty", "main").unwrap();
    let clean = sb.path().join(".worktrees/clean");
    let dirty = sb.path().join(".worktrees/dirty");
    std::fs::write(
        dirty.join("scratch.txt"),
        "wip
",
    )
    .unwrap();

    repo.remove_worktree(&clean, false).unwrap();
    assert!(!clean.exists());
    assert_eq!(sb.rev_parse("clean"), sb.rev_parse("main"), "the branch stays");

    let err = repo.remove_worktree(&dirty, false).unwrap_err();
    assert!(
        err.stderr().unwrap_or_default().contains("modified or untracked"),
        "{err}"
    );
    assert!(dirty.join("scratch.txt").is_file(), "nothing was deleted");
    repo.remove_worktree(&dirty, true).unwrap();
    assert!(!dirty.exists());
    assert_eq!(repo.worktrees().unwrap().len(), 1);
}

#[test]
fn remove_worktree_refuses_main_current_and_locked() {
    let sb = Sandbox::new();
    sb.commit("base");
    let repo = open(&sb);
    repo.create_worktree("side", "main").unwrap();
    repo.create_worktree("locked", "main").unwrap();
    let side = sb.path().join(".worktrees/side");
    let locked = sb.path().join(".worktrees/locked");
    sb.git(&["worktree", "lock", locked.to_str().unwrap()]);

    let err = repo.remove_worktree(sb.path(), true).unwrap_err();
    assert!(err.to_string().contains("running in that worktree"), "{err}");
    let inner = Repo::discover(&side, &GitCli::new(".").with_env(sb.env())).unwrap();
    let err = inner.remove_worktree(sb.path(), true).unwrap_err();
    assert!(err.to_string().contains("main worktree"), "{err}");
    let err = inner.remove_worktree(&side, false).unwrap_err();
    assert!(err.to_string().contains("running in that worktree"), "{err}");

    let err = repo.remove_worktree(&locked, true).unwrap_err();
    assert!(err.stderr().unwrap_or_default().contains("locked"), "{err}");
    assert!(locked.exists());
}

#[test]
fn remove_worktree_whose_folder_is_gone() {
    let sb = Sandbox::new();
    sb.commit("base");
    let repo = open(&sb);
    repo.create_worktree("gone", "main").unwrap();
    let gone = sb.path().join(".worktrees/gone");
    std::fs::remove_dir_all(&gone).unwrap();
    let wts = repo.worktrees().unwrap();
    assert!(wts[1].prunable.is_some(), "{wts:#?}");

    repo.remove_worktree(&wts[1].path, false).unwrap();
    assert_eq!(repo.worktrees().unwrap().len(), 1);
}

mod hunks {
    use tt_git_core::PatchOp;
    use tt_git_core::patch::{FileDiff, LineKind};

    use super::*;

    const BASE: &str = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n";

    fn side(repo: &Repo, path: &str, staged: bool) -> FileDiff {
        let e = entry(repo, path).unwrap_or_else(|| panic!("{path} not in status"));
        repo.stage_diff(&e, staged, 1 << 20).unwrap().unwrap()
    }

    /// Builds a patch from `path`'s staged or unstaged diff and applies it.
    fn apply(repo: &Repo, path: &str, op: PatchOp, pick: impl Fn(usize, usize) -> bool) {
        let e = entry(repo, path).unwrap();
        let diff = side(repo, path, op == PatchOp::Unstage);
        let patch = diff
            .patch(pick, op.direction(), e.is_untracked())
            .expect("something picked");
        repo.apply_patch(path, &patch, op, e.is_untracked())
            .unwrap_or_else(|err| panic!("{err}\n{patch}"));
    }

    fn index(sb: &Sandbox, path: &str) -> String {
        sb.git(&["show", &format!(":{path}")])
    }

    /// `text` with line `n` (from 1) replaced.
    fn with_line(text: &str, n: usize, new: &str) -> String {
        let mut lines: Vec<&str> = text.lines().collect();
        lines[n - 1] = new;
        lines.join("\n") + "\n"
    }

    /// Two hunks: line 2 changed, line 14 changed.
    fn two_hunks(sb: &Sandbox) {
        sb.commit_file("f.txt", BASE, "base");
        sb.write("f.txt", &with_line(&with_line(BASE, 2, "two"), 14, "fourteen"));
    }

    #[test]
    fn stage_one_hunk_then_unstage_it() {
        let sb = Sandbox::new();
        two_hunks(&sb);
        let repo = open(&sb);
        assert_eq!(side(&repo, "f.txt", false).hunks.len(), 2);

        apply(&repo, "f.txt", PatchOp::Stage, |h, _| h == 1);
        assert!(index(&sb, "f.txt").contains("fourteen") && !index(&sb, "f.txt").contains("two"));
        assert_eq!(side(&repo, "f.txt", false).hunks.len(), 1);
        assert_eq!(side(&repo, "f.txt", true).hunks.len(), 1);

        apply(&repo, "f.txt", PatchOp::Unstage, |_, _| true);
        assert_eq!(index(&sb, "f.txt"), BASE.trim_end());
        // The working tree never changed.
        assert!(
            std::fs::read_to_string(sb.path().join("f.txt"))
                .unwrap()
                .contains("two")
        );
    }

    #[test]
    fn stage_a_single_added_line() {
        let sb = Sandbox::new();
        sb.commit_file("f.txt", "a\nb\n", "base");
        sb.write("f.txt", "a\nx\ny\nb\n");
        let repo = open(&sb);
        let diff = side(&repo, "f.txt", false);
        let y = diff.hunks[0]
            .lines
            .iter()
            .position(|l| l.kind == LineKind::Add && l.text == "y")
            .unwrap();
        apply(&repo, "f.txt", PatchOp::Stage, |_, l| l == y);
        assert_eq!(index(&sb, "f.txt"), "a\ny\nb");
    }

    #[test]
    fn unstage_a_single_line() {
        let sb = Sandbox::new();
        sb.commit_file("f.txt", "a\nb\n", "base");
        sb.write("f.txt", "a\nx\ny\nb\n");
        sb.git(&["add", "f.txt"]);
        let repo = open(&sb);
        let diff = side(&repo, "f.txt", true);
        let x = diff.hunks[0].lines.iter().position(|l| l.text == "x").unwrap();
        apply(&repo, "f.txt", PatchOp::Unstage, |_, l| l == x);
        assert_eq!(index(&sb, "f.txt"), "a\ny\nb");
    }

    #[test]
    fn discard_one_hunk_from_the_working_tree() {
        let sb = Sandbox::new();
        two_hunks(&sb);
        let repo = open(&sb);
        apply(&repo, "f.txt", PatchOp::Discard, |h, _| h == 0);
        assert_eq!(
            std::fs::read_to_string(sb.path().join("f.txt")).unwrap(),
            with_line(BASE, 14, "fourteen")
        );
    }

    #[test]
    fn stage_part_of_an_untracked_file() {
        let sb = Sandbox::new();
        sb.commit("base");
        sb.write("new.txt", "one\ntwo\nthree\n");
        let repo = open(&sb);
        apply(&repo, "new.txt", PatchOp::Stage, |_, l| l == 0);
        assert_eq!(index(&sb, "new.txt"), "one");
        let e = entry(&repo, "new.txt").unwrap();
        assert_eq!((e.index, e.worktree), (FileState::Added, FileState::Modified));
        // The rest of it is now an ordinary unstaged change.
        apply(&repo, "new.txt", PatchOp::Stage, |_, _| true);
        assert!(!entry(&repo, "new.txt").unwrap().has_unstaged());
    }

    #[test]
    fn stage_part_of_a_deletion() {
        let sb = Sandbox::new();
        sb.commit_file("d.txt", "one\ntwo\n", "base");
        std::fs::remove_file(sb.path().join("d.txt")).unwrap();
        let repo = open(&sb);
        apply(&repo, "d.txt", PatchOp::Stage, |_, l| l == 1);
        assert_eq!(index(&sb, "d.txt"), "one");
    }

    #[test]
    fn crlf_lines_stage_exactly() {
        let sb = Sandbox::new();
        sb.git(&["config", "core.autocrlf", "false"]);
        sb.commit_file("w.txt", "a\r\nb\r\n", "base");
        sb.write("w.txt", "a\r\nB\r\nb\r\nc\r\n");
        let repo = open(&sb);
        let diff = side(&repo, "w.txt", false);
        let first = diff.hunks[0].lines.iter().position(|l| l.text == "B\r").unwrap();
        apply(&repo, "w.txt", PatchOp::Stage, |_, l| l == first);
        let blob = sb.git(&["cat-file", "-p", ":w.txt"]);
        assert_eq!(blob, "a\r\nB\r\nb");
    }

    #[test]
    fn conflicts_and_binaries_are_whole_file_only() {
        let sb = Sandbox::new();
        sb.commit("base");
        std::fs::write(sb.path().join("bin.dat"), b"a\0b").unwrap();
        let repo = open(&sb);
        let e = entry(&repo, "bin.dat").unwrap();
        assert!(repo.stage_diff(&e, false, 1 << 20).unwrap().is_err());
    }
}

mod stash {
    use tt_git_core::{CheckoutPlan, StashScope};

    use super::*;

    fn read(sb: &Sandbox, path: &str) -> Option<String> {
        std::fs::read_to_string(sb.path().join(path)).ok()
    }

    #[test]
    fn no_stash_reflog_means_no_stashes() {
        let sb = Sandbox::new();
        sb.commit("base");
        assert!(open(&sb).stashes().unwrap().is_empty());
    }

    #[test]
    fn push_everything_then_pop() {
        let sb = Sandbox::new();
        sb.commit_file("f.txt", "1\n", "base");
        sb.write("f.txt", "2\n").write("new.txt", "new\n");
        let repo = open(&sb);
        repo.stash_push(Some("wip"), &StashScope::All).unwrap();
        assert!(repo.status().unwrap().entries.is_empty());

        let stashes = repo.stashes().unwrap();
        assert_eq!(stashes.len(), 1);
        assert_eq!(stashes[0].name(), "stash@{0}");
        assert!(stashes[0].message.ends_with(": wip"), "{}", stashes[0].message);
        assert!(stashes[0].untracked.is_some());
        let details = repo.stash_details(&stashes[0]).unwrap();
        assert!(details.contains("+2") && details.contains("── untracked ──") && details.contains("+new"));

        repo.stash_apply(0, true).unwrap();
        assert_eq!(read(&sb, "f.txt").as_deref(), Some("2\n"));
        assert_eq!(read(&sb, "new.txt").as_deref(), Some("new\n"));
        assert!(repo.stashes().unwrap().is_empty());
    }

    #[test]
    fn apply_keeps_and_drop_removes() {
        let sb = Sandbox::new();
        sb.commit_file("f.txt", "1\n", "base");
        sb.write("f.txt", "2\n");
        let repo = open(&sb);
        repo.stash_push(None, &StashScope::All).unwrap();
        sb.write("g.txt", "g\n");
        repo.stash_push(Some("second"), &StashScope::All).unwrap();
        let stashes = repo.stashes().unwrap();
        assert_eq!(stashes.len(), 2);
        assert!(stashes[1].message.starts_with("WIP on "), "{}", stashes[1].message);

        repo.stash_apply(1, false).unwrap();
        assert_eq!(read(&sb, "f.txt").as_deref(), Some("2\n"));
        assert_eq!(repo.stashes().unwrap().len(), 2);
        repo.stash_drop(0).unwrap();
        let left = repo.stashes().unwrap();
        assert_eq!(left.len(), 1);
        assert!(left[0].message.starts_with("WIP on "));
    }

    #[test]
    fn staged_only_and_keep_staged() {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a\n", "base");
        sb.commit_file("b.txt", "b\n", "more");
        sb.write("a.txt", "A\n").write("b.txt", "B\n");
        sb.git(&["add", "a.txt"]);
        let repo = open(&sb);

        repo.stash_push(None, &StashScope::Staged).unwrap();
        assert_eq!(read(&sb, "a.txt").as_deref(), Some("a\n"));
        assert_eq!(read(&sb, "b.txt").as_deref(), Some("B\n"));
        repo.stash_apply(0, true).unwrap();

        sb.git(&["add", "a.txt"]);
        repo.stash_push(None, &StashScope::KeepStaged).unwrap();
        assert_eq!(read(&sb, "a.txt").as_deref(), Some("A\n"));
        assert_eq!(read(&sb, "b.txt").as_deref(), Some("b\n"));
    }

    #[test]
    fn push_one_untracked_path() {
        let sb = Sandbox::new();
        sb.commit_file("f.txt", "1\n", "base");
        sb.write("f.txt", "2\n").write("new.txt", "new\n");
        let repo = open(&sb);
        repo.stash_push(None, &StashScope::Paths(vec!["new.txt".into()]))
            .unwrap();
        assert_eq!(read(&sb, "new.txt"), None);
        assert_eq!(read(&sb, "f.txt").as_deref(), Some("2\n"));
    }

    const TEN: &str = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";

    /// a.txt: staged "1" -> "ONE" and unstaged "10" -> "TEN"; b.txt
    /// unstaged; c.txt staged; u.txt untracked.
    fn mixed() -> Sandbox {
        let sb = Sandbox::new();
        sb.write("a.txt", TEN).write("b.txt", "b\n").write("c.txt", "c\n");
        sb.git(&["add", "."]);
        sb.commit_staged("base");
        sb.write("a.txt", &TEN.replace("1\n2", "ONE\n2"));
        sb.git(&["add", "a.txt"]);
        sb.write("a.txt", &TEN.replace("1\n2", "ONE\n2").replace("10", "TEN"));
        sb.write("b.txt", "B\n").write("c.txt", "C\n").write("u.txt", "u\n");
        sb.git(&["add", "c.txt"]);
        sb
    }

    fn short_status(sb: &Sandbox) -> String {
        sb.git(&["status", "--short"])
    }

    #[test]
    fn unstaged_only_keeps_the_staged_changes() {
        let sb = mixed();
        let repo = open(&sb);
        let staged = sb.git(&["diff", "--cached"]);

        repo.stash_push(Some("rest"), &StashScope::Unstaged).unwrap();
        assert_eq!(sb.git(&["diff", "--cached"]), staged);
        assert_eq!(sb.git(&["diff"]), "");
        assert_eq!(read(&sb, "a.txt"), Some(TEN.replace("1\n2", "ONE\n2")));
        assert_eq!(read(&sb, "b.txt").as_deref(), Some("b\n"));
        assert_eq!(read(&sb, "c.txt").as_deref(), Some("C\n"));
        assert_eq!(read(&sb, "u.txt"), None);
        let stashes = repo.stashes().unwrap();
        assert_eq!(stashes.len(), 1, "{stashes:?}");
        assert!(stashes[0].message.ends_with(": rest"));

        // The stash holds only the unstaged changes: it applies on top of
        // the staged ones without conflict.
        let shown = sb.git(&["stash", "show", "--include-untracked", "--name-only", "stash@{0}"]);
        let mut files: Vec<&str> = shown.lines().collect();
        files.sort();
        assert_eq!(files, ["a.txt", "b.txt", "u.txt"]);
        repo.stash_apply(0, true).unwrap();
        assert_eq!(
            read(&sb, "a.txt"),
            Some(TEN.replace("1\n2", "ONE\n2").replace("10", "TEN"))
        );
        assert_eq!(read(&sb, "u.txt").as_deref(), Some("u\n"));
        assert_eq!(sb.git(&["diff", "--cached"]), staged);
    }

    #[test]
    fn unstaged_with_nothing_staged_is_a_plain_stash() {
        let sb = Sandbox::new();
        sb.commit_file("f.txt", "1\n", "base");
        sb.write("f.txt", "2\n").write("new.txt", "new\n");
        let repo = open(&sb);
        repo.stash_push(None, &StashScope::Unstaged).unwrap();
        assert_eq!(short_status(&sb), "");
        assert_eq!(repo.stashes().unwrap().len(), 1);
    }

    #[test]
    fn unstaged_with_nothing_unstaged_refuses() {
        let sb = Sandbox::new();
        sb.commit_file("f.txt", "1\n", "base");
        sb.write("f.txt", "2\n");
        sb.git(&["add", "f.txt"]);
        let repo = open(&sb);
        let err = repo.stash_push(None, &StashScope::Unstaged).unwrap_err();
        assert_eq!(err.to_string(), "no unstaged changes to stash");
        assert!(repo.stashes().unwrap().is_empty());
        assert_eq!(short_status(&sb), "M  f.txt");
    }

    #[test]
    fn overlapping_staged_and_unstaged_changes_refuse_cleanly() {
        let sb = Sandbox::new();
        sb.commit_file("f.txt", "1\n", "base");
        sb.write("f.txt", "2\n");
        sb.git(&["add", "f.txt"]);
        sb.write("f.txt", "3\n");
        let repo = open(&sb);
        for scope in [StashScope::Staged, StashScope::Unstaged] {
            let err = repo.stash_push(None, &scope).unwrap_err();
            assert!(err.to_string().contains("overlap"), "{scope:?}: {err}");
            // git saves a stash before it fails; it's dropped again.
            assert!(repo.stashes().unwrap().is_empty(), "{scope:?}");
            assert_eq!(short_status(&sb), "MM f.txt");
            assert_eq!(read(&sb, "f.txt").as_deref(), Some("3\n"));
            assert_eq!(sb.git(&["show", ":f.txt"]), "2");
        }
    }

    #[test]
    fn a_path_is_not_a_glob() {
        let sb = Sandbox::new();
        sb.write("a[b].txt", "x\n").write("ab.txt", "y\n");
        sb.git(&["add", "."]);
        sb.commit_staged("base");
        sb.write("a[b].txt", "X\n").write("ab.txt", "Y\n");
        let repo = open(&sb);
        repo.stash_push(None, &StashScope::Paths(paths(&["a[b].txt"]))).unwrap();
        assert_eq!(read(&sb, "a[b].txt").as_deref(), Some("x\n"));
        assert_eq!(read(&sb, "ab.txt").as_deref(), Some("Y\n"));
    }

    #[test]
    fn push_a_staged_rename() {
        let sb = Sandbox::new();
        sb.write("old.txt", "o\n").write("other.txt", "1\n");
        sb.git(&["add", "."]);
        sb.commit_staged("base");
        sb.git(&["mv", "old.txt", "new.txt"]);
        sb.write("other.txt", "2\n");
        let repo = open(&sb);
        repo.stash_push(None, &StashScope::Paths(paths(&["new.txt", "old.txt"])))
            .unwrap();
        assert_eq!(short_status(&sb), " M other.txt");
        assert_eq!(read(&sb, "old.txt").as_deref(), Some("o\n"));
        assert_eq!(repo.stashes().unwrap().len(), 1);

        // Back as an added file and an unstaged deletion.
        sb.git(&["stash", "pop", "--index"]);
        assert_eq!(read(&sb, "new.txt").as_deref(), Some("o\n"));
        assert_eq!(read(&sb, "old.txt"), None);
    }

    #[test]
    fn push_a_staged_deletion() {
        let sb = Sandbox::new();
        sb.write("gone.txt", "g\n").write("other.txt", "1\n");
        sb.git(&["add", "."]);
        sb.commit_staged("base");
        sb.git(&["rm", "-q", "gone.txt"]);
        sb.write("other.txt", "2\n");
        let repo = open(&sb);
        repo.stash_push(None, &StashScope::Paths(paths(&["gone.txt"]))).unwrap();
        assert_eq!(short_status(&sb), " M other.txt");
        assert_eq!(read(&sb, "gone.txt").as_deref(), Some("g\n"));
        assert_eq!(repo.stashes().unwrap().len(), 1);
    }

    #[test]
    fn branch_from_a_stash() {
        let sb = Sandbox::new();
        sb.commit_file("f.txt", "1\n", "base");
        sb.write("f.txt", "2\n");
        let repo = open(&sb);
        repo.stash_push(None, &StashScope::All).unwrap();
        sb.commit("moved on");
        repo.stash_branch("rescued", 0).unwrap();
        assert_eq!(sb.git(&["branch", "--show-current"]), "rescued");
        assert_eq!(read(&sb, "f.txt").as_deref(), Some("2\n"));
        assert!(repo.stashes().unwrap().is_empty());
    }

    #[test]
    fn stash_then_check_out_a_commit() {
        let sb = Sandbox::new();
        let base = sb.commit_file("f.txt", "1\n", "base");
        sb.commit_file("f.txt", "2\n", "next");
        sb.write("f.txt", "local\n").write("new.txt", "new\n");
        let repo = open(&sb);
        repo.stash_and_checkout(&CheckoutPlan::Detach(base.clone())).unwrap();
        assert_eq!(sb.rev_parse("HEAD"), base);
        assert_eq!(sb.git(&["branch", "--show-current"]), "");
        assert_eq!(read(&sb, "f.txt").as_deref(), Some("1\n"));
        assert_eq!(read(&sb, "new.txt"), None);
        assert_eq!(repo.stashes().unwrap().len(), 1);
    }

    #[test]
    fn failed_checkout_pops_the_changes_back() {
        let sb = Sandbox::new();
        sb.commit_file("f.txt", "1\n", "base");
        sb.write("f.txt", "local\n").write("new.txt", "new\n");
        let repo = open(&sb);
        repo.stash_and_checkout(&CheckoutPlan::Local("no-such-branch".into()))
            .unwrap_err();
        assert_eq!(read(&sb, "f.txt").as_deref(), Some("local\n"));
        assert_eq!(read(&sb, "new.txt").as_deref(), Some("new\n"));
        assert!(repo.stashes().unwrap().is_empty());
    }

    #[test]
    fn failed_checkout_with_nothing_stashed_leaves_old_stashes() {
        let sb = Sandbox::new();
        sb.commit_file("f.txt", "1\n", "base");
        sb.write("f.txt", "old\n");
        let repo = open(&sb);
        repo.stash_push(Some("older"), &StashScope::All).unwrap();
        repo.stash_and_checkout(&CheckoutPlan::Local("no-such-branch".into()))
            .unwrap_err();
        assert_eq!(read(&sb, "f.txt").as_deref(), Some("1\n"));
        assert_eq!(repo.stashes().unwrap().len(), 1);
    }
}
