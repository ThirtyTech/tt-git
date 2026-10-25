//! The status pane: changed files as a list or a folder tree, drawn from
//! its view model.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use super::graph::span;
use super::{Draw, Hits, pane_block};
use crate::panes::{PaneId, StatusPane};
use crate::tui_theme::Theme;

impl Draw for StatusPane {
    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme, _hits: &mut Hits) {
        let block = pane_block(PaneId::Status, self.title(), focused, theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if let Some(msg) = self.empty_message() {
            frame.render_widget(Paragraph::new(msg).style(theme.muted), inner);
            return;
        }
        let banner = self.banner_rows() as u16;
        let list = Rect {
            y: inner.y + banner.min(inner.height),
            height: inner.height.saturating_sub(banner),
            ..inner
        };
        let height = list.height as usize;
        self.set_viewport(height);
        let view = self.view(height, inner.width as usize);
        if let Some(text) = view.banner {
            frame.render_widget(Paragraph::new(text).style(theme.label_tag), Rect { height: 1, ..inner });
        }
        let lines: Vec<Line> = view
            .rows
            .into_iter()
            .map(|row| {
                let line = Line::from(row.spans.into_iter().map(|s| span(s, theme)).collect::<Vec<_>>());
                if row.picked {
                    line.patch_style(if focused {
                        theme.selected
                    } else {
                        theme.selected_unfocused
                    })
                } else {
                    line
                }
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), list);
    }
}

/// The text of each row of `pane` as drawn (without colors), for tests.
pub fn row_texts(pane: &StatusPane) -> Vec<String> {
    pane.row_texts()
}
