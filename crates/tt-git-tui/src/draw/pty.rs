//! Plugin panes: the vt100 screen of a plugin's pseudo-terminal, drawn from
//! the app's view model (`tt_git_app::view::terminal`).

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Clear};

use super::centered;
use crate::pty_pane::PtyPane;
use crate::tui_theme::{self, Theme};
use crate::view::terminal::{TerminalSpot, leave_hint};

/// The area this pane occupies given the main area and the details area
/// (used by split panes).
pub fn area(pane: &PtyPane, main: Rect, details: Rect) -> Rect {
    match pane.spot(main.width, main.height) {
        TerminalSpot::Details => details,
        TerminalSpot::Centered { cols, rows } => centered(main, cols, rows),
        TerminalSpot::Main => main,
    }
}

/// Draws the pane. Returns whether it placed the terminal cursor.
pub fn draw(
    pane: &mut PtyPane,
    frame: &mut Frame,
    area: Rect,
    focused: bool,
    release_hint: &str,
    theme: &Theme,
) -> bool {
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(if focused { theme.border_focused } else { theme.border })
        .title(Span::styled(
            format!(" {} ", pane.frame_title()),
            if focused { theme.title_focused } else { theme.title },
        ))
        .title_bottom(Span::styled(format!(" {} ", leave_hint(release_hint)), theme.muted));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let view = pane.view(inner.height, inner.width, focused);
    let buf = frame.buffer_mut();
    for (row, runs) in (0..).zip(&view.rows) {
        for run in runs {
            let style = tui_theme::style(run.style);
            let step = if run.wide { 2 } else { 1 };
            for (i, text) in (0..).zip(run.cells()) {
                let target = &mut buf[(inner.x + run.col + i * step, inner.y + row)];
                target.set_symbol(text);
                target.set_style(style);
            }
        }
    }
    match view.cursor {
        Some(c) => {
            frame.set_cursor_position(Position::new(inner.x + c.col, inner.y + c.row));
            true
        }
        None => false,
    }
}
