//! The non-interactive CLI (the plugin API surface that works without a
//! running instance).

use std::process::{Command, Output};

use serde_json::Value;
use tt_git_testkit::{PluginFixture, Sandbox};

const BIN: &str = env!("CARGO_BIN_EXE_tt-git");

fn tt(sb: &Sandbox, args: &[&str]) -> Output {
    let mut cmd = Command::new(BIN);
    cmd.args(args).current_dir(sb.path());
    sb.apply_env(&mut cmd);
    cmd.output().expect("run tt-git")
}

fn ok(sb: &Sandbox, args: &[&str]) -> String {
    let out = tt(sb, args);
    assert!(
        out.status.success(),
        "tt-git {args:?} failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn fails(sb: &Sandbox, args: &[&str]) -> String {
    let out = tt(sb, args);
    assert!(!out.status.success(), "tt-git {args:?} should fail");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn plugin_lifecycle_commands() {
    let sb = Sandbox::new();
    let dir = PluginFixture::new(&sb, "acme.cli", BIN.as_ref())
        .action("go", "Go", &[], &["echo", "hi"])
        .write();
    let dir = dir.to_string_lossy().into_owned();

    assert_eq!(ok(&sb, &["plugin", "list"]).trim(), "no plugins installed");
    assert!(ok(&sb, &["plugin", "link", &dir]).contains("linked acme.cli (enabled)"));

    let list: Value = serde_json::from_str(&ok(&sb, &["plugin", "list", "--json"])).unwrap();
    assert_eq!(list[0]["id"], "acme.cli");
    assert_eq!(list[0]["enabled"], true);
    assert_eq!(list[0]["runnable"], true);

    ok(&sb, &["plugin", "disable", "acme.cli"]);
    assert!(ok(&sb, &["plugin", "list"]).contains("acme.cli 0.1.0 (disabled)"));
    ok(&sb, &["plugin", "enable", "acme.cli"]);

    let cfg = ok(&sb, &["plugin", "config-dir", "acme.cli"]);
    assert!(std::path::Path::new(cfg.trim()).is_dir(), "{cfg}");

    ok(&sb, &["plugin", "unlink", "acme.cli"]);
    assert!(fails(&sb, &["plugin", "unlink", "acme.cli"]).contains("plugin_not_found"));
    assert!(fails(&sb, &["plugin", "config-dir", "acme.cli"]).contains("plugin_not_found"));
}

#[test]
fn link_rejects_bad_manifest() {
    let sb = Sandbox::new();
    let dir = sb.root().join("bad-plugin");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("tt-git-plugin.toml"),
        "id = \"bad id\"\nname = \"x\"\nversion = \"1.0.0\"\nmin_tt_git_version = \"0.1.0\"\n",
    )
    .unwrap();
    let err = fails(&sb, &["plugin", "link", &dir.to_string_lossy()]);
    assert!(err.contains("invalid_plugin_id"), "{err}");
}

#[test]
fn socket_commands_need_a_running_instance() {
    let sb = Sandbox::new();
    let err = fails(&sb, &["ui", "notify", "hi"]);
    assert!(err.contains("no running tt-git"), "{err}");
    let err = fails(&sb, &["api", "ping", "--socket", &tt_git_plugin::socket::new_address()]);
    assert!(err.contains("could not connect"), "{err}");
}

#[test]
fn status_and_branch_list() {
    let sb = Sandbox::new();
    sb.commit("base");
    sb.branch("topic");
    sb.write("dirty.txt", "x");

    assert_eq!(ok(&sb, &["status"]).trim(), "?? dirty.txt");
    let st: Value = serde_json::from_str(&ok(&sb, &["status", "--json"])).unwrap();
    assert_eq!(st["branch"]["head"], "main");
    assert_eq!(st["entries"][0]["path"], "dirty.txt");

    let text = ok(&sb, &["branch", "list"]);
    assert!(text.lines().any(|l| l == "* main"), "{text}");
    let branches: Value = serde_json::from_str(&ok(&sb, &["branch", "list", "--json"])).unwrap();
    let names: Vec<&str> = branches
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"topic"));

    // TTGIT_REPO_PATH (set for plugins) picks the repo regardless of cwd.
    let mut cmd = Command::new(BIN);
    cmd.args(["status"]).current_dir(sb.root());
    sb.apply_env(&mut cmd);
    cmd.env("TTGIT_REPO_PATH", sb.path());
    let out = cmd.output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "?? dirty.txt");
}

#[test]
fn update_refuses_a_cargo_build_before_going_online() {
    let sb = Sandbox::new();
    // The test binary lives in a cargo profile directory.
    let err = fails(&sb, &["update"]);
    assert!(
        err.starts_with("tt-git update: tt-git is running from a cargo build directory"),
        "{err}"
    );
    let err = fails(&sb, &["update", "v0.2.0"]);
    assert!(err.contains("cargo build directory"), "{err}");
    let err = fails(&sb, &["update", "--check", "v0.2.0"]);
    assert!(err.contains("cannot be used with"), "{err}");
}
