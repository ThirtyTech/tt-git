//! Screenshots of the desktop app for the docs site, drawn by the real app
//! in real windows on the docs' mock repository (`tt_git_testkit::mock`).
//!
//! ```sh
//! cargo run -p tt-git-gui --example guishots -- docs-site/screens/gui-captures
//! ```
//!
//! Writes one PNG per screen, named after the screen and the system it was
//! taken on (`overview-windows.png`). `docs-site/screens/gui.mjs` turns them
//! into the site's images. Each window is drawn offscreen with
//! `Window::render_to_image`, which GPUI only has on Windows and macOS.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::component::Root;
use gpui::{
    AnyWindowHandle, AsyncApp, Entity, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    PlatformInput, Point, WindowHandle, point, px,
};
use tt_git_app::themes::ThemeOptions;
use tt_git_app::{App, Config};
use tt_git_core::time::FixedClock;
use tt_git_core::{GitCli, Repo};
use tt_git_gui::look::UiStyle;
use tt_git_gui::model::AppModel;
use tt_git_gui::shell::Shell;
use tt_git_gui::window_state::{GuiState, SavedWindow, WindowMode};
use tt_git_testkit::mock::{Mock, REPO};
use tt_git_testkit::{NOW, Sandbox};

/// The window's size in logical pixels.
const SIZE: (f32, f32) = (1280., 760.);

/// Set in the child process, which takes the shots (see `main`).
const CHILD: &str = "TTGIT_GUISHOTS_CHILD";

/// How long to wait for the app to settle before giving up.
const TIMEOUT: Duration = Duration::from_secs(30);

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).expect("usage: guishots <out-dir>"));
    std::fs::create_dir_all(&out).expect("create out dir");
    if std::env::var_os(CHILD).is_some() {
        shoot(out);
        return;
    }
    // The start window shows paths in the home folder as `~\…`. Run again
    // with a made-up home, so it shows the mock's folders, not the
    // machine's.
    let home = tempfile::Builder::new()
        .prefix("tt-git-guishots-")
        .tempdir()
        .expect("temp home");
    let status = Command::new(std::env::current_exe().expect("current exe"))
        .arg(&out)
        .env(CHILD, "1")
        .env("USERPROFILE", home.path())
        .env("HOME", home.path())
        .status()
        .expect("run guishots");
    assert!(status.success(), "guishots failed: {status}");
}

/// A screen: a name, the config and the keys pressed before the shot, and
/// what the mouse does then.
struct Shot {
    name: &'static str,
    config: fn() -> Config,
    keys: &'static [&'static str],
    mouse: Mouse,
    /// The window style it's drawn in.
    style: UiStyle,
}

enum Mouse {
    None,
    /// Opens the title bar's menu at this place (0 is the first menu).
    Menu(usize),
    /// Right-clicks this row of the graph.
    RightClickGraph(usize),
}

fn shoot(out: PathBuf) {
    let sb = Sandbox::named(REPO);
    let mock = Mock::new(&sb);
    mock.build();
    mock.working_changes();
    // Blame, in a repo of its own as in the terminal app's shots.
    let blamed = Sandbox::named(REPO);
    let mock = Mock::new(&blamed);
    mock.build();
    mock.blame_history();
    let mut blame_keys = vec!["2", "b"];
    blame_keys.extend(std::iter::repeat_n("j", 32));
    let blame_keys: &'static [&'static str] = blame_keys.leak();

    let shots = vec![
        Shot::keys("overview", &[]),
        Shot::keys("details", &["j", "enter"]),
        Shot::keys("staging", &["2", "j", "j", "j", "enter"]),
        Shot::keys("palette", &[":", "text:sta"]),
        Shot::keys("help", &["?"]),
        Shot::keys("help-about", &["?", "tab"]),
        Shot::keys("about", &[":", "text:about", "enter"]),
        Shot {
            name: "light",
            config: || Config {
                theme: Some("catppuccin-latte".into()),
                theme_background: true,
                ..Config::default()
            },
            keys: &["j", "space"],
            mouse: Mouse::None,
            style: UiStyle::Native,
        },
        Shot {
            style: UiStyle::Terminal,
            ..Shot::keys("terminal", &[])
        },
        Shot {
            mouse: Mouse::Menu(3),
            ..Shot::keys("menu", &[])
        },
        Shot {
            mouse: Mouse::RightClickGraph(3),
            ..Shot::keys("context-menu", &[])
        },
    ];

    // What the start window lists: two folders in the home folder and one
    // that's gone.
    let home = tt_git_core::paths::home_dir().expect("home folder");
    let code = home.join("code");
    for dir in ["lumen-web", "lumen-api"] {
        std::fs::create_dir_all(code.join(dir)).expect("create recent folder");
    }
    let state_dir = tempfile::tempdir().expect("state dir");
    let state_path = state_dir.path().join(tt_git_gui::window_state::FILE);
    GuiState {
        window: Some(SavedWindow {
            x: 40.,
            y: 40.,
            width: SIZE.0,
            height: SIZE.1,
            mode: WindowMode::Windowed,
        }),
        recent: vec![
            code.join("lumen-web"),
            code.join("lumen-api"),
            code.join("old-prototype"),
        ],
    }
    .save(&state_path)
    .expect("write gui.json");

    gpui::application().with_assets(tt_git_gui::Assets).run(move |cx| {
        tt_git_gui::init(cx);
        tt_git_gui::window_state::install(state_path, cx);
        cx.spawn(async move |cx| {
            // The app quits when its last window closes, so each
            // window closes once the next one is open.
            let mut last = None;
            for shot in &shots {
                // macOS's menus are in its menu bar, outside the window.
                if matches!(shot.mouse, Mouse::Menu(_)) && !tt_git_gui::menus::bar::DRAWS_MENUS {
                    continue;
                }
                last = Some(take(shot, &sb, &out, last, cx).await);
            }
            let last = take(&Shot::keys("blame", blame_keys), &blamed, &out, last, cx).await;
            start(&out, last, cx).await;
            println!("wrote screenshots to {}", out.display());
            cx.update(|cx| cx.quit());
            // Kept until here: dropping them deletes the repos.
            drop((sb, blamed, state_dir));
        })
        .detach();
    });
}

impl Shot {
    fn keys(name: &'static str, keys: &'static [&'static str]) -> Self {
        Self {
            name,
            config: Config::default,
            keys,
            mouse: Mouse::None,
            style: UiStyle::Native,
        }
    }
}

/// Opens a window on `sb`'s repo, closes `last`, presses the shot's keys,
/// and saves the window. Returns it, still open.
async fn take(
    shot: &Shot,
    sb: &Sandbox,
    out: &Path,
    last: Option<AnyWindowHandle>,
    cx: &mut AsyncApp,
) -> AnyWindowHandle {
    let mut config = (shot.config)();
    config.watch = false;
    let git = GitCli::new(sb.path()).with_env(sb.env());
    let repo = Repo::discover(sb.path(), &git).expect("sandbox repo");
    let (mut app, rx) = App::new(config, repo, Arc::new(FixedClock(NOW))).expect("app starts");
    app.enable_themes(ThemeOptions::default());
    let style = shot.style;
    let window = cx
        .update(|cx| {
            tt_git_gui::look::set(style, cx);
            tt_git_gui::open_window(app, rx, cx)
        })
        .expect("window opens");
    if let Some(last) = last {
        close(last, cx);
    }
    let model = window
        .update(cx, |root, _, cx| {
            let shell = root.view().clone().downcast::<Shell>().expect("shell");
            shell.read(cx).model().clone()
        })
        .expect("window is open");
    settle(&model, window.into(), cx).await;
    for key in shot.keys {
        match key.strip_prefix("text:") {
            Some(text) => {
                for c in text.chars() {
                    press(&model, &c.to_string(), cx);
                }
            }
            None => press(&model, key, cx),
        }
        settle(&model, window.into(), cx).await;
    }
    match shot.mouse {
        Mouse::None => {}
        Mouse::Menu(ix) => {
            let at = menu_position(window, ix, cx);
            click(window.into(), at, MouseButton::Left, cx);
        }
        Mouse::RightClickGraph(row) => {
            let at = window
                .update(cx, |root, _, cx| {
                    let shell = root.view().clone().downcast::<Shell>().expect("shell");
                    let ui = shell.read(cx).ui();
                    let rows = ui.measures.get(tt_git_gui::panes::graph::ROWS).expect("graph drawn");
                    let line = ui.font.line_height;
                    point(rows.origin.x + px(260.), rows.origin.y + line * (row as f32 + 0.5))
                })
                .expect("window is open");
            click(window.into(), at, MouseButton::Right, cx);
        }
    }
    settle(&model, window.into(), cx).await;
    save(window.into(), &out.join(file_name(shot.name)), cx);
    window.into()
}

/// The start window, outside a repository.
async fn start(out: &Path, last: AnyWindowHandle, cx: &mut AsyncApp) {
    // Only the made-up recent folders, not the sandboxes the other shots
    // opened.
    let window = cx
        .update(|cx| {
            let home = tt_git_core::paths::home_dir().expect("home folder");
            for path in tt_git_gui::window_state::state(cx).recent {
                if !path.starts_with(&home) {
                    tt_git_gui::window_state::forget_repo(&path, cx);
                }
            }
            tt_git_gui::start::open(None, cx)
        })
        .expect("start window opens");
    close(last, cx);
    frames(window.into(), cx).await;
    save(window.into(), &out.join(file_name("start")), cx);
    close(window.into(), cx);
}

fn press(model: &Entity<AppModel>, key: &str, cx: &mut AsyncApp) {
    let key = if key == " " { "space" } else { key };
    let combo = key.parse().unwrap_or_else(|e| panic!("{key}: {e}"));
    model.update(cx, |model, cx| model.key(combo, cx));
}

/// Waits until the app is idle and the window has drawn what it has.
async fn settle(model: &Entity<AppModel>, window: AnyWindowHandle, cx: &mut AsyncApp) {
    let start = Instant::now();
    loop {
        frames(window, cx).await;
        if model.read_with(cx, |model, _| model.app().is_idle()) {
            frames(window, cx).await;
            if model.read_with(cx, |model, _| model.app().is_idle()) {
                return;
            }
        }
        assert!(start.elapsed() < TIMEOUT, "the app didn't settle");
    }
}

/// Lets the window draw a few frames: each may measure something new and
/// ask for another.
async fn frames(window: AnyWindowHandle, cx: &mut AsyncApp) {
    for _ in 0..4 {
        window.update(cx, |_, window, _| window.refresh()).ok();
        cx.background_executor().timer(Duration::from_millis(60)).await;
    }
}

fn click(window: AnyWindowHandle, at: Point<Pixels>, button: MouseButton, cx: &mut AsyncApp) {
    let modifiers = Modifiers::default();
    let events = [
        PlatformInput::MouseMove(MouseMoveEvent {
            position: at,
            pressed_button: None,
            modifiers,
        }),
        PlatformInput::MouseDown(MouseDownEvent {
            button,
            position: at,
            modifiers,
            click_count: 1,
            first_mouse: false,
        }),
        PlatformInput::MouseUp(MouseUpEvent {
            button,
            position: at,
            modifiers,
            click_count: 1,
        }),
    ];
    for event in events {
        window
            .update(cx, |_, window, cx| window.dispatch_event(event, cx))
            .expect("window is open");
    }
}

/// The middle of the title bar's menu `ix`, found in the drawn title bar:
/// its icon, then each menu's name, are runs of drawn columns with gaps
/// between them.
fn menu_position(window: WindowHandle<Root>, ix: usize, cx: &mut AsyncApp) -> Point<Pixels> {
    let (image, scale) = window
        .update(cx, |_, window, _| {
            (window.render_to_image().expect("render"), window.scale_factor())
        })
        .expect("window is open");
    // The title bar's middle line, in device pixels, and the color most of
    // it has.
    let mid = (17. * scale) as u32;
    let band = (mid - (5. * scale) as u32)..(mid + (5. * scale) as u32);
    let bg = *image.get_pixel(image.width() / 2, 2);
    let ink = |x: u32| {
        band.clone().any(|y| {
            let p = image.get_pixel(x, y);
            (0..3).map(|c| (p[c] as i32 - bg[c] as i32).abs()).sum::<i32>() > 60
        })
    };
    let gap = (5. * scale) as u32;
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for x in 0..image.width() / 2 {
        if !ink(x) {
            continue;
        }
        match runs.last_mut() {
            Some((_, end)) if x - *end <= gap => *end = x,
            _ => runs.push((x, x)),
        }
    }
    // The first run is the icon.
    let (from, to) = runs
        .get(ix + 1)
        .copied()
        .unwrap_or_else(|| panic!("menu {ix} drawn: {runs:?}"));
    point(px((from + to) as f32 / 2. / scale), px(17.))
}

fn save(window: AnyWindowHandle, path: &Path, cx: &mut AsyncApp) {
    let image = window
        .update(cx, |_, window, _| window.render_to_image())
        .expect("window is open")
        .expect("render the window");
    image.save(path).expect("save screenshot");
    println!("{}", path.display());
}

fn close(window: AnyWindowHandle, cx: &mut AsyncApp) {
    window.update(cx, |_, window, _| window.remove_window()).ok();
}

fn file_name(name: &str) -> String {
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    };
    format!("{name}-{os}.png")
}
