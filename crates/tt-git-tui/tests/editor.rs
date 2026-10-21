//! `e` opens the selected file in the editor, at the line in view.

use tt_git_testkit::{Sandbox, TestDriver};
use tt_git_tui::editor::{EditorLaunch, abs_path};
use tt_git_tui::panes::{PaneId, StageSideId};
use tt_git_tui::{Action, Config};

const BASE: &str = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n";

/// `text` with line `n` (from 1) replaced.
fn with_line(text: &str, n: usize, new: &str) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    lines[n - 1] = new;
    lines.join("\n") + "\n"
}

fn driver(sb: &Sandbox) -> TestDriver {
    let cfg = Config::parse("[editor]\ncommand = \"ed {file} {line}\"\nterminal = true\n").unwrap();
    TestDriver::with_size(cfg, sb, 100, 30)
}

/// The editor `e` asked for, as (file relative to the repo, line).
fn launched(t: &mut TestDriver) -> (String, String) {
    let launch: EditorLaunch = t.app_mut().take_editor().expect("an editor launch");
    assert_eq!(launch.program, "ed");
    assert!(launch.terminal);
    let root = t.app().repo().root().to_path_buf();
    let file = launch.args[0].clone();
    let rel = std::path::Path::new(&file)
        .strip_prefix(&root)
        .unwrap_or_else(|_| panic!("{file} not under {}", root.display()))
        .to_string_lossy()
        .replace('\\', "/");
    assert_eq!(file, abs_path(&root, &rel).display().to_string());
    (rel, launch.args[1].clone())
}

fn select_status(t: &mut TestDriver, path: &str) {
    t.press("2");
    let idx = t
        .app()
        .status()
        .entries()
        .iter()
        .position(|e| e.path == path)
        .unwrap_or_else(|| panic!("{path} not in status"));
    t.press("g");
    for _ in 0..idx {
        t.press("j");
    }
    t.wait_idle();
}

#[test]
fn status_opens_the_selected_file_at_its_first_change() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", BASE, "base");
    sb.commit_file("s.txt", BASE, "base s");
    sb.write("f.txt", &with_line(BASE, 5, "five"));
    // s.txt: line 9 staged, then two lines added above it.
    sb.write("s.txt", &with_line(BASE, 9, "nine"));
    sb.git(&["add", "s.txt"]);
    sb.write("s.txt", &format!("a\nb\n{}", with_line(BASE, 9, "nine")));
    sb.write("new.txt", "a\nb\n");
    let mut t = driver(&sb);

    select_status(&mut t, "f.txt");
    t.press("e").wait_idle();
    assert_eq!(launched(&mut t), ("f.txt".into(), "5".into()));

    // The unstaged change comes first.
    select_status(&mut t, "s.txt");
    t.press("e").wait_idle();
    assert_eq!(launched(&mut t), ("s.txt".into(), "1".into()));

    select_status(&mut t, "new.txt");
    t.press("e").wait_idle();
    assert_eq!(launched(&mut t), ("new.txt".into(), "1".into()));
}

#[test]
fn status_carries_a_staged_change_to_the_working_tree() {
    let sb = Sandbox::new();
    sb.commit_file("s.txt", BASE, "base");
    sb.write("s.txt", &with_line(BASE, 9, "nine"));
    sb.git(&["add", "s.txt"]);
    let mut t = driver(&sb);
    select_status(&mut t, "s.txt");
    t.press("e").wait_idle();
    assert_eq!(launched(&mut t), ("s.txt".into(), "9".into()));
}

#[test]
fn status_opens_a_conflicted_file_at_its_first_marker() {
    let sb = Sandbox::new();
    sb.commit_file("c.txt", BASE, "base");
    sb.checkout_new("other");
    sb.commit_file("c.txt", &with_line(BASE, 6, "theirs"), "theirs");
    sb.checkout("main");
    sb.commit_file("c.txt", &with_line(BASE, 6, "ours"), "ours");
    // The merge stops on the conflict, so it fails.
    let mut merge = std::process::Command::new("git");
    sb.apply_env(&mut merge);
    let out = merge.current_dir(sb.path()).args(["merge", "other"]).output().unwrap();
    assert!(!out.status.success());
    let mut t = driver(&sb);
    select_status(&mut t, "c.txt");
    t.press("e").wait_idle();
    assert_eq!(launched(&mut t), ("c.txt".into(), "6".into()));
}

#[test]
fn staging_view_opens_the_selected_hunk_or_line() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", BASE, "base");
    sb.write("f.txt", &with_line(&with_line(BASE, 2, "two"), 14, "fourteen"));
    let mut t = driver(&sb);
    select_status(&mut t, "f.txt");
    t.press("enter").wait_idle();

    t.press("e");
    assert_eq!(launched(&mut t).1, "2");
    t.press("j").press("e");
    assert_eq!(launched(&mut t).1, "14");
    // Line mode: the second changed line of the hunk is the added one.
    t.press("a").press("j").press("e");
    assert_eq!(launched(&mut t).1, "14");
}

#[test]
fn staged_lines_are_carried_to_the_working_tree() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", BASE, "base");
    // Line 14 changed in the index; three lines added above it since.
    sb.write("f.txt", &with_line(BASE, 14, "fourteen"));
    sb.git(&["add", "f.txt"]);
    sb.write("f.txt", &format!("a\nb\nc\n{}", with_line(BASE, 14, "fourteen")));
    let mut t = driver(&sb);
    select_status(&mut t, "f.txt");
    t.press("enter").wait_idle();
    if t.app().details().stage().unwrap().side() == StageSideId::Unstaged {
        t.press("tab");
    }
    assert_eq!(t.app().details().stage().unwrap().side(), StageSideId::Staged);

    t.press("e");
    assert_eq!(launched(&mut t), ("f.txt".into(), "17".into()));
}

/// The editor `e` asked for, opening a copy outside the working tree, as
/// (the copy's contents, line).
fn launched_copy(t: &mut TestDriver) -> (String, String) {
    let launch: EditorLaunch = t.app_mut().take_editor().expect("an editor launch");
    let file = std::path::Path::new(&launch.args[0]);
    assert!(!file.starts_with(t.app().repo().root()), "{}", file.display());
    assert!(std::fs::metadata(file).unwrap().permissions().readonly());
    (std::fs::read_to_string(file).unwrap(), launch.args[1].clone())
}

#[test]
fn commit_files_view_opens_the_file_in_view() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", BASE, "base");
    sb.write("a.txt", &with_line(BASE, 9, "nine"));
    sb.commit_file(
        "gone.txt", "x
", "add gone",
    );
    sb.git(&["add", "a.txt"]);
    sb.git(&["rm", "-q", "gone.txt"]);
    sb.commit_staged("change a, remove gone");
    let mut t = driver(&sb);

    // The graph's newest commit, as files beside a diff.
    t.press("1").press("enter").wait_idle();
    assert_eq!(t.app().focus(), PaneId::Details);
    t.press("e");
    assert_eq!(launched(&mut t), ("a.txt".into(), "9".into()));

    // A deleted file opens as it was before the commit.
    t.press("j").wait_idle().press("e").wait_idle();
    assert_eq!(
        launched_copy(&mut t),
        (
            "x
"
            .into(),
            "1".into()
        )
    );
    let notice = t.app().notice().unwrap();
    assert!(
        notice.starts_with("gone.txt isn't in the working tree: opened a read-only copy from "),
        "{notice}"
    );
}

#[test]
fn files_only_on_another_branch_open_as_a_copy() {
    let sb = Sandbox::new();
    sb.commit_file(
        "a.txt", "a
", "base",
    );
    sb.checkout_new("other");
    sb.commit_file("only.txt", BASE, "add only");
    sb.commit_file("only.txt", &with_line(BASE, 9, "nine"), "change only");
    sb.checkout("main");
    let mut t = driver(&sb);

    // The newest commit, on `other`, in both commit views.
    t.press("1").press("g").press("enter").wait_idle();
    t.press("e").wait_idle();
    assert_eq!(launched_copy(&mut t), (with_line(BASE, 9, "nine"), "9".into()));
    let short = sb.git(&["rev-parse", "--short", "other"]);
    assert_eq!(
        t.app().notice(),
        Some(format!("only.txt isn't in the working tree: opened a read-only copy from {short}").as_str())
    );
}

#[test]
fn refuses_files_not_in_the_working_tree() {
    let sb = Sandbox::new();
    sb.commit_file("gone.txt", "x\n", "base");
    std::fs::remove_file(sb.path().join("gone.txt")).unwrap();
    let mut t = driver(&sb);
    select_status(&mut t, "gone.txt");
    t.press("e");
    assert!(t.app_mut().take_editor().is_none());
    assert_eq!(t.app().notice(), Some("gone.txt isn't in the working tree"));
}

#[test]
fn reports_how_the_editor_ended() {
    let sb = Sandbox::new();
    sb.commit_file("f.txt", BASE, "base");
    let mut t = driver(&sb);
    t.send(Action::EditorFinished(Ok(Some(2))));
    assert_eq!(t.app().notice(), Some("editor exited with status 2"));
    t.send(Action::EditorFinished(Err("couldn't start `ed`".into())));
    assert_eq!(t.app().error(), Some("couldn't start `ed`"));
}
