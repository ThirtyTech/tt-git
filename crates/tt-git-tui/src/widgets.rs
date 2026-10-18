//! Small helpers shared by the panes.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::keymap::Command;

/// Cursor and scroll offset of a list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    pub selected: usize,
    pub offset: usize,
    /// Visible rows as of the last draw (used for paging).
    pub height: usize,
}

impl Cursor {
    /// Applies a movement command to a list of `len` items. Returns whether
    /// it was a movement command.
    pub fn apply(&mut self, cmd: Command, len: usize) -> bool {
        let page = self.height.max(1);
        let last = len.saturating_sub(1);
        self.selected = match cmd {
            Command::MoveUp => self.selected.saturating_sub(1),
            Command::MoveDown => (self.selected + 1).min(last),
            Command::PageUp => self.selected.saturating_sub(page),
            Command::PageDown => (self.selected + page).min(last),
            Command::Top => 0,
            Command::Bottom => last,
            _ => return false,
        };
        true
    }

    /// Selects the item shown on visible row `row`, if there is one.
    pub fn click(&mut self, row: usize, len: usize) -> bool {
        let idx = self.offset + row;
        if idx < len {
            self.selected = idx;
            true
        } else {
            false
        }
    }

    pub fn clamp(&mut self, len: usize) {
        self.selected = self.selected.min(len.saturating_sub(1));
    }

    /// Adjusts the offset so the selection is within `height` rows.
    pub fn scroll_into_view(&mut self, height: usize) {
        self.height = height;
        if height == 0 {
            return;
        }
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + height {
            self.offset = self.selected + 1 - height;
        }
    }
}

/// Truncates `s` to `width` display columns, ending with `…` when cut.
pub fn truncate(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Pads or truncates `s` to exactly `width` columns.
pub fn fit(s: &str, width: usize) -> String {
    let t = truncate(s, width);
    let pad = width.saturating_sub(t.width());
    format!("{t}{}", " ".repeat(pad))
}

/// Right-aligns `s` in `width` columns.
pub fn fit_right(s: &str, width: usize) -> String {
    let t = truncate(s, width);
    let pad = width.saturating_sub(t.width());
    format!("{}{t}", " ".repeat(pad))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_by_display_width() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 6), "hello…");
        assert_eq!(truncate("日本語テキスト", 5), "日本…");
        assert_eq!(truncate("abc", 0), "");
        assert_eq!(fit("ab", 4), "ab  ");
        assert_eq!(fit_right("ab", 4), "  ab");
    }

    #[test]
    fn cursor_moves_and_scrolls() {
        let mut c = Cursor::default();
        c.scroll_into_view(3);
        for _ in 0..5 {
            c.apply(Command::MoveDown, 10);
        }
        c.scroll_into_view(3);
        assert_eq!((c.selected, c.offset), (5, 3));
        c.apply(Command::Bottom, 10);
        c.scroll_into_view(3);
        assert_eq!((c.selected, c.offset), (9, 7));
        c.apply(Command::PageUp, 10);
        c.scroll_into_view(3);
        assert_eq!((c.selected, c.offset), (6, 6));
        c.apply(Command::Top, 10);
        c.scroll_into_view(3);
        assert_eq!((c.selected, c.offset), (0, 0));
        assert!(!c.apply(Command::Quit, 10));
        c.apply(Command::MoveDown, 0);
        assert_eq!(c.selected, 0);
    }
}
