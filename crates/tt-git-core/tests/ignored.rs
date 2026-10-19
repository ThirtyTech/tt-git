//! `IgnoreRules` must hide exactly the changes git status can't show.

use std::path::PathBuf;
use std::process::Command;

use tt_git_core::{GitCli, IgnoreRules, Repo};
use tt_git_testkit::Sandbox;

fn open(sb: &Sandbox) -> (Repo, PathBuf) {
    let repo = Repo::discover(sb.path(), &GitCli::new(".").with_env(sb.env())).expect("discover");
    let root = std::fs::canonicalize(repo.root()).unwrap();
    (repo, root)
}

/// Whether git ignores `rel` (and doesn't track it), by `git check-ignore`.
fn git_ignores(sb: &Sandbox, rel: &str) -> bool {
    let mut cmd = Command::new("git");
    cmd.current_dir(sb.path()).args(["check-ignore", "-q", "--", rel]);
    sb.apply_env(&mut cmd);
    let code = cmd.status().unwrap().code();
    assert!(matches!(code, Some(0) | Some(1)), "check-ignore {rel}: {code:?}");
    code == Some(0)
}

#[test]
fn hides_what_git_check_ignore_ignores() {
    let sb = Sandbox::new();
    sb.write(
        ".gitignore",
        "target/\n*.log\n!keep.log\n/build\ndocs/*.tmp\nnode_modules\n**/cache/**\n# comment\n\\#hash\nspace\\ \n",
    );
    sb.write("sub/.gitignore", "!debug.log\nlocal/\n*.o\n");
    sb.write("sub/deeper/.gitignore", "!important.o\n");
    sb.write("tracked-dir/.gitignore", "*\n!.gitignore\n");
    sb.write("tracked-dir/kept.txt", "tracked though ignored");
    sb.write("excludes", "*.bak\n");
    sb.git(&["config", "core.excludesFile", "excludes"]);
    sb.write(".git/info/exclude", "secret.txt\n!keep.bak\n");
    sb.git(&[
        "add",
        ".gitignore",
        "sub/.gitignore",
        "sub/deeper/.gitignore",
        "tracked-dir/.gitignore",
    ]);
    sb.git(&["add", "-f", "tracked-dir/kept.txt"]);
    sb.commit_staged("base");

    let paths = [
        "target/debug/a.o",
        "target/x",
        "a.log",
        "keep.log",
        "dir/b.log",
        "sub/debug.log",
        "sub/x.log",
        "sub/local/z",
        "local/z",
        "sub/a.o",
        "sub/deeper/a.o",
        "sub/deeper/important.o",
        "build/out.js",
        "src/build/out.js",
        "docs/a.tmp",
        "docs/sub/a.tmp",
        "node_modules/x/index.js",
        "sub/node_modules/y.js",
        "a/cache/b/c",
        "#hash",
        "space ",
        "secret.txt",
        "x.bak",
        "keep.bak",
        "tracked-dir/kept.txt",
        "tracked-dir/scratch",
        "src/main.rs",
    ];
    for p in paths {
        sb.write(p, "x");
    }
    let (repo, root) = open(&sb);
    let rules = IgnoreRules::load(&repo, &root).unwrap();
    let mut wrong = Vec::new();
    for p in paths {
        let (ours, git) = (rules.hides(&root.join(p)), git_ignores(&sb, p));
        if ours != git {
            wrong.push(format!("{p}: ours {ours}, git {git}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn directories_holding_tracked_files_are_not_hidden() {
    let sb = Sandbox::new();
    sb.write(".gitignore", "target/\n");
    sb.write("target/keep.txt", "x");
    sb.git(&["add", ".gitignore"]);
    sb.git(&["add", "-f", "target/keep.txt"]);
    sb.commit_staged("base");
    sb.write("target/debug/a.o", "o");
    let (repo, root) = open(&sb);
    let rules = IgnoreRules::load(&repo, &root).unwrap();
    assert!(rules.hides(&root.join("target/debug/a.o")));
    assert!(rules.hides(&root.join("target/debug")));
    assert!(!rules.hides(&root.join("target/keep.txt")));
    // Renamed or deleted as a whole, the tracked file goes with it.
    assert!(!rules.hides(&root.join("target")));
}

#[test]
fn a_gitignore_is_hidden_only_behind_an_ignored_directory() {
    let sb = Sandbox::new();
    sb.write(".gitignore", "target/\n.gitignore\n");
    sb.commit("base");
    let (repo, root) = open(&sb);
    let rules = IgnoreRules::load(&repo, &root).unwrap();
    // Ignored itself, but git still reads it.
    assert!(!rules.hides(&root.join(".gitignore")));
    assert!(rules.hides(&root.join("target/.gitignore")));
}

#[test]
fn case_follows_core_ignorecase() {
    let sb = Sandbox::new();
    sb.write(".gitignore", "target/\n");
    sb.commit("base");
    sb.write("Target/x", "x");
    for (ignorecase, hidden) in [("false", false), ("true", true)] {
        sb.git(&["config", "core.ignorecase", ignorecase]);
        let (repo, root) = open(&sb);
        let rules = IgnoreRules::load(&repo, &root).unwrap();
        assert_eq!(
            rules.hides(&root.join("Target/x")),
            hidden,
            "core.ignorecase={ignorecase}"
        );
    }
}

#[test]
fn rules_go_stale_when_a_file_outside_the_tree_changes() {
    let sb = Sandbox::new();
    sb.commit("base");
    let (repo, root) = open(&sb);
    let rules = IgnoreRules::load(&repo, &root).unwrap();
    assert!(!rules.stale());
    std::fs::write(repo.common_dir().join("info/exclude"), "x\n").unwrap();
    assert!(rules.stale());

    let rules = IgnoreRules::load(&repo, &root).unwrap();
    std::fs::create_dir_all(sb.home().join(".config/git")).unwrap();
    std::fs::write(sb.home().join(".config/git/ignore"), "*.tmp\n").unwrap();
    assert!(rules.stale(), "the default excludes file appearing");

    let rules = IgnoreRules::load(&repo, &root).unwrap();
    sb.git(&["config", "core.excludesFile", "~/mine"]);
    assert!(rules.stale(), "the repo config changing");
}

#[test]
fn patterns_read_differently_from_git_fail_to_load() {
    let sb = Sandbox::new();
    sb.write(".gitignore", "*.py[cod]\n[]x]\n\\[literal\n");
    sb.commit("base");
    let (repo, root) = open(&sb);
    assert!(
        IgnoreRules::load(&repo, &root).is_ok(),
        "closed classes and escapes are fine"
    );
    // git matches an unclosed bracket against nothing; the matcher would
    // take it literally.
    sb.write(".gitignore", "[\nfoo[bar\n");
    sb.write("[", "x");
    assert!(!git_ignores(&sb, "["));
    assert!(IgnoreRules::load(&repo, &root).is_err());
}
