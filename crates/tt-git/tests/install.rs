//! `tt-git plugin install/update/uninstall` against local bare repositories
//! standing in for GitHub (`TTGIT_PLUGIN_GITHUB_BASE`).

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tt_git_plugin::registry::{PluginDirs, Registry, Source};
use tt_git_testkit::{Sandbox, read_records};

const BIN: &str = env!("CARGO_BIN_EXE_tt-git");

fn base_url(sb: &Sandbox) -> String {
    let p = sb.root().join("gh").to_string_lossy().replace('\\', "/");
    if p.starts_with('/') {
        format!("file://{p}")
    } else {
        format!("file:///{p}")
    }
}

fn tt(sb: &Sandbox, args: &[&str]) -> Output {
    let mut cmd = Command::new(BIN);
    cmd.args(args).current_dir(sb.root()).stdin(Stdio::null());
    sb.apply_env(&mut cmd);
    cmd.env("TTGIT_PLUGIN_GITHUB_BASE", base_url(sb));
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

fn toml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\"))
}

/// A "GitHub" repo `owner/name` with a plugin manifest at `subdir` (or the
/// root). Returns the working clone used to push new versions.
fn publish(sb: &Sandbox, owner: &str, name: &str, subdir: Option<&str>, manifest: &str) -> PathBuf {
    let bare = sb.root().join("gh").join(owner).join(format!("{name}.git"));
    std::fs::create_dir_all(&bare).unwrap();
    sb.git_in(&bare, &["init", "-q", "--bare", "-b", "main"]);
    let work = sb.root().join(format!("work-{owner}-{name}"));
    std::fs::create_dir_all(&work).unwrap();
    sb.git_in(&work, &["init", "-q", "-b", "main"]);
    sb.git_in(&work, &["remote", "add", "origin", &bare.to_string_lossy()]);
    push_version(sb, &work, subdir, manifest, "v1");
    work
}

fn push_version(sb: &Sandbox, work: &Path, subdir: Option<&str>, manifest: &str, msg: &str) {
    let dir = subdir.map_or(work.to_path_buf(), |s| work.join(s));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tt-git-plugin.toml"), manifest).unwrap();
    sb.git_in(work, &["add", "-A"]);
    sb.git_in(work, &["commit", "-q", "-m", msg]);
    sb.git_in(work, &["push", "-q", "origin", "HEAD:main"]);
}

fn manifest(id: &str, version: &str, build_record: Option<&Path>) -> String {
    let mut m = format!(
        "id = \"{id}\"\nname = \"Tools\"\nversion = \"{version}\"\nmin_tt_git_version = \"0.1.0\"\n\
         platforms = [\"linux\", \"macos\", \"windows\"]\n\
         [[actions]]\nid = \"go\"\ntitle = \"Go\"\ncommand = [{}, \"__test-plugin\", \"echo\", \"hi\"]\n",
        toml_str(BIN)
    );
    if let Some(rec) = build_record {
        m.push_str(&format!(
            "[[build]]\ncommand = [{}, \"__test-plugin\", \"record\", {}]\n",
            toml_str(BIN),
            toml_str(&rec.to_string_lossy())
        ));
    }
    m
}

fn registry(sb: &Sandbox) -> Registry {
    Registry::read(&sb.plugin_dirs()).unwrap()
}

fn generations(dirs: &PluginDirs, id: &str) -> Vec<PathBuf> {
    std::fs::read_dir(dirs.installations_dir().join(id))
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default()
}

#[test]
fn install_requires_confirmation_then_builds_and_registers() {
    let sb = Sandbox::without_repo();
    let built = sb.root().join("built.jsonl");
    publish(
        &sb,
        "acme",
        "tools",
        None,
        &manifest("acme.tools", "1.0.0", Some(&built)),
    );

    let out = tt(&sb, &["plugin", "install", "acme/tools"]);
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("Build commands") && stdout.contains("__test-plugin record"),
        "{stdout}"
    );
    assert!(stderr.contains("pass --yes"), "{stderr}");
    assert!(
        registry(&sb).plugins.is_empty(),
        "nothing installed without confirmation"
    );
    assert!(read_records(&built).is_empty(), "nothing built without confirmation");

    let out = ok(&sb, &["plugin", "install", "acme/tools", "--yes"]);
    assert!(out.contains("installed acme.tools 1.0.0"), "{out}");
    assert_eq!(read_records(&built).len(), 1, "build step ran once");
    let env = &read_records(&built)[0]["env"];
    assert!(env.get("TTGIT_PLUGIN_ID").is_none(), "build steps get no runtime env");

    let reg = registry(&sb);
    let p = &reg.plugins[0];
    assert_eq!(p.id, "acme.tools");
    let Source::Github {
        owner,
        repo,
        resolved_commit,
        managed_path,
        ..
    } = &p.source
    else {
        panic!("github source expected: {:?}", p.source)
    };
    assert_eq!((owner.as_str(), repo.as_str()), ("acme", "tools"));
    assert_eq!(resolved_commit.len(), 40);
    assert!(managed_path.join("tt-git-plugin.toml").exists());
    assert!(ok(&sb, &["plugin", "list"]).contains("acme.tools 1.0.0 (enabled)"));
}

#[test]
fn update_skips_unchanged_and_replaces_generation() {
    let sb = Sandbox::without_repo();
    let work = publish(&sb, "acme", "tools", None, &manifest("acme.tools", "1.0.0", None));
    ok(&sb, &["plugin", "install", "acme/tools", "--yes"]);
    let dirs = sb.plugin_dirs();
    assert_eq!(generations(&dirs, "acme.tools").len(), 1);

    assert!(ok(&sb, &["plugin", "update", "--yes"]).contains("acme.tools is up to date"));

    push_version(&sb, &work, None, &manifest("acme.tools", "1.1.0", None), "v2");
    // A running instance holds a lease on the current files.
    let old = tt_git_plugin::install::registered_paths(&dirs);
    let lease = tt_git_plugin::install::Lease::acquire(&dirs, &old).unwrap();
    let out = ok(&sb, &["plugin", "update", "acme/tools", "--yes"]);
    assert!(out.contains("updated acme.tools 1.1.0"), "{out}");
    assert!(ok(&sb, &["plugin", "list"]).contains("acme.tools 1.1.0"));
    assert_eq!(generations(&dirs, "acme.tools").len(), 2, "leased generation kept");

    drop(lease);
    tt_git_plugin::install::gc(&dirs);
    assert_eq!(generations(&dirs, "acme.tools").len(), 1, "old generation collected");
}

#[test]
fn install_from_subdirectory_and_uninstall() {
    let sb = Sandbox::without_repo();
    publish(
        &sb,
        "acme",
        "multi",
        Some("plugins/one"),
        &manifest("acme.one", "0.2.0", None),
    );
    ok(&sb, &["plugin", "install", "acme/multi/plugins/one", "--yes"]);
    let reg = registry(&sb);
    assert_eq!(reg.plugins[0].id, "acme.one");
    assert!(reg.plugins[0].manifest_path.ends_with("plugins/one/tt-git-plugin.toml"));

    let out = ok(&sb, &["plugin", "uninstall", "acme/multi/plugins/one"]);
    assert!(out.contains("uninstalled acme.one"));
    assert!(registry(&sb).plugins.is_empty());
    assert!(generations(&sb.plugin_dirs(), "acme.one").is_empty(), "files collected");
}

#[test]
fn failed_build_installs_nothing() {
    let sb = Sandbox::without_repo();
    let m = format!(
        "{}[[build]]\ncommand = [{}, \"__test-plugin\", \"exit\", \"7\"]\n",
        manifest("acme.broken", "1.0.0", None),
        toml_str(BIN)
    );
    publish(&sb, "acme", "broken", None, &m);
    let out = tt(&sb, &["plugin", "install", "acme/broken", "--yes"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("plugin_build_failed"));
    assert!(registry(&sb).plugins.is_empty());
    assert!(generations(&sb.plugin_dirs(), "acme.broken").is_empty());
}

#[test]
fn installing_over_a_local_link_is_refused() {
    let sb = Sandbox::without_repo();
    publish(&sb, "acme", "tools", None, &manifest("acme.tools", "1.0.0", None));
    let local = sb.root().join("local-tools");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(local.join("tt-git-plugin.toml"), manifest("acme.tools", "0.0.1", None)).unwrap();
    ok(&sb, &["plugin", "link", &local.to_string_lossy()]);

    let out = tt(&sb, &["plugin", "install", "acme/tools", "--yes"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("plugin_linked_locally"));
    assert_eq!(registry(&sb).plugins[0].source, Source::Local);

    // Bulk updates ignore linked plugins.
    assert!(ok(&sb, &["plugin", "update"]).contains("no GitHub-installed plugins"));
}

#[test]
fn bad_sources_are_rejected() {
    let sb = Sandbox::without_repo();
    let out = tt(&sb, &["plugin", "install", "not-a-spec", "--yes"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("expected owner/repo"));
    let out = tt(&sb, &["plugin", "install", "acme/missing", "--yes"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("git fetch"));
    // Staging is cleaned up after a failed fetch.
    let staging = sb.plugin_dirs().installations_dir().join(".staging");
    assert!(std::fs::read_dir(&staging).map(|d| d.count()).unwrap_or(0) == 0);
}
