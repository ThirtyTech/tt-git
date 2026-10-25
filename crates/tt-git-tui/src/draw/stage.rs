//! Staging in the details pane: a file's unstaged and staged changes, drawn
//! from its view model.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::Hits;
use crate::panes::{StageSideId, StageView};
use crate::tui_theme::Theme;
use crate::view::TextSpan;
use crate::view::stage::StageLayout;

pub fn draw_stage(stage: &mut StageView, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme, hits: &mut Hits) {
    let (first, second) = match stage.layout(area.width as usize) {
        StageLayout::Message(msg) => {
            frame.render_widget(Paragraph::new(msg).style(theme.muted), area);
            return;
        }
        StageLayout::One { full } => {
            // The empty side shrinks to its title along the bottom.
            let [main, rest] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
            if full == StageSideId::Unstaged {
                (main, rest)
            } else {
                (rest, main)
            }
        }
        StageLayout::Beside => {
            let [left, right] = Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]).areas(area);
            let block = Block::new().borders(Borders::RIGHT).border_style(theme.border);
            let inner = block.inner(left);
            frame.render_widget(block, left);
            (
                inner,
                Rect {
                    x: right.x + 1,
                    width: right.width.saturating_sub(1),
                    ..right
                },
            )
        }
        StageLayout::Stacked => {
            let [top, bottom] = Layout::vertical([Constraint::Fill(1), Constraint::Fill(1)]).areas(area);
            (top, bottom)
        }
    };
    draw_side(stage, frame, StageSideId::Unstaged, first, focused, theme, hits);
    draw_side(stage, frame, StageSideId::Staged, second, focused, theme, hits);
}

fn draw_side(
    stage: &mut StageView,
    frame: &mut Frame,
    id: StageSideId,
    area: Rect,
    focused: bool,
    theme: &Theme,
    hits: &mut Hits,
) {
    if area.height == 0 {
        return;
    }
    let [title_area, body] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    let title = line(stage.side_title(id, focused), theme);
    frame.render_widget(Paragraph::new(title), title_area);

    if let Some(why) = stage.side_error(id) {
        let style = theme.span(&why);
        frame.render_widget(Paragraph::new(why.text).style(style), body);
        return;
    }
    let height = body.height as usize;
    stage.set_viewport(id, height);
    if height > 0 {
        hits.stage.push((id, body));
    }
    let highlight = if focused {
        theme.selected
    } else {
        theme.selected_unfocused
    };
    let lines: Vec<Line> = stage
        .side_rows(id, height, body.width as usize)
        .into_iter()
        .map(|row| {
            let mut line = line(row.spans, theme);
            if let Some(back) = row.back {
                line = line.style(theme.role(back));
            }
            if row.picked { line.patch_style(highlight) } else { line }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), body);
}

fn line<'a>(spans: Vec<TextSpan<'a>>, theme: &Theme) -> Line<'a> {
    Line::from(
        spans
            .into_iter()
            .map(|s| {
                let style = theme.span(&s);
                Span::styled(s.text, style)
            })
            .collect::<Vec<_>>(),
    )
}
