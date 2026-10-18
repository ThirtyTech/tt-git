//! The four main panes.

mod branches;
mod details;
mod graph;
mod status;

pub use branches::BranchesPane;
pub use details::{DetailsPane, DetailsSubject};
pub use graph::{GraphPane, GraphScope, LabelKind, RefLabel, build_labels};
pub use status::StatusPane;

use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};

use crate::theme::Theme;

/// Identifies a pane for focus and layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneId {
    Status,
    Branches,
    Graph,
    Details,
}

impl PaneId {
    /// Focus cycle order.
    pub const ALL: [PaneId; 4] = [PaneId::Status, PaneId::Branches, PaneId::Graph, PaneId::Details];

    pub fn number(self) -> usize {
        PaneId::ALL.iter().position(|p| *p == self).unwrap_or(0) + 1
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
