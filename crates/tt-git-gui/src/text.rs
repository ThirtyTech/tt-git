//! A line of view-model spans as GPUI text: [`StyledLine`].

use std::ops::Range;

use gpui::{HighlightStyle, SharedString, StyledText, combine_highlights};
use tt_git_app::view::{StyleRole, TextSpan};

use crate::theme::GuiTheme;

/// One line of [`TextSpan`]s, each in its role's look: text and highlight
/// ranges, ready to draw. Owns its text, so it can outlive the view model
/// it was made from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyledLine {
    pub text: SharedString,
    pub highlights: Vec<(Range<usize>, HighlightStyle)>,
}

impl StyledLine {
    /// `spans`, laid over `row` (the look of the whole line, such as
    /// [`StyleRole::Selected`]) when there is one.
    pub fn new(theme: &GuiTheme, spans: &[TextSpan], row: Option<StyleRole>) -> Self {
        let mut text = String::new();
        let mut highlights = Vec::with_capacity(spans.len());
        for span in spans {
            let start = text.len();
            text.push_str(&span.text);
            let style = theme.span_on(span, row);
            if start < text.len() && style != HighlightStyle::default() {
                highlights.push((start..text.len(), style));
            }
        }
        Self {
            text: text.into(),
            highlights,
        }
    }

    /// The line as a text element.
    pub fn element(&self) -> StyledText {
        StyledText::new(self.text.clone()).with_highlights(self.highlights.clone())
    }

    /// Lays the selected text's look over `bytes`.
    pub fn select(&mut self, theme: &GuiTheme, bytes: Range<usize>) {
        let bytes = bytes.start.min(self.text.len())..bytes.end.min(self.text.len());
        let look = HighlightStyle {
            background_color: Some(theme.text_selection()),
            ..HighlightStyle::default()
        };
        self.highlights = combine_highlights(std::mem::take(&mut self.highlights), [(bytes, look)]).collect();
    }
}

/// The row look of a selected (`picked`) row in a pane that is or isn't
/// `focused`; `None` for a row that isn't picked.
pub fn selection(picked: bool, focused: bool) -> Option<StyleRole> {
    match (picked, focused) {
        (false, _) => None,
        (true, true) => Some(StyleRole::Selected),
        (true, false) => Some(StyleRole::SelectedUnfocused),
    }
}
