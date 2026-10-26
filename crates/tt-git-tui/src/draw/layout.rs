//! The layout tree solved into terminal areas.

use ratatui::layout::{Constraint, Direction, Rect};

use crate::layout::{Layout, Placed, Split, View};
use crate::panes::PaneId;

/// Where each visible pane goes inside `area`.
pub fn place(layout: &Layout, area: Rect, view: View) -> Vec<(PaneId, Rect)> {
    let mut out = Vec::new();
    if let Some(placed) = layout.resolve(view) {
        assign(&placed, area, &mut out);
    }
    out
}

fn assign(placed: &Placed, area: Rect, out: &mut Vec<(PaneId, Rect)>) {
    match placed {
        Placed::Pane(pane) => out.push((*pane, area)),
        Placed::Split(split, children) => {
            let direction = match split {
                Split::Rows => Direction::Vertical,
                Split::Columns => Direction::Horizontal,
            };
            let areas = ratatui::layout::Layout::default()
                .direction(direction)
                .constraints(children.iter().map(|(w, _)| Constraint::Fill(*w)))
                .split(area);
            for ((_, child), area) in children.iter().zip(areas.iter()) {
                assign(child, *area, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    const AREA: Rect = Rect::new(0, 0, 100, 40);

    fn areas(layout: &Layout, view: View) -> Vec<(PaneId, Rect)> {
        place(layout, AREA, view)
    }

    #[test]
    fn default_puts_graph_over_status_and_branches() {
        let placed = areas(&Layout::default(), View::default());
        assert_eq!(
            placed,
            vec![
                (PaneId::Graph, Rect::new(0, 0, 100, 28)),
                (PaneId::Status, Rect::new(0, 28, 50, 12)),
                (PaneId::Branches, Rect::new(50, 28, 50, 12)),
            ]
        );
    }

    #[test]
    fn opened_details_replaces_its_host() {
        let placed = areas(
            &Layout::default(),
            View {
                details: true,
                ..View::default()
            },
        );
        assert_eq!(
            placed,
            vec![
                (PaneId::Graph, Rect::new(0, 0, 100, 28)),
                (PaneId::Details, Rect::new(0, 28, 100, 12)),
            ]
        );
    }

    fn classic() -> Layout {
        Config::parse(
            r#"
            [layout]
            split = "columns"
            children = [
              { size = 33, split = "rows", children = [{ pane = "status", size = 40 }, { pane = "branches" }] },
              { split = "rows", children = [{ pane = "graph", size = 65 }, { pane = "details" }] },
            ]
            "#,
        )
        .unwrap()
        .layout
    }

    #[test]
    fn details_leaf_is_always_shown() {
        let panes: Vec<_> = areas(&classic(), View::default()).into_iter().map(|(p, _)| p).collect();
        assert_eq!(
            panes,
            [PaneId::Status, PaneId::Branches, PaneId::Graph, PaneId::Details]
        );
    }

    #[test]
    fn wide_keeps_graph_and_details_at_full_width() {
        let wide = View {
            details: true,
            wide: true,
            ..View::default()
        };
        for layout in [Layout::default(), classic()] {
            let placed = areas(&layout, wide);
            assert_eq!(placed.len(), 2, "{placed:?}");
            assert_eq!(placed[0].0, PaneId::Graph);
            assert_eq!(placed[1].0, PaneId::Details);
            assert!(placed.iter().all(|(_, r)| r.x == 0 && r.width == 100), "{placed:?}");
        }
    }

    #[test]
    fn wide_with_details_closed_gives_the_graph_everything() {
        let wide = View {
            wide: true,
            ..View::default()
        };
        assert_eq!(areas(&Layout::default(), wide), vec![(PaneId::Graph, AREA)]);
    }
}
