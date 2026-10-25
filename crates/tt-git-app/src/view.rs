//! View models: what a pane shows, as text and the part each piece of it
//! plays, for any front end to draw in its own way. The front end picks how
//! each [`StyleRole`] looks from the theme
//! ([`Theme::role`](crate::theme::Theme::role) gives the theme's choice).
//!
//! So far only the graph has one ([`graph`]).

use std::borrow::Cow;

pub mod graph;

/// What a piece of text is, which decides how it looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StyleRole {
    /// The text's own look, with nothing added.
    Plain,
    /// Secondary text, such as a dimmed merge commit or a note.
    Muted,
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
}

/// A piece of text in one role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSpan<'a> {
    pub text: Cow<'a, str>,
    pub role: StyleRole,
}

impl<'a> TextSpan<'a> {
    pub fn new(text: impl Into<Cow<'a, str>>, role: StyleRole) -> Self {
        Self {
            text: text.into(),
            role,
        }
    }

    /// Text with no style of its own.
    pub fn plain(text: impl Into<Cow<'a, str>>) -> Self {
        Self::new(text, StyleRole::Plain)
    }
}
