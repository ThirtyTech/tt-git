//! The view models for what opens over the panes: the palette and pickers,
//! prompts, confirms, discard-all, the graph filter panel, the help popup,
//! the error popup and the About dialog.
//!
//! Each gives the text a dialog shows, in roles, and the width it wants in
//! columns (border included); the front end lays it out, wraps it and says
//! how tall it is.
//!
//! | Dialog | View model |
//! |---|---|
//! | `Modal::Prompt`, `Modal::Confirm`, `Modal::DiscardAll` | [`Modal::dialog_view`](crate::modal::Modal::dialog_view) ([`DialogView`]) |
//! | the error popup | [`error_view`] ([`DialogView`]) |
//! | `Modal::Palette` | [`Modal::palette_view`](crate::modal::Modal::palette_view) ([`PaletteView`]) |
//! | `Modal::Filter` | [`FilterPanel::view`](crate::filter::FilterPanel::view) ([`FilterView`]) |
//! | the keys popup | [`App::help_view`](crate::App::help_view) ([`HelpView`]) |
//! | `Modal::About` | none: each front end lays it out, from the text in [`about`] |

use unicode_width::UnicodeWidthStr;

use super::{StyleRole, TextSpan};

pub mod about;
mod filter;
mod help;
mod palette;
mod prompt;

pub use filter::{FILTER_HINT, FILTER_TITLE, FilterView};
pub use help::{HELP_KEYS_WIDTH, HELP_TAB_HINT, HELP_TITLE, HelpTab, HelpView};
pub use palette::PaletteView;

/// One line of a dialog, as spans.
pub type Line<'a> = Vec<TextSpan<'a>>;

/// The width of a line, in columns.
pub fn line_width(line: &[TextSpan]) -> usize {
    line.iter().map(|s| s.text.width()).sum()
}

/// A line's text, its spans joined.
pub fn line_text(line: &[TextSpan]) -> String {
    line.iter().map(|s| s.text.as_ref()).collect()
}

/// One entry of a dialog's list on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRow<'a> {
    /// It's the highlighted one, which enter picks.
    pub picked: bool,
    pub spans: Vec<TextSpan<'a>>,
}

/// How a dialog's border and title look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogLook {
    /// [`StyleRole::BorderFocused`] and [`StyleRole::TitleFocused`].
    Normal,
    /// [`StyleRole::Error`] for both.
    Error,
}

impl DialogLook {
    pub fn border(self) -> StyleRole {
        match self {
            DialogLook::Normal => StyleRole::BorderFocused,
            DialogLook::Error => StyleRole::Error,
        }
    }

    pub fn title(self) -> StyleRole {
        match self {
            DialogLook::Normal => StyleRole::TitleFocused,
            DialogLook::Error => StyleRole::Error,
        }
    }
}

/// A boxed dialog of text: a prompt, a confirm, discard-all or the error
/// popup. Its lines wrap at [`inner_width`](Self::inner_width).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogView<'a> {
    pub title: String,
    pub look: DialogLook,
    /// Columns across, the border included.
    pub width: usize,
    pub body: Vec<Line<'a>>,
    /// The keys, under the body after a blank line (muted).
    pub hint: Option<String>,
}

impl DialogView<'_> {
    /// Columns inside the border.
    pub fn inner_width(&self) -> usize {
        self.width.saturating_sub(2).max(1)
    }

    /// The rows the body takes, wrapped to [`inner_width`](Self::inner_width).
    pub fn body_rows(&self) -> usize {
        let inner = self.inner_width();
        self.body.iter().map(|l| line_width(l).div_ceil(inner).max(1)).sum()
    }
}

/// The error popup for `message`: its lines as they are, in a red box up to
/// 70 columns of `cols`.
pub fn error_view(message: &str, cols: usize) -> DialogView<'_> {
    DialogView {
        title: "Error".into(),
        look: DialogLook::Error,
        width: 70.min(cols),
        body: message.lines().map(|l| vec![TextSpan::plain(l)]).collect(),
        hint: None,
    }
}

/// `label` as a choice in a row of them: underlined in the key color when
/// it's the one `on`, muted when not.
pub(crate) fn choice<'a>(label: impl Into<std::borrow::Cow<'a, str>>, on: bool) -> TextSpan<'a> {
    if on {
        TextSpan::new(label, StyleRole::HelpKey).with_over(StyleRole::Underlined)
    } else {
        TextSpan::new(label, StyleRole::Muted)
    }
}

/// The ` · ` between choices.
pub(crate) fn separator() -> TextSpan<'static> {
    TextSpan::new(" · ", StyleRole::Muted)
}

/// The block cursor at the end of typed text.
pub(crate) fn cursor() -> TextSpan<'static> {
    TextSpan::new("█", StyleRole::HelpKey)
}

/// `> ` and the typed text, ending in the cursor.
pub(crate) fn search_line(input: &str) -> Line<'_> {
    vec![
        TextSpan::new("> ", StyleRole::HelpKey),
        TextSpan::plain(input),
        cursor(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_error_popup_keeps_the_message_lines() {
        let view = error_view("first\nsecond line", 200);
        assert_eq!(view.width, 70);
        assert_eq!(view.look, DialogLook::Error);
        assert_eq!(view.title, "Error");
        let lines: Vec<String> = view.body.iter().map(|l| line_text(l)).collect();
        assert_eq!(lines, ["first", "second line"]);
        assert_eq!(view.hint, None);
        assert_eq!(error_view("x", 30).width, 30, "narrowed to the screen");
    }

    #[test]
    fn body_rows_count_wrapped_lines() {
        let mut view = error_view("", 12);
        view.body = vec![
            vec![TextSpan::plain("0123456789")],
            vec![TextSpan::plain("0123456789x")],
            vec![],
        ];
        assert_eq!(view.inner_width(), 10);
        assert_eq!(view.body_rows(), 1 + 2 + 1, "an empty line still takes a row");
    }

    #[test]
    fn the_picked_choice_is_underlined() {
        let on = choice("Branch", true);
        assert_eq!((on.role, on.over), (StyleRole::HelpKey, Some(StyleRole::Underlined)));
        let off = choice("Worktree", false);
        assert_eq!((off.role, off.over), (StyleRole::Muted, None));
    }
}
