//! [`AppModel`]: the GPUI entity that owns tt-git's app and does for it what
//! the TUI's runner does: feeds it its actions and ticks, and carries out
//! what it asks of the front end (redraw, copy, start an editor, quit).

use std::cell::RefCell;
use std::future::poll_fn;
use std::process::Stdio;
use std::rc::Rc;

use gpui::{ClipboardItem, Context, EventEmitter, Task};
use tokio::sync::mpsc::UnboundedReceiver;
use tt_git_app::app::TICK;
use tt_git_app::editor::EditorLaunch;
use tt_git_app::keymap::KeyCombo;
use tt_git_app::mouse::MouseInput;
use tt_git_app::{Action, App};
use tt_git_plugin::runtime::resolve_program;

use crate::gpui_tokio::Tokio;

/// What [`AppModel`] tells the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvent {
    /// The app asked to quit: close the window.
    Quit,
    /// "Open repository" ran: ask which.
    OpenRepository,
}

/// tt-git's [`App`] as a GPUI entity. Input goes in through
/// [`key`](Self::key), [`mouse`](Self::mouse) and [`send`](Self::send),
/// each of which carries out the app's requests afterwards and notifies
/// observers when the app says the screen may have changed.
pub struct AppModel {
    app: App,
    /// The app's action channel. Shared with the task that waits on it, which
    /// borrows it only while polling, so input can apply what's queued at once.
    rx: Rc<RefCell<UnboundedReceiver<Action>>>,
    quit_sent: bool,
    _actions: Task<()>,
    _ticks: Task<()>,
}

impl EventEmitter<AppEvent> for AppModel {}

impl AppModel {
    /// Takes over `app` and its action channel `rx`, set up as
    /// `tt-git/src/main.rs` sets it up (config, themes, plugins), and starts
    /// it: its jobs run on the tokio runtime from [`crate::gpui_tokio`].
    pub fn new(app: App, rx: UnboundedReceiver<Action>, cx: &mut Context<Self>) -> Self {
        let mut app = app;
        app.set_runtime(Tokio::handle(cx));
        app.enable_open_repository();
        app.enable_ui_zoom();
        // The install scripts behind `tt-git update` only know the TUI's
        // archives, not the app's packages.
        app.disable_self_update();
        app.start();
        let rx = Rc::new(RefCell::new(rx));
        let waiting = rx.clone();
        let actions = cx.spawn(async move |this, cx| {
            while let Some(first) = poll_fn(|task| waiting.borrow_mut().poll_recv(task)).await {
                // `after_input` applies the rest of what's queued.
                let applied = this.update(cx, |model, cx| {
                    model.app.update(first);
                    model.after_input(cx);
                });
                if applied.is_err() {
                    break;
                }
            }
        });
        let ticks = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |model, cx| model.send(Action::Tick, cx)).is_err() {
                    break;
                }
            }
        });
        let mut model = Self {
            app,
            rx,
            quit_sent: false,
            _actions: actions,
            _ticks: ticks,
        };
        model.after_input(cx);
        cx.notify();
        model
    }

    pub fn app(&self) -> &App {
        &self.app
    }

    /// For drawing, which tells panes their viewport. Input goes through
    /// [`key`](Self::key), [`mouse`](Self::mouse) and [`send`](Self::send).
    pub fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    /// A key press, for tt-git's keymap to resolve.
    pub fn key(&mut self, key: KeyCombo, cx: &mut Context<Self>) {
        tracing::debug!(code = ?key.code, modifiers = ?key.modifiers, "key");
        self.app.key_pressed(key);
        self.after_input(cx);
    }

    /// A mouse event, hit-tested by the view it happened in.
    pub fn mouse(&mut self, input: MouseInput, cx: &mut Context<Self>) {
        if self.app.wants_mouse() {
            self.app.handle_mouse(input);
            self.after_input(cx);
        }
    }

    /// An action, as if a job had sent it.
    pub fn send(&mut self, action: Action, cx: &mut Context<Self>) {
        self.app.update(action);
        self.after_input(cx);
    }

    /// The window changed size.
    pub fn resized(&mut self, cx: &mut Context<Self>) {
        self.app.resized();
        self.after_input(cx);
    }

    /// Applies the actions already queued, as the TUI's runner does after
    /// every input (so a key that opens a dialog has opened it before the
    /// next key arrives), then carries out what the app asked for, and
    /// redraws if it may look different.
    fn after_input(&mut self, cx: &mut Context<Self>) {
        loop {
            let Ok(action) = self.rx.borrow_mut().try_recv() else {
                break;
            };
            self.app.update(action);
        }
        if self.app.should_quit() {
            if !self.quit_sent {
                self.quit_sent = true;
                cx.emit(AppEvent::Quit);
            }
            return;
        }
        if self.app.take_open_repository() {
            cx.emit(AppEvent::OpenRepository);
        }
        if let Some(step) = self.app.take_ui_zoom() {
            // Every window redraws at the new zoom (`Shell` watches it).
            let percent = crate::zoom::step(step, cx);
            self.app.ui_zoomed(percent);
        }
        if let Some(text) = self.app.take_clipboard() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
        if let Some(launch) = self.app.take_editor() {
            let finished = if launch.terminal {
                // In a pane over the main area, which reports
                // `EditorFinished` itself when the editor exits.
                self.app.open_editor_pane(&launch).err().map(Err)
            } else {
                Some(start_detached(&launch))
            };
            if let Some(result) = finished {
                self.app.update(Action::EditorFinished(result));
            }
        }
        if self.app.take_redraw() {
            cx.notify();
        }
    }
}

/// Starts an editor with a window of its own and leaves it running.
fn start_detached(launch: &EditorLaunch) -> Result<Option<i32>, String> {
    let mut cmd = std::process::Command::new(resolve_program(&launch.program, &launch.cwd));
    cmd.args(&launch.args).current_dir(&launch.cwd);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| {
        format!(
            "couldn't start `{}` (from {}): {e}. Set [editor] command in config.toml.",
            launch.program, launch.from
        )
    })?;
    // Reap it when it exits.
    let _ = std::thread::Builder::new()
        .name("tt-git-editor".into())
        .stack_size(tt_git_app::IO_THREAD_STACK)
        .spawn(move || child.wait());
    Ok(None)
}
