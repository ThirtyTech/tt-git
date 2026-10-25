//! The graph pane's view model: one row of spans per commit on screen, the
//! lanes on the left, then the commit's labels, summary, author, date and
//! hash, fitted to the width.

use std::borrow::Cow;
use std::ops::Range;

use tt_git_core::time::{clock_time, relative};
use tt_git_graph::RowRef;
use unicode_width::UnicodeWidthStr;

use super::{StyleRole, TextSpan};
use crate::panes::{DateStyle, GraphPane, LabelKind, RefLabel};
use crate::widgets::{fit, fit_right};

const AUTHOR_WIDTH: usize = 16;
const SHA_WIDTH: usize = 7;

/// What the graph shows in a viewport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphView<'a> {
    /// The commits on screen, by index into [`GraphPane::commits`].
    pub range: Range<usize>,
    /// Lane columns drawn left of each commit; 0 with lanes hidden.
    pub lanes: usize,
    /// One per commit in `range`, top to bottom.
    pub rows: Vec<GraphRow<'a>>,
}

/// One commit's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow<'a> {
    /// The commit's index into [`GraphPane::commits`].
    pub index: usize,
    /// It's selected, or inside the range selection.
    pub picked: bool,
    pub spans: Vec<TextSpan<'a>>,
}

impl GraphPane {
    /// What the graph says instead of rows when there are no commits.
    pub fn empty_message(&self) -> Option<&'static str> {
        if !self.commits().is_empty() {
            return None;
        }
        Some(match () {
            _ if self.is_loading() => "loading…",
            _ if self.is_filtered() => "no commits match the filter (f or / to change it, esc to clear it)",
            _ => "no commits",
        })
    }

    /// The rows a viewport of `rows` lines and `width` columns shows, with
    /// relative dates as of `now`. Call
    /// [`set_viewport`](GraphPane::set_viewport) first.
    pub fn view(&self, rows: usize, width: usize, now: i64) -> GraphView<'_> {
        let range = self.visible_range(rows);
        let lanes = if !self.lanes {
            0
        } else {
            range
                .clone()
                .filter_map(|i| self.rows().get(i))
                .map(|r| r.width())
                .max()
                .unwrap_or(1)
                // Past the cap the rows don't keep their columns.
                .clamp(1, self.max_lanes.max(2).min(self.rows().cap()))
        };
        let selection = self.selected_range();
        let rows = range
            .clone()
            .map(|index| GraphRow {
                index,
                picked: match selection {
                    Some((first, last)) => (first..=last).contains(&index),
                    None => index == self.selected_index(),
                },
                spans: row_spans(self, index, lanes, width, now),
            })
            .collect();
        GraphView { range, lanes, rows }
    }
}

/// Commit `idx` with `lanes` lane columns, or none when `lanes` is 0.
pub fn row_spans(graph: &GraphPane, idx: usize, lanes: usize, width: usize, now: i64) -> Vec<TextSpan<'_>> {
    let commit = &graph.commits()[idx];
    let row: RowRef = graph.rows().get(idx).expect("a row per commit");
    let mut spans: Vec<TextSpan> = Vec::new();

    // Graph columns.
    if lanes > 0 {
        let node_glyph = if row.is_merge() { "◉" } else { "●" };
        let overflow = row.width() > lanes;
        let shown = if overflow { lanes.saturating_sub(1) } else { row.width() };
        for i in 0..shown {
            let cell = row.cell(i);
            spans.push(TextSpan::new(cell.glyph_str(node_glyph), StyleRole::Lane(cell.color)));
            let filler = match row.filler(i) {
                Some(c) if i + 1 < shown || overflow => TextSpan::new("─", StyleRole::Lane(c)),
                _ => TextSpan::plain(" "),
            };
            spans.push(filler);
        }
        if overflow {
            let glyph = match (row.node_lane() >= shown, row.is_merge()) {
                (true, true) => "◉ ",
                (true, false) => "● ",
                (false, _) => "… ",
            };
            spans.push(TextSpan::new(glyph, StyleRole::Lane(row.node_color())));
        }
        let graph_used = (shown + usize::from(overflow)) * 2;
        spans.push(TextSpan::plain(spaces((lanes * 2).saturating_sub(graph_used) + 1)));
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
        let (text, role) = label_text(label);
        let w = text.width() + 1;
        if w > text_width {
            break;
        }
        text_width -= w;
        spans.push(TextSpan::new(text, role));
        spans.push(TextSpan::plain(" "));
    }
    if let Some(note) = graph.note(commit) {
        let text = format!("({note})");
        let w = text.width() + 1;
        if w <= text_width {
            text_width -= w;
            spans.push(TextSpan::new(text, StyleRole::Muted));
            spans.push(TextSpan::plain(" "));
        }
    }
    // Merge commits are dimmed so the work itself stands out. The commit,
    // not the row, says so: a filtered graph lays out only the parents
    // it lists.
    let dim = |role: StyleRole| if commit.is_merge() { StyleRole::Muted } else { role };
    push_fit(&mut spans, &commit.summary, text_width, dim(StyleRole::Plain));

    if meta > 0 {
        if show_author {
            spans.push(TextSpan::plain(" "));
            push_fit(&mut spans, &commit.author_name, AUTHOR_WIDTH, dim(StyleRole::Author));
        }
        spans.push(TextSpan::plain(" "));
        let date = match graph.dates {
            DateStyle::Relative => relative(now, commit.commit_time),
            DateStyle::Clock => clock_time(commit.commit_time, graph.clock().utc_offset(commit.commit_time)),
        };
        spans.push(TextSpan::new(fit_right(&date, date_width), dim(StyleRole::Date)));
        spans.push(TextSpan::plain(" "));
        spans.push(TextSpan::new(commit.short_oid().to_string(), dim(StyleRole::Sha)));
    }
    spans
}

/// `n` spaces, borrowed when there are few.
fn spaces(n: usize) -> Cow<'static, str> {
    const SPACES: &str = "                                                                ";
    match SPACES.get(..n) {
        Some(s) => s.into(),
        None => " ".repeat(n).into(),
    }
}

/// Pushes what [`fit`] makes of `s`, borrowing `s` when it needn't be cut.
fn push_fit<'a>(spans: &mut Vec<TextSpan<'a>>, s: &'a str, width: usize, role: StyleRole) {
    let used = s.width();
    if used > width {
        spans.push(TextSpan::new(fit(s, width), role));
        return;
    }
    spans.push(TextSpan::new(s, role));
    if used < width {
        spans.push(TextSpan::new(spaces(width - used), role));
    }
}

fn label_text(label: &RefLabel) -> (String, StyleRole) {
    match label.kind {
        LabelKind::Head => (format!(" {} ", label.name), StyleRole::LabelHead),
        LabelKind::DetachedHead => (" HEAD ".to_string(), StyleRole::LabelDetached),
        LabelKind::Local => (format!("[{}]", label.name), StyleRole::LabelLocal),
        LabelKind::Remote => (format!("[{}]", label.name), StyleRole::LabelRemote),
        LabelKind::Tag => (format!("<{}>", label.name), StyleRole::LabelTag),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use tt_git_core::time::FixedClock;
    use tt_git_core::{Commit, GitRef, Head, Oid, RefKind};
    use tt_git_graph::{GraphBuilder, NodeId};

    use super::*;
    use crate::action::RefsSnapshot;
    use crate::component::Component;
    use crate::jobs::Ctx;
    use crate::keymap::Command;

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

    fn ctx() -> Ctx {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        Ctx::new(tx)
    }

    /// A graph with `commits` loaded.
    fn loaded(commits: Vec<Commit>) -> GraphPane {
        let mut graph = GraphPane::new(Arc::new(FixedClock(0)));
        let load = graph.begin_load();
        graph.apply_batch(load.generation(), commits, true, &ctx());
        graph
    }

    fn text(spans: &[TextSpan]) -> String {
        spans.iter().map(|s| s.text.as_ref()).collect()
    }

    fn role_of(spans: &[TextSpan], text: &str) -> StyleRole {
        spans.iter().find(|s| s.text.starts_with(text)).expect(text).role
    }

    /// The graph columns as drawn before rows were packed: from the whole
    /// row as laid out.
    fn unpacked_graph_spans(row: &tt_git_graph::Row, lanes: usize) -> Vec<TextSpan<'static>> {
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
            spans.push(TextSpan::new(
                cell.glyph(node_glyph).to_string(),
                StyleRole::Lane(cell.color),
            ));
            let filler = match row.fillers[i] {
                Some(c) if i + 1 < shown || overflow => TextSpan::new("─", StyleRole::Lane(c)),
                _ => TextSpan::plain(" "),
            };
            spans.push(filler);
        }
        if overflow {
            let glyph = if row.node_lane >= shown { node_glyph } else { '…' };
            spans.push(TextSpan::new(format!("{glyph} "), StyleRole::Lane(row.node_color)));
        }
        let graph_used = (shown + usize::from(overflow)) * 2;
        spans.push(TextSpan::plain(" ".repeat((lanes * 2).saturating_sub(graph_used) + 1)));
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

        for max_lanes in [2, 5, 16, 64] {
            let mut graph = GraphPane::new(Arc::new(FixedClock(0)));
            graph.max_lanes = max_lanes;
            let load = graph.begin_load();
            graph.apply_batch(load.generation(), commits.clone(), true, &ctx());
            for lanes in 1..=max_lanes.max(2) {
                for (i, row) in whole.iter().enumerate() {
                    let want = unpacked_graph_spans(row, lanes);
                    let spans = row_spans(&graph, i, lanes, 200, 0);
                    assert_eq!(spans[..want.len()], want[..], "max {max_lanes}, {lanes} lanes, row {i}");
                }
            }
        }
    }

    #[test]
    fn merge_commits_are_dimmed() {
        let (m, a, b) = ("a".repeat(40), "b".repeat(40), "c".repeat(40));
        let graph = loaded(vec![commit(&m, &[&a, &b]), commit(&a, &[]), commit(&b, &[])]);
        let merge = row_spans(&graph, 0, 2, 200, 0);
        for text in ["a commit", "Ann", "aaaaaaa"] {
            assert_eq!(role_of(&merge, text), StyleRole::Muted, "merge {text}");
        }
        let plain = row_spans(&graph, 1, 2, 200, 0);
        assert_eq!(role_of(&plain, "a commit"), StyleRole::Plain);
        assert_eq!(role_of(&plain, "Ann"), StyleRole::Author);
        assert_eq!(role_of(&plain, "bbbbbbb"), StyleRole::Sha);
    }

    #[test]
    fn filtered_merges_stay_dimmed_without_open_lanes() {
        // The filter matched the merge and an unrelated commit, not the
        // merge's parents.
        let (m, a, b, c) = ("a".repeat(40), "b".repeat(40), "c".repeat(40), "d".repeat(40));
        let mut graph = GraphPane::new(Arc::new(FixedClock(0)));
        graph.filter.message = Some("commit".into());
        let load = graph.begin_load();
        graph.apply_batch(
            load.generation(),
            vec![commit(&m, &[&a, &b]), commit(&c, &[&a])],
            true,
            &ctx(),
        );

        assert!(graph.commits()[0].is_merge(), "keeps its parents");
        let rows: Vec<(usize, usize)> = graph.rows().iter().map(|r| (r.node_lane(), r.width())).collect();
        assert_eq!(rows, [(0, 1), (0, 1)], "no lanes to the commits left out");
        assert_eq!(role_of(&row_spans(&graph, 0, 2, 200, 0), "a commit"), StyleRole::Muted);
    }

    #[test]
    fn fitted_text_draws_as_fit_does() {
        for s in ["", "abc", "abcdef", "日本語", "a\u{301}b"] {
            for width in 0..8 {
                let mut spans = Vec::new();
                push_fit(&mut spans, s, width, StyleRole::Author);
                assert_eq!(text(&spans), fit(s, width), "{s:?} in {width}");
                assert!(spans.iter().all(|s| s.role == StyleRole::Author));
            }
        }
        assert_eq!(spaces(3), "   ");
        assert_eq!(spaces(100), " ".repeat(100));
    }

    #[test]
    fn rows_fill_the_width_and_drop_columns_when_narrow() {
        let (a, b) = ("a".repeat(40), "b".repeat(40));
        let graph = loaded(vec![commit(&a, &[&b]), commit(&b, &[])]);
        // One lane: "● " then a space before the text.
        for width in [80, 40, 20, 8] {
            let spans = row_spans(&graph, 0, 1, width, 0);
            assert_eq!(text(&spans).width(), width.max(3), "{width} columns");
        }
        let wide = text(&row_spans(&graph, 0, 1, 80, 0));
        assert!(wide.contains("Ann") && wide.ends_with("aaaaaaa"), "{wide}");
        let small = text(&row_spans(&graph, 0, 1, 40, 0));
        assert!(!small.contains("Ann") && small.ends_with("aaaaaaa"), "{small}");
        let tiny = text(&row_spans(&graph, 0, 1, 20, 0));
        assert!(!tiny.contains("aaaaaaa"), "{tiny}");
    }

    #[test]
    fn labels_come_first_and_give_way_to_the_width() {
        let (a, b) = ("a".repeat(40), "b".repeat(40));
        let mut graph = loaded(vec![commit(&a, &[&b]), commit(&b, &[])]);
        let r = |full: &str, short: &str, kind| GitRef {
            full_name: full.into(),
            short_name: short.into(),
            kind,
            target: a.clone(),
            date: 0,
            upstream: None,
            ahead: 0,
            behind: 0,
            upstream_gone: false,
            is_head: false,
            symref: None,
            subject: String::new(),
        };
        let refs = vec![
            r("refs/heads/main", "main", RefKind::LocalBranch),
            r("refs/tags/v1", "v1", RefKind::Tag),
        ];
        graph.set_refs(&RefsSnapshot {
            head: Head::Branch {
                name: "main".into(),
                oid: a.clone(),
            },
            refs,
            checkouts: Vec::new(),
            branches: Vec::new(),
            base: None,
            stashes: Vec::new(),
            worktrees: Vec::new(),
        });
        let spans = row_spans(&graph, 0, 0, 80, 0);
        assert_eq!(spans[0], TextSpan::new(" main ", StyleRole::LabelHead));
        assert_eq!(spans[2], TextSpan::new("<v1>", StyleRole::LabelTag));
        assert_eq!(role_of(&spans, "a commit"), StyleRole::Plain);
        // Too narrow for the tag: it's left out, the summary isn't.
        let narrow = row_spans(&graph, 0, 0, 10, 0);
        assert_eq!(text(&narrow), " main  a …");
    }

    #[test]
    fn view_marks_the_selection_and_its_range() {
        let oids: Vec<String> = (0..6).map(|i| format!("{i:040x}")).collect();
        let commits = (0..6)
            .map(|i| {
                let parents: Vec<&str> = oids.get(i + 1).map(String::as_str).into_iter().collect();
                commit(&oids[i], &parents)
            })
            .collect();
        let mut graph = loaded(commits);
        graph.set_viewport(3);
        let view = graph.view(3, 80, 0);
        assert_eq!(view.range, 0..3);
        assert_eq!(view.lanes, 1);
        let picked: Vec<bool> = view.rows.iter().map(|r| r.picked).collect();
        assert_eq!(picked, [true, false, false]);

        // Down to the fifth commit: the viewport scrolls to keep it in view.
        let ctx = ctx();
        for _ in 0..4 {
            graph.on_command(Command::MoveDown, &ctx);
        }
        graph.set_viewport(3);
        let view = graph.view(3, 80, 0);
        assert_eq!(view.range, 2..5);
        assert_eq!(view.rows.iter().map(|r| r.index).collect::<Vec<_>>(), [2, 3, 4]);
        assert_eq!(
            view.rows.iter().map(|r| r.picked).collect::<Vec<_>>(),
            [false, false, true]
        );

        // A range from there up two rows picks all three.
        graph.on_command(Command::SelectRange, &ctx);
        graph.on_command(Command::MoveUp, &ctx);
        graph.on_command(Command::MoveUp, &ctx);
        graph.set_viewport(3);
        let view = graph.view(3, 80, 0);
        assert!(view.rows.iter().all(|r| r.picked), "{view:?}");

        graph.lanes = false;
        assert_eq!(graph.view(3, 80, 0).lanes, 0);
        assert!(graph.view(3, 80, 0).rows[0].spans[0].text.starts_with("a commit"));
    }

    #[test]
    fn empty_graphs_say_why() {
        let mut graph = GraphPane::new(Arc::new(FixedClock(0)));
        assert_eq!(graph.empty_message(), Some("no commits"));
        graph.begin_load();
        assert_eq!(graph.empty_message(), Some("loading…"));
        let graph = loaded(vec![commit(&"a".repeat(40), &[])]);
        assert_eq!(graph.empty_message(), None);
    }
}
