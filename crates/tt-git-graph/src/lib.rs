//! Commit graph lane layout.
//!
//! Commits are fed newest-first in topological order (children before
//! parents, as `git log --topo-order` prints them). Each commit produces one
//! [`Row`] describing what to draw in each lane column. Lanes are *stable*
//! (GitLens / GitKraken style): once a line of history has a column it keeps
//! it until it reaches its target commit, so lines run straight down rather
//! than shifting sideways.
//!
//! The algorithm is incremental and only looks at the current lane state, so
//! rows can be produced while the log is still streaming in.
//!
//! Per commit:
//! 1. Lanes waiting for this commit are its children's lines. The leftmost
//!    becomes the commit's column; the others end here, joining it with a
//!    horizontal line (`╯`). With no waiting lane the commit is a branch tip
//!    and takes the first free column.
//! 2. The first parent continues in the commit's column.
//! 3. Each further (merge) parent joins a lane already waiting for it (`┤`),
//!    or opens a new lane in the first free column (`╮` / `╭`). Columns freed
//!    in step 1 are not reused in the same row, so a line never appears to
//!    pass straight through a merge point.

use smallvec::SmallVec;

/// Caller-assigned dense id for a commit (index into the caller's table).
pub type NodeId = u32;

/// Number of distinct lane colors handed out before they repeat.
pub const PALETTE_SIZE: u8 = 8;

/// Connection bits of a cell.
pub mod bits {
    pub const UP: u8 = 1;
    pub const DOWN: u8 = 2;
    pub const LEFT: u8 = 4;
    pub const RIGHT: u8 = 8;
}

/// One lane column within a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cell {
    /// [`bits`] flags: which sides this cell's lines touch.
    pub bits: u8,
    /// This cell holds the row's commit.
    pub node: bool,
    /// Lane color index (`0..PALETTE_SIZE`).
    pub color: u8,
}

impl Cell {
    pub fn is_empty(&self) -> bool {
        self.bits == 0 && !self.node
    }

    /// Box-drawing glyph for the cell's connections (rounded corners).
    /// Node cells return `node_glyph`.
    pub fn glyph(&self, node_glyph: char) -> char {
        use bits::*;
        if self.node {
            return node_glyph;
        }
        match self.bits {
            0 => ' ',
            b if b == UP | DOWN => '│',
            b if b == LEFT | RIGHT => '─',
            b if b == UP | LEFT => '╯',
            b if b == UP | RIGHT => '╰',
            b if b == DOWN | LEFT => '╮',
            b if b == DOWN | RIGHT => '╭',
            b if b == UP | DOWN | LEFT => '┤',
            b if b == UP | DOWN | RIGHT => '├',
            b if b == UP | LEFT | RIGHT => '┴',
            b if b == DOWN | LEFT | RIGHT => '┬',
            b if b == UP | DOWN | LEFT | RIGHT => '┼',
            b if b == UP => '╵',
            b if b == DOWN => '╷',
            b if b == LEFT => '╴',
            _ => '╶',
        }
    }
}

/// What to draw for one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Column of the commit.
    pub node_lane: usize,
    /// Color of the commit's lane.
    pub node_color: u8,
    /// The commit has more than one (distinct) parent.
    pub is_merge: bool,
    /// One entry per lane column.
    pub cells: SmallVec<[Cell; 8]>,
    /// `fillers[i]` is the gap between column `i` and `i + 1`: `Some(color)`
    /// when a horizontal line crosses it. Same length as `cells`.
    pub fillers: SmallVec<[Option<u8>; 8]>,
}

impl Row {
    pub fn width(&self) -> usize {
        self.cells.len()
    }

    /// Plain-text rendering (two characters per lane), for tests and debugging.
    pub fn to_ascii(&self) -> String {
        let mut s = String::with_capacity(self.cells.len() * 2);
        for (cell, filler) in self.cells.iter().zip(&self.fillers) {
            s.push(cell.glyph(if self.is_merge { '◉' } else { '●' }));
            s.push(if filler.is_some() { '─' } else { ' ' });
        }
        s.trim_end().to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Lane {
    target: NodeId,
    color: u8,
}

/// Incremental layout state.
#[derive(Debug, Clone, Default)]
pub struct GraphBuilder {
    lanes: Vec<Option<Lane>>,
    next_color: u8,
}

impl GraphBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of lanes currently open (waiting for a parent below).
    pub fn open_lanes(&self) -> usize {
        self.lanes.iter().filter(|l| l.is_some()).count()
    }

    fn alloc_color(&mut self) -> u8 {
        let c = self.next_color;
        self.next_color = (self.next_color + 1) % PALETTE_SIZE;
        c
    }

    fn first_free(&mut self, reserved: &[usize]) -> usize {
        let idx = (0..self.lanes.len())
            .find(|i| self.lanes[*i].is_none() && !reserved.contains(i))
            .unwrap_or(self.lanes.len());
        if idx == self.lanes.len() {
            self.lanes.push(None);
        }
        idx
    }

    /// Lays out the next commit. `parents` must be in git order (first
    /// parent first); ids may refer to commits not pushed yet.
    pub fn push(&mut self, id: NodeId, parents: &[NodeId]) -> Row {
        let before: SmallVec<[bool; 8]> = self.lanes.iter().map(Option::is_some).collect();

        let matching: SmallVec<[usize; 4]> = self
            .lanes
            .iter()
            .enumerate()
            .filter(|(_, l)| l.is_some_and(|l| l.target == id))
            .map(|(i, _)| i)
            .collect();

        let (node, node_color) = match matching.first() {
            Some(&first) => (first, self.lanes[first].expect("matching lane").color),
            None => {
                let color = self.alloc_color();
                (self.first_free(&[]), color)
            }
        };

        // (other lane, color) of every horizontal line in this row.
        let mut edges: SmallVec<[(usize, u8); 4]> = SmallVec::new();
        let mut reserved: SmallVec<[usize; 4]> = SmallVec::new();
        reserved.push(node);

        for &m in matching.iter().skip(1) {
            let lane = self.lanes[m].take().expect("matching lane");
            edges.push((m, lane.color));
            reserved.push(m);
        }

        let mut distinct: SmallVec<[NodeId; 2]> = SmallVec::new();
        for p in parents {
            if *p != id && !distinct.contains(p) {
                distinct.push(*p);
            }
        }

        self.lanes[node] = distinct.first().map(|&p| Lane {
            target: p,
            color: node_color,
        });

        for &p in distinct.iter().skip(1) {
            let existing = self
                .lanes
                .iter()
                .enumerate()
                .find(|(i, l)| *i != node && l.is_some_and(|l| l.target == p))
                .map(|(i, l)| (i, l.expect("checked").color));
            match existing {
                Some((j, color)) => edges.push((j, color)),
                None => {
                    let color = self.alloc_color();
                    let slot = self.first_free(&reserved);
                    self.lanes[slot] = Some(Lane { target: p, color });
                    reserved.push(slot);
                    edges.push((slot, color));
                }
            }
        }

        let width = self
            .lanes
            .len()
            .max(before.len())
            .max(node + 1)
            .max(edges.iter().map(|(j, _)| j + 1).max().unwrap_or(0));

        let mut cells: SmallVec<[Cell; 8]> = SmallVec::from_elem(Cell::default(), width);
        let mut fillers: SmallVec<[Option<u8>; 8]> = SmallVec::from_elem(None, width);

        for (i, cell) in cells.iter_mut().enumerate() {
            let up = before.get(i).copied().unwrap_or(false);
            let after = self.lanes.get(i).copied().flatten();
            if up {
                cell.bits |= bits::UP;
            }
            if let Some(lane) = after {
                cell.bits |= bits::DOWN;
                cell.color = lane.color;
            }
        }
        // Lanes that ended this row keep the color they had above.
        for &(j, color) in &edges {
            if self.lanes.get(j).copied().flatten().is_none() {
                cells[j].color = color;
            }
        }

        for &(j, color) in &edges {
            let (lo, hi) = if j < node { (j, node) } else { (node, j) };
            cells[lo].bits |= bits::RIGHT;
            cells[hi].bits |= bits::LEFT;
            for cell in &mut cells[lo + 1..hi] {
                let vertical = cell.bits & (bits::UP | bits::DOWN) == (bits::UP | bits::DOWN);
                cell.bits |= bits::LEFT | bits::RIGHT;
                if !vertical {
                    cell.color = color;
                }
            }
            for f in &mut fillers[lo..hi] {
                *f = Some(color);
            }
            cells[j].color = color;
        }

        cells[node].node = true;
        cells[node].color = node_color;

        while self.lanes.last().is_some_and(Option::is_none) {
            self.lanes.pop();
        }
        // Trim trailing columns that draw nothing.
        while cells.len() > node + 1 && cells.last().is_some_and(Cell::is_empty) {
            cells.pop();
            fillers.pop();
        }
        if let Some(last) = fillers.last_mut() {
            *last = None;
        }

        Row {
            node_lane: node,
            node_color,
            is_merge: distinct.len() > 1,
            cells,
            fillers,
        }
    }
}

/// Lays out a whole list of `(id, parents)` at once.
pub fn layout<'a>(commits: impl IntoIterator<Item = (NodeId, &'a [NodeId])>) -> Vec<Row> {
    let mut builder = GraphBuilder::new();
    commits
        .into_iter()
        .map(|(id, parents)| builder.push(id, parents))
        .collect()
}

#[cfg(test)]
mod tests;
