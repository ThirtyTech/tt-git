//! The graph pane: one row per commit, lanes on the left, drawn from its
//! view model.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Draw, Hits, pane_block};
use crate::panes::{GraphPane, PaneId};
use crate::tui_theme::Theme;
use crate::view::TextSpan;

impl Draw for GraphPane {
    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme, _hits: &mut Hits) {
        let block = pane_block(PaneId::Graph, self.title(), focused, theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let now = self.clock().now();

        if let Some(msg) = self.empty_message() {
            self.note_drawn(now, 0..0);
            frame.render_widget(Paragraph::new(msg).style(theme.muted), inner);
            return;
        }

        let height = inner.height as usize;
        self.set_viewport(height);
        let view = self.view(height, inner.width as usize, now);
        let range = view.range.clone();
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
        frame.render_widget(Paragraph::new(lines), inner);
        self.note_drawn(now, range);
    }
}

/// A view model span in its theme style.
pub fn span<'a>(s: TextSpan<'a>, theme: &Theme) -> Span<'a> {
    let style = theme.span(&s);
    Span::styled(s.text, style)
}
