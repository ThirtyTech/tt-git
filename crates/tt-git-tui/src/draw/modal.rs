//! Dialogs: prompts, confirmations, the command palette and pickers, the
//! keys popup, the error popup and the About dialog, drawn from their view
//! models (`view::dialogs`).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};

use super::centered;
use super::filter::draw_filter;
use super::graph::span;
use crate::app::App;
use crate::modal::Modal;
use crate::tui_theme::Theme;
use crate::view::TextSpan;
use crate::view::dialogs::about::{ABOUT_DEVELOPER, ABOUT_HINT, ABOUT_TAGLINE, ABOUT_TITLE, REPO_URL, about_heading};
use crate::view::dialogs::{HELP_TITLE, ListRow, PaletteView, error_view, line_text};

pub fn draw_modal(modal: &Modal, frame: &mut Frame, area: Rect, theme: &Theme) {
    if let Some(view) = modal.palette_view(area.width as usize) {
        return draw_palette(frame, area, theme, view);
    }
    if let Modal::Filter(panel) = modal {
        return draw_filter(panel, frame, area, theme);
    }
    if let Modal::About = modal {
        return draw_about(frame, area, theme);
    }
    let Some(view) = modal.dialog_view(area.width as usize) else {
        return;
    };
    let width = view.width as u16;
    let height = (view.body_rows() as u16 + 4).min(area.height);
    let popup = crate::draw::centered(area, width, height);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.role(view.look.border()))
        .title(Span::styled(format!(" {} ", view.title), theme.role(view.look.title())));
    let mut lines: Vec<Line> = view.body.into_iter().map(|l| line(l, theme)).collect();
    lines.push(Line::raw(""));
    lines.push(Line::styled(view.hint.unwrap_or_default(), theme.muted));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), popup);
}

/// A view model line in its theme styles.
pub(super) fn line<'a>(spans: Vec<TextSpan<'a>>, theme: &Theme) -> Line<'a> {
    Line::from(spans.into_iter().map(|s| span(s, theme)).collect::<Vec<_>>())
}

/// A list row, patched with the selection when picked.
pub(super) fn list_row<'a>(row: ListRow<'a>, theme: &Theme) -> Line<'a> {
    let line = line(row.spans, theme);
    if row.picked {
        line.patch_style(theme.selected)
    } else {
        line
    }
}

fn draw_palette(frame: &mut Frame, area: Rect, theme: &Theme, view: PaletteView) {
    let width = view.width as u16;
    let height = (view.list_rows as u16 + 4).min(area.height);
    let popup = Rect {
        y: area.y + area.height.saturating_sub(height) / 4,
        ..crate::draw::centered(area, width, height)
    };
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border_focused)
        .title(Span::styled(format!(" {} ", view.title), theme.title_focused));
    let mut lines = vec![line(view.input, theme), Line::raw("")];
    if let Some(empty) = view.empty {
        lines.push(Line::styled(empty, theme.muted));
    }
    lines.extend(view.rows.into_iter().map(|r| list_row(r, theme)));
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

/// The keys popup, narrowed to what's typed.
pub(super) fn draw_help_popup(app: &App, frame: &mut Frame, area: Rect, theme: &Theme) {
    let view = app.help_view();
    let mut lines = vec![line(view.search, theme)];
    if let Some(empty) = view.empty {
        lines.push(Line::styled(empty, theme.muted));
    }
    lines.extend(view.rows.into_iter().map(|r| line(r, theme)));

    let popup = centered(area, view.width as u16, view.height as u16);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border_focused)
        .title(Span::styled(format!(" {HELP_TITLE} "), theme.title_focused))
        .title_top(Line::styled(format!(" {} ", view.version), theme.muted).right_aligned());
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

/// The error popup.
pub(super) fn draw_error_popup(frame: &mut Frame, area: Rect, msg: &str, theme: &Theme) {
    let view = error_view(msg, area.width as usize);
    let inner_width = view.inner_width();
    let text_lines: usize = view
        .body
        .iter()
        .map(|l| line_text(l).chars().count().div_ceil(inner_width).max(1))
        .sum();
    let popup = centered(area, view.width as u16, text_lines as u16 + 2);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.role(view.look.border()))
        .title(Span::styled(format!(" {} ", view.title), theme.role(view.look.title())));
    let lines: Vec<Line> = view.body.into_iter().map(|l| line(l, theme)).collect();
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), popup);
}

/// The About dialog's width in columns, the border included.
const ABOUT_WIDTH: u16 = 58;

/// Ratatui, which the terminal app is drawn with.
pub const RATATUI_URL: &str = "https://ratatui.rs/";

/// The About dialog: the version, who made tt-git, its source and what
/// it's built with, laid out as the desktop app's is. The URLs are plain
/// text; most terminals make them clickable.
fn draw_about(frame: &mut Frame, area: Rect, theme: &Theme) {
    let mut lines = vec![
        Line::raw(""),
        Line::styled(about_heading(), theme.title_focused).centered(),
    ];
    // The tagline over two lines, broken after its comma.
    lines.extend(
        ABOUT_TAGLINE
            .split_inclusive(", ")
            .map(|part| Line::styled(part.trim_end(), theme.muted).centered()),
    );
    lines.extend([
        Line::raw(""),
        Line::raw(ABOUT_DEVELOPER),
        Line::from(vec![Span::raw("Source: "), Span::styled(REPO_URL, theme.help_key)]),
        Line::raw(""),
        Line::raw("Prefer a window? Run `tt-git-gui` for the desktop app."),
        Line::from(vec![
            Span::raw("Built with Ratatui: "),
            Span::styled(RATATUI_URL, theme.help_key),
        ]),
        Line::raw(""),
        Line::styled(ABOUT_HINT, theme.muted),
    ]);
    let popup = centered(area, ABOUT_WIDTH, lines.len() as u16 + 2);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border_focused)
        .padding(Padding::horizontal(1))
        .title(Span::styled(format!(" {ABOUT_TITLE} "), theme.title_focused));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), popup);
}
