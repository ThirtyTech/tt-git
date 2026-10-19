//! Plugin panes: plugin entry points running in a PTY inside tt-git.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tt_git_plugin::manifest::Placement;
use tt_git_plugin::socket;
use tt_git_testkit::{PluginFixture, Sandbox, TestDriver, read_records};
use tt_git_tui::Config;
use tt_git_tui::panes::PaneId;

const BIN: &str = env!("CARGO_BIN_EXE_tt-git");

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

fn call_api(t: &mut TestDriver, method: &str, params: Value) -> Result<Value, socket::ClientError> {
    let addr = t.app().socket_address().expect("socket").to_string();
    let method = method.to_string();
    let handle = std::thread::spawn(move || socket::call(&addr, &method, params));
    while !handle.is_finished() {
        t.wait_idle();
        std::thread::sleep(Duration::from_millis(5));
    }
    handle.join().unwrap()
}

fn pane_text(t: &TestDriver) -> String {
    t.app()
        .plugin_panes()
        .last()
        .map(|p| p.screen_text())
        .unwrap_or_default()
}

fn setup(placement: &str) -> Sandbox {
    let sb = Sandbox::new();
    sb.commit("base");
    PluginFixture::new(&sb, "acme.panes", Path::new(BIN))
        .pane("board", "Board", placement, &["interactive", "hello pane"])
        .install(&sb);
    sb
}

#[test]
fn overlay_pane_from_palette_takes_input_and_closes_on_exit() {
    let sb = setup("overlay");
    let mut t = TestDriver::with_plugins(Config::default(), &sb, Path::new(BIN));
    t.press(":").type_text("open board").press("enter");
    assert_eq!(t.app().plugin_panes().len(), 1);
    until(&mut t, "pane prompt", |t| pane_text(t).contains("hello pane"));
    // The emulator is fed on the reader thread; draw a fresh frame.
    t.render();
    assert!(t.screen().contains("Board"), "pane drawn with its title");
    assert!(t.screen().contains("hello pane"));

    // Keys go to the pane, not tt-git ("q" would otherwise quit).
    t.type_text("abc").press("enter");
    until(&mut t, "echo", |t| pane_text(t).contains("you typed: abc"));
    assert!(!t.app().should_quit());

    t.type_text("q").press("enter");
    until(&mut t, "pane closed after exit", |t| t.app().plugin_panes().is_empty());
    assert!(t.app().notice().unwrap_or_default().contains("process exited"));
    assert!(!t.screen().contains("hello pane"));
}

#[test]
fn release_key_closes_overlay() {
    let sb = setup("overlay");
    let mut t = TestDriver::with_plugins(Config::default(), &sb, Path::new(BIN));
    t.press(":").type_text("open board").press("enter");
    until(&mut t, "pane prompt", |t| pane_text(t).contains("hello pane"));
    t.press("f12");
    assert!(t.app().plugin_panes().is_empty());
    // Back to normal key handling.
    t.press("3");
    assert_eq!(t.app().focus(), PaneId::Branches);
}

#[test]
fn split_pane_replaces_details_and_shares_focus() {
    let sb = setup("split");
    let mut t = TestDriver::with_plugins(Config::default(), &sb, Path::new(BIN));
    let opened = call_api(
        &mut t,
        "plugin.pane.open",
        json!({ "plugin_id": "acme.panes", "entrypoint": "board" }),
    )
    .unwrap();
    let id = opened["pane_id"].as_u64().unwrap();
    assert_eq!(t.app().plugin_panes()[0].placement, Placement::Split);
    assert_eq!(t.app().focus(), PaneId::Details);
    until(&mut t, "split prompt", |t| {
        t.pane_lines(PaneId::Details).iter().any(|l| l.contains("hello pane"))
    });

    // Leave the pane: it stays open, other panes get keys again.
    t.press("f12");
    assert_eq!(t.app().focus(), PaneId::Graph);
    assert_eq!(t.app().plugin_panes().len(), 1);
    t.press("j");

    // Back in: typing reaches the process.
    t.press("4").type_text("hi").press("enter");
    until(&mut t, "echo", |t| pane_text(t).contains("you typed: hi"));

    let screen = call_api(&mut t, "plugin.pane.screen", json!({ "pane_id": id })).unwrap();
    assert!(screen["text"].as_str().unwrap().contains("you typed: hi"));

    t.type_text("q").press("enter");
    until(&mut t, "exit shown", |t| t.app().plugin_panes()[0].exited.is_some());
    // Any key dismisses the finished split pane and the details pane returns.
    t.press("x");
    assert!(t.app().plugin_panes().is_empty());
    assert!(t.pane_title(PaneId::Details).contains("[4]"));
}

#[test]
fn popup_is_a_singleton_and_can_be_closed_via_api() {
    let sb = setup("popup");
    let mut t = TestDriver::with_plugins(Config::default(), &sb, Path::new(BIN));
    call_api(
        &mut t,
        "plugin.pane.open",
        json!({ "plugin_id": "acme.panes", "entrypoint": "board", "width": "50%", "height": 10 }),
    )
    .unwrap();
    until(&mut t, "popup prompt", |t| pane_text(t).contains("hello pane"));
    let err = call_api(
        &mut t,
        "plugin.pane.open",
        json!({ "plugin_id": "acme.panes", "entrypoint": "board" }),
    )
    .unwrap_err();
    assert!(err.to_string().starts_with("ui_busy"), "{err}");

    let list = call_api(&mut t, "plugin.pane.list", Value::Null).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    let closed = call_api(&mut t, "popup.close", Value::Null).unwrap();
    assert!(closed["closed"].as_u64().is_some());
    assert!(t.app().plugin_panes().is_empty());
}

#[test]
fn pane_gets_entrypoint_and_custom_env() {
    let sb = Sandbox::new();
    sb.commit("base");
    let out = sb.root().join("pane-env.jsonl");
    PluginFixture::new(&sb, "acme.envpane", Path::new(BIN))
        .pane(
            "show",
            "Show",
            "overlay",
            &["record", &out.to_string_lossy(), "interactive", "ready"],
        )
        .install(&sb);
    let mut t = TestDriver::with_plugins(Config::default(), &sb, Path::new(BIN));
    call_api(
        &mut t,
        "plugin.pane.open",
        json!({ "plugin_id": "acme.envpane", "entrypoint": "show", "env": { "GITHUB_URL": "https://example.com/x" } }),
    )
    .unwrap();
    until(&mut t, "recorded", |_| !read_records(&out).is_empty());
    let env = &read_records(&out)[0]["env"];
    assert_eq!(env["TTGIT_PLUGIN_ENTRYPOINT_ID"], "show");
    assert_eq!(env["TTGIT_PLUGIN_ID"], "acme.envpane");
    let err = call_api(
        &mut t,
        "plugin.pane.open",
        json!({ "plugin_id": "acme.envpane", "entrypoint": "nope" }),
    )
    .unwrap_err();
    assert!(err.to_string().contains("plugin_pane_not_found"), "{err}");
    t.press("f12");
}

#[test]
fn ticks_keep_redrawing_while_a_pane_shows_the_cursor() {
    use tt_git_tui::Action;
    let sb = setup("overlay");
    let mut t = TestDriver::with_plugins(Config::default(), &sb, Path::new(BIN));
    t.press(":").type_text("open board").press("enter");
    until(&mut t, "pane prompt", |t| pane_text(t).contains("hello pane"));
    // Each frame re-sends show-cursor and its position; ticks always drew
    // one, so a focused pane's cursor still gets them four times a second.
    t.app_mut().take_redraw();
    t.send(Action::Tick);
    assert!(t.app_mut().take_redraw(), "tick with the pane's cursor shown");

    t.press("f12");
    assert!(t.app().plugin_panes().is_empty());
    t.app_mut().take_redraw();
    t.send(Action::Tick);
    assert!(!t.app_mut().take_redraw(), "tick with no cursor on screen");
}
