//! The graph pane (Phase B1): one row per commit, from the graph's view
//! model. The commit text (labels, summary, author, date, hash) is a
//! [`StyledRows`] list as in the TUI; the lanes left of it are painted over
//! the list on one canvas, as lines, rounded corners and round nodes, from
//! each row's [`LaneRow`].
//!
//! The lane columns keep the TUI's widths, two text columns per lane, so the
//! text after them lines up as it does in the terminal and the view model
//! still fits it to the width in columns.

use std::rc::Rc;

use gpui::{
    AnyElement, BorderStyle, Bounds, ContentMask, Hsla, IntoElement, ParentElement, PathBuilder, Pixels, Point, Size,
    Styled, Window, canvas, div, point, prelude::FluentBuilder, px, quad, transparent_black,
};
use tt_git_app::panes::PaneId;
use tt_git_app::view::graph::{GraphRow, LaneOverflow, LaneRow};
use tt_git_app::view::{StyleRole, TextSpan};

use super::PaneCx;
use crate::frame::PaneFrame;
use crate::rows::{StyledRow, StyledRows};
use crate::text::selection;
use crate::theme::GuiTheme;

/// The id the commit list is measured as.
pub const ROWS: &str = "graph";

/// How many lane colors there are (`StyleRole::Lane(0..)`), as the graph
/// layout hands them out.
const LANE_COLORS: usize = 8;

pub fn render(cx: &mut PaneCx) -> AnyElement {
    let (ui, focused) = (cx.ui, cx.focused);
    let graph = cx.app.graph_mut();
    let now = graph.clock().now();
    let frame = PaneFrame::new(ui, PaneId::Graph, focused).title(graph.title());

    if let Some(msg) = graph.empty_message() {
        graph.note_drawn(now, 0..0);
        return frame
            .child(div().text_color(ui.theme.fg(StyleRole::Muted)).child(msg))
            .into_any_element();
    }

    let fit = ui.fit(ROWS);
    graph.set_viewport(fit.rows);
    let view = graph.view(fit.rows, fit.cols, now);
    let range = view.range.clone();
    let rows = view
        .rows
        .iter()
        .map(|row| StyledRow::new(&ui.theme, &text_spans(row), selection(row.picked, focused)))
        .collect();
    let lanes = (view.lanes > 0).then(|| {
        let rows: Vec<LaneRow> = view.rows.iter().map(|r| r.lanes.clone()).collect();
        lanes_canvas(ui.theme.clone(), ui.font.cell(), view.lanes, rows)
    });
    graph.note_drawn(now, range);
    frame
        .child(
            div()
                .relative()
                .size_full()
                .child(StyledRows::new(ui, ROWS, PaneId::Graph, rows))
                .when_some(lanes, |d, lanes| d.child(lanes)),
        )
        .into_any_element()
}

/// A row's text: blank where the lanes are painted, then the commit.
fn text_spans<'a>(row: &'a GraphRow<'a>) -> Vec<TextSpan<'a>> {
    let blank: usize = row.spans[..row.lane_spans].iter().map(|s| s.text.chars().count()).sum();
    let mut spans = Vec::with_capacity(row.spans.len() - row.lane_spans + 1);
    if blank > 0 {
        spans.push(TextSpan::plain(" ".repeat(blank)));
    }
    spans.extend(row.text().iter().cloned());
    spans
}

/// The lanes of `rows` (top to bottom, one line each) in `lanes` columns,
/// painted over the list's left edge.
fn lanes_canvas(theme: Rc<GuiTheme>, cell: Size<Pixels>, lanes: usize, rows: Vec<LaneRow>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let colors: Vec<Hsla> = (0..LANE_COLORS as u8).map(|c| theme.fg(StyleRole::Lane(c))).collect();
            let shapes = shapes(&rows, cell);
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                paint(&shapes, bounds.origin, cell, &colors, window);
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .h_full()
    .w(cell.width * (lanes * 2) as f32)
}

/// Something drawn for the lanes, in pixels from the top left of the list,
/// in lane color `color`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Shape {
    /// A straight line.
    Line {
        from: Point<Pixels>,
        to: Point<Pixels>,
        color: u8,
    },
    /// A line that turns a corner: straight from `from` to `bend`, then
    /// curving round `ctrl` to `to`.
    Corner {
        from: Point<Pixels>,
        bend: Point<Pixels>,
        ctrl: Point<Pixels>,
        to: Point<Pixels>,
        color: u8,
    },
    /// A commit: a dot, or a ring around one for a merge.
    Node {
        center: Point<Pixels>,
        merge: bool,
        color: u8,
    },
    /// The lanes go on past the ones drawn: three dots.
    More { center: Point<Pixels>, color: u8 },
}

/// Lane `i`'s line runs down the middle of its two columns.
fn lane_x(i: usize, cell: Size<Pixels>) -> Pixels {
    cell.width * (i * 2 + 1) as f32
}

/// How far from the middle of a cell a corner starts to turn.
fn corner_radius(cell: Size<Pixels>) -> Pixels {
    cell.width.min(cell.height / 2.)
}

/// A node's radius.
fn node_radius(cell: Size<Pixels>) -> Pixels {
    (cell.width.min(cell.height / 2.) * 0.7).round()
}

/// The width of a lane's line.
fn stroke_width(cell: Size<Pixels>) -> Pixels {
    (cell.height / 10.).max(px(1.5))
}

/// What to draw for `rows`, one per line of `cell.height`.
///
/// Each cell's lines run from the middle of the row to its top (`up`) and
/// bottom (`down`), so a lane is one unbroken line down the rows. A cell
/// with one line up or down and one to the side is a corner, drawn round.
/// The lines across gaps join the lanes, stopping short of a corner (which
/// curves the rest of the way) or a node (drawn over them).
fn shapes(rows: &[LaneRow], cell: Size<Pixels>) -> Vec<Shape> {
    let corner = corner_radius(cell);
    let node = node_radius(cell);
    let mut shapes = Vec::new();
    let mut nodes = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let top = cell.height * r as f32;
        let mid = top + cell.height / 2.;
        let bottom = top + cell.height;
        let is_corner = |i: usize| {
            row.cells
                .get(i)
                .is_some_and(|c| !c.node && c.up != c.down && c.left != c.right)
        };
        for (i, c) in row.cells.iter().enumerate() {
            let x = lane_x(i, cell);
            let color = c.color;
            if is_corner(i) {
                let (from, bend) = if c.up {
                    (point(x, top), point(x, mid - corner))
                } else {
                    (point(x, bottom), point(x, mid + corner))
                };
                let to = if c.left {
                    point(x - corner, mid)
                } else {
                    point(x + corner, mid)
                };
                shapes.push(Shape::Corner {
                    from,
                    bend,
                    ctrl: point(x, mid),
                    to,
                    color,
                });
            } else {
                // Lines into a node stop at its edge, so a merge's ring
                // stays open.
                let gap = if c.node { node } else { px(0.) };
                match (c.up, c.down) {
                    (true, true) if !c.node => shapes.push(Shape::Line {
                        from: point(x, top),
                        to: point(x, bottom),
                        color,
                    }),
                    (up, down) => {
                        if up {
                            shapes.push(Shape::Line {
                                from: point(x, top),
                                to: point(x, mid - gap),
                                color,
                            });
                        }
                        if down {
                            shapes.push(Shape::Line {
                                from: point(x, mid + gap),
                                to: point(x, bottom),
                                color,
                            });
                        }
                    }
                }
            }
            if let Some(gap_color) = c.gap {
                let next = i + 1;
                let next_is_node =
                    row.cells.get(next).is_some_and(|c| c.node) || (next == row.cells.len() && row.overflow.is_some());
                let start = match () {
                    _ if c.node => x + node,
                    _ if is_corner(i) => x + corner,
                    _ => x,
                };
                let end_x = lane_x(next, cell);
                let end = match () {
                    _ if next_is_node => end_x - node,
                    _ if is_corner(next) => end_x - corner,
                    _ => end_x,
                };
                shapes.push(Shape::Line {
                    from: point(start, mid),
                    to: point(end, mid),
                    color: gap_color,
                });
            }
            if c.node {
                nodes.push(Shape::Node {
                    center: point(x, mid),
                    merge: row.merge,
                    color,
                });
            }
        }
        if let Some(mark) = row.overflow {
            let center = point(lane_x(row.cells.len(), cell), mid);
            let color = row.node_color;
            nodes.push(match mark {
                LaneOverflow::Node => Shape::Node {
                    center,
                    merge: row.merge,
                    color,
                },
                LaneOverflow::More => Shape::More { center, color },
            });
        }
    }
    // Nodes go on top of the lines.
    shapes.extend(nodes);
    shapes
}

/// Paints `shapes` with the list's top left at `origin`, lines batched into
/// one path per color.
fn paint(shapes: &[Shape], origin: Point<Pixels>, cell: Size<Pixels>, colors: &[Hsla], window: &mut Window) {
    let width = stroke_width(cell);
    let color = |c: u8| colors.get(usize::from(c)).copied().unwrap_or_default();
    let mut lines: Vec<Option<PathBuilder>> = (0..colors.len()).map(|_| None).collect();
    fn builder(lines: &mut [Option<PathBuilder>], c: u8, width: Pixels) -> Option<&mut PathBuilder> {
        let slot = lines.get_mut(usize::from(c))?;
        Some(slot.get_or_insert_with(|| PathBuilder::stroke(width)))
    }
    let at = |p: Point<Pixels>| origin + p;
    for shape in shapes {
        match *shape {
            Shape::Line { from, to, color } => {
                if let Some(b) = builder(&mut lines, color, width) {
                    b.move_to(at(from));
                    b.line_to(at(to));
                }
            }
            Shape::Corner {
                from,
                bend,
                ctrl,
                to,
                color,
            } => {
                if let Some(b) = builder(&mut lines, color, width) {
                    b.move_to(at(from));
                    b.line_to(at(bend));
                    b.curve_to(at(to), at(ctrl));
                }
            }
            Shape::Node { .. } | Shape::More { .. } => {}
        }
    }
    for (c, path) in lines.into_iter().enumerate() {
        if let Some(path) = path.and_then(|b| b.build().ok()) {
            window.paint_path(path, colors[c]);
        }
    }

    let dot = |center: Point<Pixels>, radius: Pixels, fill: Hsla, window: &mut Window| {
        let bounds = Bounds::centered_at(at(center), Size::new(radius * 2., radius * 2.));
        window.paint_quad(quad(
            bounds,
            radius,
            fill,
            px(0.),
            transparent_black(),
            BorderStyle::Solid,
        ));
    };
    let radius = node_radius(cell);
    for shape in shapes {
        match *shape {
            Shape::Node {
                center,
                merge: false,
                color: c,
            } => dot(center, radius, color(c), window),
            Shape::Node {
                center,
                merge: true,
                color: c,
            } => {
                let bounds = Bounds::centered_at(at(center), Size::new(radius * 2., radius * 2.));
                window.paint_quad(quad(
                    bounds,
                    radius,
                    transparent_black(),
                    width,
                    color(c),
                    BorderStyle::Solid,
                ));
                dot(center, (radius * 0.4).max(px(1.5)), color(c), window);
            }
            Shape::More { center, color: c } => {
                let r = (width * 0.75).max(px(1.));
                let step = cell.width / 3.;
                for k in [-1., 0., 1.] {
                    dot(point(center.x + step * k, center.y), r, color(c), window);
                }
            }
            Shape::Line { .. } | Shape::Corner { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Modifiers, TestAppContext, size};
    use tt_git_app::view::graph::LaneCell;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::testing::Gui;

    const CELL: Size<Pixels> = Size {
        width: px(8.),
        height: px(20.),
    };

    fn cell(up: bool, down: bool, left: bool, right: bool, node: bool) -> LaneCell {
        LaneCell {
            up,
            down,
            left,
            right,
            node,
            ..LaneCell::default()
        }
    }

    fn lines(shapes: &[Shape]) -> Vec<(Point<Pixels>, Point<Pixels>)> {
        shapes
            .iter()
            .filter_map(|s| match *s {
                Shape::Line { from, to, .. } => Some((from, to)),
                _ => None,
            })
            .collect()
    }

    fn p(x: f32, y: f32) -> Point<Pixels> {
        point(px(x), px(y))
    }

    #[test]
    fn lanes_run_down_the_middle_of_their_columns() {
        // Row 0: a commit in lane 0, lane 1 running past.
        let row = LaneRow {
            cells: vec![
                cell(true, true, false, false, true),
                cell(true, true, false, false, false),
            ],
            ..LaneRow::default()
        };
        let shapes = shapes(&[row.clone(), row], CELL);
        let node = f32::from(node_radius(CELL));
        assert_eq!(
            lines(&shapes),
            [
                (p(8., 0.), p(8., 10. - node)),
                (p(8., 10. + node), p(8., 20.)),
                (p(24., 0.), p(24., 20.)),
                (p(8., 20.), p(8., 30. - node)),
                (p(8., 30. + node), p(8., 40.)),
                (p(24., 20.), p(24., 40.)),
            ]
        );
        // The nodes come last, over the lines.
        assert_eq!(
            shapes[shapes.len() - 2..],
            [
                Shape::Node {
                    center: p(8., 10.),
                    merge: false,
                    color: 0,
                },
                Shape::Node {
                    center: p(8., 30.),
                    merge: false,
                    color: 0,
                }
            ]
        );
    }

    #[test]
    fn merges_curve_into_their_lanes() {
        // A merge in lane 0 opening lane 1 for its second parent (╮), in
        // lane color 3.
        let row = LaneRow {
            cells: vec![
                LaneCell {
                    gap: Some(3),
                    ..cell(false, true, false, true, true)
                },
                LaneCell {
                    color: 3,
                    ..cell(false, true, true, false, false)
                },
            ],
            merge: true,
            ..LaneRow::default()
        };
        let shapes = shapes(&[row], CELL);
        let (node, corner) = (f32::from(node_radius(CELL)), f32::from(corner_radius(CELL)));
        assert!(shapes.contains(&Shape::Corner {
            from: p(24., 20.),
            bend: p(24., 10. + corner),
            ctrl: p(24., 10.),
            to: p(24. - corner, 10.),
            color: 3,
        }));
        // The line across the gap goes from the node's edge to the corner.
        assert!(shapes.contains(&Shape::Line {
            from: p(8. + node, 10.),
            to: p(24. - corner, 10.),
            color: 3,
        }));
        assert!(shapes.contains(&Shape::Node {
            center: p(8., 10.),
            merge: true,
            color: 0,
        }));
    }

    #[test]
    fn rows_too_wide_end_in_a_mark() {
        let row = |mark| LaneRow {
            cells: vec![LaneCell {
                gap: Some(1),
                ..cell(true, true, false, true, false)
            }],
            overflow: Some(mark),
            node_color: 5,
            ..LaneRow::default()
        };
        let more = shapes(&[row(LaneOverflow::More)], CELL);
        assert_eq!(
            more.last(),
            Some(&Shape::More {
                center: p(24., 10.),
                color: 5
            })
        );
        let node = shapes(&[row(LaneOverflow::Node)], CELL);
        assert!(node.contains(&Shape::Node {
            center: p(24., 10.),
            merge: false,
            color: 5,
        }));
        // The gap line stops at the node's edge.
        assert!(node.contains(&Shape::Line {
            from: p(8., 10.),
            to: p(24. - f32::from(node_radius(CELL)), 10.),
            color: 1,
        }));
    }

    #[test]
    fn sizes_scale_with_the_font() {
        let big = size(px(10.), px(24.));
        assert!(node_radius(big) > node_radius(CELL));
        assert!(stroke_width(CELL) >= px(1.5));
        assert_eq!(corner_radius(CELL), px(8.));
    }

    /// A sandbox with a merge: `main` and `topic` both off `base`.
    fn merged() -> Sandbox {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "1", "base");
        let main = sb.current_branch();
        sb.checkout_new("topic");
        sb.commit_file("b.txt", "1", "on topic");
        sb.checkout(&main);
        sb.commit_file("a.txt", "2", "on main");
        sb.merge("topic");
        sb
    }

    #[gpui::test]
    fn shows_the_sandbox_commits(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        for i in 1..=5 {
            sb.commit_file("a.txt", &i.to_string(), &format!("commit {i}"));
        }
        let mut gui = Gui::open(cx, &sb);
        let fit = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(ROWS));
        assert!(
            fit.rows > 5 && fit.cols > 60,
            "the list was drawn and measured: {fit:?}"
        );
        let text = gui.model.update(&mut gui.cx, |model, _| {
            let graph = model.app_mut().graph_mut();
            let view = graph.view(fit.rows, fit.cols, graph.clock().now());
            assert_eq!(view.range, 0..5);
            view.rows
                .iter()
                .map(|r| text_spans(r).iter().map(|s| s.text.as_ref()).collect::<String>())
                .collect::<Vec<_>>()
        });
        assert!(
            text[0].starts_with("   ") && text[0].contains("commit 5") && text[4].contains("commit 1"),
            "{text:#?}"
        );
        assert!(
            text.iter().all(|t| !t.contains('●') && !t.contains('│')),
            "the lanes are painted, not written: {text:#?}"
        );
        // The pane told the graph how many rows it shows.
        gui.press("ctrl+d");
        assert_eq!(
            gui.read(|app| app.graph().selected_index()),
            4,
            "a page down goes to the end"
        );
    }

    #[gpui::test]
    fn a_merge_draws_two_lanes(cx: &mut TestAppContext) {
        let sb = merged();
        let mut gui = Gui::open(cx, &sb);
        let fit = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(ROWS));
        let font = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().font.cell());
        let (lanes, rows) = gui.model.update(&mut gui.cx, |model, _| {
            let graph = model.app_mut().graph_mut();
            let view = graph.view(fit.rows, fit.cols, graph.clock().now());
            (
                view.lanes,
                view.rows.iter().map(|r| r.lanes.clone()).collect::<Vec<_>>(),
            )
        });
        assert_eq!(lanes, 2);
        assert_eq!(rows.len(), 4);
        assert!(rows[0].merge, "the merge is on top");
        let shapes = shapes(&rows, font);
        let nodes = shapes.iter().filter(|s| matches!(s, Shape::Node { .. })).count();
        let corners = shapes.iter().filter(|s| matches!(s, Shape::Corner { .. })).count();
        assert_eq!(nodes, 4, "one per commit");
        assert_eq!(corners, 2, "out of the merge and back into base");
    }

    #[gpui::test]
    fn clicks_and_the_wheel_move_the_selection(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        for i in 1..=60 {
            sb.commit_file("a.txt", &i.to_string(), &format!("commit {i}"));
        }
        let mut gui = Gui::open(cx, &sb);
        let (bounds, line) = gui.shell.read_with(&gui.cx, |shell, _| {
            let ui = shell.ui();
            (ui.measures.get(ROWS).expect("list measured"), ui.font.line_height)
        });
        // The third line, over the lanes.
        let third = point(bounds.left() + px(4.), bounds.top() + line * 2.5);
        gui.cx.simulate_click(third, Modifiers::none());
        gui.wait_idle();
        assert_eq!(gui.read(|app| app.graph().selected_index()), 2);
        assert_eq!(gui.read(|app| app.focus()), PaneId::Graph);

        // A range from there, picked with keys as in the TUI.
        gui.press_all(&["v", "j", "j"]);
        let picked = gui.model.update(&mut gui.cx, |model, _| {
            let graph = model.app_mut().graph_mut();
            let view = graph.view(10, 80, graph.clock().now());
            view.rows
                .iter()
                .filter(|r| r.picked)
                .map(|r| r.index)
                .collect::<Vec<_>>()
        });
        assert_eq!(picked, [2, 3, 4], "the range is picked");
        gui.press("esc");

        // Two notches down: the app moves six rows.
        gui.cx.simulate_event(gpui::ScrollWheelEvent {
            position: third,
            delta: gpui::ScrollDelta::Lines(point(0., -6.)),
            ..Default::default()
        });
        gui.wait_idle();
        assert_eq!(gui.read(|app| app.graph().selected_index()), 4 + 6);
    }
}
