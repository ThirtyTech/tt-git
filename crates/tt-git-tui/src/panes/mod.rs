//! The four main panes.

mod blame;
mod branches;
mod details;
mod graph;
mod stage;
mod status;

pub use blame::BlameView;
pub use branches::{BranchTab, BranchesPane, DECORATION_WIDTH};
pub use details::{DetailsPane, DetailsSubject, SavedDetails};
pub use graph::{BranchFocus, DateStyle, GraphPane, GraphScope, LabelKind, LoadToken, RefLabel, build_labels};
pub use stage::{StageSideId, StageView};
pub use status::StatusPane;

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};
use tt_git_core::time::relative_changes_at;

use crate::theme::Theme;

/// When the relative dates last drawn were drawn, and when the first of
/// them changes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrawnDates {
    at: i64,
    until: i64,
}

impl Default for DrawnDates {
    fn default() -> Self {
        Self {
            at: i64::MIN,
            until: i64::MAX,
        }
    }
}

impl DrawnDates {
    /// Starts tracking a frame drawn at `now`.
    pub(crate) fn new(now: i64) -> Self {
        Self {
            at: now,
            until: i64::MAX,
        }
    }

    /// Notes a date shown as `relative(now, then)`.
    pub(crate) fn add(&mut self, then: i64) {
        self.until = self.until.min(relative_changes_at(self.at, then));
    }

    /// Whether some date drawn would read differently at `now`.
    pub(crate) fn stale(&self, now: i64) -> bool {
        now < self.at || now >= self.until
    }
}

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

/// The column where a pane drawn in `area` starts its title text, after the
/// corner and the `[n] ` [`pane_block`] puts first.
pub(crate) fn title_x(pane: PaneId, area: Rect) -> u16 {
    area.x + 1 + format!("[{}] ", pane.number()).len() as u16
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
