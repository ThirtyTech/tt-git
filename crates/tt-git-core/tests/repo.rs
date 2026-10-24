use tt_git_core::{
    BranchHistory, FileState, GitCli, GitError, Head, LogFilter, LogScope, Recency, RefKind, Repo, message_prefixes,
};
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
    assert_eq!(repo.start_prefix(), Some("a/b/"));
    assert_eq!(open(&sb).start_prefix(), None, "none at the root");
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
    let ids: Vec<String> = all.iter().map(|c| c.oid.to_string()).collect();
    assert_eq!(ids.len(), 5);
    assert_eq!(ids[0], side);
    let pos = |id: &str| ids.iter().position(|x| *x == id).unwrap();
    // Children before parents.
    assert!(pos(&merge) < pos(&m1) && pos(&merge) < pos(&f1));
    assert!(pos(&m1) < pos(&base) && pos(&f1) < pos(&base));

    let merge_commit = &all[pos(&merge)];
    assert_eq!(*merge_commit.parents, vec![m1.clone(), f1.clone()]);
    assert_eq!(&*merge_commit.author_name, "Test User");
    assert!(merge_commit.commit_time > BASE_TIME);

    // Current-branch scope excludes "side".
    let head_only = repo.commits(&LogScope::Head { upstream: None }, 100).unwrap();
    assert_eq!(head_only.len(), 4);
    assert!(head_only.iter().all(|c| c.oid != side));

    // Limit is honoured.
    assert_eq!(repo.commits(&LogScope::All, 2).unwrap().len(), 2);

    // Streaming from an offset continues where a smaller limit stopped.
    for (skip, limit) in [(2, 4), (2, 100), (0, 3), (5, 100), (4, 2)] {
        let mut rest = Vec::new();
        repo.stream_commits_from(&LogScope::All, skip, limit, |c| {
            rest.push(c.oid.to_string());
            true
        })
        .unwrap();
        let want: Vec<String> = ids.iter().take(limit).skip(skip).cloned().collect();
        assert_eq!(rest, want, "skip {skip}, limit {limit}");
    }
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
    assert_eq!(&*with[0].summary, "incoming");
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
fn remote_ref_activity_counts_pushes_not_fetches() {
    let sb = Sandbox::new();
    sb.commit("one");
    let a = "1".repeat(40);
    let b = "2".repeat(40);
    let line = |t: i64, msg: &str| format!("{a} {b} Test User <t@e.com> {t} +0000\t{msg}\n");
    sb.write(
        ".git/logs/refs/remotes/origin/pushed",
        &[
            line(1_700_000_000, "fetch: fast-forward"),
            line(1_700_000_100, "update by push"),
            line(1_700_000_200, "pull --no-edit: fast-forward"),
            line(1_700_000_300, "fetch --all --no-write-fetch-head: fast-forward"),
        ]
        .concat(),
    );
    sb.write(
        ".git/logs/refs/remotes/origin/fetched",
        &[
            line(1_700_000_000, "clone: from x"),
            line(1_700_000_300, "fetch: fast-forward"),
        ]
        .concat(),
    );
    let repo = open(&sb);
    assert_eq!(repo.ref_activity("refs/remotes/origin/pushed"), Some(1_700_000_100));
    assert_eq!(repo.ref_activity("refs/remotes/origin/fetched"), None);
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
    assert_eq!(&*commits[0].author_name, "Real Name");
    assert_eq!(&*commits[0].author_email, "real@example.com");
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
    let summaries = |h: &BranchHistory| h.commits.iter().map(|c| c.summary.to_string()).collect::<Vec<_>>();

    // Only the branch's first-parent commits: neither `main`'s commits it
    // pulled in nor `sub`'s.
    let h = repo.branch_history(&sb.rev_parse("feat"), "main", 100).unwrap();
    assert_eq!(h.merge.map(|o| o.to_string()).as_deref(), Some(merge.as_str()));
    assert_eq!(h.fork.map(|o| o.to_string()).as_deref(), Some(fork.as_str()));
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
    assert_eq!(*h.commits[0].parents, vec![fork.clone(), sub_merge.clone()]);
    assert_eq!(*h.commits[1].parents, vec![sync.clone()], "sub's side is dropped");
    assert_eq!(*h.commits[2].parents, vec![f1.clone()], "main's side is dropped");
    assert_eq!(*h.commits[3].parents, vec![fork.clone()]);
    assert!(h.commits[4].parents.is_empty());

    // Unmerged: the base's tip, joined to the fork.
    let h = repo.branch_history(&o1, "main", 100).unwrap();
    assert_eq!(h.merge, None);
    assert_eq!(h.fork.map(|o| o.to_string()).as_deref(), Some(merge.as_str()));
    assert_eq!(summaries(&h), vec!["m5", "o1", "Merge branch 'feat'"]);
    assert_eq!(h.commits[0].oid, base_tip);
    assert_eq!(*h.commits[0].parents, vec![merge.clone()]);

    // A branch with nothing of its own left is just its tip.
    let h = repo.branch_history(&base_tip, "main", 100).unwrap();
    assert_eq!(summaries(&h), vec!["m5"]);
    assert_eq!((h.fork, h.merge), (None, None));

    // Hitting the limit leaves the fork unknown.
    let h = repo.branch_history(&sb.rev_parse("feat"), "main", 2).unwrap();
    assert_eq!(h.fork, None);
    assert_eq!(h.commits.len(), 3);
    assert_eq!(*h.commits[0].parents, vec![sub_merge]);
}

/// Commits a change to `file` as `author` (`Name <email>`).
fn commit_as(sb: &Sandbox, author: &str, file: &str, msg: &str) -> String {
    sb.write(file, &format!("{msg}\n"));
    sb.git(&["add", "--", file]);
    sb.tick();
    sb.git(&["commit", "-q", &format!("--author={author}"), "-m", msg]);
    sb.rev_parse("HEAD")
}

#[test]
fn filters_commits_by_author_and_path() {
    let sb = Sandbox::new();
    let alice = "Alice <alice@example.com>";
    let a1 = commit_as(&sb, alice, "src/a.txt", "a1");
    sb.commit_file("b.txt", "b1\n", "b1");
    sb.checkout_new("side");
    let a2 = commit_as(&sb, alice, "src/a.txt", "a2 side");
    sb.commit_file("b.txt", "b2\n", "b2 side");
    sb.checkout("main");
    let a3 = commit_as(&sb, alice, "c.txt", "a3");
    let repo = open(&sb);
    let oids = |commits: &[tt_git_core::Commit]| commits.iter().map(|c| c.oid.to_string()).collect::<Vec<_>>();

    let by_alice = LogFilter {
        author: Some(alice.into()),
        path: None,
        message: None,
    };
    let commits = repo.filtered_commits(&LogScope::All, &by_alice, 100).unwrap();
    let mut got = oids(&commits);
    got.sort();
    let mut want = vec![a1.clone(), a2.clone(), a3.clone()];
    want.sort();
    assert_eq!(got, want);
    // Parents are the real ones, even outside the result (a2's is the b1
    // commit), so a merge still reads as one.
    let b1 = sb.rev_parse("main~1");
    let a2_commit = commits.iter().find(|c| c.oid == a2).unwrap();
    assert_eq!(*a2_commit.parents, vec![b1]);

    // A path filter rewrites parents to the previous commit that touched it.
    let in_src = LogFilter {
        author: None,
        path: Some("src".into()),
        message: None,
    };
    let commits = repo.filtered_commits(&LogScope::All, &in_src, 100).unwrap();
    assert_eq!(oids(&commits), vec![a2.clone(), a1.clone()]);
    assert_eq!(*commits[0].parents, vec![a1.clone()]);

    let both = LogFilter {
        author: Some(alice.into()),
        path: Some("c.txt".into()),
        message: None,
    };
    assert_eq!(
        oids(&repo.filtered_commits(&LogScope::All, &both, 100).unwrap()),
        vec![a3]
    );
    let current = LogScope::Head { upstream: None };
    assert_eq!(oids(&repo.filtered_commits(&current, &in_src, 100).unwrap()), vec![a1]);
    let nobody = LogFilter {
        author: Some("Bob <bob@example.com>".into()),
        path: None,
        message: None,
    };
    assert!(repo.filtered_commits(&LogScope::All, &nobody, 100).unwrap().is_empty());

    assert_eq!(
        repo.authors().unwrap(),
        vec![(alice.to_string(), 3), ("Test User <test@example.com>".to_string(), 2)]
    );
    assert_eq!(repo.tracked_files().unwrap(), vec!["b.txt", "c.txt", "src/a.txt"]);
}

#[test]
fn filters_commits_by_message_regex() {
    let sb = Sandbox::new();
    let alice = "Alice <alice@example.com>";
    let feat = commit_as(&sb, alice, "a.txt", "feat(graph): Filter by message");
    sb.commit_file(
        "b.txt",
        "b
",
        "fix: Not a feature

feat in the body doesn't count",
    );
    sb.checkout_new("side");
    let side = sb.commit_file(
        "c.txt",
        "c
",
        "Feat: capital, on a branch",
    );
    sb.checkout("main");
    let docs = commit_as(&sb, "Bob <bob@example.com>", "d.txt", "docs: Describe it");
    let repo = open(&sb);
    let matching = |author: Option<&str>, message: &str, limit: usize| {
        let filter = LogFilter {
            author: author.map(Into::into),
            path: None,
            message: Some(message.into()),
        };
        let mut got: Vec<String> = repo
            .filtered_commits(&LogScope::All, &filter, limit)
            .unwrap()
            .into_iter()
            .map(|c| c.oid.to_string())
            .collect();
        got.sort();
        got
    };
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };

    // Lowercase ignores case and matches the subject only.
    assert_eq!(matching(None, "^feat", 100), sorted(vec![feat.clone(), side.clone()]));
    // A capital makes it match case.
    assert_eq!(matching(None, "^Feat", 100), vec![side.clone()]);
    assert_eq!(
        matching(None, "^(feat|docs)", 100),
        sorted(vec![feat.clone(), side, docs])
    );
    // With an author, both must match.
    assert_eq!(matching(Some(alice), "^feat", 100), vec![feat]);
    // The limit counts matching commits.
    assert_eq!(matching(None, "^feat", 1).len(), 1);

    let bad = LogFilter {
        message: Some("(".into()),
        ..LogFilter::default()
    };
    let err = repo.filtered_commits(&LogScope::All, &bad, 100).unwrap_err();
    assert!(err.to_string().starts_with("invalid regex"), "{err}");
}

#[test]
fn message_prefixes_ranks_subject_prefixes_by_use() {
    let summaries = [
        "docs(graph): Describe it",
        "feat(blame)!: Break it",
        "Fix: capital",
        "fix: Mend it",
        "feat: Add it",
        "Merge branch 'side'",
        "c++: Not a word",
        "123: Starts with a digit",
        "wip(unclosed: no",
        "ci : space before the colon",
        "fix: Again",
    ];
    // Counts ignore case; ties keep the order first seen.
    assert_eq!(message_prefixes(summaries, 8), ["^fix", "^feat", "^docs"]);
    assert_eq!(message_prefixes(summaries, 2), ["^fix", "^feat"]);
    assert_eq!(message_prefixes(["a.b: dotted"], 8), Vec::<String>::new());
    assert_eq!(message_prefixes(["build-sys: x"], 8), ["^build-sys"]);
}

#[test]
fn author_filter_includes_every_mailmapped_account() {
    let sb = Sandbox::new();
    // One person committing from three accounts, on two branches.
    let work = commit_as(&sb, "Jon Sheely <jon@work.com>", "a.txt", "from work");
    sb.checkout_new("side");
    let home = commit_as(&sb, "jsheely <jon@home.net>", "a.txt", "from home");
    let github = commit_as(&sb, "Jonathan <1+js@users.noreply.github.com>", "b.txt", "from github");
    sb.checkout("main");
    commit_as(&sb, "Someone Else <else@example.com>", "c.txt", "someone else");
    // Both mailmap forms: by email alone, and by name and email.
    sb.write(
        ".mailmap",
        "Jon Sheely <jon@work.com> <jon@home.net>\n\
         Jon Sheely <jon@work.com> Jonathan <1+js@users.noreply.github.com>\n",
    );
    let repo = open(&sb);

    // The picker lists the person once, counting every account.
    assert_eq!(
        repo.authors().unwrap(),
        vec![
            ("Jon Sheely <jon@work.com>".to_string(), 3),
            ("Someone Else <else@example.com>".to_string(), 1),
        ]
    );
    // And filtering by them finds the commits from all three.
    let filter = LogFilter {
        author: Some("Jon Sheely <jon@work.com>".into()),
        path: None,
        message: None,
    };
    let mut got: Vec<String> = repo
        .filtered_commits(&LogScope::All, &filter, 100)
        .unwrap()
        .into_iter()
        .map(|c| c.oid.to_string())
        .collect();
    got.sort();
    let mut want = vec![work, home, github];
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn ahead_behind_counts_both_sides() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("release");
    sb.commit("release fix");
    sb.git(&["tag", "v1.0.1"]);
    sb.checkout("main");
    sb.commit("main 1");
    sb.commit("main 2");
    let repo = open(&sb);
    let d = repo.ahead_behind("refs/heads/main", &sb.rev_parse("v1.0.1")).unwrap();
    assert_eq!((d.ahead, d.behind), (1, 2));
    let d = repo.ahead_behind("refs/heads/main", "main").unwrap();
    assert_eq!((d.ahead, d.behind), (0, 0));
}

#[test]
fn ref_fingerprint_changes_with_anything_a_log_starts_from() {
    let sb = Sandbox::new();
    let repo = open(&sb);
    let git_dir = sb.path().join(".git");
    let fp = || repo.ref_fingerprint().expect("files backend");
    let unborn = fp();
    let base = sb.commit("base");
    let main = sb.commit("main work");
    let before = fp();
    assert_ne!(unborn, before);

    // Pseudorefs and reflogs aren't walked.
    std::fs::write(git_dir.join("FETCH_HEAD"), format!("{main}\t\tbranch 'main' of x\n")).unwrap();
    std::fs::write(git_dir.join("ORIG_HEAD"), format!("{base}\n")).unwrap();
    assert_eq!(fp(), before);

    // A new ref on a commit already shown changes it, and deleting it
    // changes it back.
    sb.git(&["branch", "side", &base]);
    assert_ne!(fp(), before);
    sb.git(&["branch", "-q", "-D", "side"]);
    assert_eq!(fp(), before);

    // HEAD moving to a commit another ref already points at.
    sb.git(&["branch", "other", &main]);
    let with_other = fp();
    sb.git(&["switch", "-q", "other"]);
    assert_ne!(fp(), with_other);
    sb.git(&["switch", "-q", "main"]);
    assert_eq!(fp(), with_other);

    // Packed refs.
    sb.git(&["pack-refs", "--all"]);
    let packed = fp();
    sb.git(&["update-ref", "refs/heads/other", &base]);
    assert_ne!(fp(), packed);

    // Another worktree's detached HEAD isn't a ref, but --all walks it.
    let wt = sb.root().join("wt");
    sb.git(&["worktree", "add", "-q", "--detach", wt.to_str().unwrap(), &base]);
    let added = fp();
    sb.git_in(&wt, &["commit", "-q", "--allow-empty", "-m", "worktree only"]);
    assert_ne!(fp(), added);

    // The stash.
    let unstashed = fp();
    sb.write("dirty.txt", "x");
    sb.git(&["add", "dirty.txt"]);
    sb.git(&["stash", "-q"]);
    assert_ne!(fp(), unstashed);

    // Where shallow history stops.
    let deep = fp();
    std::fs::write(git_dir.join("shallow"), format!("{base}\n")).unwrap();
    assert_ne!(fp(), deep);
}

#[test]
fn relative_path_climbs_and_descends() {
    use tt_git_core::repo::relative_path;
    let sb = Sandbox::new();
    let side = sb.path().join(".worktrees/side");
    std::fs::create_dir_all(&side).unwrap();
    assert_eq!(relative_path(sb.path(), sb.path()), ".");
    assert_eq!(relative_path(&side, sb.path()), ".worktrees/side");
    assert_eq!(relative_path(sb.path(), &side), "../..");
    // git prints forward slashes, and a folder that's gone can't be resolved.
    let git_style = sb.path().to_string_lossy().replace('\\', "/");
    let gone = std::path::Path::new(&git_style).join(".worktrees/gone");
    assert_eq!(
        relative_path(&gone, std::path::Path::new(&git_style)),
        ".worktrees/gone"
    );
}

#[test]
fn disk_usage_sums_files_and_skips_nested_worktrees() {
    use tt_git_core::repo::disk_usage;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a"), [0u8; 100]).unwrap();
    std::fs::create_dir_all(dir.path().join("sub/deeper")).unwrap();
    std::fs::write(dir.path().join("sub/deeper/b"), [0u8; 23]).unwrap();
    std::fs::create_dir_all(dir.path().join(".worktrees/x")).unwrap();
    std::fs::write(dir.path().join(".worktrees/x/c"), [0u8; 1000]).unwrap();
    assert_eq!(disk_usage(dir.path(), &[]).unwrap(), 1123);
    let nested = dir.path().join(".worktrees/x");
    assert_eq!(disk_usage(dir.path(), &[nested]).unwrap(), 123);
    assert!(disk_usage(&dir.path().join("missing"), &[]).is_err());
}

#[test]
fn file_versions_step_through_the_commits_that_changed_the_file() {
    let sb = Sandbox::new();
    let one = sb.commit_file("old.txt", "a\nb\nc\n", "add old");
    let two = sb.commit_file("old.txt", "a\nB\nc\n", "edit b");
    sb.commit_file("other.txt", "x\n", "elsewhere");
    sb.git(&["mv", "old.txt", "new.txt"]);
    let moved = sb.commit_staged("rename");
    let four = sb.commit_file("new.txt", "a\nB\nC\n", "edit c");
    sb.commit_file("other.txt", "y\n", "elsewhere again");
    let repo = open(&sb);
    let at = |oid: &str, path: &str| Some((oid.to_string(), path.to_string()));

    // Back from a clean working tree: past HEAD's version (four's) to the one before.
    assert_eq!(
        repo.file_version_before(None, "new.txt").unwrap(),
        at(&moved, "new.txt")
    );
    // Back from a commit that didn't change the file goes past its version too.
    assert_eq!(
        repo.file_version_before(Some("HEAD"), "new.txt").unwrap(),
        at(&moved, "new.txt")
    );
    // Past the rename, to the old name.
    assert_eq!(
        repo.file_version_before(Some(&moved), "new.txt").unwrap(),
        at(&two, "old.txt")
    );
    assert_eq!(
        repo.file_version_before(Some(&two), "old.txt").unwrap(),
        at(&one, "old.txt")
    );
    assert_eq!(repo.file_version_before(Some(&one), "old.txt").unwrap(), None);

    // Forward again, toward the working tree, back over the rename.
    assert_eq!(
        repo.file_version_after(&one, "old.txt", None, "new.txt").unwrap(),
        at(&two, "old.txt")
    );
    assert_eq!(
        repo.file_version_after(&two, "old.txt", None, "new.txt").unwrap(),
        at(&moved, "new.txt")
    );
    // The next is the working tree's own version: nothing in between.
    assert_eq!(
        repo.file_version_after(&moved, "new.txt", None, "new.txt").unwrap(),
        None
    );
    // Toward a commit: stops short of its version.
    assert_eq!(
        repo.file_version_after(&one, "old.txt", Some(&moved), "new.txt")
            .unwrap(),
        at(&two, "old.txt")
    );
    assert_eq!(
        repo.file_version_after(&two, "old.txt", Some(&moved), "new.txt")
            .unwrap(),
        None
    );

    // With changes in the working tree, back goes to HEAD first, and
    // forward stops at HEAD's version on the way.
    sb.write("new.txt", "a\nB\nC\nd\n");
    assert_eq!(
        repo.file_version_before(None, "new.txt").unwrap(),
        Some(("HEAD".into(), "new.txt".into()))
    );
    assert_eq!(
        repo.file_version_after(&moved, "new.txt", None, "new.txt").unwrap(),
        at(&four, "new.txt")
    );
    // A file not yet committed has nothing before it.
    sb.write("fresh.txt", "new\n");
    assert_eq!(repo.file_version_before(None, "fresh.txt").unwrap(), None);
}

#[test]
fn file_recency_says_whether_a_version_is_heads() {
    let sb = Sandbox::new();
    let text = |last: &str| format!("one\ntwo\nthree\nfour\nfive\n{last}\n");
    let one = sb.commit_file("old.txt", &text("a"), "add old");
    let two = sb.commit_file("old.txt", &text("b"), "edit old");
    let other = sb.commit_file("other.txt", "x\n", "elsewhere");
    let repo = open(&sb);

    // A commit after the file's last change still has HEAD's version.
    assert_eq!(repo.file_recency("HEAD", "old.txt").unwrap(), Recency::Latest);
    assert_eq!(repo.file_recency(&two, "old.txt").unwrap(), Recency::Latest);
    assert_eq!(repo.file_recency(&other, "old.txt").unwrap(), Recency::Latest);
    assert_eq!(repo.file_recency(&one, "old.txt").unwrap(), Recency::Behind(1));

    // Renamed since, then changed under its new name: both count.
    sb.git(&["mv", "old.txt", "new.txt"]);
    sb.commit_staged("rename");
    sb.commit_file("new.txt", &text("c"), "edit new");
    assert_eq!(repo.file_recency(&one, "old.txt").unwrap(), Recency::Behind(3));
    assert_eq!(repo.file_recency(&two, "old.txt").unwrap(), Recency::Behind(2));

    // A version on another branch, not in HEAD's history.
    sb.git(&["checkout", "-q", "-b", "side", &two]);
    let side = sb.commit_file("old.txt", "side\n", "side edit");
    sb.git(&["checkout", "-q", "-"]);
    assert_eq!(repo.file_recency(&side, "old.txt").unwrap(), Recency::Elsewhere);
}

#[test]
fn blame_names_the_commit_behind_each_line() {
    let sb = Sandbox::new();
    let first = sb.commit_file("f.txt", "one\ntwo\nthree\n", "add f");
    let second = sb.commit_file("f.txt", "one\nTWO\nthree\n", "shout two");
    sb.write("f.txt", "one\nTWO\nthree\nfour\n");
    let repo = open(&sb);

    // The working tree: the new line isn't committed yet.
    let blame = repo.blame(None, "f.txt").unwrap();
    let who: Vec<_> = blame
        .lines
        .iter()
        .map(|l| (l.text.as_str(), blame.commits[l.commit].oid.clone()))
        .collect();
    assert_eq!(who[0], ("one", first.clone()));
    assert_eq!(who[1], ("TWO", second.clone()));
    assert_eq!(who[2], ("three", first.clone()));
    assert!(blame.commit_of(3).unwrap().is_uncommitted());

    let shout = blame.commit_of(1).unwrap();
    assert_eq!(shout.summary, "shout two");
    assert_eq!(shout.previous, Some((first.clone(), "f.txt".to_string())));
    assert_eq!(blame.lines[1].orig_line, 2);

    // As of a commit, and before it: where stepping back goes.
    let before = repo.blame(Some(&first), "f.txt").unwrap();
    assert_eq!(before.lines[1].text, "two");
    assert_eq!(before.commit_of(1).unwrap().previous, None);

    // When the file last changed: as of HEAD, as of the first commit, never.
    assert_eq!(repo.file_changed_at(None, "f.txt").unwrap(), Some(shout.author_time));
    let added = before.commit_of(1).unwrap().author_time;
    assert!(added < shout.author_time);
    assert_eq!(repo.file_changed_at(Some(&first), "f.txt").unwrap(), Some(added));
    assert_eq!(repo.file_changed_at(None, "missing.txt").unwrap(), None);

    // .git-blame-ignore-revs passes over a commit, to the one before.
    sb.write(".git-blame-ignore-revs", &format!("{second}\n"));
    let ignored = repo.blame(Some("HEAD"), "f.txt").unwrap();
    assert_eq!(ignored.commit_of(1).unwrap().oid, first);

    assert!(repo.blame(None, "missing.txt").is_err());
}

#[test]
fn merge_conflicts_without_touching_the_repo() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", "a\nb\nc\n", "base");
    sb.branch("clean").branch("conflict");
    let main = sb.commit_file("f.txt", "a\nmain\nc\n", "main");
    sb.checkout("clean");
    let clean = sb.commit_file("g.txt", "new\n", "clean");
    sb.checkout("conflict");
    sb.write("sub dir/n m.txt", "q\n");
    sb.git(&["add", "."]);
    let conflict = sb.commit_file("f.txt", "a\nconflict\nc\n", "conflict");
    sb.checkout("main");
    let repo = open(&sb);
    let objects = sb.git(&["count-objects", "-v"]);
    let got = repo
        .merge_conflicts(&main, &[&clean, &conflict, &main])
        .unwrap()
        .expect("git 2.40 or newer");
    assert_eq!(got[&clean], Vec::<String>::new());
    assert_eq!(got[&conflict], vec!["f.txt".to_string()]);
    assert_eq!(got[&main], Vec::<String>::new());
    // The merged trees went to a scratch directory, not the repository.
    assert_eq!(sb.git(&["count-objects", "-v"]), objects);
    assert_eq!(sb.git(&["status", "--porcelain"]), "");
    assert!(repo.merge_conflicts(&main, &[]).unwrap().unwrap().is_empty());
}

#[test]
fn merge_conflicts_name_every_conflicted_file() {
    let sb = Sandbox::new();
    sb.write("one.txt", "1\n").write("sub dir/two words.txt", "2\n");
    sb.git(&["add", "."]);
    sb.commit_staged("base");
    sb.checkout_new("topic");
    sb.write("one.txt", "topic\n").write("sub dir/two words.txt", "topic\n");
    sb.git(&["add", "."]);
    let topic = sb.commit_staged("topic");
    sb.checkout("main");
    sb.write("one.txt", "main\n").write("sub dir/two words.txt", "main\n");
    sb.git(&["add", "."]);
    let main = sb.commit_staged("main");
    let got = open(&sb).merge_conflicts(&main, &[&topic]).unwrap().unwrap();
    assert_eq!(
        got[&topic],
        vec!["one.txt".to_string(), "sub dir/two words.txt".to_string()]
    );
}
