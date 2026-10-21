//! End-to-end tests: the real binary in a real PTY (ConPTY on Windows),
//! asserting on the emulated screen grid.

use std::time::Duration;

use termlens::{Key, Terminal};
use tt_git_testkit::Sandbox;

/// Host variables a child needs to run at all (PATH to find git, and the
/// Windows system dirs). Everything else comes from the sandbox.
const PASSTHROUGH_ENV: &[&str] = &[
    "PATH",
    "SystemRoot",
    "SYSTEMROOT",
    "WINDIR",
    "TEMP",
    "TMP",
    "COMSPEC",
    "PATHEXT",
];

fn spawn(sb: &Sandbox, args: &[&str]) -> Terminal {
    let mut builder = termlens::Terminal::builder()
        .size(120, 32)
        .env_clear()
        .timeout(Duration::from_secs(20))
        .current_dir(sb.path())
        .envs(sb.env())
        .args(args);
    for var in PASSTHROUGH_ENV {
        if let Some(val) = std::env::var_os(var) {
            builder = builder.env(var, val);
        }
    }
    builder.spawn(env!("CARGO_BIN_EXE_tt-git")).expect("spawn tt-git")
}

#[test]
fn starts_toggles_and_quits() {
    let sb = Sandbox::new();
    let mut t = spawn(&sb, &[]);
    t.wait_until(|s| s.contains("Graph [all]") && s.contains("Branches"))
        .expect("layout drawn");

    t.send_str("a").unwrap();
    t.wait_until(|s| s.contains("Graph [current]")).expect("scope toggled");

    t.send_str("q").unwrap();
    let status = t.wait_exit().expect("exits");
    assert!(status.success(), "exit status {status:?}");
}

#[test]
fn help_popup_opens_and_closes() {
    let sb = Sandbox::new();
    let mut t = spawn(&sb, &[]);
    t.wait_until(|s| s.contains("Graph [all]")).unwrap();
    t.send_str("?").unwrap();
    t.wait_until(|s| s.contains("Keybindings")).unwrap();
    t.send(Key::Esc).unwrap();
    t.wait_until(|s| !s.contains("Keybindings")).unwrap();
    t.send_str("q").unwrap();
    assert!(t.wait_exit().unwrap().success());
}

#[test]
fn shows_graph_of_a_real_repo() {
    let sb = Sandbox::new();
    sb.commit("first commit");
    sb.checkout_new("feature");
    sb.commit("feature commit");
    sb.checkout("main");
    let mut t = spawn(&sb, &[]);
    t.wait_until(|s| s.contains("feature commit") && s.contains("first commit"))
        .expect("graph rendered");
    t.wait_until(|s| s.contains("Status: repo · main"))
        .expect("status summary");
    t.send_str("a").unwrap();
    t.wait_until(|s| s.contains("Graph [current]") && !s.contains("feature commit"))
        .expect("current-branch scope");
    t.send_str("q").unwrap();
    assert!(t.wait_exit().unwrap().success());
}

#[test]
fn outside_a_repo_exits_with_error() {
    let sb = Sandbox::without_repo();
    // Wide enough that the message, which names the (temp, so on CI long)
    // path, doesn't wrap.
    let mut t = termlens::Terminal::builder()
        .size(400, 10)
        .env_clear()
        .timeout(Duration::from_secs(20))
        .current_dir(sb.path())
        .envs(sb.env())
        .env("GIT_CEILING_DIRECTORIES", sb.root());
    for var in PASSTHROUGH_ENV {
        if let Some(val) = std::env::var_os(var) {
            t = t.env(var, val);
        }
    }
    let mut t = t.spawn(env!("CARGO_BIN_EXE_tt-git")).unwrap();
    t.wait_until(|s| s.contains("is not inside a")).unwrap();
    assert!(!t.wait_exit().unwrap().success());
}

#[test]
fn stage_and_commit_in_the_real_binary() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.write("hello.txt", "hi\n");
    let mut t = spawn(&sb, &[]);
    t.wait_until(|s| s.contains("?? hello.txt")).expect("status lists file");
    t.send_str("2").unwrap();
    t.send(Key::Char(' ')).unwrap();
    t.wait_until(|s| s.contains("A  hello.txt")).expect("staged");
    t.send_str("c").unwrap();
    t.wait_until(|s| s.contains("Commit message")).expect("prompt");
    t.send_str("Add hello").unwrap();
    t.send(Key::Enter).unwrap();
    t.wait_until(|s| s.contains("working tree clean") && s.contains("Add hello"))
        .expect("committed");
    t.send_str("q").unwrap();
    assert!(t.wait_exit().unwrap().success());
    assert_eq!(sb.git(&["log", "-1", "--format=%s"]), "Add hello");
}

#[test]
fn undo_a_commit_in_the_real_binary() {
    let sb = Sandbox::new();
    let base = sb.commit("base");
    sb.commit("second");
    let mut t = spawn(&sb, &[]);
    t.wait_until(|s| s.contains("second")).expect("graph rendered");
    t.send_str("z").unwrap();
    t.wait_until(|s| s.contains("✓ undo commit: second")).expect("undone");
    t.wait_until(|_| sb.rev_parse("HEAD") == base)
        .expect("HEAD back on base");
    // The commit's file is back in the status pane, staged.
    t.wait_until(|s| s.contains("A  second.txt")).expect("changes kept");
    t.send_str("q").unwrap();
    assert!(t.wait_exit().unwrap().success());
}

#[test]
fn e_hands_the_terminal_to_the_editor_and_takes_it_back() {
    use tt_git_testkit::read_records;

    let sb = Sandbox::new();
    sb.commit_file("f.txt", "1\n2\n3\n4\n", "base");
    sb.write("f.txt", "1\n2\nthree\n4\n");
    // A stand-in terminal editor: records its arguments, then echoes typed
    // lines until `q`.
    let record = sb.root().join("editor.jsonl");
    let config_dir = sb.root().join("tt-config");
    std::fs::create_dir_all(&config_dir).unwrap();
    let command = format!(
        r#""{}" __test-plugin record "{}" echo {{file}}:{{line}} interactive editing"#,
        env!("CARGO_BIN_EXE_tt-git"),
        record.display()
    );
    std::fs::write(
        config_dir.join("config.toml"),
        format!("[editor]\ncommand = '{command}'\nterminal = true\n"),
    )
    .unwrap();

    let mut t = spawn(&sb, &[]);
    t.wait_until(|s| s.contains(" M f.txt")).expect("status lists file");
    t.send_str("2").unwrap();
    t.send_str("e").unwrap();
    t.wait_until(|s| s.contains("editing") && !s.contains("Status:"))
        .expect("editor has the terminal");
    t.send_str("hello").unwrap();
    t.send(Key::Enter).unwrap();
    t.wait_until(|s| s.contains("you typed: hello"))
        .expect("editor reads keys");
    t.send_str("q").unwrap();
    t.send(Key::Enter).unwrap();
    t.wait_until(|s| s.contains("Status:") && !s.contains("you typed"))
        .expect("tt-git is back");

    let records = read_records(&record);
    assert_eq!(records.len(), 1, "{records:?}");
    let args: Vec<String> = serde_json::from_value(records[0]["args"].clone()).unwrap();
    let opened = &args[3];
    assert!(opened.ends_with(":3"), "{args:?}");
    assert!(opened.contains("f.txt"), "{args:?}");

    // Keys reach tt-git again.
    t.send_str("q").unwrap();
    assert!(t.wait_exit().unwrap().success());
}

#[test]
fn external_cli_talks_to_running_instance_over_the_socket() {
    use tt_git_testkit::{PluginFixture, read_records};
    let sb = Sandbox::new();
    sb.commit("base");
    let out = sb.root().join("startup.jsonl");
    PluginFixture::new(&sb, "acme.e2e", env!("CARGO_BIN_EXE_tt-git").as_ref())
        .startup(&["record", &out.to_string_lossy()])
        .install(&sb);
    let mut t = spawn(&sb, &[]);
    t.wait_until(|s| s.contains("Graph [all]")).unwrap();

    // The startup command recorded the instance's socket address.
    let start = std::time::Instant::now();
    let socket = loop {
        if let Some(r) = read_records(&out).first() {
            break r["env"]["TTGIT_SOCKET_PATH"].as_str().unwrap().to_string();
        }
        assert!(start.elapsed() < Duration::from_secs(20), "startup command never ran");
        std::thread::sleep(Duration::from_millis(50));
    };

    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_tt-git"));
    cmd.args(["ui", "notify", "hello from outside", "--socket", &socket]);
    sb.apply_env(&mut cmd);
    assert!(cmd.status().unwrap().success());
    t.wait_until(|s| s.contains("hello from outside"))
        .expect("notice shown");

    t.send_str("q").unwrap();
    assert!(t.wait_exit().unwrap().success());
}

#[test]
fn opens_another_worktree_in_place() {
    let sb = Sandbox::new();
    sb.commit("base");
    let side = sb.path().join(".worktrees").join("side");
    sb.git(&["worktree", "add", "-q", "-b", "side", side.to_str().unwrap(), "main"]);
    sb.git_in(&side, &["commit", "-q", "--allow-empty", "-m", "side work"]);
    std::fs::write(
        sb.path().join(".git/info/exclude"),
        "/.worktrees/
",
    )
    .unwrap();
    let mut t = spawn(&sb, &[]);
    // The tab shows once the refs (and worktrees) are loaded.
    t.wait_until(|s| s.contains("Status: repo · main") && s.contains("Worktrees"))
        .expect("main opened");
    t.send_str("3").unwrap();
    t.send_str("[").unwrap();
    t.wait_until(|s| s.contains(".worktrees/side")).expect("worktrees tab");
    t.send_str("j").unwrap();
    t.send(Key::Enter).unwrap();
    t.wait_until(|s| s.contains("Status: side · side"))
        .expect("side opened");
    // A fresh start there: the graph is back in front.
    t.wait_until(|s| s.contains("Graph [all]") && s.contains("side work"))
        .unwrap();
    t.send_str("q").unwrap();
    assert!(t.wait_exit().unwrap().success());
}
