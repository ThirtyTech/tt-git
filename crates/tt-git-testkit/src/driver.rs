//! In-process UI driver, modelled on lazygit's integration test driver.
//!
//! The app runs on a ratatui `TestBackend`. Each key press goes through the
//! same `App::handle_key` path as the real terminal, then the driver waits
//! until the app is idle (no background jobs in flight and no queued actions)
//! before rendering. Tests therefore never sleep.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::UnboundedReceiver;
use tt_git_core::time::FixedClock;
use tt_git_core::{GitCli, Repo};
use tt_git_tui::app::PluginOptions;
use tt_git_tui::keymap::KeyCombo;
use tt_git_tui::panes::PaneId;
use tt_git_tui::{Action, App, Config};
use unicode_width::UnicodeWidthStr;

use crate::Sandbox;

pub const DEFAULT_WIDTH: u16 = 150;
pub const DEFAULT_HEIGHT: u16 = 50;
/// "Now" for rendered relative dates: one day after the sandbox epoch.
pub const NOW: i64 = crate::BASE_TIME + 86_400;
const IDLE_TIMEOUT: Duration = Duration::from_secs(20);

pub struct TestDriver {
    runtime: Runtime,
    app: App,
    rx: UnboundedReceiver<Action>,
    terminal: Terminal<TestBackend>,
}

impl TestDriver {
    pub fn new(sb: &Sandbox) -> Self {
        Self::with_config(Config::default(), sb)
    }

    pub fn with_config(config: Config, sb: &Sandbox) -> Self {
        Self::with_size(config, sb, DEFAULT_WIDTH, DEFAULT_HEIGHT)
    }

    /// Like [`with_config`](Self::with_config) but with the file watcher on
    /// (it is off by default so its events can't race the assertions).
    pub fn with_watcher(mut config: Config, sb: &Sandbox) -> Self {
        config.watch = true;
        Self::build(config, sb, DEFAULT_WIDTH, DEFAULT_HEIGHT)
    }

    /// Opens the sandbox repo with hermetic git env and a fixed clock, then
    /// waits for the initial loads to finish.
    pub fn with_size(mut config: Config, sb: &Sandbox, width: u16, height: u16) -> Self {
        config.watch = false;
        Self::build(config, sb, width, height)
    }

    /// With plugins from the sandbox registry enabled and the API socket
    /// listening. `bin` becomes `TTGIT_BIN_PATH` (the built `tt-git`).
    pub fn with_plugins(mut config: Config, sb: &Sandbox, bin: &std::path::Path) -> Self {
        config.watch = false;
        let opts = PluginOptions {
            dirs: sb.plugin_dirs(),
            bin_path: Some(bin.to_path_buf()),
            socket: true,
            extra_env: sb.env(),
        };
        Self::build_with(config, sb, DEFAULT_WIDTH, DEFAULT_HEIGHT, Some(opts))
    }

    /// Like [`with_size`](Self::with_size), opened at `path` (a linked
    /// worktree of the sandbox repo, say) instead of the sandbox repo.
    pub fn at(mut config: Config, sb: &Sandbox, path: &std::path::Path, width: u16, height: u16) -> Self {
        config.watch = false;
        Self::build_at(config, sb, path, width, height, None)
    }

    /// Like [`at`](Self::at) but with the file watcher on.
    pub fn with_watcher_at(mut config: Config, sb: &Sandbox, path: &std::path::Path) -> Self {
        config.watch = true;
        Self::build_at(config, sb, path, DEFAULT_WIDTH, DEFAULT_HEIGHT, None)
    }

    fn build(config: Config, sb: &Sandbox, width: u16, height: u16) -> Self {
        Self::build_with(config, sb, width, height, None)
    }

    fn build_with(config: Config, sb: &Sandbox, width: u16, height: u16, plugins: Option<PluginOptions>) -> Self {
        Self::build_at(config, sb, sb.path(), width, height, plugins)
    }

    fn build_at(
        config: Config,
        sb: &Sandbox,
        path: &std::path::Path,
        width: u16,
        height: u16,
        plugins: Option<PluginOptions>,
    ) -> Self {
        let git = GitCli::new(path).with_env(sb.env());
        let repo = Repo::discover(path, &git).expect("sandbox repo");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let (app, rx) = {
            let _guard = runtime.enter();
            let (mut app, rx) = App::new(config, repo, Arc::new(FixedClock(NOW))).expect("app starts");
            app.set_runtime(runtime.handle().clone());
            if let Some(opts) = plugins {
                app.enable_plugins(opts);
            }
            app.start();
            (app, rx)
        };
        let terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        let mut driver = Self {
            runtime,
            app,
            rx,
            terminal,
        };
        driver.wait_idle();
        driver
    }

    pub fn app(&self) -> &App {
        &self.app
    }

    pub fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    /// Presses one key, written like a config binding (`"j"`, `"ctrl+c"`, `"enter"`).
    pub fn press(&mut self, key: &str) -> &mut Self {
        let combo: KeyCombo = key.parse().unwrap_or_else(|e| panic!("{e}"));
        {
            let _guard = self.runtime.enter();
            self.app.handle_key(combo);
        }
        self.wait_idle()
    }

    /// Presses one key and renders the next frame before anything it queued
    /// is applied, as the real terminal does when background jobs haven't
    /// reported back yet. Catches in-between states that
    /// [`press`](Self::press) would skip; call [`wait_idle`](Self::wait_idle)
    /// to finish the jobs.
    pub fn press_frame(&mut self, key: &str) -> &mut Self {
        let combo: KeyCombo = key.parse().unwrap_or_else(|e| panic!("{e}"));
        {
            let _guard = self.runtime.enter();
            self.app.handle_key(combo);
        }
        self.render();
        self
    }

    /// Presses each key in order.
    pub fn press_all(&mut self, keys: &[&str]) -> &mut Self {
        for key in keys {
            self.press(key);
        }
        self
    }

    /// Types literal text one character at a time.
    pub fn type_text(&mut self, text: &str) -> &mut Self {
        for c in text.chars() {
            {
                let _guard = self.runtime.enter();
                self.app.handle_key(KeyCombo::char(c));
            }
            self.wait_idle();
        }
        self
    }

    /// Left-clicks the cell at (`x`, `y`), optionally with ctrl held.
    pub fn click(&mut self, x: u16, y: u16, ctrl: bool) -> &mut Self {
        use crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let modifiers = if ctrl {
            KeyModifiers::CONTROL
        } else {
            KeyModifiers::NONE
        };
        let ev = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers,
        });
        {
            let _guard = self.runtime.enter();
            tt_git_tui::input::handle_event(&mut self.app, ev);
        }
        self.wait_idle()
    }

    /// Clicks the `row`-th body line of a pane.
    pub fn click_pane_row(&mut self, pane: PaneId, row: u16) -> &mut Self {
        let area = self.app.pane_area(pane).expect("pane drawn");
        self.click(area.x + 2, area.y + 1 + row, false)
    }

    /// Clicks the first cell of the first place `text` is on screen.
    #[track_caller]
    pub fn click_text(&mut self, text: &str) -> &mut Self {
        let (x, y) = self
            .find_text(text)
            .unwrap_or_else(|| panic!("{text:?} is not on screen:\n{}", self.screen()));
        self.click(x, y, false)
    }

    /// The cell where `text` first starts on screen, top to bottom.
    pub fn find_text(&self, text: &str) -> Option<(u16, u16)> {
        let buf = self.buffer();
        for y in buf.area.top()..buf.area.bottom() {
            let mut line = String::new();
            // Byte offset in `line` at which each drawn cell starts.
            let mut starts = Vec::new();
            let mut skip = 0;
            for x in buf.area.left()..buf.area.right() {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let sym = buf[(x, y)].symbol();
                starts.push((line.len(), x));
                line.push_str(sym);
                skip = sym.width().saturating_sub(1);
            }
            if let Some(at) = line.find(text) {
                return starts.iter().find(|(b, _)| *b == at).map(|&(_, x)| (x, y));
            }
        }
        None
    }

    /// Mouse wheel over (`x`, `y`).
    pub fn scroll(&mut self, x: u16, y: u16, down: bool) -> &mut Self {
        use crossterm::event::{Event, KeyModifiers, MouseEvent, MouseEventKind};
        let kind = if down {
            MouseEventKind::ScrollDown
        } else {
            MouseEventKind::ScrollUp
        };
        let ev = Event::Mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        });
        {
            let _guard = self.runtime.enter();
            tt_git_tui::input::handle_event(&mut self.app, ev);
        }
        self.wait_idle()
    }

    /// Sends an action as if a background job had produced it.
    pub fn send(&mut self, action: Action) -> &mut Self {
        {
            let _guard = self.runtime.enter();
            self.app.update(action);
        }
        self.wait_idle()
    }

    /// Like [`send`](Self::send), but renders the next frame without
    /// applying anything the action queued, as [`press_frame`](Self::press_frame) does.
    pub fn send_frame(&mut self, action: Action) -> &mut Self {
        {
            let _guard = self.runtime.enter();
            self.app.update(action);
        }
        self.render();
        self
    }

    /// Lets `n` ticks of the app's timer go by and renders, without applying
    /// anything queued, so a git operation still running stays running.
    pub fn tick(&mut self, n: u64) -> &mut Self {
        {
            let _guard = self.runtime.enter();
            for _ in 0..n {
                self.app.update(Action::Tick);
            }
        }
        self.render();
        self
    }

    /// Processes actions until `pred` holds, for events the driver can't wait
    /// on directly (the file watcher). Panics after `timeout`.
    pub fn wait_until(&mut self, timeout: Duration, mut pred: impl FnMut(&App) -> bool) -> &mut Self {
        let start = Instant::now();
        loop {
            self.wait_idle();
            if pred(&self.app) {
                return self;
            }
            assert!(
                start.elapsed() < timeout,
                "condition not met within {timeout:?}:\n{}",
                self.screen()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Applies queued actions until no jobs are running and the queue is
    /// empty, then renders a frame. Panics after a generous timeout.
    pub fn wait_idle(&mut self) -> &mut Self {
        let start = Instant::now();
        loop {
            let mut progressed = false;
            {
                let _guard = self.runtime.enter();
                while let Ok(action) = self.rx.try_recv() {
                    self.app.update(action);
                    progressed = true;
                }
            }
            if !progressed && self.app.is_idle() && self.rx.is_empty() {
                break;
            }
            assert!(
                start.elapsed() < IDLE_TIMEOUT,
                "app did not become idle within {IDLE_TIMEOUT:?} ({} jobs in flight)",
                self.app.jobs().in_flight()
            );
            if !progressed {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        self.render();
        self
    }

    pub fn render(&mut self) -> &Buffer {
        let app = &mut self.app;
        let _guard = self.runtime.enter();
        self.terminal.draw(|f| app.draw(f)).expect("draw");
        self.terminal.backend().buffer()
    }

    pub fn resize(&mut self, width: u16, height: u16) -> &mut Self {
        self.terminal.backend_mut().resize(width, height);
        self.terminal.autoresize().expect("resize");
        self.render();
        self
    }

    pub fn buffer(&self) -> &Buffer {
        self.terminal.backend().buffer()
    }

    /// The whole screen as text, one line per row, trailing spaces trimmed.
    pub fn screen(&self) -> String {
        let buf = self.buffer();
        rows(buf, buf.area).join("\n")
    }

    /// Lines inside a pane's border.
    pub fn pane_lines(&self, pane: PaneId) -> Vec<String> {
        let area = self
            .app
            .pane_area(pane)
            .unwrap_or_else(|| panic!("{pane:?} was not drawn"));
        let inner = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };
        rows(self.buffer(), inner)
    }

    /// The pane's top border row, which carries its title.
    pub fn pane_title(&self, pane: PaneId) -> String {
        let area = self
            .app
            .pane_area(pane)
            .unwrap_or_else(|| panic!("{pane:?} was not drawn"));
        let title_row = Rect::new(area.x, area.y, area.width, 1);
        rows(self.buffer(), title_row).remove(0)
    }

    /// Asserts some line of `pane` contains `needle`.
    #[track_caller]
    pub fn assert_pane_contains(&self, pane: PaneId, needle: &str) -> &Self {
        let lines = self.pane_lines(pane);
        assert!(
            lines.iter().any(|l| l.contains(needle)),
            "{pane:?} does not contain {needle:?}:\n{}",
            lines.join("\n")
        );
        self
    }
}

/// Text of each row of `area`, skipping the filler cells after wide chars.
fn rows(buf: &Buffer, area: Rect) -> Vec<String> {
    let area = area.intersection(buf.area);
    (area.top()..area.bottom())
        .map(|y| {
            let mut line = String::new();
            let mut skip = 0;
            for x in area.left()..area.right() {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let sym = buf[(x, y)].symbol();
                line.push_str(sym);
                skip = sym.width().saturating_sub(1);
            }
            line.trim_end().to_string()
        })
        .collect()
}
