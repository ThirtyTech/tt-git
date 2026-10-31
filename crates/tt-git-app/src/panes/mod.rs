//! The four main panes.

pub mod blame;
pub mod branches;
pub mod details;
pub mod graph;
pub mod selection;
pub mod stage;
pub mod status;

pub use blame::BlameView;
pub use branches::{BranchTab, BranchesPane, DECORATION_WIDTH};
pub use details::{DetailsPane, DetailsSubject, SavedDetails};
pub use graph::{BranchFocus, DateStyle, GraphPane, GraphScope, LabelKind, LoadToken, RefLabel, build_labels};
pub use selection::{TextPos, TextSelection, col_at};
pub use stage::{StageSideId, StageView};
pub use status::StatusPane;

use tt_git_core::time::relative_changes_at;

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
