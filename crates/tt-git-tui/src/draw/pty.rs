//! Plugin panes: the vt100 screen of a plugin's pseudo-terminal.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Clear};
use tt_git_plugin::manifest::Placement;

use super::centered;
use crate::pty_pane::PtyPane;
use crate::tui_theme::Theme;

/// The area this pane occupies given the main area and the details area
/// (used by split panes).
pub fn area(pane: &PtyPane, main: Rect, details: Rect) -> Rect {
    match pane.placement {
        Placement::Split => details,
        Placement::Popup => {
            let w = pane
                .width
                .as_ref()
                .map_or(main.width * 4 / 5, |d| d.resolve(main.width));
            let h = pane
                .height
                .as_ref()
                .map_or(main.height * 4 / 5, |d| d.resolve(main.height));
            centered(main, w.max(10), h.max(4))
        }
        _ => main,
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
    let mut title = format!(" {} ", pane.title);
    if let Some(why) = &pane.exited {
        title.push_str(&format!("[{why}] "));
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(if focused { theme.border_focused } else { theme.border })
        .title(Span::styled(
            title,
            if focused { theme.title_focused } else { theme.title },
        ))
        .title_bottom(Span::styled(format!(" {release_hint} to leave "), theme.muted));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    pane.resize(inner.height, inner.width);

    let parser = pane.lock_parser();
    let screen = parser.screen();
    let buf = frame.buffer_mut();
    for row in 0..inner.height {
        for col in 0..inner.width {
            let Some(cell) = screen.cell(row, col) else { continue };
            if cell.is_wide_continuation() {
                continue;
            }
            let target = &mut buf[(inner.x + col, inner.y + row)];
            let contents = cell.contents();
            target.set_symbol(if contents.is_empty() { " " } else { contents });
            target.set_style(cell_style(cell));
        }
    }
    if focused && pane.exited.is_none() && !screen.hide_cursor() {
        let (r, c) = screen.cursor_position();
        if r < inner.height && c < inner.width {
            frame.set_cursor_position(Position::new(inner.x + c, inner.y + r));
            return true;
        }
    }
    false
}

fn convert_color(c: vt100::Color) -> Option<Color> {
    match c {
        vt100::Color::Default => None,
        vt100::Color::Idx(i) => Some(Color::Indexed(i)),
        vt100::Color::Rgb(r, g, b) => Some(Color::Rgb(r, g, b)),
    }
}

fn cell_style(cell: &vt100::Cell) -> Style {
    let mut style = Style::new();
    if let Some(fg) = convert_color(cell.fgcolor()) {
        style = style.fg(fg);
    }
    if let Some(bg) = convert_color(cell.bgcolor()) {
        style = style.bg(bg);
    }
    let mut m = Modifier::empty();
    if cell.bold() {
        m |= Modifier::BOLD;
    }
    if cell.italic() {
        m |= Modifier::ITALIC;
    }
    if cell.underline() {
        m |= Modifier::UNDERLINED;
    }
    if cell.inverse() {
        m |= Modifier::REVERSED;
    }
    style.add_modifier(m)
}
