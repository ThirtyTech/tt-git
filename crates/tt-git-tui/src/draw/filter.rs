//! The graph's filter panel, drawn from its view model (`FilterPanel::view`).

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::graph::span;
use super::modal::{line, list_row};
use crate::filter::{FIRST_CHOICE_ROW, FilterPanel, FilterTab, ROWS};
use crate::mouse::FilterSpot;
use crate::tui_theme::Theme;
use crate::view::dialogs::{FILTER_HINT, FILTER_TITLE};

/// What's at cell `pos` of the panel drawn in `area` (what
/// [`draw`](FilterPanel::draw) was given).
pub fn spot_at(panel: &FilterPanel, area: Rect, pos: Position) -> FilterSpot {
    let popup = popup_area(area);
    if !popup.contains(pos) {
        return FilterSpot::Outside;
    }
    let inner = Block::bordered().inner(popup);
    if !inner.contains(pos) {
        return FilterSpot::Nothing;
    }
    let row = (pos.y - inner.y) as usize;
    if row == 0 {
        let mut x = inner.x;
        let mut hit = FilterSpot::Nothing;
        for tab in FilterTab::ALL {
            if tab.index() > 0 {
                x += 3;
            }
            let width = panel.tab_text(tab).width() as u16;
            if (x..x + width).contains(&pos.x) {
                hit = FilterSpot::Tab(tab);
            }
            x += width;
        }
        return hit;
    }
    match row.checked_sub(FIRST_CHOICE_ROW) {
        Some(shown) => FilterSpot::Choice(shown),
        None => FilterSpot::Nothing,
    }
}

pub fn draw_filter(panel: &FilterPanel, frame: &mut Frame, area: Rect, theme: &Theme) {
    let popup = popup_area(area);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border_focused)
        .title(Span::styled(format!(" {FILTER_TITLE} "), theme.title_focused));
    let view = panel.view(area.width as usize);
    let mut lines = vec![line(view.tab_line(), theme), line(view.input, theme), Line::raw("")];
    if let Some(message) = view.message {
        lines.push(Line::from(span(message, theme)));
    }
    lines.extend(view.rows.into_iter().map(|r| list_row(r, theme)));

    // Keep the hint on the last row.
    let body_rows = popup.height.saturating_sub(2) as usize;
    while lines.len() + 1 < body_rows {
        lines.push(Line::raw(""));
    }
    lines.push(Line::styled(FILTER_HINT, theme.muted));
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

/// Where the panel goes in `area`: a fixed height, so it doesn't jump
/// about while typing.
fn popup_area(area: Rect) -> Rect {
    let width = 70.min(area.width);
    let height = (ROWS as u16 + 7).min(area.height);
    Rect {
        y: area.y + area.height.saturating_sub(height) / 4,
        ..crate::draw::centered(area, width, height)
    }
}
