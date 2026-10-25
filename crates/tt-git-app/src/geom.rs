//! Screen geometry the front end reports: where it drew each pane. Units
//! are the front end's own: terminal cells for the TUI, logical pixels for
//! the GUI. A pane's PTY size comes from a rect through
//! [`view::terminal::pty_size`](crate::view::terminal::pty_size) and the
//! front end's cell size (1×1 in the TUI).

/// A rectangle: its top-left corner and its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self { x, y, width, height }
    }

    pub const fn left(self) -> u16 {
        self.x
    }

    pub const fn top(self) -> u16 {
        self.y
    }

    pub const fn right(self) -> u16 {
        self.x.saturating_add(self.width)
    }

    pub const fn bottom(self) -> u16 {
        self.y.saturating_add(self.height)
    }

    /// Whether the point (`x`, `y`) is inside.
    pub const fn contains(self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// The smallest rectangle holding both.
    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self {
            x,
            y,
            width: self.right().max(other.right()) - x,
            height: self.bottom().max(other.bottom()) - y,
        }
    }
}
