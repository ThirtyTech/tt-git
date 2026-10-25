//! Terminal editors from the GUI: they run in a pane over the main area,
//! take the keys, and report `EditorFinished` with their exit code when
//! they exit. The "editor" is this test binary, run again as a child: the
//! ignored test [`fake_editor`] reads a line, writes it to the file and
//! exits, which works the same on every OS.

use std::time::{Duration, Instant};

use gpui::TestAppContext;
use tt_git_app::panes::PaneId;
use tt_git_app::pty_pane::{BUILTIN, EDITOR_PANE};
use tt_git_app::{App, Config};
use tt_git_testkit::Sandbox;

use crate::testing::Gui;

/// What the fake editor prints once it's waiting for its line.
const READY: &str = "FAKE EDITOR READY";
/// The line that makes the fake editor fail, and the code it exits with.
const FAIL: &str = "fail";
const FAIL_CODE: i32 = 3;

/// Run as a child by the tests below (`--ignored` and the file to edit
/// after it), never by `cargo test` itself: prints [`READY`], reads a
/// line, writes it to the file and exits, with [`FAIL_CODE`] for [`FAIL`].
#[test]
#[ignore = "the editor tests run it as a fake terminal editor"]
fn fake_editor() {
    let Some(file) = std::env::args()
        .next_back()
        .filter(|f| std::path::Path::new(f).is_file())
    else {
        return;
    };
    println!("{READY}");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).expect("a line");
    let line = line.trim_end();
    std::fs::write(&file, format!("{line}\n")).expect("the file is written");
    std::process::exit(if line == FAIL { FAIL_CODE } else { 0 });
}

/// `[editor]` running [`fake_editor`] in the terminal.
fn fake_editor_config() -> Config {
    let exe = std::env::current_exe().expect("the test binary");
    let command = format!(
        "\"{}\" terminal::editor_tests::fake_editor --exact --ignored --nocapture --test-threads=1 {{file}}",
        exe.display()
    );
    let toml = format!("[editor]\ncommand = '{command}'\nterminal = true\n");
    Config::parse(&toml).expect("config")
}

/// A sandbox whose `a.txt` is changed, so the status pane lists it.
fn sandbox() -> Sandbox {
    let sb = Sandbox::new();
    sb.commit_file("a.txt", "a\n", "first");
    sb.write("a.txt", "changed\n");
    sb
}

fn until(gui: &mut Gui, what: &str, mut done: impl FnMut(&App) -> bool) {
    let start = Instant::now();
    loop {
        gui.draw();
        if gui.read(&mut done) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}: notice {:?}, error {:?}",
            gui.read(|app| app.notice().map(str::to_string)),
            gui.read(|app| app.error().map(str::to_string)),
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Opens `a.txt` with `e` in the status pane and waits for the editor.
fn open_editor(gui: &mut Gui) {
    gui.press("tab");
    assert_eq!(gui.read(|app| app.focus()), PaneId::Status);
    gui.press("e");
    until(gui, "the editor", |app| {
        app.plugin_panes()
            .last()
            .is_some_and(|p| p.screen_text().contains(READY))
    });
}

fn type_line(gui: &mut Gui, text: &str) {
    for c in text.chars() {
        gui.press(&c.to_string());
    }
    // Through the window's own key dispatch.
    gui.keystrokes("enter");
}

#[gpui::test]
fn a_terminal_editor_runs_in_a_pane_and_reports_back(cx: &mut TestAppContext) {
    let sb = sandbox();
    let mut gui = Gui::with_config(cx, &sb, fake_editor_config());
    open_editor(&mut gui);
    let (plugin, entrypoint, modal, title, leave) = gui.read(|app| {
        let p = app.plugin_panes().last().unwrap();
        (
            p.plugin_id.clone(),
            p.entrypoint.clone(),
            p.is_modal(),
            p.frame_title(),
            p.leave_text("f12"),
        )
    });
    assert_eq!((plugin.as_str(), entrypoint.as_str()), (BUILTIN, EDITOR_PANE));
    assert!(modal, "it covers the main area");
    assert!(title.ends_with(" a.txt"), "the file relative to the repo: {title}");
    assert!(
        leave.starts_with("quit ") && leave.ends_with(" to come back"),
        "{leave}"
    );
    assert!(gui.read(App::editor_running));
    assert!(gui.read(|app| app.error().is_none()));

    // Keys go to the editor, not tt-git's keymap ("q" would quit).
    type_line(&mut gui, "quiet edit");
    until(&mut gui, "the editor to exit", |app| app.plugin_panes().is_empty());
    assert_eq!(
        std::fs::read_to_string(sb.path().join("a.txt")).unwrap(),
        "quiet edit\n"
    );
    assert!(!gui.read(App::should_quit));
    assert!(gui.read(|app| app.error().is_none()));
    assert!(!gui.read(App::editor_running));
    // A clean exit says nothing, and the refresh that follows
    // `EditorFinished` has picked up the edit with the watcher off.
    assert_eq!(gui.read(|app| app.notice().map(str::to_string)), None);
    until(&mut gui, "the refresh", |app| {
        app.status().entries().iter().any(|e| e.path == "a.txt")
    });
}

#[gpui::test]
fn the_exit_code_reaches_the_app(cx: &mut TestAppContext) {
    let sb = sandbox();
    let mut gui = Gui::with_config(cx, &sb, fake_editor_config());
    open_editor(&mut gui);
    type_line(&mut gui, FAIL);
    until(&mut gui, "the editor's status", |app| {
        app.plugin_panes().is_empty()
            && app
                .notice()
                .is_some_and(|n| n == format!("editor exited with status {FAIL_CODE}"))
    });
}

#[gpui::test]
fn the_release_key_stops_the_editor_only_when_pressed_twice(cx: &mut TestAppContext) {
    let sb = sandbox();
    let mut gui = Gui::with_config(cx, &sb, fake_editor_config());
    open_editor(&mut gui);

    gui.press("f12");
    assert_eq!(gui.read(|app| app.plugin_panes().len()), 1, "still open");
    let notice = gui.read(|app| app.notice().unwrap_or_default().to_string());
    assert!(
        notice.contains("is still running") && notice.contains("again to stop it"),
        "{notice}"
    );
    // Another key in between starts over.
    gui.press("x");
    gui.press("f12");
    assert!(gui.read(App::editor_running));

    gui.press("f12");
    until(&mut gui, "the editor to stop", |app| app.plugin_panes().is_empty());
    let notice = gui.read(|app| app.notice().unwrap_or_default().to_string());
    assert!(notice.starts_with("stopped "), "{notice}");
    assert!(gui.read(|app| app.error().is_none()));
    assert_eq!(std::fs::read_to_string(sb.path().join("a.txt")).unwrap(), "changed\n");
}

#[gpui::test]
fn an_editor_that_wont_start_shows_the_error(cx: &mut TestAppContext) {
    let sb = sandbox();
    let missing = sb.path().join("no-such-editor");
    let toml = format!("[editor]\ncommand = '\"{}\"'\nterminal = true\n", missing.display());
    let mut gui = Gui::with_config(cx, &sb, Config::parse(&toml).unwrap());
    gui.press("tab");
    gui.press("e");
    until(&mut gui, "the error", |app| app.error().is_some());
    let error = gui.read(|app| app.error().unwrap().to_string());
    assert!(
        error.contains("couldn't start") && error.contains("[editor] command"),
        "{error}"
    );
    assert!(gui.read(|app| app.plugin_panes().is_empty()));
}

#[gpui::test]
fn a_windowed_editor_still_starts_detached(cx: &mut TestAppContext) {
    let sb = sandbox();
    // `git --version` stands in for an editor with a window of its own:
    // it starts, isn't waited on, and no pane opens.
    let cfg = Config::parse("[editor]\ncommand = \"git --version\"\nterminal = false\n").unwrap();
    let mut gui = Gui::with_config(cx, &sb, cfg);
    gui.press("tab");
    gui.press("e");
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(500) {
        gui.draw();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(gui.read(|app| app.plugin_panes().is_empty()));
    assert!(
        gui.read(|app| app.error().is_none()),
        "{:?}",
        gui.read(|app| app.error().map(str::to_string))
    );
    assert!(!gui.read(App::editor_running));
}
