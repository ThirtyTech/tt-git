use tt_git_core::{BranchHistory, FileState, GitCli, GitError, Head, LogScope, RefKind, Repo};
use tt_git_testkit::{BASE_TIME, Sandbox};

fn open(sb: &Sandbox) -> Repo {
    Repo::discover(sb.path(), &GitCli::new(".").with_env(sb.env())).expect("discover")
}

#[test]
fn discovers_from_subdirectory() {
    let sb = Sandbox::new();
    sb.write("a/b/file.txt", "x");
    let repo = Repo::discover(&sb.path().join("a/b"), &GitCli::new(".").with_env(sb.env())).unwrap();
    assert_eq!(repo.name(), "repo");
    assert_eq!(
        std::fs::canonicalize(repo.root()).unwrap(),
        std::fs::canonicalize(sb.path()).unwrap()
    );
    assert!(repo.common_dir().join("HEAD").exists());
}

#[test]
fn not_a_repo() {
    let sb = Sandbox::without_repo();
    // Stop discovery from walking up into whatever contains the temp dir.
    let git = GitCli::new(".")
        .with_env(sb.env())
        .with_env([("GIT_CEILING_DIRECTORIES", sb.root().as_os_str())]);
    let err = Repo::discover(sb.path(), &git).unwrap_err();
    assert!(matches!(err, GitError::NotARepo(_)), "{err:?}");
}

#[test]
fn head_states() {
    let sb = Sandbox::new();
    let repo = open(&sb);
    assert_eq!(repo.head().unwrap(), Head::Unborn { name: "main".into() });
    assert!(repo.commits(&LogScope::All, 100).unwrap().is_empty());
    assert!(
        repo.commits(&LogScope::Head { upstream: None }, 100)
            .unwrap()
            .is_empty()
    );

    let c1 = sb.commit("one");
    assert_eq!(
        repo.head().unwrap(),
        Head::Branch {
            name: "main".into(),
            oid: c1.clone()
        }
    );

    sb.git(&["switch", "-q", "--detach", "HEAD"]);
    assert_eq!(repo.head().unwrap(), Head::Detached { oid: c1 });
}

#[test]
fn loads_commits_in_topo_order() {
    let sb = Sandbox::new();
    let base = sb.commit("base");
    sb.checkout_new("feature");
    let f1 = sb.commit("feature 1");
    sb.checkout("main");
    let m1 = sb.commit("main 1");
    let merge = sb.merge("feature");
    sb.checkout_new("side");
    let side = sb.commit("side only");
    sb.checkout("main");

    let repo = open(&sb);
    let all = repo.commits(&LogScope::All, 100).unwrap();
    let ids: Vec<&str> = all.iter().map(|c| c.oid.as_str()).collect();
    assert_eq!(ids.len(), 5);
    assert_eq!(ids[0], side);
    let pos = |id: &str| ids.iter().position(|x| *x == id).unwrap();
    // Children before parents.
    assert!(pos(&merge) < pos(&m1) && pos(&merge) < pos(&f1));
    assert!(pos(&m1) < pos(&base) && pos(&f1) < pos(&base));

    let merge_commit = &all[pos(&merge)];
    assert_eq!(merge_commit.parents, vec![m1.clone(), f1.clone()]);
    assert_eq!(merge_commit.author_name, "Test User");
    assert!(merge_commit.commit_time > BASE_TIME);

    // Current-branch scope excludes "side".
    let head_only = repo.commits(&LogScope::Head { upstream: None }, 100).unwrap();
    assert_eq!(head_only.len(), 4);
    assert!(head_only.iter().all(|c| c.oid != side));

    // Limit is honoured.
    assert_eq!(repo.commits(&LogScope::All, 2).unwrap().len(), 2);
}

#[test]
fn head_scope_includes_upstream_commits() {
    let sb = Sandbox::new();
    sb.commit("base");
    let remote = sb.add_remote("origin");
    sb.git(&["branch", "-q", "--set-upstream-to=origin/main"]);
    // Someone else pushes a commit.
    let other = sb.clone_to(&remote, "other");
    sb.git_in(&other, &["commit", "-q", "--allow-empty", "-m", "incoming"]);
    sb.git_in(&other, &["push", "-q", "origin", "main"]);
    sb.git(&["fetch", "-q", "origin"]);

    let repo = open(&sb);
    let without = repo.commits(&LogScope::Head { upstream: None }, 100).unwrap();
    let with = repo
        .commits(
            &LogScope::Head {
                upstream: Some("refs/remotes/origin/main".into()),
            },
            100,
        )
        .unwrap();
    assert_eq!(without.len(), 1);
    assert_eq!(with.len(), 2);
    assert_eq!(with[0].summary, "incoming");
}

#[test]
fn stash_is_excluded_from_all() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.write("base.txt", "dirty");
    sb.git(&["stash", "-q"]);
    let repo = open(&sb);
    let all = repo.commits(&LogScope::All, 100).unwrap();
    assert_eq!(all.len(), 1, "{all:#?}");
}

#[test]
fn loads_refs_with_tracking() {
    let sb = Sandbox::new();
    let c1 = sb.commit("one");
    sb.branch("feature");
    sb.git(&["tag", "-a", "v1", "-m", "release"]);
    sb.add_remote("origin");
    sb.git(&["branch", "-q", "--set-upstream-to=origin/main"]);
    sb.commit("two");

    let repo = open(&sb);
    let refs = repo.refs().unwrap();
    let find = |name: &str| {
        refs.iter()
            .find(|r| r.full_name == name)
            .unwrap_or_else(|| panic!("{name}"))
    };

    let main = find("refs/heads/main");
    assert!(main.is_head);
    assert_eq!(main.upstream.as_deref(), Some("refs/remotes/origin/main"));
    assert_eq!((main.ahead, main.behind), (1, 0));

    let remote_feature = find("refs/remotes/origin/feature");
    assert_eq!(
        remote_feature.kind,
        RefKind::RemoteBranch {
            remote: "origin".into()
        }
    );
    assert_eq!(remote_feature.branch_name(), "feature");
    assert_eq!(remote_feature.target, c1);

    let tag = find("refs/tags/v1");
    assert_eq!(tag.kind, RefKind::Tag);
    assert_eq!(tag.target, c1);
}

#[test]
fn status_reports_changes() {
    let sb = Sandbox::new();
    sb.commit_file("tracked.txt", "v1\n", "one");
    sb.commit_file("rename-me.txt", "same content for rename\n", "two");
    sb.write("tracked.txt", "v2\n");
    sb.write("new dir/ünï.txt", "x");
    sb.write("staged.txt", "s");
    sb.git(&["add", "staged.txt"]);
    sb.git(&["mv", "rename-me.txt", "renamed.txt"]);

    let repo = open(&sb);
    let st = repo.status().unwrap();
    assert_eq!(st.branch.head.as_deref(), Some("main"));
    let get = |p: &str| {
        st.entries
            .iter()
            .find(|e| e.path == p)
            .unwrap_or_else(|| panic!("{p}: {st:#?}"))
    };
    assert_eq!(get("tracked.txt").worktree, FileState::Modified);
    assert!(get("new dir/ünï.txt").is_untracked());
    assert_eq!(get("staged.txt").index, FileState::Added);
    let renamed = get("renamed.txt");
    assert_eq!(renamed.index, FileState::Renamed);
    assert_eq!(renamed.orig_path.as_deref(), Some("rename-me.txt"));
}

#[test]
fn status_reports_conflicts() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "base\n", "base");
    sb.checkout_new("other");
    sb.commit_file("f.txt", "theirs\n", "theirs");
    sb.checkout("main");
    sb.commit_file("f.txt", "ours\n", "ours");
    let mut cmd = std::process::Command::new("git");
    cmd.args(["merge", "other"]).current_dir(sb.path());
    sb.apply_env(&mut cmd);
    assert!(!cmd.output().unwrap().status.success());

    let st = open(&sb).status().unwrap();
    assert_eq!(st.entries.len(), 1);
    assert!(st.entries[0].is_conflicted());
    assert_eq!(st.entries[0].short_code(), "UU");
}

#[test]
fn checkout_history_and_ref_activity() {
    let sb = Sandbox::new();
    sb.commit("one");
    sb.tick();
    sb.checkout_new("feature");
    sb.tick();
    sb.checkout("main");
    let repo = open(&sb);
    let checkouts = repo.checkouts(50).unwrap();
    assert_eq!(checkouts.len(), 2);
    assert_eq!(checkouts[0].to, "main");
    assert_eq!(checkouts[1].to, "feature");
    assert!(checkouts[0].time >= checkouts[1].time);

    let t = repo.ref_activity("refs/heads/main").expect("main has a reflog");
    assert!(t > BASE_TIME);
    assert_eq!(repo.ref_activity("refs/heads/nope"), None);
}

#[test]
fn command_log_records_calls() {
    let sb = Sandbox::new();
    sb.commit("one");
    let repo = open(&sb);
    repo.head().unwrap();
    let entries = repo.git().log().entries();
    assert!(
        entries
            .iter()
            .any(|e| e.args.starts_with("rev-parse") && e.success == Some(true))
    );
    assert!(entries.iter().all(|e| e.duration.is_some()));
}

#[test]
fn author_names_respect_mailmap() {
    let sb = Sandbox::new();
    sb.commit("one");
    sb.write(
        ".mailmap",
        "Real Name <real@example.com> Test User <test@example.com>\n",
    );

    let repo = open(&sb);
    let commits = repo.commits(&LogScope::All, 100).unwrap();
    assert_eq!(commits[0].author_name, "Real Name");
    assert_eq!(commits[0].author_email, "real@example.com");
}

#[test]
fn commit_files_and_their_diffs() {
    let sb = Sandbox::new();
    sb.write("keep.txt", "one\n").write("gone.txt", "bye\n");
    sb.write(
        "old name.txt",
        "a fairly long line so rename detection has something to match\n",
    );
    sb.git(&["add", "."]);
    sb.commit_staged("base");
    sb.write("keep.txt", "two\n").write("new.txt", "hello\n");
    sb.git(&["rm", "-q", "gone.txt"]);
    sb.git(&["mv", "old name.txt", "new name.txt"]);
    sb.git(&["add", "."]);
    let oid = sb.commit_staged("change things");
    let repo = open(&sb);

    let files = repo.commit_files(&oid).unwrap();
    let mut summary: Vec<_> = files
        .iter()
        .map(|f| (f.state, f.orig_path.as_deref(), f.path.as_str()))
        .collect();
    summary.sort_by_key(|(_, _, p)| *p);
    assert_eq!(
        summary,
        vec![
            (FileState::Deleted, None, "gone.txt"),
            (FileState::Modified, None, "keep.txt"),
            (FileState::Renamed, Some("old name.txt"), "new name.txt"),
            (FileState::Added, None, "new.txt"),
        ]
    );

    let keep = files.iter().find(|f| f.path == "keep.txt").unwrap();
    let diff = repo.commit_file_diff(&oid, keep).unwrap();
    assert!(diff.contains("-one") && diff.contains("+two"), "{diff}");
    assert!(!diff.contains("hello"), "{diff}");
    let renamed = files.iter().find(|f| f.state == FileState::Renamed).unwrap();
    let diff = repo.commit_file_diff(&oid, renamed).unwrap();
    assert!(diff.contains("rename from old name.txt"), "{diff}");

    // The root commit lists everything as added.
    let root = repo.commit_files(&sb.rev_parse("HEAD~1")).unwrap();
    assert!(root.iter().all(|f| f.state == FileState::Added), "{root:?}");
}

#[test]
fn merged_refs_and_authors_for_base_comparison() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("done");
    sb.commit("done work");
    sb.checkout("main");
    sb.merge("done");
    sb.checkout_new("open");
    sb.commit("open work");
    sb.checkout("main");
    sb.add_remote("origin");
    sb.write(
        ".mailmap",
        "Real Name <real@example.com> Test User <test@example.com>\n",
    );

    let repo = open(&sb);
    let merged = repo
        .merged_refs("refs/remotes/origin/main", "refs/remotes/origin/")
        .unwrap();
    assert!(merged.contains("refs/remotes/origin/done"), "{merged:?}");
    assert!(merged.contains("refs/remotes/origin/main"));
    assert!(!merged.contains("refs/remotes/origin/open"));

    let authors = repo.ref_authors("refs/remotes/origin/").unwrap();
    assert_eq!(authors["refs/remotes/origin/open"], "Real Name");

    assert_eq!(repo.base_branch_setting(), None);
    repo.set_base_branch(Some("origin/open")).unwrap();
    assert_eq!(repo.base_branch_setting().as_deref(), Some("origin/open"));
    repo.set_base_branch(None).unwrap();
    repo.set_base_branch(None).unwrap();
    assert_eq!(repo.base_branch_setting(), None);
}

#[test]
fn branch_history_from_fork_to_merge() {
    let sb = Sandbox::new();
    sb.commit("m1");
    let fork = sb.commit("m2");
    sb.checkout_new("feat");
    let f1 = sb.commit("f1");
    sb.checkout("main");
    sb.commit("m3");
    sb.checkout("feat");
    let sync = sb.merge("main");
    sb.checkout_new("sub");
    sb.commit("s1");
    sb.checkout("feat");
    let sub_merge = sb.merge("sub");
    sb.checkout("main");
    sb.commit("m4");
    let merge = sb.merge("feat");
    sb.checkout_new("open");
    let o1 = sb.commit("o1");
    sb.checkout("main");
    let base_tip = sb.commit("m5");

    let repo = open(&sb);
    let summaries = |h: &BranchHistory| h.commits.iter().map(|c| c.summary.clone()).collect::<Vec<_>>();

    // Only the branch's first-parent commits: neither `main`'s commits it
    // pulled in nor `sub`'s.
    let h = repo.branch_history(&sb.rev_parse("feat"), "main", 100).unwrap();
    assert_eq!(h.merge.as_deref(), Some(merge.as_str()));
    assert_eq!(h.fork.as_deref(), Some(fork.as_str()));
    assert_eq!(
        summaries(&h),
        vec![
            "Merge branch 'feat'",
            "Merge branch 'sub' into feat",
            "Merge branch 'main' into feat",
            "f1",
            "m2"
        ]
    );
    assert_eq!(h.commits[0].parents, vec![fork.clone(), sub_merge.clone()]);
    assert_eq!(h.commits[1].parents, vec![sync.clone()], "sub's side is dropped");
    assert_eq!(h.commits[2].parents, vec![f1.clone()], "main's side is dropped");
    assert_eq!(h.commits[3].parents, vec![fork.clone()]);
    assert!(h.commits[4].parents.is_empty());

    // Unmerged: the base's tip, joined to the fork.
    let h = repo.branch_history(&o1, "main", 100).unwrap();
    assert_eq!(h.merge, None);
    assert_eq!(h.fork.as_deref(), Some(merge.as_str()));
    assert_eq!(summaries(&h), vec!["m5", "o1", "Merge branch 'feat'"]);
    assert_eq!(h.commits[0].oid, base_tip);
    assert_eq!(h.commits[0].parents, vec![merge.clone()]);

    // A branch with nothing of its own left is just its tip.
    let h = repo.branch_history(&base_tip, "main", 100).unwrap();
    assert_eq!(summaries(&h), vec!["m5"]);
    assert_eq!((h.fork, h.merge), (None, None));

    // Hitting the limit leaves the fork unknown.
    let h = repo.branch_history(&sb.rev_parse("feat"), "main", 2).unwrap();
    assert_eq!(h.fork, None);
    assert_eq!(h.commits.len(), 3);
    assert_eq!(h.commits[0].parents, vec![sub_merge]);
}
