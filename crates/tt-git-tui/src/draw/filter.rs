//! The graph's filter panel.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::filter::{FIRST_CHOICE_ROW, FilterPanel, FilterTab, ROWS};
use crate::mouse::FilterSpot;
use crate::tui_theme::Theme;
use crate::widgets::{fit, truncate};

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
        .title(Span::styled(" Filter graph ", theme.title_focused));
    let inner_width = popup.width.saturating_sub(2) as usize;
    let list = panel.list();

    let mut tabs = Vec::new();
    for tab in FilterTab::ALL {
        if !tabs.is_empty() {
            tabs.push(Span::styled(" · ", theme.muted));
        }
        let style = if tab == panel.tab {
            theme.help_key.add_modifier(Modifier::UNDERLINED)
        } else {
            theme.muted
        };
        tabs.push(Span::styled(panel.tab_text(tab), style));
    }

    let count = match &list.items {
        Some(Ok(items)) => format!("{}/{}", list.matches.len(), items.len()),
        _ => String::new(),
    };
    let input = truncate(&list.input, inner_width.saturating_sub(count.width() + 4));
    let pad = inner_width.saturating_sub(input.width() + count.width() + 3);
    let mut lines = vec![
        Line::from(tabs),
        Line::from(vec![
            Span::styled("> ", theme.help_key),
            Span::raw(input),
            Span::styled("█", theme.help_key),
            Span::raw(" ".repeat(pad)),
            Span::styled(count, theme.muted),
        ]),
        Line::raw(""),
    ];

    match &list.items {
        None => lines.push(Line::styled("loading…", theme.muted)),
        Some(Err(e)) => lines.push(Line::styled(e.clone(), theme.error)),
        Some(Ok(_)) if list.matches.is_empty() => {
            let typed = list.input.trim();
            let msg = if panel.tab == FilterTab::File && !typed.is_empty() {
                format!("no matches · enter shows commits that changed {typed}")
            } else {
                "no matches".into()
            };
            lines.push(Line::styled(msg, theme.muted));
        }
        Some(Ok(items)) => {
            for (pos, &i) in list.matches.iter().enumerate().skip(list.first_shown()).take(ROWS) {
                let choice = &items[i];
                let hint_w = choice.hint.width();
                let label_w = inner_width.saturating_sub(hint_w + 1);
                let label_style = if choice.value.is_none() {
                    theme.muted
                } else {
                    Style::new()
                };
                let line = Line::from(vec![
                    Span::styled(fit(&choice.label, label_w), label_style),
                    Span::raw(" "),
                    Span::styled(choice.hint.clone(), theme.muted),
                ]);
                lines.push(if pos == list.selected {
                    line.patch_style(theme.selected)
                } else {
                    line
                });
            }
        }
    }

    // Keep the hint on the last row.
    let body_rows = popup.height.saturating_sub(2) as usize;
    while lines.len() + 1 < body_rows {
        lines.push(Line::raw(""));
    }
    lines.push(Line::styled(
        "enter filter · tab author/file · ↑↓ pick · esc cancel",
        theme.muted,
    ));
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
