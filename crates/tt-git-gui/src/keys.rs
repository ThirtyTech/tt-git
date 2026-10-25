//! GPUI's key presses as tt-git's [`KeyCombo`], normalized the way
//! [`KeyCombo::new`] normalizes the terminal's: an uppercase letter instead
//! of shift, back-tab for shift+tab. tt-git's keymap then resolves every key,
//! text input included.

use gpui::Keystroke;
use tt_git_app::keymap::{KeyCode, KeyCombo, KeyModifiers};

/// The key `keystroke` presses, or `None` for one tt-git leaves alone: a
/// modifier on its own, or anything with the platform key held (cmd on
/// macOS, the Windows key), which belongs to the OS and the menus.
///
/// `prefer_character_input` is GPUI's flag for keys whose modifiers type a
/// character, such as AltGr on Windows (reported as ctrl+alt): then the
/// character typed is the key, without ctrl and alt.
pub fn key_combo(keystroke: &Keystroke, prefer_character_input: bool) -> Option<KeyCombo> {
    let m = &keystroke.modifiers;
    if m.platform {
        return None;
    }
    let mut modifiers = KeyModifiers::NONE;
    if m.shift {
        modifiers.insert(KeyModifiers::SHIFT);
    }
    let typed = typed_char(keystroke);
    let alt_gr = prefer_character_input && typed.is_some();
    if !alt_gr {
        if m.control {
            modifiers.insert(KeyModifiers::CONTROL);
        }
        if m.alt {
            modifiers.insert(KeyModifiers::ALT);
        }
    }
    let code = match keystroke.key.as_str() {
        "backspace" => KeyCode::Backspace,
        "enter" => KeyCode::Enter,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "tab" if m.shift => KeyCode::BackTab,
        "tab" => KeyCode::Tab,
        "delete" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "escape" => KeyCode::Esc,
        "space" => KeyCode::Char(' '),
        "shift" | "control" | "alt" | "platform" | "cmd" | "super" | "win" | "fn" | "function" | "capslock" => {
            return None;
        }
        key => match function_key(key) {
            Some(n) => KeyCode::F(n),
            None => KeyCode::Char(char_for(keystroke, typed, alt_gr)?),
        },
    };
    Some(KeyCombo::new(code, modifiers))
}

/// The character pressed, for keys that type one.
fn char_for(keystroke: &Keystroke, typed: Option<char>, alt_gr: bool) -> Option<char> {
    let m = &keystroke.modifiers;
    let key = single_char(&keystroke.key);
    if alt_gr || !(m.control || m.alt) {
        // What was typed, shifted already: `?` for shift+/.
        return typed.or(key);
    }
    // With ctrl or alt, the key itself (option+s types `ß` on macOS), except
    // that shift still picks a symbol: alt+shift+/ is alt+?.
    match (key, typed) {
        (Some(k), Some(t)) if m.shift && !k.is_alphabetic() => Some(t),
        (Some(k), _) => Some(k),
        (None, t) => t,
    }
}

/// The printable character `keystroke` types, if it types exactly one.
fn typed_char(keystroke: &Keystroke) -> Option<char> {
    keystroke.key_char.as_deref().and_then(single_char)
}

fn single_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    let c = chars.next()?;
    (chars.next().is_none() && !c.is_control()).then_some(c)
}

fn function_key(key: &str) -> Option<u8> {
    let n: u8 = key.strip_prefix('f')?.parse().ok()?;
    (1..=24).contains(&n).then_some(n)
}

#[cfg(test)]
mod tests {
    use gpui::Modifiers;

    use super::*;

    fn stroke(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.into(),
            key_char: key_char.map(Into::into),
        }
    }

    fn combo(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Option<KeyCombo> {
        key_combo(&stroke(key, key_char, modifiers), false)
    }

    fn parsed(s: &str) -> Option<KeyCombo> {
        Some(s.parse().unwrap())
    }

    fn shift() -> Modifiers {
        Modifiers::shift()
    }

    fn ctrl() -> Modifiers {
        Modifiers::control()
    }

    fn alt() -> Modifiers {
        Modifiers::alt()
    }

    #[test]
    fn letters_and_shifted_letters() {
        let none = Modifiers::none();
        assert_eq!(combo("a", Some("a"), none), parsed("a"));
        assert_eq!(combo("a", Some("A"), shift()), parsed("A"));
        // Some platforms leave key_char off; shift still makes it uppercase.
        assert_eq!(combo("a", None, shift()), parsed("A"));
    }

    #[test]
    fn shifted_symbols_are_what_was_typed() {
        assert_eq!(combo("/", Some("?"), shift()), parsed("?"));
        assert_eq!(combo("1", Some("!"), shift()), parsed("!"));
        assert_eq!(combo("=", Some("+"), shift()), parsed("+"));
    }

    #[test]
    fn ctrl_and_alt_keep_the_key() {
        assert_eq!(combo("c", None, ctrl()), parsed("ctrl+c"));
        assert_eq!(combo("h", Some("h"), ctrl()), parsed("ctrl+h"));
        assert_eq!(
            combo("a", None, Modifiers::control_shift()),
            Some(KeyCombo::new(KeyCode::Char('A'), KeyModifiers::CONTROL))
        );
        // option+s types ß on macOS; the binding is still alt+s.
        assert_eq!(combo("s", Some("ß"), alt()), parsed("alt+s"));
        let alt_shift = Modifiers {
            alt: true,
            shift: true,
            ..Modifiers::none()
        };
        assert_eq!(
            combo("/", Some("?"), alt_shift),
            Some(KeyCombo::new(KeyCode::Char('?'), KeyModifiers::ALT))
        );
    }

    #[test]
    fn named_keys() {
        let none = Modifiers::none();
        for (key, want) in [
            ("enter", "enter"),
            ("escape", "esc"),
            ("backspace", "backspace"),
            ("tab", "tab"),
            ("space", "space"),
            ("up", "up"),
            ("down", "down"),
            ("left", "left"),
            ("right", "right"),
            ("home", "home"),
            ("end", "end"),
            ("pageup", "pageup"),
            ("pagedown", "pagedown"),
            ("delete", "delete"),
            ("insert", "insert"),
            ("f5", "f5"),
            ("f12", "f12"),
        ] {
            assert_eq!(combo(key, None, none), parsed(want), "{key}");
        }
        assert_eq!(combo("enter", None, alt()), parsed("alt+enter"));
        assert_eq!(combo("space", Some(" "), ctrl()), parsed("ctrl+space"));
    }

    #[test]
    fn shift_tab_is_back_tab() {
        assert_eq!(combo("tab", None, shift()), parsed("shift+tab"));
        assert_eq!(combo("tab", None, shift()), Some(KeyCombo::plain(KeyCode::BackTab)));
    }

    #[test]
    fn alt_gr_types_its_character() {
        // AltGr+q types @ on a German layout; Windows reports ctrl+alt.
        let ctrl_alt = Modifiers {
            control: true,
            alt: true,
            ..Modifiers::none()
        };
        assert_eq!(key_combo(&stroke("q", Some("@"), ctrl_alt), true), parsed("@"));
        // Without the flag it's a ctrl+alt binding.
        assert_eq!(
            key_combo(&stroke("q", Some("@"), ctrl_alt), false),
            Some(KeyCombo::new(
                KeyCode::Char('q'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            ))
        );
    }

    #[test]
    fn keys_tt_git_leaves_alone() {
        assert_eq!(combo("shift", None, shift()), None);
        assert_eq!(combo("control", None, ctrl()), None);
        assert_eq!(combo("c", None, Modifiers::command()), None);
        assert_eq!(combo("f25", None, Modifiers::none()), None);
    }

    #[test]
    fn other_characters_pass_through() {
        assert_eq!(combo("é", Some("é"), Modifiers::none()), Some(KeyCombo::char('é')));
        assert_eq!(combo("2", Some("2"), Modifiers::none()), parsed("2"));
    }
}
