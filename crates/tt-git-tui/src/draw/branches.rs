//! The branches pane: branches, tags, stashes and worktrees, a tab each,
//! drawn from its view model.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use super::graph::span;
use super::{Draw, Hits, pane_block, title_x};
use crate::panes::{BranchesPane, PaneId};
use crate::tui_theme::Theme;
use crate::view::TextSpan;

impl Draw for BranchesPane {
    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme, hits: &mut Hits) {
        // Past the right corner the title is cut off.
        let start = title_x(PaneId::Branches, area);
        let end = area.right().saturating_sub(1);
        let title = self.title(end.saturating_sub(start) as usize);
        hits.tabs = title
            .tabs
            .iter()
            .map(|(tab, xs)| {
                let x = |c: usize| (start as usize + c).min(end as usize) as u16;
                (*tab, area.y, x(xs.start)..x(xs.end))
            })
            .filter(|(_, _, xs)| !xs.is_empty())
            .collect();
        let mut block = pane_block(PaneId::Branches, line(title.spans, theme), focused, theme);
        let footer = self.footer(area.width.saturating_sub(2) as usize);
        if let Some(total) = footer.total {
            block = block.title_bottom(line(vec![total], theme).right_aligned());
        }
        if let Some(conflicts) = footer.conflicts {
            block = block.title_bottom(line(vec![conflicts], theme));
        }
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let now = self.clock.now();
        let height = inner.height as usize;
        if !self.visible.is_empty() {
            self.set_viewport(height);
        }
        let view = self.view(height, inner.width as usize, now);
        if let Some(empty) = view.empty {
            self.note_drawn(now, 0..0);
            frame.render_widget(Paragraph::new(empty).style(theme.muted), inner);
            return;
        }
        let range = view.range.clone();
        let lines: Vec<Line> = view
            .rows
            .into_iter()
            .map(|row| {
                let line = line(row.spans, theme);
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

/// View model spans as a line in their theme styles.
fn line(spans: Vec<TextSpan<'static>>, theme: &Theme) -> Line<'static> {
    Line::from(spans.into_iter().map(|s| span(s, theme)).collect::<Vec<_>>())
}
