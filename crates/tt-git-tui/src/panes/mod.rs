//! The four main panes.

mod branches;
mod details;
mod graph;
mod status;

pub use branches::{BranchTab, BranchesPane};
pub use details::{DetailsPane, DetailsSubject};
pub use graph::{BranchFocus, GraphPane, GraphScope, LabelKind, RefLabel, build_labels};
pub use status::StatusPane;

use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};

use crate::theme::Theme;

/// Identifies a pane for focus and layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneId {
    Status,
    Branches,
    Graph,
    Details,
}

impl PaneId {
    /// Focus cycle order.
    pub const ALL: [PaneId; 4] = [PaneId::Graph, PaneId::Status, PaneId::Branches, PaneId::Details];

    pub fn number(self) -> usize {
        PaneId::ALL.iter().position(|p| *p == self).unwrap_or(0) + 1
    }

    /// The name used in config and by plugins.
    pub fn name(self) -> &'static str {
        match self {
            PaneId::Status => "status",
            PaneId::Branches => "branches",
            PaneId::Graph => "graph",
            PaneId::Details => "details",
        }
    }
}

/// The bordered block every pane draws inside, titled `[n] title`.
pub fn pane_block<'a>(pane: PaneId, title: impl Into<Line<'a>>, focused: bool, theme: &Theme) -> Block<'a> {
    let (border, title_style) = if focused {
        (theme.border_focused, theme.title_focused)
    } else {
        (theme.border, theme.title)
    };
    let mut title: Line<'a> = title.into();
    title
        .spans
        .insert(0, Span::styled(format!("[{}] ", pane.number()), title_style));
    for span in title.spans.iter_mut().skip(1) {
        span.style = title_style.patch(span.style);
    }
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(title)
}
