//! View models: what a pane shows, as text and the part each piece of it
//! plays, for any front end to draw in its own way. The front end picks how
//! each [`StyleRole`] looks from the theme
//! ([`Theme::role`](crate::theme::Theme::role) gives the theme's choice).
//!
//! So far only the graph has one ([`graph`]). Each area ported to the GUI
//! fills in its own module here; they're declared already so no port has
//! to edit this file.

use std::borrow::Cow;

use crate::syntax::Class;

pub mod blame;
pub mod branches;
pub mod code;
pub mod details;
pub mod dialogs;
pub mod graph;
pub mod stage;
pub mod status;
pub mod terminal;

/// What a piece of text is, which decides how it looks. There's one for
/// every style in [`Theme`](crate::theme::Theme), so a view model never
/// needs a style of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StyleRole {
    /// The text's own look, with nothing added.
    Plain,
    /// Secondary text, such as a dimmed merge commit or a note.
    Muted,
    /// Something that went wrong, or a conflict.
    Error,
    /// Plain text underlined, such as the branches pane's current tab.
    Underlined,
    /// A pane's or popup's border, and when it has the focus.
    Border,
    BorderFocused,
    /// A pane's or popup's title, and when it has the focus.
    Title,
    TitleFocused,
    /// The selected row, and when its pane doesn't have the focus.
    Selected,
    SelectedUnfocused,
    /// A key in the help bar, the help popup or a prompt.
    HelpKey,
    /// The letter of a button's label that presses it, such as the `y` of
    /// `[y]es`.
    HelpKeyAccel,
    /// What a key does, beside it.
    HelpText,
    /// A git operation running, in the help bar.
    Busy,
    /// A graph lane, by its color index (any number; the theme's lane
    /// colors repeat).
    Lane(u8),
    /// The checked-out branch's label.
    LabelHead,
    /// HEAD's label when it's detached.
    LabelDetached,
    LabelLocal,
    LabelRemote,
    LabelTag,
    Author,
    Date,
    Sha,
    /// An added or deleted line's text (or `+` and `-` signs and counts).
    DiffAdd,
    DiffDel,
    /// A diff's `@@` hunk header.
    DiffHunk,
    /// A diff's file header (`diff --git`, `---`, `+++`).
    DiffHeader,
    /// Behind an added or deleted line whose code is highlighted: use it
    /// as [`TextSpan::over`] or for the whole row.
    DiffAddLine,
    DiffDelLine,
    /// Highlighted code.
    Code(Class),
    /// Blame's age stripe, by how old the line is: 0 is newest. Past the
    /// theme's colors, the last one.
    BlameHeat(u8),
    /// Blame's author color, by index (the theme's colors repeat).
    BlameAuthor(u8),
    /// Blame's lines not committed yet: their stripe and author.
    BlameUncommitted,
    /// Blame's title when it shows the file as in `HEAD`, and when newer
    /// commits have changed it.
    BlameLatest,
    BlameOlder,
    /// Text a search matched: use it as [`TextSpan::over`].
    SearchMatch,
}

impl StyleRole {
    /// Every role, with the first few of each numbered one, for tests that
    /// check each front end draws them all as the theme says.
    pub fn samples() -> Vec<StyleRole> {
        use StyleRole::*;
        let mut roles = vec![
            Plain,
            Muted,
            Error,
            Underlined,
            Border,
            BorderFocused,
            Title,
            TitleFocused,
            Selected,
            SelectedUnfocused,
            HelpKey,
            HelpKeyAccel,
            HelpText,
            Busy,
            LabelHead,
            LabelDetached,
            LabelLocal,
            LabelRemote,
            LabelTag,
            Author,
            Date,
            Sha,
            DiffAdd,
            DiffDel,
            DiffHunk,
            DiffHeader,
            DiffAddLine,
            DiffDelLine,
            BlameUncommitted,
            BlameLatest,
            BlameOlder,
            SearchMatch,
        ];
        roles.extend(Class::ALL.into_iter().map(Code));
        for i in 0..20 {
            roles.extend([Lane(i), BlameHeat(i), BlameAuthor(i)]);
        }
        roles
    }
}

/// A piece of text in one role, perhaps with a second laid over it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSpan<'a> {
    pub text: Cow<'a, str>,
    pub role: StyleRole,
    /// A role patched over [`role`](Self::role): its colors and attributes
    /// win where it sets them. For highlighted code on a changed line
    /// ([`StyleRole::DiffAddLine`]) or a search match
    /// ([`StyleRole::SearchMatch`]).
    pub over: Option<StyleRole>,
}

impl<'a> TextSpan<'a> {
    pub fn new(text: impl Into<Cow<'a, str>>, role: StyleRole) -> Self {
        Self {
            text: text.into(),
            role,
            over: None,
        }
    }

    /// Text with no style of its own.
    pub fn plain(text: impl Into<Cow<'a, str>>) -> Self {
        Self::new(text, StyleRole::Plain)
    }

    /// The same text with `over` laid over its role.
    pub fn with_over(mut self, over: StyleRole) -> Self {
        self.over = Some(over);
        self
    }
}
