//! A harness for `#[gpui::test]`s: the real window on a `tt-git-testkit`
//! [`Sandbox`] repo, with helpers to press keys, wait until the app is
//! idle, and read its state. GPUI tests check state, not pixels.
//!
//! ```ignore
//! #[gpui::test]
//! fn tab_moves_focus(cx: &mut TestAppContext) {
//!     let sb = Sandbox::new();
//!     sb.commit_file("a.txt", "a", "first");
//!     let mut gui = Gui::open(cx, &sb);
//!     gui.press("tab");
//!     assert_eq!(gui.read(|app| app.focus()), PaneId::Status);
//! }
//! ```
//!
//! Tests that open a window live next to the code they test, in a
//! `#[cfg(test)] mod tests`, so each area keeps its tests in its own file.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::component::Root;
use gpui::{Entity, TestAppContext, VisualTestContext, WindowHandle};
use tt_git_app::keymap::KeyCombo;
use tt_git_app::{App, Config};
use tt_git_core::time::FixedClock;
use tt_git_core::{GitCli, Repo};
use tt_git_testkit::{NOW, Sandbox};

use crate::model::AppModel;
use crate::shell::Shell;

/// How long [`Gui::wait_idle`] waits for the app's jobs.
const IDLE_TIMEOUT: Duration = Duration::from_secs(20);

/// The GUI open on a sandbox repo.
pub struct Gui {
    pub cx: VisualTestContext,
    pub window: WindowHandle<Root>,
    pub shell: Entity<Shell>,
    pub model: Entity<AppModel>,
}

impl Gui {
    /// Opens the window on `sb`'s repo with the default config, and waits
    /// until the first loads are done and drawn.
    pub fn open(cx: &mut TestAppContext, sb: &Sandbox) -> Self {
        Self::with_config(cx, sb, Config::default())
    }

    /// Like [`open`](Self::open) with `config` (the file watcher is turned
    /// off, so its events can't race the assertions). Dates are relative
    /// to [`NOW`], one day after the sandbox's first commit.
    pub fn with_config(cx: &mut TestAppContext, sb: &Sandbox, mut config: Config) -> Self {
        // Jobs run on tokio threads; let the test executor wait for them.
        cx.executor().allow_parking();
        config.watch = false;
        let git = GitCli::new(sb.path()).with_env(sb.env());
        let repo = Repo::discover(sb.path(), &git).expect("sandbox repo");
        let (app, rx) = App::new(config, repo, Arc::new(FixedClock(NOW))).expect("app starts");
        let window = cx.update(|cx| {
            crate::init(cx);
            crate::open_window(app, rx, cx).expect("window opens")
        });
        let shell = window
            .update(cx, |root, _, _| root.view().clone().downcast::<Shell>().expect("shell"))
            .expect("window is open");
        let model = shell.read_with(cx, |shell, _| shell.model().clone());
        let cx = VisualTestContext::from_window(window.into(), cx);
        let mut gui = Self {
            cx,
            window,
            shell,
            model,
        };
        gui.wait_idle();
        gui
    }

    /// Presses `key` (`"j"`, `"ctrl+l"`, `"shift+tab"`: the keymap's
    /// syntax) the way the window does, then waits until the app is idle.
    pub fn press(&mut self, key: &str) -> &mut Self {
        let combo: KeyCombo = key.parse().unwrap_or_else(|e| panic!("{e}"));
        self.model.update(&mut self.cx, |model, cx| model.key(combo, cx));
        self.wait_idle()
    }

    /// Presses each of `keys` in turn.
    pub fn press_all(&mut self, keys: &[&str]) -> &mut Self {
        for key in keys {
            self.press(key);
        }
        self
    }

    /// Sends GPUI keystrokes (`"j"`, `"ctrl-l"`, `"shift-tab"`: GPUI's
    /// syntax) through the window's real key dispatch, then waits until the
    /// app is idle. Checks the whole path from the platform's key event to
    /// tt-git's keymap.
    pub fn keystrokes(&mut self, keystrokes: &str) -> &mut Self {
        self.cx.simulate_keystrokes(keystrokes);
        self.wait_idle()
    }

    /// Runs GPUI's tasks and waits for the app's jobs until none are
    /// running and everything they sent is applied and drawn. Panics after
    /// a generous timeout.
    pub fn wait_idle(&mut self) -> &mut Self {
        let start = Instant::now();
        loop {
            self.draw();
            if self.read(|app| app.is_idle()) {
                // What the last jobs sent is applied now; did it start more?
                self.draw();
                if self.read(|app| app.is_idle()) {
                    return self;
                }
            }
            let in_flight = self.read(|app| app.jobs().in_flight());
            assert!(
                start.elapsed() < IDLE_TIMEOUT,
                "app did not become idle within {IDLE_TIMEOUT:?} ({in_flight} jobs in flight)"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Runs GPUI's tasks and draws the window until nothing changes.
    pub fn draw(&mut self) -> &mut Self {
        // Each frame may measure something new and ask for another.
        for _ in 0..4 {
            if !self.is_open() {
                break;
            }
            self.cx.run_until_parked();
            self.cx.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            });
        }
        self.cx.run_until_parked();
        self
    }

    /// Reads the app's state.
    pub fn read<R>(&self, f: impl FnOnce(&App) -> R) -> R {
        self.model.read_with(&self.cx, |model, _| f(model.app()))
    }

    /// Whether the window is still open.
    pub fn is_open(&self) -> bool {
        let id = self.window.window_id();
        self.cx.windows().iter().any(|w| w.window_id() == id)
    }
}
