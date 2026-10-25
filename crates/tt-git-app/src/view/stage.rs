//! The view model for staging in the details pane: a file's unstaged and
//! staged changes, side by side or one above the other, each a title over
//! its hunks' rows ([`code`](super::code) makes the code spans).
//!
//! A front end asks [`StageView::layout`] how to lay the sides out, then
//! for each side draws [`side_title`](StageView::side_title) and, below it,
//! [`side_error`](StageView::side_error) or (after
//! [`set_viewport`](StageView::set_viewport)) the rows of
//! [`side_rows`](StageView::side_rows).

use std::borrow::Cow;

use tt_git_core::patch::LineKind;

use super::code::code_spans;
use super::{StyleRole, TextSpan};
use crate::panes::stage::{Row, SIDE_BY_SIDE_WIDTH};
use crate::panes::{DetailsPane, StageSideId, StageView};
use crate::syntax::{self, Tokens};
use crate::widgets::truncate;

/// How the staging view fills the pane's body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageLayout {
    /// Nothing to stage yet: this, in [`StyleRole::Muted`], instead.
    Message(&'static str),
    /// Only `full` has anything to show. It fills the body, and the other
    /// side (`full.other()`) is only its title, along the bottom.
    One { full: StageSideId },
    /// Unstaged on the left, staged on the right, each half the width with
    /// a [`StyleRole::Border`] rule between them.
    Beside,
    /// Unstaged on top, staged below, each half the height.
    Stacked,
}

/// One row of a side: a hunk's heading or one of its lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageRow<'a> {
    pub spans: Vec<TextSpan<'a>>,
    /// The look of the whole row, under its spans: the tint behind a
    /// changed line whose code is highlighted
    /// ([`StyleRole::DiffAddLine`] or [`StyleRole::DiffDelLine`]).
    pub back: Option<StyleRole>,
    /// Selected, on the active side: drawn in the selection look, which
    /// takes the place of `back`.
    pub picked: bool,
}

impl StageView {
    /// How to lay the two sides out in a body `width` columns wide.
    pub fn layout(&self, width: usize) -> StageLayout {
        if !self.loaded() {
            return StageLayout::Message("loading…");
        }
        match (self.unstaged.has_content(), self.staged.has_content()) {
            (false, false) => StageLayout::Message("no changes"),
            (true, false) => StageLayout::One {
                full: StageSideId::Unstaged,
            },
            (false, true) => StageLayout::One {
                full: StageSideId::Staged,
            },
            (true, true) if width >= SIDE_BY_SIDE_WIDTH as usize => StageLayout::Beside,
            (true, true) => StageLayout::Stacked,
        }
    }

    /// Side `id`'s title line: a `▌` when it's the active side, its name,
    /// and how many hunks it has. In the focus look only when it's active
    /// and the pane is `focused`.
    pub fn side_title(&self, id: StageSideId, focused: bool) -> Vec<TextSpan<'static>> {
        let active = id == self.side;
        let summary = match self.diff(id) {
            Some(d) if d.is_empty() => "no changes".to_string(),
            Some(d) if d.hunks.len() == 1 => "1 hunk".to_string(),
            Some(d) => format!("{} hunks", d.hunks.len()),
            None => String::new(),
        };
        let role = match (active, focused) {
            (true, true) => StyleRole::TitleFocused,
            (true, false) => StyleRole::Title,
            _ => StyleRole::Muted,
        };
        let marker = if active { "▌" } else { " " };
        vec![
            TextSpan::new(format!("{marker}{}", id.name()), role),
            TextSpan::new(format!("  {summary}"), StyleRole::Muted),
        ]
    }

    /// Why side `id` has no diff to show, in place of its rows, in
    /// [`StyleRole::Muted`].
    pub fn side_error(&self, id: StageSideId) -> Option<TextSpan<'static>> {
        match &self.view(id).diff {
            Some(Err(why)) => Some(TextSpan::new(format!(" {why}"), StyleRole::Muted)),
            _ => None,
        }
    }

    /// The rows of side `id` on screen, `rows` lines from its offset, each
    /// fitted to `width` columns. Call
    /// [`set_viewport`](StageView::set_viewport) first.
    pub fn side_rows(&self, id: StageSideId, rows: usize, width: usize) -> Vec<StageRow<'_>> {
        let active = id == self.side;
        let view = self.view(id);
        let Some(diff) = self.diff(id) else {
            return Vec::new();
        };
        let start = view.offset.min(view.rows.len());
        let end = (start + rows).min(view.rows.len());
        view.rows[start..end]
            .iter()
            .map(|row| match *row {
                Row::Heading(h) => StageRow {
                    spans: vec![TextSpan::new(
                        truncate(&diff.hunks[h].heading(), width),
                        StyleRole::DiffHunk,
                    )],
                    back: None,
                    picked: active && !self.line_mode && h == self.hunk,
                },
                Row::Line(h, l) => {
                    let line = &diff.hunks[h].lines[l];
                    let (prefix, role, back) = match line.kind {
                        LineKind::Context => (" ", StyleRole::Plain, None),
                        LineKind::Add => ("+", StyleRole::DiffAdd, Some(StyleRole::DiffAddLine)),
                        LineKind::Del => ("-", StyleRole::DiffDel, Some(StyleRole::DiffDelLine)),
                    };
                    let picked = if self.line_mode {
                        line.is_change() && self.picked((h, l))
                    } else {
                        h == self.hunk
                    };
                    let body = syntax::shown(&line.text);
                    let tokens = view.syntax.get(h).and_then(|t| t.get(l)).and_then(Option::as_ref);
                    let (spans, back) = match tokens {
                        Some(tokens) if width > 0 => {
                            let mut spans = vec![TextSpan::new(prefix, role)];
                            spans.extend(code(body, tokens, width - 1, line.is_change()));
                            (spans, back)
                        }
                        _ => (
                            vec![TextSpan::new(truncate(&format!("{prefix}{body}"), width), role)],
                            None,
                        ),
                    };
                    StageRow {
                        spans,
                        back,
                        picked: active && picked,
                    }
                }
            })
            .collect()
    }
}

/// The highlighted code of a line, borrowed from the diff when it's shown
/// as it is.
fn code<'a>(body: Cow<'a, str>, tokens: &Tokens, width: usize, pad: bool) -> Vec<TextSpan<'a>> {
    match body {
        Cow::Borrowed(text) => code_spans(text, Some(tokens), StyleRole::Plain, width, pad, &[]),
        Cow::Owned(text) => code_spans(&text, Some(tokens), StyleRole::Plain, width, pad, &[])
            .into_iter()
            .map(|TextSpan { text, role, over }| TextSpan {
                text: Cow::Owned(text.into_owned()),
                role,
                over,
            })
            .collect(),
    }
}

/// The details pane's title while it stages a file: the staging view's own,
/// what `esc` goes back to, and whether the details are loading, as the
/// TUI's details pane titles it.
pub fn pane_title(details: &DetailsPane) -> Option<String> {
    let mut title = details.stage()?.title();
    if let Some(trail) = &details.trail {
        title.push_str(&format!("  ‹ {trail}"));
    }
    if details.loading {
        title.push_str(" loading…");
    }
    Some(title)
}

#[cfg(test)]
mod tests {
    use tt_git_core::patch::FileDiff;
    use tt_git_core::{FileState, StageSide, StatusEntry};

    use super::*;
    use crate::component::EventState;
    use crate::jobs::Ctx;
    use crate::keymap::Command;
    use crate::syntax::Class;

    fn entry(path: &str) -> StatusEntry {
        StatusEntry {
            path: path.into(),
            orig_path: None,
            index: FileState::Unmodified,
            worktree: FileState::Modified,
        }
    }

    /// Two hunks of `path`: a line changed near the top, one added lower down.
    fn two_hunks(path: &str) -> FileDiff {
        FileDiff::parse(&format!(
            "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n\
             @@ -1,2 +1,2 @@ fn main\n-let a = 1;\n+let a = 2;\n x\n\
             @@ -10,1 +10,2 @@\n y\n+let b = 3;\n"
        ))
        .expect("a diff")
    }

    fn none() -> StageSide {
        Ok(FileDiff::default())
    }

    /// The staging view of `path` with `unstaged` and `staged` loaded.
    fn view(path: &str, unstaged: StageSide, staged: StageSide) -> StageView {
        let tokens = |side: &StageSide| side.as_ref().map(|d| syntax::hunks(path, d)).unwrap_or_default();
        let syntax = (tokens(&unstaged), tokens(&staged));
        let mut view = StageView::new(entry(path));
        view.set_diffs(entry(path), unstaged, staged, syntax);
        view
    }

    fn ctx() -> Ctx {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        Ctx::new(tx)
    }

    fn text(row: &StageRow) -> String {
        row.spans.iter().map(|s| s.text.as_ref()).collect()
    }

    #[test]
    fn says_so_while_loading_and_when_there_is_nothing() {
        assert_eq!(
            StageView::new(entry("f.txt")).layout(200),
            StageLayout::Message("loading…")
        );
        assert_eq!(
            view("f.txt", none(), none()).layout(200),
            StageLayout::Message("no changes")
        );
    }

    #[test]
    fn lays_the_sides_out_by_what_they_have_and_the_width() {
        let both = view("f.txt", Ok(two_hunks("f.txt")), Ok(two_hunks("f.txt")));
        let wide = SIDE_BY_SIDE_WIDTH as usize;
        assert_eq!(both.layout(wide), StageLayout::Beside);
        assert_eq!(both.layout(wide - 1), StageLayout::Stacked);
        assert_eq!(
            view("f.txt", Ok(two_hunks("f.txt")), none()).layout(wide),
            StageLayout::One {
                full: StageSideId::Unstaged
            }
        );
        // Nothing unstaged: the staged side is active and fills the body.
        let staged = view("f.txt", none(), Ok(two_hunks("f.txt")));
        assert_eq!(
            staged.layout(wide),
            StageLayout::One {
                full: StageSideId::Staged
            }
        );
        assert_eq!(staged.side(), StageSideId::Staged);
        // A side that can't be shown still says why.
        let err = view("f.txt", Err("binary file".into()), none());
        assert_eq!(
            err.layout(wide),
            StageLayout::One {
                full: StageSideId::Unstaged
            }
        );
        assert_eq!(
            err.side_error(StageSideId::Unstaged),
            Some(TextSpan::new(" binary file", StyleRole::Muted))
        );
        assert!(err.side_rows(StageSideId::Unstaged, 10, 80).is_empty());
        assert_eq!(err.side_error(StageSideId::Staged), None);
    }

    #[test]
    fn titles_mark_the_active_side_and_count_its_hunks() {
        let v = view("f.txt", Ok(two_hunks("f.txt")), none());
        let shown = |id, focused| {
            v.side_title(id, focused)
                .into_iter()
                .map(|s| (s.text.into_owned(), s.role))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            shown(StageSideId::Unstaged, true),
            vec![
                ("▌Unstaged".into(), StyleRole::TitleFocused),
                ("  2 hunks".into(), StyleRole::Muted)
            ]
        );
        assert_eq!(shown(StageSideId::Unstaged, false)[0].1, StyleRole::Title);
        assert_eq!(
            shown(StageSideId::Staged, true),
            vec![
                (" Staged".into(), StyleRole::Muted),
                ("  no changes".into(), StyleRole::Muted)
            ]
        );
    }

    #[test]
    fn rows_show_headings_and_lines_with_the_hunk_picked() {
        let v = view("f.txt", Ok(two_hunks("f.txt")), Ok(two_hunks("f.txt")));
        let rows = v.side_rows(StageSideId::Unstaged, 100, 80);
        let shown: Vec<_> = rows.iter().map(|r| (text(r), r.picked)).collect();
        assert_eq!(
            shown,
            vec![
                ("@@ -1,2 +1,2 @@ fn main".into(), true),
                ("-let a = 1;".into(), true),
                ("+let a = 2;".into(), true),
                (" x".into(), true),
                ("@@ -10,1 +10,2 @@".into(), false),
                (" y".into(), false),
                ("+let b = 3;".into(), false),
            ]
        );
        assert_eq!(rows[0].spans[0].role, StyleRole::DiffHunk);
        // Plain text isn't highlighted: the sign's role is the whole line's.
        assert_eq!(rows[2].spans.len(), 1);
        assert_eq!(rows[2].spans[0].role, StyleRole::DiffAdd);
        assert_eq!(rows[2].back, None);
        // The inactive side picks nothing.
        assert!(v.side_rows(StageSideId::Staged, 100, 80).iter().all(|r| !r.picked));
        // From the offset, as many as fit.
        assert_eq!(v.side_rows(StageSideId::Unstaged, 2, 80).len(), 2);
    }

    #[test]
    fn highlighted_changes_are_tinted_and_padded_to_the_width() {
        let v = view("f.rs", Ok(two_hunks("f.rs")), none());
        let rows = v.side_rows(StageSideId::Unstaged, 100, 20);
        let (del, add, context) = (&rows[1], &rows[2], &rows[3]);
        assert_eq!(del.back, Some(StyleRole::DiffDelLine));
        assert_eq!(add.back, Some(StyleRole::DiffAddLine));
        assert_eq!(context.back, None);
        assert_eq!(add.spans[0], TextSpan::new("+", StyleRole::DiffAdd));
        assert_eq!(add.spans[1], TextSpan::new("let", StyleRole::Code(Class::Keyword)));
        assert_eq!(text(add), format!("{:<20}", "+let a = 2;"));
        // Context lines aren't padded.
        assert_eq!(text(context), " x");
    }

    #[test]
    fn rows_are_cut_to_the_width() {
        let v = view("f.txt", Ok(two_hunks("f.txt")), none());
        let rows = v.side_rows(StageSideId::Unstaged, 100, 8);
        assert_eq!(text(&rows[0]), "@@ -1,2…");
        assert_eq!(text(&rows[1]), "-let a …");
        let v = view("f.rs", Ok(two_hunks("f.rs")), none());
        assert_eq!(text(&v.side_rows(StageSideId::Unstaged, 100, 8)[2]), "+let a …");
        // No room for even the sign.
        assert!(
            v.side_rows(StageSideId::Unstaged, 100, 0)
                .iter()
                .all(|r| text(r).is_empty())
        );
    }

    #[test]
    fn tabs_are_expanded() {
        let diff = FileDiff::parse("@@ -1,1 +1,1 @@\n-\tx\n+\ty\n").unwrap();
        for path in ["f.txt", "f.rs"] {
            let v = view(path, Ok(diff.clone()), none());
            let rows = v.side_rows(StageSideId::Unstaged, 100, 40);
            assert!(!text(&rows[2]).contains('\t'), "{path}: {:?}", text(&rows[2]));
            assert!(text(&rows[2]).trim_end().ends_with('y'));
        }
    }

    #[test]
    fn line_mode_picks_the_changed_lines() {
        let mut v = view("f.txt", Ok(two_hunks("f.txt")), none());
        let ctx = ctx();
        assert_eq!(v.on_command(Command::ToggleLineMode, &ctx), EventState::Consumed);
        let picked = |v: &StageView| -> Vec<String> {
            v.side_rows(StageSideId::Unstaged, 100, 80)
                .iter()
                .filter(|r| r.picked)
                .map(text)
                .collect()
        };
        assert_eq!(picked(&v), vec!["-let a = 1;"]);
        v.on_command(Command::SelectRange, &ctx);
        v.on_command(Command::MoveDown, &ctx);
        v.on_command(Command::MoveDown, &ctx);
        assert_eq!(picked(&v), vec!["-let a = 1;", "+let a = 2;", "+let b = 3;"]);
    }

    #[test]
    fn the_pane_title_says_what_esc_goes_back_to() {
        let mut details = DetailsPane::default();
        assert_eq!(pane_title(&details), None);
        details.request_stage(entry("f.txt"));
        assert_eq!(pane_title(&details).unwrap(), "Stage f.txt · hunks loading…");
        details.set_stage_diffs(entry("f.txt"), Ok(two_hunks("f.txt")), none(), Default::default());
        details.trail = Some("Status".into());
        assert_eq!(pane_title(&details).unwrap(), "Stage f.txt · hunks  ‹ Status");
    }
}
