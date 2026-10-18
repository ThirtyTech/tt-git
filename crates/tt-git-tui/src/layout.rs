//! The configurable pane layout: a tree of row/column splits with panes at
//! the leaves (`[layout]` in `config.toml`).
//!
//! ```toml
//! [layout]
//! split = "rows"
//! children = [
//!   { pane = "graph", size = 70 },
//!   { split = "columns", details = true, children = [{ pane = "status" }, { pane = "branches" }] },
//! ]
//! ```
//!
//! The details pane either has a leaf of its own (always shown) or opens in
//! place of the one node marked `details = true`.

use ratatui::layout::{Constraint, Direction, Rect};
use serde::Deserialize;

use crate::panes::PaneId;

/// How a split arranges its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Split {
    /// Stacked top to bottom.
    Rows,
    /// Side by side, left to right.
    Columns,
}

/// One node of the layout tree as written in the config.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    #[serde(default)]
    pub pane: Option<PaneId>,
    #[serde(default)]
    pub split: Option<Split>,
    #[serde(default)]
    pub children: Vec<Node>,
    /// Percentage of the parent split. Children without one share what's left.
    #[serde(default)]
    pub size: Option<u16>,
    /// The details pane opens in place of this node.
    #[serde(default)]
    pub details: bool,
}

impl Node {
    fn pane(pane: PaneId, size: Option<u16>) -> Self {
        Self {
            pane: Some(pane),
            split: None,
            children: Vec::new(),
            size,
            details: false,
        }
    }

    fn split(split: Split, size: Option<u16>, children: Vec<Node>) -> Self {
        Self {
            pane: None,
            split: Some(split),
            children,
            size,
            details: false,
        }
    }

    fn walk<'a>(&'a self, out: &mut Vec<&'a Node>) {
        out.push(self);
        for child in &self.children {
            child.walk(out);
        }
    }

    fn contains(&self, pane: PaneId) -> bool {
        self.pane == Some(pane) || self.children.iter().any(|c| c.contains(pane))
    }
}

/// A validated layout tree.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "Node")]
pub struct Layout {
    root: Node,
}

/// Graph across the top, status and branches below; details replaces the
/// bottom row when opened.
impl Default for Layout {
    fn default() -> Self {
        let mut bottom = Node::split(
            Split::Columns,
            None,
            vec![Node::pane(PaneId::Status, None), Node::pane(PaneId::Branches, None)],
        );
        bottom.details = true;
        let root = Node::split(Split::Rows, None, vec![Node::pane(PaneId::Graph, Some(70)), bottom]);
        Self::try_from(root).expect("default layout is valid")
    }
}

impl TryFrom<Node> for Layout {
    type Error = String;

    fn try_from(root: Node) -> Result<Self, String> {
        let mut nodes = Vec::new();
        root.walk(&mut nodes);
        for node in &nodes {
            match (node.pane, node.split) {
                (Some(_), Some(_)) => return Err("a layout node has either `pane` or `split`, not both".into()),
                (None, None) => return Err("every layout node needs `pane` or `split`".into()),
                (Some(p), None) if !node.children.is_empty() => {
                    return Err(format!("pane `{}` can't have children", p.name()));
                }
                (None, Some(_)) if node.children.is_empty() => return Err("a split needs children".into()),
                _ => {}
            }
            if let Some(size) = node.size
                && !(1..=99).contains(&size)
            {
                return Err(format!("size {size} is not between 1 and 99"));
            }
            let sized: u16 = node.children.iter().filter_map(|c| c.size).sum();
            let unsized_children = node.children.iter().any(|c| c.size.is_none());
            if sized > 100 || (sized >= 100 && unsized_children) {
                return Err("the sizes in a split leave no room for its other children".into());
            }
        }
        for pane in [PaneId::Status, PaneId::Branches, PaneId::Graph] {
            match nodes.iter().filter(|n| n.pane == Some(pane)).count() {
                1 => {}
                0 => return Err(format!("the layout has no `{}` pane", pane.name())),
                _ => return Err(format!("`{}` appears more than once", pane.name())),
            }
        }
        let leaves = nodes.iter().filter(|n| n.pane == Some(PaneId::Details)).count();
        let hosts: Vec<_> = nodes.iter().filter(|n| n.details).collect();
        match (leaves, hosts.as_slice()) {
            (1, []) => {}
            (0, [host]) if host.pane == Some(PaneId::Details) => {}
            (0, [host]) if host.contains(PaneId::Graph) => {
                return Err("details can't replace a node that holds the graph".into());
            }
            (0, [_]) => {}
            (0, []) => return Err("add a `details` pane or mark one node `details = true`".into()),
            _ => return Err("place details once: one `details` pane or one `details = true` node".into()),
        }
        Ok(Self { root })
    }
}

/// What the layout should show right now.
#[derive(Debug, Clone, Copy, Default)]
pub struct View {
    /// Details has been opened (its host node shows details instead).
    pub details: bool,
    /// Only the graph and, if it's showing, details.
    pub wide: bool,
}

/// A node with hidden panes pruned and sizes turned into fill weights.
enum Placed {
    Pane(PaneId),
    Split(Split, Vec<(u16, Placed)>),
}

impl Layout {
    /// Where each visible pane goes inside `area`.
    pub fn place(&self, area: Rect, view: View) -> Vec<(PaneId, Rect)> {
        let mut out = Vec::new();
        if let Some(placed) = resolve(&self.root, view) {
            assign(&placed, area, &mut out);
        }
        out
    }

    /// Whether `pane` is on screen in `view`.
    pub fn shows(&self, pane: PaneId, view: View) -> bool {
        self.place(Rect::new(0, 0, 100, 100), view)
            .iter()
            .any(|(p, _)| *p == pane)
    }
}

fn resolve(node: &Node, view: View) -> Option<Placed> {
    if node.details && view.details {
        return Some(Placed::Pane(PaneId::Details));
    }
    if let Some(pane) = node.pane {
        let hidden = view.wide && !matches!(pane, PaneId::Graph | PaneId::Details);
        return (!hidden).then_some(Placed::Pane(pane));
    }
    let sized: u16 = node.children.iter().filter_map(|c| c.size).sum();
    let unsized_count = node.children.iter().filter(|c| c.size.is_none()).count() as u16;
    let share = (100 - sized).checked_div(unsized_count).unwrap_or(0).max(1);
    let mut children: Vec<(u16, Placed)> = node
        .children
        .iter()
        .filter_map(|c| resolve(c, view).map(|p| (c.size.unwrap_or(share), p)))
        .collect();
    match children.len() {
        0 => None,
        1 => children.pop().map(|(_, p)| p),
        _ => Some(Placed::Split(node.split.expect("validated"), children)),
    }
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
        layout.place(AREA, view)
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
        assert_eq!(panes, PaneId::ALL);
    }

    #[test]
    fn wide_keeps_graph_and_details_at_full_width() {
        let wide = View {
            details: true,
            wide: true,
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

    fn parse_err(layout: &str) -> String {
        Config::parse(&format!("[layout]\n{layout}")).unwrap_err().to_string()
    }

    #[test]
    fn rejects_bad_layouts() {
        let cases = [
            (
                r#"split = "rows"
                children = [{ pane = "graph" }, { pane = "status" }, { pane = "details" }]"#,
                "no `branches` pane",
            ),
            (
                r#"split = "rows"
                children = [{ pane = "graph" }, { pane = "status" }, { pane = "branches" }]"#,
                "add a `details` pane",
            ),
            (
                r#"split = "rows"
                details = true
                children = [{ pane = "graph" }, { pane = "status" }, { pane = "branches" }]"#,
                "holds the graph",
            ),
            (
                r#"split = "rows"
                children = [{ pane = "graph", size = 80 }, { pane = "status", size = 20 }, { pane = "branches", details = true }]"#,
                "no room",
            ),
            (
                r#"split = "rows"
                children = [{ pane = "graph" }, { pane = "graph" }, { pane = "status" }, { pane = "branches", details = true }]"#,
                "more than once",
            ),
            (r#"pane = "sidebar""#, "unknown variant"),
        ];
        for (layout, expected) in cases {
            let err = parse_err(layout);
            assert!(err.contains(expected), "{expected:?} not in: {err}");
        }
    }
}
