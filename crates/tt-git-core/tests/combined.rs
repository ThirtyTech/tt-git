//! Reads done in one git command that used to take several must give what
//! the separate commands give.

use std::path::PathBuf;

use tt_git_core::{FileState, GitCli, RefKind, Repo};
use tt_git_testkit::Sandbox;

fn open(sb: &Sandbox) -> Repo {
    Repo::discover(sb.path(), &GitCli::new(".").with_env(sb.env())).expect("discover")
}

#[test]
fn remote_config_lists_what_git_remote_does() {
    let sb = Sandbox::new();
    sb.commit("base");
    let repo = open(&sb);
    let none = repo.remote_config().unwrap();
    assert_eq!(none.remotes, repo.remotes().unwrap());
    assert!(none.remotes.is_empty());
    assert_eq!(none.base_branch, None);

    sb.add_remote("origin");
    for (key, value) in [
        ("remote.dotted.name.url", "https://example.com/d.git"),
        ("remote.a/b.url", "https://example.com/ab.git"),
        ("remote.fetchonly.fetch", "+refs/heads/*:refs/remotes/fetchonly/*"),
        ("remote.pushonly.pushurl", "https://example.com/p.git"),
        ("remote.Upper.url", "https://example.com/u.git"),
        ("remote.odd.skipDefaultUpdate", "true"),
        ("remote.pushDefault", "origin"),
    ] {
        sb.git(&["config", key, value]);
    }
    // Section and key names in any case, as a hand-edited file may have.
    let config = sb.path().join(".git/config");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str("[REMOTE \"shouty\"]\n\tURL = https://example.com/s.git\n");
    std::fs::write(&config, text).unwrap();
    let got = repo.remote_config().unwrap();
    let want = repo.remotes().unwrap();
    assert_eq!(got.remotes, want);
    assert!(
        want.contains(&"dotted.name".to_string()) && want.contains(&"shouty".to_string()),
        "{want:?}"
    );
    assert_eq!(got.base_branch, repo.base_branch_setting());

    for setting in ["origin/main", "  spaced  "] {
        sb.git(&["config", "tt-git.baseBranch", setting]);
        assert_eq!(repo.remote_config().unwrap().base_branch, repo.base_branch_setting());
    }
    // Several values: the last one counts.
    sb.git(&["config", "--add", "tt-git.basebranch", "main"]);
    assert_eq!(repo.base_branch_setting().as_deref(), Some("main"));
    assert_eq!(repo.remote_config().unwrap().base_branch.as_deref(), Some("main"));
}

#[test]
fn ref_authors_from_the_refs_load_match_their_own_for_each_ref() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("feature");
    sb.git(&[
        "-c",
        "user.name=Other Person",
        "-c",
        "user.email=other@example.com",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "theirs",
    ]);
    sb.checkout("main");
    sb.git(&["tag", "-a", "v1", "-m", "release"]);
    sb.add_remote("origin");
    sb.write(
        ".mailmap",
        "Real Name <real@example.com> Test User <test@example.com>\n",
    );

    let repo = open(&sb);
    let (refs, authors) = repo.refs_with_authors(&repo.remotes().unwrap()).unwrap();
    assert_eq!(refs, repo.refs().unwrap());
    for prefix in [
        "refs/heads/",
        "refs/remotes/origin/",
        "refs/heads/fea",
        "refs/heads/feature",
    ] {
        assert_eq!(
            repo.ref_authors_of(&authors, prefix).unwrap(),
            repo.ref_authors(prefix).unwrap(),
            "{prefix}"
        );
    }
    let heads = repo.ref_authors_of(&authors, "refs/heads/").unwrap();
    assert_eq!(heads["refs/heads/main"], "Real Name");
    assert_eq!(heads["refs/heads/feature"], "Other Person");
}

#[test]
fn a_contact_git_reads_as_an_option_still_leaves_its_batch_unmapped() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.checkout_new("dash");
    sb.git(&[
        "-c",
        "user.name=-Leading Dash",
        "-c",
        "user.email=dash@example.com",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "dash",
    ]);
    sb.checkout("main");
    sb.write(
        ".mailmap",
        "Real Name <real@example.com> Test User <test@example.com>\n",
    );
    let repo = open(&sb);
    // Contacts used to go to check-mailmap as arguments, where this one is
    // an unknown option that fails the whole batch.
    let failed = repo.git().run(&[
        "check-mailmap",
        "-Leading Dash <dash@example.com>",
        "Test User <test@example.com>",
    ]);
    assert!(failed.is_err());
    let (_, authors) = repo.refs_with_authors(&[]).unwrap();
    let heads = repo.ref_authors_of(&authors, "refs/heads/").unwrap();
    assert_eq!(heads["refs/heads/main"], "Test User");
    assert_eq!(heads["refs/heads/dash"], "-Leading Dash");
}

#[test]
fn mailmap_over_stdin_takes_any_amount_of_output() {
    let sb = Sandbox::new();
    sb.commit("base");
    let git = GitCli::new(sb.path()).with_env(sb.env());
    // Far more than a pipe holds, so writing it all before reading the
    // output would deadlock.
    let input: String = (0..20_000)
        .map(|i| format!("Person {i} <p{i}@example.com>\n"))
        .collect();
    let out = git
        .run_with_stdin(&["check-mailmap", "--stdin"], Some(input.as_bytes()))
        .unwrap();
    assert_eq!(out.stdout_str(), input);
}

#[test]
fn ahead_behind_many_matches_one_at_a_time() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.git(&["tag", "v0"]);
    sb.checkout_new("release");
    sb.commit("release fix");
    sb.git(&["tag", "-a", "v1.0.1", "-m", "annotated"]);
    sb.commit("release fix 2");
    sb.git(&["tag", "v1.0.2"]);
    sb.git(&["tag", "-a", "nested-outer", "-m", "outer", "v1.0.1"]);
    sb.checkout("main");
    sb.commit("main 1");
    sb.commit("main 2");
    let repo = open(&sb);
    let refs = repo.refs().unwrap();
    let tags: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Tag).collect();
    assert_eq!(tags.len(), 4);
    let pairs: Vec<(&str, &str)> = tags.iter().map(|t| (t.full_name.as_str(), t.target.as_str())).collect();
    let many = repo.ahead_behind_many("refs/heads/main", &pairs).expect("git 2.41+");
    assert_eq!(many.len(), tags.len());
    for tag in &tags {
        let one = repo.ahead_behind("refs/heads/main", &tag.target).unwrap();
        assert_eq!(many[&tag.full_name], one, "{}", tag.full_name);
    }

    // A tag that moved since the refs were read is left out.
    let v0 = tags.iter().find(|t| t.short_name == "v0").unwrap();
    let main = sb.rev_parse("main");
    let stale = repo
        .ahead_behind_many("refs/heads/main", &[(v0.full_name.as_str(), main.as_str())])
        .unwrap();
    assert!(stale.is_empty());

    // A tag of a tree can't be compared (one at a time it's an error),
    // so it's left out, and doesn't stop the others.
    let tree = sb.rev_parse("main^{tree}");
    sb.git(&["tag", "tree-tag", &tree]);
    assert!(repo.ahead_behind("refs/heads/main", &tree).is_err());
    let v0_target = v0.target.as_str();
    let mixed = repo
        .ahead_behind_many(
            "refs/heads/main",
            &[
                ("refs/tags/tree-tag", tree.as_str()),
                (v0.full_name.as_str(), v0_target),
            ],
        )
        .unwrap_or_default();
    assert!(!mixed.contains_key("refs/tags/tree-tag"));
    if let Some(d) = mixed.get(&v0.full_name) {
        assert_eq!(*d, repo.ahead_behind("refs/heads/main", v0_target).unwrap());
    }
}

#[test]
fn status_and_ignored_dirs_has_the_plain_status() {
    let sb = Sandbox::new();
    sb.write(".gitignore", "target/\n*.o\nbuild/\n/logs\n");
    sb.write("tracked-build/keep.txt", "x");
    sb.write("tracked-build/.gitignore", "*\n");
    sb.git(&["add", "-f", ".gitignore", "tracked-build/keep.txt"]);
    sb.commit_staged("base");
    sb.write("target/debug/a.o", "o");
    sb.write("target/debug/deep/b", "b");
    sb.write("src/x.o", "o");
    sb.write("src/new.rs", "new");
    sb.write("objs/only.o", "o");
    sb.write("nested/build/out", "out");
    sb.write("logs/today", "log");
    sb.write("tracked-build/keep.txt", "changed");
    sb.write("tracked-build/scratch", "ignored file in a dir with tracked files");

    let repo = open(&sb);
    let (status, mut dirs) = repo.status_and_ignored_dirs().unwrap();
    assert_eq!(status, repo.status().unwrap());
    assert!(status.entries.iter().all(|e| e.index != FileState::Ignored));
    dirs.sort();
    // Not objs/ (all of it ignored, but not itself) or tracked-build/
    // (it has tracked files).
    assert_eq!(dirs, ["logs", "nested/build", "target"]);
}

#[test]
fn ignore_rule_files_cover_config_excludes_and_includes() {
    let sb = Sandbox::new();
    sb.commit("base");
    let repo = open(&sb);
    let files = repo.ignore_rules().unwrap().expect("all located").files;
    let home = sb.home();
    let has = |files: &[PathBuf], p: &PathBuf| files.iter().any(|f| f == p);
    for p in [
        home.join(".gitconfig"),
        home.join(".config/git/ignore"),
        repo.common_dir().join("config"),
        repo.git_dir().join("info/exclude"),
        repo.git_dir().join("HEAD"),
    ] {
        assert!(has(&files, &p), "{} not in {files:?}", p.display());
    }

    sb.git(&["config", "core.excludesFile", "~/my-ignores"]);
    sb.git(&["config", "include.path", "extra.inc"]);
    sb.git(&["config", "includeIf.onbranch:main.path", "~/branch.inc"]);
    let files = repo.ignore_rules().unwrap().expect("all located").files;
    assert!(has(&files, &home.join("my-ignores")), "{files:?}");
    // Relative to the file the include is in.
    assert!(has(&files, &repo.common_dir().join("extra.inc")), "{files:?}");
    assert!(has(&files, &home.join("branch.inc")), "{files:?}");

    sb.git(&["config", "core.excludesFile", "%(prefix)/etc/ignore"]);
    assert_eq!(repo.ignore_rules().unwrap(), None);
}

#[test]
fn ignore_rules_cover_the_system_config_where_git_looks_for_it() {
    let sb = Sandbox::new();
    sb.commit("base");
    // Not there: creating it could still change what's ignored.
    let system = sb.root().join("no-such-dir").join("gitconfig");
    let git = GitCli::new(".").with_env(sb.env()).with_env([
        ("GIT_CONFIG_NOSYSTEM", std::ffi::OsString::from("0")),
        ("GIT_CONFIG_SYSTEM", system.clone().into_os_string()),
    ]);
    let repo = Repo::discover(sb.path(), &git).expect("discover");
    let files = repo.ignore_rules().unwrap().expect("all located").files;
    assert!(files.contains(&system), "{files:?}");
}

#[test]
fn ignore_rules_say_when_status_may_use_the_untracked_cache() {
    let sb = Sandbox::new();
    sb.commit("base");
    let repo = open(&sb);
    let cache = || repo.ignore_rules().unwrap().expect("all located").untracked_cache;
    assert!(!cache());
    // Not with -uall unless that's the configured mode too.
    sb.git(&["config", "core.untrackedCache", "true"]);
    assert!(!cache());
    sb.git(&["config", "status.showUntrackedFiles", "all"]);
    assert!(cache());
    // Unset or `keep`, it's used if the index has one.
    sb.git(&["config", "core.untrackedCache", "keep"]);
    assert!(cache());
    sb.git(&["config", "--unset", "core.untrackedCache"]);
    assert!(cache());
    for off in ["false", "off", "0"] {
        sb.git(&["config", "core.untrackedCache", off]);
        assert!(!cache(), "{off}");
    }
}
