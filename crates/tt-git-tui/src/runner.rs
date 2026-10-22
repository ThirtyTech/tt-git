//! The real terminal event loop.

use std::process::Stdio;
use std::time::Duration;

use crossterm::clipboard::CopyToClipboard;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, EventStream};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, SetTitle, disable_raw_mode, enable_raw_mode};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::UnboundedReceiver;
use tt_git_plugin::runtime::resolve_program;

use crate::action::Action;
use crate::app::App;
use crate::editor::EditorLaunch;

pub const TICK: Duration = Duration::from_millis(250);

/// Runs until the app asks to quit or input ends.
pub async fn run(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    rx: &mut UnboundedReceiver<Action>,
) -> std::io::Result<()> {
    let mut events = EventStream::new();
    let mut ticker = tokio::time::interval(TICK);
    app.start();
    app.take_redraw();
    terminal.draw(|f| app.draw(f))?;
    let mut size = terminal.size().ok();
    loop {
        tokio::select! {
            ev = events.next() => match ev {
                Some(Ok(ev)) => app.handle_event(ev),
                Some(Err(e)) => return Err(e),
                None => break,
            },
            Some(action) = rx.recv() => app.update(action),
            _ = ticker.tick() => app.update(Action::Tick),
        }
        // Apply everything already queued before paying for a redraw.
        while let Ok(action) = rx.try_recv() {
            app.update(action);
        }
        if app.should_quit() {
            break;
        }
        if let Some(text) = app.take_clipboard()
            && let Err(e) = crossterm::execute!(terminal.backend_mut(), CopyToClipboard::to_clipboard_from(text))
        {
            app.update(Action::Notify(format!("couldn't copy: {e}")));
        }
        if let Some(launch) = app.take_editor() {
            let result = if launch.terminal {
                // Stop reading input so the editor gets every key, and
                // start again once it's done.
                drop(events);
                let result = run_in_terminal(terminal, &launch, app.wants_mouse(), &app.terminal_title()).await;
                events = EventStream::new();
                ticker.reset();
                result
            } else {
                start_detached(&launch)
            };
            app.update(Action::EditorFinished(result));
        }
        // Skip frames nothing could have changed (key releases, mouse moves,
        // most ticks): they'd draw the same screen. A size change is caught
        // here too, in case it came without a resize event.
        let now_size = terminal.size().ok();
        if app.take_redraw() || now_size != size {
            terminal.draw(|f| app.draw(f))?;
            size = now_size;
        }
    }
    Ok(())
}

/// Hands the terminal to an editor that runs in it, waits for it to exit,
/// then takes the terminal back. Returns its exit code.
async fn run_in_terminal(
    terminal: &mut DefaultTerminal,
    launch: &EditorLaunch,
    mouse: bool,
    title: &str,
) -> Result<Option<i32>, String> {
    let mut out = std::io::stdout();
    if mouse {
        let _ = crossterm::execute!(out, DisableMouseCapture);
    }
    let _ = terminal.show_cursor();
    let _ = crossterm::execute!(out, LeaveAlternateScreen);
    let _ = disable_raw_mode();

    let mut cmd = editor_command(launch);
    let status = tokio::task::spawn_blocking(move || cmd.status()).await;

    let _ = enable_raw_mode();
    let _ = crossterm::execute!(out, EnterAlternateScreen, SetTitle(title));
    if mouse {
        let _ = crossterm::execute!(out, EnableMouseCapture);
    }
    // The editor drew over everything: the next frame redraws it all.
    let _ = terminal.clear();
    match status {
        Ok(Ok(status)) => Ok(status.code()),
        Ok(Err(e)) => Err(start_error(launch, &e)),
        Err(e) => Err(e.to_string()),
    }
}

/// Starts an editor with a window of its own and leaves it running.
fn start_detached(launch: &EditorLaunch) -> Result<Option<i32>, String> {
    let mut cmd = editor_command(launch);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| start_error(launch, &e))?;
    // Reap it when it exits.
    let _ = std::thread::Builder::new()
        .name("tt-git-editor".into())
        .stack_size(crate::IO_THREAD_STACK)
        .spawn(move || child.wait());
    Ok(None)
}

fn editor_command(launch: &EditorLaunch) -> std::process::Command {
    let mut cmd = std::process::Command::new(resolve_program(&launch.program, &launch.cwd));
    cmd.args(&launch.args).current_dir(&launch.cwd);
    cmd
}

fn start_error(launch: &EditorLaunch, e: &std::io::Error) -> String {
    format!(
        "couldn't start `{}` (from {}): {e}. Set [editor] command in config.toml.",
        launch.program, launch.from
    )
}
