//! The graph pane: one row per commit, lanes on the left.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tt_git_core::time::{clock_time, relative};
use tt_git_graph::RowRef;
use unicode_width::UnicodeWidthStr;

use super::{Draw, Hits, pane_block};
use crate::panes::{DateStyle, GraphPane, LabelKind, PaneId, RefLabel};
use crate::tui_theme::Theme;
use crate::widgets::{fit, fit_right};

const AUTHOR_WIDTH: usize = 16;
const SHA_WIDTH: usize = 7;

impl Draw for GraphPane {
    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme, _hits: &mut Hits) {
        let block = pane_block(PaneId::Graph, self.title(), focused, theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let now = self.clock().now();

        if self.commits().is_empty() {
            self.note_drawn(now, 0..0);
            let msg = match () {
                _ if self.is_loading() => "loading…",
                _ if self.is_filtered() => "no commits match the filter (f or / to change it, esc to clear it)",
                _ => "no commits",
            };
            frame.render_widget(Paragraph::new(msg).style(theme.muted), inner);
            return;
        }

        let height = inner.height as usize;
        self.set_viewport(height);
        let std::ops::Range { start, end } = self.visible_range(height);
        let lanes = if !self.lanes {
            0
        } else {
            (start..end)
                .filter_map(|i| self.rows().get(i))
                .map(|r| r.width())
                .max()
                .unwrap_or(1)
                // Past the cap the rows don't keep their columns.
                .clamp(1, self.max_lanes.max(2).min(self.rows().cap()))
        };
        let width = inner.width as usize;

        let range = self.selected_range();
        let lines: Vec<Line> = (start..end)
            .map(|i| {
                let line = render_row(self, i, lanes, width, now, theme);
                let picked = match range {
                    Some((first, last)) => (first..=last).contains(&i),
                    None => i == self.selected_index(),
                };
                if picked {
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
        self.note_drawn(now, start..end);
    }
}

/// Draws commit `idx` with `lanes` lane columns, or none when `lanes` is 0.
fn render_row<'a>(graph: &'a GraphPane, idx: usize, lanes: usize, width: usize, now: i64, theme: &Theme) -> Line<'a> {
    let commit = &graph.commits()[idx];
    let row: RowRef = graph.rows().get(idx).expect("a row per commit");
    let mut spans: Vec<Span<'a>> = Vec::new();

    // Graph columns.
    if lanes > 0 {
        let node_glyph = if row.is_merge() { "◉" } else { "●" };
        let overflow = row.width() > lanes;
        let shown = if overflow { lanes.saturating_sub(1) } else { row.width() };
        for i in 0..shown {
            let cell = row.cell(i);
            spans.push(Span::styled(cell.glyph_str(node_glyph), theme.lane(cell.color)));
            let filler = match row.filler(i) {
                Some(c) if i + 1 < shown || overflow => Span::styled("─", theme.lane(c)),
                _ => Span::raw(" "),
            };
            spans.push(filler);
        }
        if overflow {
            let glyph = match (row.node_lane() >= shown, row.is_merge()) {
                (true, true) => "◉ ",
                (true, false) => "● ",
                (false, _) => "… ",
            };
            spans.push(Span::styled(glyph, theme.lane(row.node_color())));
        }
        let graph_used = (shown + usize::from(overflow)) * 2;
        spans.push(Span::raw(spaces((lanes * 2).saturating_sub(graph_used) + 1)));
    }

    // Metadata columns, dropped from the right when narrow.
    let graph_cols = if lanes > 0 { lanes * 2 + 1 } else { 0 };
    let date_width = graph.dates.width();
    let meta_full = 1 + AUTHOR_WIDTH + 1 + date_width + 1 + SHA_WIDTH;
    let meta_small = 1 + date_width + 1 + SHA_WIDTH;
    let rest = width.saturating_sub(graph_cols);
    let (show_author, meta) = if rest >= meta_full + 20 {
        (true, meta_full)
    } else if rest >= meta_small + 12 {
        (false, meta_small)
    } else {
        (false, 0)
    };
    let mut text_width = rest.saturating_sub(meta);

    // Ref labels, then the summary.
    for label in graph.labels(commit) {
        let (text, style) = label_span(label, theme);
        let w = text.width() + 1;
        if w > text_width {
            break;
        }
        text_width -= w;
        spans.push(Span::styled(text, style));
        spans.push(Span::raw(" "));
    }
    if let Some(note) = graph.note(commit) {
        let text = format!("({note})");
        let w = text.width() + 1;
        if w <= text_width {
            text_width -= w;
            spans.push(Span::styled(text, theme.muted));
            spans.push(Span::raw(" "));
        }
    }
    // Merge commits are dimmed so the work itself stands out. The commit,
    // not the row, says so: a filtered graph lays out only the parents
    // it lists.
    let dim = |style: Style| if commit.is_merge() { theme.muted } else { style };
    push_fit(&mut spans, &commit.summary, text_width, dim(Style::default()));

    if meta > 0 {
        if show_author {
            spans.push(Span::raw(" "));
            push_fit(&mut spans, &commit.author_name, AUTHOR_WIDTH, dim(theme.author));
        }
        spans.push(Span::raw(" "));
        let date = match graph.dates {
            DateStyle::Relative => relative(now, commit.commit_time),
            DateStyle::Clock => clock_time(commit.commit_time, graph.clock().utc_offset(commit.commit_time)),
        };
        spans.push(Span::styled(fit_right(&date, date_width), dim(theme.date)));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(commit.short_oid().to_string(), dim(theme.sha)));
    }
    Line::from(spans)
}

/// `n` spaces, borrowed when there are few.
fn spaces(n: usize) -> std::borrow::Cow<'static, str> {
    const SPACES: &str = "                                                                ";
    match SPACES.get(..n) {
        Some(s) => s.into(),
        None => " ".repeat(n).into(),
    }
}

/// Pushes what [`fit`] makes of `s`, borrowing `s` when it needn't be cut.
fn push_fit<'a>(spans: &mut Vec<Span<'a>>, s: &'a str, width: usize, style: Style) {
    let used = s.width();
    if used > width {
        spans.push(Span::styled(fit(s, width), style));
        return;
    }
    spans.push(Span::styled(s, style));
    if used < width {
        spans.push(Span::styled(spaces(width - used), style));
    }
}

fn label_span(label: &RefLabel, theme: &Theme) -> (String, Style) {
    match label.kind {
        LabelKind::Head => (format!(" {} ", label.name), theme.label_head),
        LabelKind::DetachedHead => (" HEAD ".to_string(), theme.label_detached),
        LabelKind::Local => (format!("[{}]", label.name), theme.label_local),
        LabelKind::Remote => (format!("[{}]", label.name), theme.label_remote),
        LabelKind::Tag => (format!("<{}>", label.name), theme.label_tag),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use tt_git_core::{Commit, Oid};
    use tt_git_graph::{GraphBuilder, NodeId};

    use super::*;
    use crate::jobs::Ctx;

    fn commit(oid: &str, parents: &[&str]) -> Commit {
        Commit {
            oid: oid.parse().unwrap(),
            parents: parents.iter().map(|p| p.parse().unwrap()).collect(),
            author_name: "Ann".into(),
            author_email: "ann@example.com".into(),
            author_time: 0,
            commit_time: 0,
            summary: "a commit".into(),
        }
    }

    fn intern(ids: &mut HashMap<Oid, NodeId>, oid: Oid) -> NodeId {
        let next = ids.len() as NodeId;
        *ids.entry(oid).or_insert(next)
    }

    /// The graph columns as drawn before rows were packed: from the whole
    /// row as laid out.
    fn unpacked_graph_spans(row: &tt_git_graph::Row, lanes: usize, theme: &Theme) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        let node_glyph = if row.is_merge { '◉' } else { '●' };
        let overflow = row.cells.len() > lanes;
        let shown = if overflow {
            lanes.saturating_sub(1)
        } else {
            row.cells.len()
        };
        for i in 0..shown {
            let cell = row.cells[i];
            spans.push(Span::styled(cell.glyph(node_glyph).to_string(), theme.lane(cell.color)));
            let filler = match row.fillers[i] {
                Some(c) if i + 1 < shown || overflow => Span::styled("─", theme.lane(c)),
                _ => Span::raw(" "),
            };
            spans.push(filler);
        }
        if overflow {
            let glyph = if row.node_lane >= shown { node_glyph } else { '…' };
            spans.push(Span::styled(format!("{glyph} "), theme.lane(row.node_color)));
        }
        let graph_used = (shown + usize::from(overflow)) * 2;
        spans.push(Span::raw(" ".repeat((lanes * 2).saturating_sub(graph_used) + 1)));
        spans
    }

    #[test]
    fn wide_graphs_draw_as_from_whole_rows() {
        // 30 tips taking turns, merging each other in now and then, so rows
        // run far past the lanes drawn.
        let n = 400;
        let oid = |i: usize| format!("{i:040x}");
        let commits: Vec<Commit> = (0..n)
            .map(|i| {
                let mut parents = Vec::new();
                if i + 30 < n {
                    parents.push(oid(i + 30));
                }
                if i % 7 == 0 && i + 41 < n {
                    parents.push(oid(i + 41));
                }
                let parents: Vec<&str> = parents.iter().map(String::as_str).collect();
                commit(&oid(i), &parents)
            })
            .collect();
        let mut ids = HashMap::new();
        let mut builder = GraphBuilder::new();
        let whole: Vec<tt_git_graph::Row> = commits
            .iter()
            .map(|c| {
                let id = intern(&mut ids, c.oid);
                let parents: Vec<NodeId> = c.parents.iter().map(|p| intern(&mut ids, *p)).collect();
                builder.push(id, &parents)
            })
            .collect();
        assert!(whole.iter().any(|r| r.width() > 30), "wide enough");

        let theme = Theme::default();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = Ctx::new(tx);
        for max_lanes in [2, 5, 16, 64] {
            let mut graph = GraphPane::new(Arc::new(tt_git_core::time::FixedClock(0)));
            graph.max_lanes = max_lanes;
            let load = graph.begin_load();
            graph.apply_batch(load.generation(), commits.clone(), true, &ctx);
            for lanes in 1..=max_lanes.max(2) {
                for (i, row) in whole.iter().enumerate() {
                    let want = unpacked_graph_spans(row, lanes, &theme);
                    let line = render_row(&graph, i, lanes, 200, 0, &theme);
                    assert_eq!(
                        line.spans[..want.len()],
                        want[..],
                        "max {max_lanes}, {lanes} lanes, row {i}"
                    );
                }
            }
        }
    }

    #[test]
    fn merge_commits_are_dimmed() {
        let (m, a, b) = ("a".repeat(40), "b".repeat(40), "c".repeat(40));
        let commits = vec![commit(&m, &[&a, &b]), commit(&a, &[]), commit(&b, &[])];
        let theme = Theme::default();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = Ctx::new(tx);
        let mut graph = GraphPane::new(Arc::new(tt_git_core::time::FixedClock(0)));
        let load = graph.begin_load();
        graph.apply_batch(load.generation(), commits, true, &ctx);

        let style_of = |i: usize, text: &str| {
            let line = render_row(&graph, i, 2, 200, 0, &theme);
            line.spans
                .iter()
                .find(|s| s.content.starts_with(text))
                .expect(text)
                .style
        };
        for text in ["a commit", "Ann", "aaaaaaa"] {
            assert_eq!(style_of(0, text), theme.muted, "merge {text}");
        }
        assert_eq!(style_of(1, "a commit"), Style::default());
        assert_eq!(style_of(1, "Ann"), theme.author);
        assert_eq!(style_of(1, "bbbbbbb"), theme.sha);
    }

    #[test]
    fn filtered_merges_stay_dimmed_without_open_lanes() {
        // The filter matched the merge and an unrelated commit, not the
        // merge's parents.
        let (m, a, b, c) = ("a".repeat(40), "b".repeat(40), "c".repeat(40), "d".repeat(40));
        let commits = vec![commit(&m, &[&a, &b]), commit(&c, &[&a])];
        let theme = Theme::default();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = Ctx::new(tx);
        let mut graph = GraphPane::new(Arc::new(tt_git_core::time::FixedClock(0)));
        graph.filter.message = Some("commit".into());
        let load = graph.begin_load();
        graph.apply_batch(load.generation(), commits, true, &ctx);

        assert!(graph.commits()[0].is_merge(), "keeps its parents");
        let rows: Vec<(usize, usize)> = graph.rows().iter().map(|r| (r.node_lane(), r.width())).collect();
        assert_eq!(rows, [(0, 1), (0, 1)], "no lanes to the commits left out");
        let line = render_row(&graph, 0, 2, 200, 0, &theme);
        let summary = line.spans.iter().find(|s| s.content.starts_with("a commit")).unwrap();
        assert_eq!(summary.style, theme.muted);
    }

    #[test]
    fn fitted_text_draws_as_fit_does() {
        let theme = Theme::default();
        for s in ["", "abc", "abcdef", "日本語", "a\u{301}b"] {
            for width in 0..8 {
                let mut spans = Vec::new();
                push_fit(&mut spans, s, width, theme.author);
                let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
                assert_eq!(text, fit(s, width), "{s:?} in {width}");
                assert!(spans.iter().all(|s| s.style == theme.author));
            }
        }
        assert_eq!(spaces(3), "   ");
        assert_eq!(spaces(100), " ".repeat(100));
    }
}
