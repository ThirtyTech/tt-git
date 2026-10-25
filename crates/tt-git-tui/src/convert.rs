//! Conversions between tt-git's own types and crossterm's and ratatui's.
//! Plain functions rather than `From`, so they keep working once the types
//! live in a crate of their own.

use crossterm::event::{KeyCode as TermKeyCode, KeyEvent, KeyModifiers as TermModifiers};

use crate::geom;
use crate::keymap::{KeyCode, KeyCombo, KeyModifiers};

/// A key press from the terminal, normalized as [`KeyCombo::new`] does.
pub fn key_combo(key: &KeyEvent) -> KeyCombo {
    let code = match key.code {
        TermKeyCode::Backspace => KeyCode::Backspace,
        TermKeyCode::Enter => KeyCode::Enter,
        TermKeyCode::Left => KeyCode::Left,
        TermKeyCode::Right => KeyCode::Right,
        TermKeyCode::Up => KeyCode::Up,
        TermKeyCode::Down => KeyCode::Down,
        TermKeyCode::Home => KeyCode::Home,
        TermKeyCode::End => KeyCode::End,
        TermKeyCode::PageUp => KeyCode::PageUp,
        TermKeyCode::PageDown => KeyCode::PageDown,
        TermKeyCode::Tab => KeyCode::Tab,
        TermKeyCode::BackTab => KeyCode::BackTab,
        TermKeyCode::Delete => KeyCode::Delete,
        TermKeyCode::Insert => KeyCode::Insert,
        TermKeyCode::F(n) => KeyCode::F(n),
        TermKeyCode::Char(c) => KeyCode::Char(c),
        TermKeyCode::Esc => KeyCode::Esc,
        _ => KeyCode::Null,
    };
    let mut modifiers = KeyModifiers::NONE;
    for (term, ours) in [
        (TermModifiers::SHIFT, KeyModifiers::SHIFT),
        (TermModifiers::CONTROL, KeyModifiers::CONTROL),
        (TermModifiers::ALT, KeyModifiers::ALT),
    ] {
        if key.modifiers.contains(term) {
            modifiers.insert(ours);
        }
    }
    KeyCombo::new(code, modifiers)
}

pub fn rect(r: geom::Rect) -> ratatui::layout::Rect {
    ratatui::layout::Rect::new(r.x, r.y, r.width, r.height)
}

pub fn geom_rect(r: ratatui::layout::Rect) -> geom::Rect {
    geom::Rect::new(r.x, r.y, r.width, r.height)
}
