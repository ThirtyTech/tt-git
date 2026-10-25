//! Mouse input as the app takes it: what happened and what it was over.
//! The front end works out what's under the pointer; the app only hears
//! which pane, row or part of a popup it was.

use crate::filter::FilterTab;
use crate::panes::{BranchTab, PaneId, StageSideId};

/// A mouse event over something the front end has hit-tested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MouseInput {
    pub kind: MouseKind,
    /// Ctrl was held.
    pub ctrl: bool,
    pub target: MouseTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    /// The left button went down.
    Click,
    ScrollUp,
    ScrollDown,
    /// Anything else: a move, a release, another button.
    Other,
}

/// What a mouse event was over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MouseTarget {
    /// Nothing that takes the mouse.
    None,
    /// The graph's filter panel while it's open, or outside it.
    Filter(FilterSpot),
    /// A pane.
    Pane {
        pane: PaneId,
        /// The line of the pane's body, counted from the first line inside
        /// its border; `None` on the border.
        body_row: Option<usize>,
        /// A part of the pane with clicks of its own, tried first.
        part: Option<PanePart>,
    },
}

/// A part of a pane that takes clicks or the wheel itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanePart {
    /// A tab in the branches pane's title.
    Tab(BranchTab),
    /// The `row`-th line of the details pane's file list.
    FileRow(usize),
    /// The diff beside or below the details pane's file list.
    Diff,
    /// The `row`-th line on screen of a staging side's rows, under the
    /// side's title.
    StageRow { side: StageSideId, row: usize },
}

/// Where in (or outside) the filter panel the mouse was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterSpot {
    Outside,
    /// Inside, on nothing that takes clicks.
    Nothing,
    Tab(FilterTab),
    /// The `n`-th choice row on screen (it may be past the last choice).
    Choice(usize),
}
