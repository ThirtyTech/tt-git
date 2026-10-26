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
//!
//! [`Layout::resolve`] says which panes show and how the splits share the
//! space; each front end turns that into areas on its screen.

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
    /// Status in a full-height column on the left, with the rest of the
    /// layout beside it.
    pub status_column: bool,
}

/// How much of the width the status column takes when status is zoomed, in
/// percent.
pub const STATUS_COLUMN_SIZE: u16 = 27;

/// The layout tree with hidden panes pruned and sizes turned into fill
/// weights: each child of a split takes its weight's share of the space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placed {
    Pane(PaneId),
    Split(Split, Vec<(u16, Placed)>),
}

impl Placed {
    fn contains(&self, pane: PaneId) -> bool {
        match self {
            Placed::Pane(p) => *p == pane,
            Placed::Split(_, children) => children.iter().any(|(_, c)| c.contains(pane)),
        }
    }
}

impl Layout {
    /// The panes on screen in `view`, and how they share it; `None` when
    /// none are.
    pub fn resolve(&self, view: View) -> Option<Placed> {
        if view.status_column {
            let status = (STATUS_COLUMN_SIZE, Placed::Pane(PaneId::Status));
            let Some(rest) = resolve(&self.root, view) else {
                return Some(status.1);
            };
            return Some(Placed::Split(
                Split::Columns,
                vec![status, (100 - STATUS_COLUMN_SIZE, rest)],
            ));
        }
        resolve(&self.root, view)
    }

    /// Whether `pane` is on screen in `view`.
    pub fn shows(&self, pane: PaneId, view: View) -> bool {
        self.resolve(view).is_some_and(|placed| placed.contains(pane))
    }
}

fn resolve(node: &Node, view: View) -> Option<Placed> {
    if node.details && view.details {
        return Some(Placed::Pane(PaneId::Details));
    }
    if let Some(pane) = node.pane {
        // The status column draws status itself, beside the rest of the layout.
        let hidden = (view.wide && !matches!(pane, PaneId::Graph | PaneId::Details))
            || (view.status_column && pane == PaneId::Status);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

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
    fn shows_what_the_view_leaves() {
        let open = View {
            details: true,
            ..View::default()
        };
        let wide = View {
            wide: true,
            ..View::default()
        };
        let default = Layout::default();
        assert!(default.shows(PaneId::Status, View::default()));
        assert!(!default.shows(PaneId::Details, View::default()));
        assert!(default.shows(PaneId::Details, open));
        assert!(!default.shows(PaneId::Status, open));
        assert!(!default.shows(PaneId::Branches, wide));
        assert!(default.shows(PaneId::Graph, wide));
        assert!(classic().shows(PaneId::Details, View::default()));
    }

    #[test]
    fn status_column_puts_status_left_of_the_rest_of_the_layout() {
        let column = View {
            status_column: true,
            ..View::default()
        };
        let status = (27, Placed::Pane(PaneId::Status));
        let rest = |bottom| {
            Placed::Split(
                Split::Rows,
                vec![(70, Placed::Pane(PaneId::Graph)), (30, Placed::Pane(bottom))],
            )
        };
        // Branches keeps its place under the graph, and details replaces it.
        assert_eq!(
            Layout::default().resolve(column),
            Some(Placed::Split(
                Split::Columns,
                vec![status.clone(), (73, rest(PaneId::Branches))]
            ))
        );
        let open = View {
            details: true,
            ..column
        };
        assert_eq!(
            Layout::default().resolve(open),
            Some(Placed::Split(Split::Columns, vec![status, (73, rest(PaneId::Details))]))
        );
        // Every other pane stays where the layout puts it.
        for pane in [PaneId::Branches, PaneId::Graph, PaneId::Details] {
            assert!(classic().shows(pane, column), "{pane:?}");
        }
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
