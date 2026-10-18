//! Plugin system tests: the real app in-process (TestDriver) with plugins
//! run as real subprocesses via `tt-git __test-plugin`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tt_git_plugin::runtime::CommandStatus;
use tt_git_plugin::socket;
use tt_git_testkit::{PluginFixture, Sandbox, TestDriver, read_records};
use tt_git_tui::Config;
use tt_git_tui::panes::PaneId;

const BIN: &str = env!("CARGO_BIN_EXE_tt-git");

fn bin() -> &'static Path {
    Path::new(BIN)
}

fn record_file(sb: &Sandbox, name: &str) -> PathBuf {
    sb.root().join(format!("{name}.jsonl"))
}

fn rec(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Pumps the app until `pred` holds (plugin processes run asynchronously).
fn until(t: &mut TestDriver, what: &str, mut pred: impl FnMut(&TestDriver) -> bool) {
    let start = Instant::now();
    loop {
        t.wait_idle();
        if pred(t) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "timed out waiting for {what}:\n{}",
            t.screen()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Calls the app's socket from another thread while pumping the app.
fn call_api(t: &mut TestDriver, method: &str, params: Value) -> Result<Value, socket::ClientError> {
    let addr = t.app().socket_address().expect("socket listening").to_string();
    let method = method.to_string();
    let handle = std::thread::spawn(move || socket::call(&addr, &method, params));
    while !handle.is_finished() {
        t.wait_idle();
        std::thread::sleep(Duration::from_millis(5));
    }
    handle.join().unwrap()
}

fn repo() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit("first");
    sb.commit("second");
    sb
}

#[test]
fn keybinding_runs_action_with_host_env_and_context() {
    let sb = repo();
    let out = record_file(&sb, "action");
    let dir = PluginFixture::new(&sb, "acme.tools", bin())
        .action("go", "Go", &["commit"], &["record", &rec(&out)])
        .install(&sb);
    let cfg =
        Config::parse("[[keys.command]]\nkey = \"ctrl+o\"\ntype = \"plugin_action\"\ncommand = \"acme.tools.go\"\n")
            .unwrap();
    let mut t = TestDriver::with_plugins(cfg, &sb, bin());
    t.press("j").press("ctrl+o");
    until(&mut t, "action record", |_| !read_records(&out).is_empty());

    let r = &read_records(&out)[0];
    let env = &r["env"];
    assert_eq!(env["TTGIT_ENV"], "1");
    assert_eq!(env["TTGIT_PLUGIN_ID"], "acme.tools");
    assert_eq!(env["TTGIT_PLUGIN_ACTION_ID"], "acme.tools.go");
    assert_eq!(env["TTGIT_BIN_PATH"], BIN);
    assert_eq!(env["TTGIT_SOCKET_PATH"], t.app().socket_address().unwrap());
    assert!(env["TTGIT_PLUGIN_CONFIG_DIR"].as_str().unwrap().ends_with("acme.tools"));
    assert!(Path::new(env["TTGIT_PLUGIN_STATE_DIR"].as_str().unwrap()).is_dir());
    assert_eq!(
        std::fs::canonicalize(r["cwd"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(&dir).unwrap(),
        "runs in the plugin root"
    );
    let ctx: Value = serde_json::from_str(env["TTGIT_PLUGIN_CONTEXT_JSON"].as_str().unwrap()).unwrap();
    assert_eq!(ctx["selected_commit"], t.app().graph().selected().unwrap().oid);
    assert_eq!(ctx["branch"], "main");
    assert_eq!(ctx["focused_pane"], "graph");
    assert_eq!(ctx["invocation_source"], "keybinding");
    assert_eq!(ctx["repo_name"], "repo");
}

#[test]
fn palette_lists_and_runs_plugin_actions_for_the_focused_context() {
    let sb = repo();
    let out = record_file(&sb, "palette");
    PluginFixture::new(&sb, "acme.tools", bin())
        .action("for-commit", "Commit thing", &["commit"], &["record", &rec(&out)])
        .action("for-branch", "Branch thing", &["branch"], &["echo", "x"])
        .install(&sb);
    let mut t = TestDriver::with_plugins(Config::default(), &sb, bin());
    t.press(":");
    let screen = t.screen();
    assert!(screen.contains("Commit thing"), "{screen}");
    assert!(
        !screen.contains("Branch thing"),
        "branch actions hidden in graph:\n{screen}"
    );
    t.type_text("fetch");
    assert!(t.screen().contains("Fetch"), "built-ins listed too");
    for _ in "fetch".chars() {
        t.press("backspace");
    }
    t.type_text("commit th").press("enter");
    until(&mut t, "palette action", |_| !read_records(&out).is_empty());
    let ctx: Value = serde_json::from_str(
        read_records(&out)[0]["env"]["TTGIT_PLUGIN_CONTEXT_JSON"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(ctx["invocation_source"], "palette");

    t.press("2").press("ctrl+p").type_text("thing");
    assert!(t.screen().contains("Branch thing"));
    t.press("esc");
}

#[test]
fn palette_runs_builtin_commands() {
    let sb = repo();
    let mut t = TestDriver::with_plugins(Config::default(), &sb, bin());
    t.press(":").type_text("branches").press("enter");
    assert_eq!(t.app().focus(), PaneId::Branches);
}

#[test]
fn event_hooks_get_event_json() {
    let sb = repo();
    sb.branch("feature");
    let out = record_file(&sb, "events");
    PluginFixture::new(&sb, "acme.hooks", bin())
        .event("branch.checked_out", &["record", &rec(&out)])
        .event("commit.created", &["record", &rec(&out)])
        .event("head.changed", &["record", &rec(&out)])
        .install(&sb);
    let mut t = TestDriver::with_plugins(Config::default(), &sb, bin());

    t.press("2");
    let idx = t.app().branches().visible().position(|b| b.name == "feature").unwrap();
    for _ in 0..idx {
        t.press("j");
    }
    t.press("enter");
    until(&mut t, "checkout + head events", |_| read_records(&out).len() >= 2);

    sb.write("new.txt", "x");
    t.press("ctrl+r")
        .press("1")
        .press("a")
        .press("c")
        .type_text("from ui")
        .press("enter");
    until(&mut t, "commit event", |_| {
        read_records(&out)
            .iter()
            .any(|r| r["env"]["TTGIT_PLUGIN_EVENT"] == "commit.created")
    });

    let records = read_records(&out);
    let event = |name: &str| -> Value {
        let r = records
            .iter()
            .find(|r| r["env"]["TTGIT_PLUGIN_EVENT"] == name)
            .unwrap_or_else(|| panic!("no {name} event in {records:#?}"));
        serde_json::from_str(r["env"]["TTGIT_PLUGIN_EVENT_JSON"].as_str().unwrap()).unwrap()
    };
    let checkout = event("branch.checked_out");
    assert_eq!(checkout["event"], "branch.checked_out");
    assert_eq!(checkout["data"]["branch"], "feature");
    assert_eq!(event("head.changed")["data"]["branch"], "feature");
    let commit = event("commit.created");
    assert_eq!(commit["data"]["summary"], "from ui");
    assert_eq!(commit["data"]["oid"], sb.rev_parse("HEAD"));
}

#[test]
fn startup_and_app_started() {
    let sb = repo();
    let out = record_file(&sb, "startup");
    PluginFixture::new(&sb, "acme.boot", bin())
        .startup(&["record", &rec(&out)])
        .event("app.started", &["record", &rec(&out)])
        .event("repo.opened", &["record", &rec(&out)])
        .install(&sb);
    let mut t = TestDriver::with_plugins(Config::default(), &sb, bin());
    until(&mut t, "startup records", |_| read_records(&out).len() == 3);
    let events: Vec<String> = read_records(&out)
        .iter()
        .map(|r| r["env"]["TTGIT_PLUGIN_EVENT"].as_str().unwrap().to_string())
        .collect();
    for e in ["startup", "app.started", "repo.opened"] {
        assert!(events.contains(&e.to_string()), "{events:?}");
    }
}

#[test]
fn plugin_calls_back_through_the_cli() {
    let sb = repo();
    PluginFixture::new(&sb, "acme.talk", bin())
        .action(
            "hello",
            "Hello",
            &["global"],
            &["cli", "ui", "notify", "hello from a plugin"],
        )
        .action("focus", "Focus", &["global"], &["cli", "ui", "focus", "branches"])
        .install(&sb);
    let mut t = TestDriver::with_plugins(Config::default(), &sb, bin());
    t.press(":").type_text("hello").press("enter");
    until(&mut t, "notify from plugin", |t| {
        t.app().notice() == Some("hello from a plugin")
    });
    assert!(t.screen().contains("hello from a plugin"));

    t.press(":").type_text("focus").press("enter");
    until(&mut t, "focus from plugin", |t| t.app().focus() == PaneId::Branches);
}

#[test]
fn failures_truncation_and_logs() {
    let sb = repo();
    PluginFixture::new(&sb, "acme.bad", bin())
        .action("fail", "Fail", &[], &["stderr", "something broke", "exit", "3"])
        .action("big", "Big", &[], &["big", "70000"])
        .install(&sb);
    let mut t = TestDriver::with_plugins(Config::default(), &sb, bin());

    t.press(":").type_text("fail").press("enter");
    until(&mut t, "failure notice", |t| {
        t.app().notice().is_some_and(|n| n.contains("✗ plugin acme.bad"))
    });
    assert!(t.app().notice().unwrap().contains("exit 3"));

    t.press(":").type_text("big").press("enter");
    until(&mut t, "big output", |t| {
        let logs = t.app().plugin_host().unwrap().runner().logs(Some("acme.bad"), 10);
        logs.iter()
            .any(|l| l.action_id.as_deref() == Some("acme.bad.big") && l.status != CommandStatus::Running)
    });
    let logs = t.app().plugin_host().unwrap().runner().logs(Some("acme.bad"), 10);
    let big = logs
        .iter()
        .find(|l| l.action_id.as_deref() == Some("acme.bad.big"))
        .unwrap();
    assert!(
        big.stdout
            .ends_with("[tt-git truncated plugin output after 65536 bytes]")
    );
    let fail = logs
        .iter()
        .find(|l| l.action_id.as_deref() == Some("acme.bad.fail"))
        .unwrap();
    assert_eq!((fail.status, fail.exit_code), (CommandStatus::Failed, Some(3)));
    assert!(fail.stderr.contains("something broke"));

    // The in-app log view.
    t.press(":").type_text("plugin logs").press("enter");
    assert_eq!(t.app().focus(), PaneId::Details);
    t.assert_pane_contains(PaneId::Details, "plugin acme.bad (enabled)");
    t.assert_pane_contains(PaneId::Details, "! something broke");
}

#[test]
fn concurrency_limit() {
    let sb = repo();
    let gate = sb.root().join("gate");
    PluginFixture::new(&sb, "acme.slow", bin())
        .action("wait", "Wait", &[], &["wait-file", &rec(&gate)])
        .install(&sb);
    let mut t = TestDriver::with_plugins(Config::default(), &sb, bin());
    let host = t.app().plugin_host().unwrap();
    let ctx = tt_git_plugin::InvocationContext::default();
    for _ in 0..32 {
        host.invoke_action("acme.slow.wait", ctx.clone())
            .expect("under the limit");
    }
    let err = host.invoke_action("acme.slow.wait", ctx).unwrap_err();
    assert!(err.to_string().starts_with("plugin_command_limit_reached"), "{err}");
    std::fs::write(&gate, "").unwrap();
    until(&mut t, "all finish", |t| t.app().is_idle());
}

#[test]
fn disabled_plugins_dont_run_and_registry_changes_reload() {
    let sb = repo();
    let out = record_file(&sb, "late");
    let mut t = TestDriver::with_plugins(Config::default(), &sb, bin());
    assert!(t.app().plugin_host().unwrap().actions().is_empty());

    // Linked while the app is running (as `tt-git plugin link` would).
    let dir = PluginFixture::new(&sb, "acme.late", bin())
        .action("go", "Late action", &[], &["record", &rec(&out)])
        .write();
    tt_git_plugin::registry::link(&sb.plugin_dirs(), &dir, false, &tt_git_plugin::host_version()).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    t.send(tt_git_tui::Action::Tick);
    assert!(t.app().plugin_host().unwrap().actions().is_empty(), "disabled");
    let err = t
        .app()
        .plugin_host()
        .unwrap()
        .invoke_action("acme.late.go", Default::default())
        .unwrap_err();
    assert!(err.to_string().starts_with("plugin_disabled"), "{err}");

    tt_git_plugin::registry::set_enabled(&sb.plugin_dirs(), "acme.late", true).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    t.send(tt_git_tui::Action::Tick);
    assert_eq!(t.app().plugin_host().unwrap().actions().len(), 1);
}

#[test]
fn socket_api() {
    let sb = repo();
    sb.branch("other");
    let out = record_file(&sb, "socket");
    PluginFixture::new(&sb, "acme.api", bin())
        .action("go", "Go", &[], &["record", &rec(&out)])
        .install(&sb);
    let mut t = TestDriver::with_plugins(Config::default(), &sb, bin());

    let pong = call_api(&mut t, "ping", Value::Null).unwrap();
    assert_eq!(pong["pong"], true);

    let info = call_api(&mut t, "repo.info", Value::Null).unwrap();
    assert_eq!(info["name"], "repo");
    assert_eq!(info["branch"], "main");
    assert_eq!(info["head"], sb.rev_parse("HEAD"));

    let branches = call_api(&mut t, "branch.list", Value::Null).unwrap();
    let names: Vec<&str> = branches
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"main") && names.contains(&"other"), "{names:?}");
    assert_eq!(branches[0]["state"]["state"], "local_only");

    let sel = call_api(&mut t, "graph.selection", Value::Null).unwrap();
    assert_eq!(sel["commit"]["summary"], "second");

    let actions = call_api(&mut t, "plugin.action.list", Value::Null).unwrap();
    assert_eq!(actions[0]["action_id"], "acme.api.go");

    let invoked = call_api(
        &mut t,
        "plugin.action.invoke",
        json!({ "action_id": "go", "plugin_id": "acme.api" }),
    )
    .unwrap();
    assert!(invoked["log_id"].as_u64().is_some());
    until(&mut t, "socket-invoked action", |_| !read_records(&out).is_empty());
    let logs = call_api(&mut t, "plugin.log.list", json!({ "plugin_id": "acme.api" })).unwrap();
    assert_eq!(logs[0]["action_id"], "acme.api.go");

    let plugins = call_api(&mut t, "plugin.list", Value::Null).unwrap();
    assert_eq!(plugins[0]["id"], "acme.api");
    assert_eq!(plugins[0]["source"]["kind"], "local");

    call_api(&mut t, "ui.notify", json!({ "message": "over the socket" })).unwrap();
    assert_eq!(t.app().notice(), Some("over the socket"));

    let err = call_api(&mut t, "nope.nope", Value::Null).unwrap_err();
    assert!(err.to_string().starts_with("unknown_method"), "{err}");
    let err = call_api(&mut t, "ui.notify", Value::Null).unwrap_err();
    assert!(err.to_string().starts_with("invalid_params"), "{err}");

    call_api(&mut t, "plugin.disable", json!({ "plugin_id": "acme.api" })).unwrap();
    assert!(t.app().plugin_host().unwrap().actions().is_empty());
}

#[test]
fn events_subscribe_streams_events() {
    let sb = repo();
    sb.branch("feature");
    let mut t = TestDriver::with_plugins(Config::default(), &sb, bin());
    let addr = t.app().socket_address().unwrap().to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let stream = socket::subscribe(&addr, json!({ "events": ["branch.checked_out"] })).unwrap();
        for ev in stream {
            let _ = tx.send(ev);
        }
    });
    // Let the subscription register before acting.
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(300) {
        t.wait_idle();
        std::thread::sleep(Duration::from_millis(10));
    }
    t.press("2");
    let idx = t.app().branches().visible().position(|b| b.name == "feature").unwrap();
    for _ in 0..idx {
        t.press("j");
    }
    t.press("enter");
    let ev = rx.recv_timeout(Duration::from_secs(10)).expect("event streamed");
    assert_eq!(ev["event"], "branch.checked_out");
    assert_eq!(ev["data"]["branch"], "feature");
}

#[test]
fn link_handlers_receive_the_clicked_url() {
    let sb = Sandbox::new();
    sb.commit_file(
        "a.txt",
        "a",
        "Fix crash\n\nSee https://github.com/acme/app/issues/42 for details.",
    );
    sb.commit_file("b.txt", "b", "Docs: https://example.com/guide");
    let out = record_file(&sb, "links");
    PluginFixture::new(&sb, "acme.links", bin())
        .action("issue", "Open issue", &[], &["record", &rec(&out)])
        .raw("[[link_handlers]]\nid = \"gh-issue\"\ntitle = \"GitHub issue\"\npattern = '^https://github[.]com/[^/]+/[^/]+/issues/[0-9]+$'\naction = \"issue\"\n")
        .install(&sb);
    let mut t = TestDriver::with_plugins(Config::default(), &sb, bin());

    // Newest commit's link has no handler.
    t.press("4").press("o");
    assert_eq!(t.app().notice(), Some("no plugin handles https://example.com/guide"));

    t.press("3").press("j").press("4").press("o");
    until(&mut t, "link handler ran", |_| !read_records(&out).is_empty());
    let env = &read_records(&out)[0]["env"];
    assert_eq!(env["TTGIT_PLUGIN_CLICKED_URL"], "https://github.com/acme/app/issues/42");
    assert_eq!(env["TTGIT_PLUGIN_LINK_HANDLER_ID"], "gh-issue");
    assert_eq!(env["TTGIT_PLUGIN_ACTION_ID"], "acme.links.issue");
    let ctx: Value = serde_json::from_str(env["TTGIT_PLUGIN_CONTEXT_JSON"].as_str().unwrap()).unwrap();
    assert_eq!(ctx["invocation_source"], "link_click");
}

#[test]
fn ctrl_click_on_a_link_runs_its_handler() {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "a", "Fixes https://github.com/acme/app/issues/7");
    let out = record_file(&sb, "click");
    PluginFixture::new(&sb, "acme.click", bin())
        .action("issue", "Open issue", &[], &["record", &rec(&out)])
        .raw("[[link_handlers]]\nid = \"gh\"\ntitle = \"GitHub\"\npattern = 'issues/[0-9]+$'\naction = \"issue\"\n")
        .install(&sb);
    let mut t = TestDriver::with_plugins(Config::default(), &sb, bin());
    t.press("space");
    let area = t.app().pane_area(PaneId::Details).unwrap();
    let row = t
        .pane_lines(PaneId::Details)
        .iter()
        .position(|l| l.contains("issues/7"))
        .expect("url visible") as u16;
    t.click(area.x + 6, area.y + 1 + row, true);
    until(&mut t, "handler ran", |_| !read_records(&out).is_empty());
    assert_eq!(
        read_records(&out)[0]["env"]["TTGIT_PLUGIN_CLICKED_URL"],
        "https://github.com/acme/app/issues/7"
    );
}
