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

/// [`Cell::glyph`] of a non-node cell, by its bits.
const GLYPHS: [&str; 16] = [
    " ", "╵", "╷", "│", "╴", "╯", "╮", "┤", "╶", "╰", "╭", "├", "─", "┴", "┬", "┼",
];

impl Cell {
    pub fn is_empty(&self) -> bool {
        self.bits == 0 && !self.node
    }

    /// [`glyph`](Self::glyph) as a string, without allocating.
    pub fn glyph_str(&self, node_glyph: &'static str) -> &'static str {
        if self.node {
            return node_glyph;
        }
        GLYPHS.get(usize::from(self.bits)).copied().unwrap_or("╶")
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
    /// Which lanes were open before the commit being pushed; kept between
    /// pushes so wide graphs don't allocate it every row.
    before: Vec<bool>,
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
        let mut before = std::mem::take(&mut self.before);
        before.clear();
        before.extend(self.lanes.iter().map(Option::is_some));
        let row = self.lay_out(id, parents, &before);
        self.before = before;
        row
    }

    fn lay_out(&mut self, id: NodeId, parents: &[NodeId], before: &[bool]) -> Row {
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

// A packed cell keeps its color in three bits.
const _: () = assert!(PALETTE_SIZE <= 8);

/// [`Row`]s stored compactly, for keeping one per commit of a long history:
/// two bytes per lane column, in one buffer shared by every row, and at
/// most [`cap`](Self::cap) columns per row. Columns past that are left out
/// (though counted in the row's [`width`](RowRef::width)), for a caller that
/// never draws them. Colors must be below [`PALETTE_SIZE`], as the builder's
/// are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rows {
    heads: Vec<RowHead>,
    cells: Vec<u16>,
    cap: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RowHead {
    /// Where the row's cells start in `cells`.
    start: usize,
    width: u32,
    node_lane: u32,
    node_color: u8,
    is_merge: bool,
}

/// Packed [`Cell`] and filler: bits 0-3 [`bits`], 4 node, 5-7 color,
/// 8 filler present, 9-11 its color.
fn pack(cell: Cell, filler: Option<u8>) -> u16 {
    debug_assert!(cell.color < PALETTE_SIZE && filler.is_none_or(|c| c < PALETTE_SIZE));
    let mut v = u16::from(cell.bits & 0xf) | u16::from(cell.node) << 4 | u16::from(cell.color & 7) << 5;
    if let Some(c) = filler {
        v |= 1 << 8 | u16::from(c & 7) << 9;
    }
    v
}

impl Rows {
    /// Keeps up to `cap` columns of each row; `usize::MAX` keeps them all.
    pub fn new(cap: usize) -> Self {
        Self {
            heads: Vec::new(),
            cells: Vec::new(),
            cap,
        }
    }

    /// Most columns kept per row.
    pub fn cap(&self) -> usize {
        self.cap
    }

    pub fn len(&self) -> usize {
        self.heads.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heads.is_empty()
    }

    /// Removes every row (keeping the memory) and sets a new `cap`.
    pub fn reset(&mut self, cap: usize) {
        self.heads.clear();
        self.cells.clear();
        self.cap = cap;
    }

    pub fn push(&mut self, row: &Row) {
        let start = self.cells.len();
        let kept = row.cells.len().min(self.cap);
        self.cells.extend(
            row.cells[..kept]
                .iter()
                .zip(&row.fillers[..kept])
                .map(|(cell, filler)| pack(*cell, *filler)),
        );
        self.heads.push(RowHead {
            start,
            width: u32::try_from(row.cells.len()).unwrap_or(u32::MAX),
            node_lane: u32::try_from(row.node_lane).unwrap_or(u32::MAX),
            node_color: row.node_color,
            is_merge: row.is_merge,
        });
    }

    pub fn get(&self, i: usize) -> Option<RowRef<'_>> {
        let head = self.heads.get(i)?;
        let end = self.heads.get(i + 1).map_or(self.cells.len(), |next| next.start);
        Some(RowRef {
            head,
            cells: &self.cells[head.start..end],
        })
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = RowRef<'_>> + '_ {
        (0..self.len()).map(|i| self.get(i).expect("in range"))
    }
}

/// One row of [`Rows`].
#[derive(Debug, Clone, Copy)]
pub struct RowRef<'a> {
    head: &'a RowHead,
    cells: &'a [u16],
}

impl RowRef<'_> {
    /// Number of lane columns, as [`Row::width`] (kept or not).
    pub fn width(&self) -> usize {
        self.head.width as usize
    }

    /// Number of columns kept: the width, up to the rows' cap.
    pub fn kept(&self) -> usize {
        self.cells.len()
    }

    pub fn node_lane(&self) -> usize {
        self.head.node_lane as usize
    }

    pub fn node_color(&self) -> u8 {
        self.head.node_color
    }

    pub fn is_merge(&self) -> bool {
        self.head.is_merge
    }

    /// Column `i`, which must be one of the [`kept`](Self::kept) ones.
    pub fn cell(&self, i: usize) -> Cell {
        let v = self.cells[i];
        Cell {
            bits: (v & 0xf) as u8,
            node: v & 1 << 4 != 0,
            color: (v >> 5 & 7) as u8,
        }
    }

    /// The gap after column `i`, as [`Row::fillers`]; `i` must be kept.
    pub fn filler(&self, i: usize) -> Option<u8> {
        let v = self.cells[i];
        (v & 1 << 8 != 0).then_some((v >> 9 & 7) as u8)
    }

    /// The kept columns as a [`Row`].
    pub fn to_row(&self) -> Row {
        Row {
            node_lane: self.node_lane(),
            node_color: self.node_color(),
            is_merge: self.is_merge(),
            cells: (0..self.kept()).map(|i| self.cell(i)).collect(),
            fillers: (0..self.kept()).map(|i| self.filler(i)).collect(),
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
