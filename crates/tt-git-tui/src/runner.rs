//! The real terminal event loop.

use std::time::Duration;

use crossterm::clipboard::CopyToClipboard;
use futures::StreamExt;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::action::Action;
use crate::app::App;

const TICK: Duration = Duration::from_millis(250);

/// Runs until the app asks to quit or input ends.
pub async fn run(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    rx: &mut UnboundedReceiver<Action>,
) -> std::io::Result<()> {
    let mut events = crossterm::event::EventStream::new();
    let mut ticker = tokio::time::interval(TICK);
    app.start();
    terminal.draw(|f| app.draw(f))?;
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
        terminal.draw(|f| app.draw(f))?;
    }
    Ok(())
}
