//! The details pane's view model: the title, the commit or diff text with
//! syntax highlighting and search matches ([`code`](super::code) makes the
//! code spans), and in the files view the file list beside or below it, the
//! commit header and how the space is shared between them.
//!
//! Staging and blame draw their own bodies ([`stage`](super::stage),
//! [`blame`](super::blame)); the title here covers them too.

use std::borrow::Cow;

use tt_git_core::{CommitFile, FileState};
use unicode_width::UnicodeWidthStr;

use super::code::code_spans;
use super::{StyleRole, TextSpan};
use crate::panes::{BlameView, DetailsPane, StageView};
use crate::syntax::Tokens;
use crate::widgets::truncate;

/// What the pane says when it has nothing to show.
pub const NOTHING_SELECTED: &str = "nothing selected";

/// One line of the text or diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRow<'a> {
    pub spans: Vec<TextSpan<'a>>,
    /// Behind the whole row: [`StyleRole::DiffAddLine`] or
    /// [`StyleRole::DiffDelLine`] for an added or deleted line of
    /// highlighted code.
    pub back: Option<StyleRole>,
    /// Laid over [`back`](Self::back) and under the spans: the line blame
    /// came from ([`StyleRole::Selected`]) or the match `n` and `N` went to
    /// last ([`StyleRole::SelectedUnfocused`]).
    pub pick: Option<StyleRole>,
}

/// One row of the files view's file list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow<'a> {
    /// The file's index into the list.
    pub index: usize,
    /// It's the selected file.
    pub picked: bool,
    pub spans: Vec<TextSpan<'a>>,
}

/// How the files view shares the pane's body, in columns and rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilesLayout {
    /// The list is left of the diff (fullscreen), not along the bottom.
    pub beside: bool,
    /// Beside: the list's columns, its right border included. Below: the
    /// whole width.
    pub list_width: usize,
    /// The list's file rows, under its title line (beside) or its rule
    /// carrying the title (below).
    pub list_rows: usize,
    /// The commit header's rows above the diff, then a rule when there are any.
    pub header_rows: usize,
    /// The diff's rows and columns.
    pub diff_rows: usize,
    pub diff_width: usize,
}

impl FilesLayout {
    /// Body rows above the diff: the header and its rule.
    pub fn diff_top(&self) -> usize {
        self.header_rows + usize::from(self.header_rows > 0)
    }

    /// The body row the list's first file is on.
    pub fn list_top(&self, height: usize) -> usize {
        if self.beside {
            1
        } else {
            height.saturating_sub(self.list_rows)
        }
    }
}

impl DetailsPane {
    /// The pane's title, after `[n] `: what it shows, what `esc` goes back
    /// to, blame's position, the search, and whether it's loading. Blame's
    /// date and recency go at the right, separately.
    pub fn frame_title(&self) -> String {
        let view_title =
            (self.stage.as_ref().map(StageView::title)).or_else(|| self.blame.as_ref().map(BlameView::title));
        let mut title = match view_title {
            Some(t) => t,
            None if self.subject.is_some() => self.title.clone(),
            None => "Details".to_string(),
        };
        // Blame's own steps back name the version before; otherwise, what esc goes back to.
        if let Some(trail) = &self.trail
            && self.blame.as_ref().is_none_or(|v| v.depth() == 1)
        {
            title.push_str(&format!("  ‹ {trail}"));
        }
        if let Some(position) = self.blame.as_ref().and_then(BlameView::position) {
            title.push_str(&format!(" {position}"));
        }
        if let Some(count) = self.blame.as_ref().and_then(BlameView::count) {
            title.push_str(&format!(" go to {count}…"));
        }
        let search = match &self.blame {
            Some(view) => view.search_title(),
            None if self.stage.is_none() => self.search.title(self.found, !self.loading),
            None => None,
        };
        if let Some(search) = search {
            title.push_str(&search);
        }
        if self.loading || self.blame.as_ref().is_some_and(BlameView::loading) {
            title.push_str(" loading…");
        }
        title
    }

    /// What the pane says instead of its body when there's nothing to show.
    pub fn empty_message(&self) -> Option<&'static str> {
        self.subject.is_none().then_some(NOTHING_SELECTED)
    }

    /// The text (in the files view, the selected file's diff) on screen in
    /// a viewport of `rows` lines and `width` columns, from the scroll
    /// offset. Call [`set_viewport`](Self::set_viewport) first (and
    /// [`set_files_viewport`](Self::set_files_viewport) before it in the
    /// files view).
    pub fn text_rows(&self, rows: usize, width: usize) -> Vec<TextRow<'_>> {
        let mark = self.files_mark();
        let start = self.cursor.offset.min(self.lines.len());
        let end = (start + rows).min(self.lines.len());
        (start..end)
            .map(|row| {
                let text = &self.lines[row];
                let tokens = self.syntax.get(row).and_then(Option::as_ref);
                let found = self.search.ranges(text);
                match mark {
                    Some((at, line)) if at == row => {
                        let note = format!("◂ line {line} in blame ");
                        let pad = width.saturating_sub(text.width() + note.width());
                        let mut marked = diff_row(text, tokens, 0, &found);
                        marked.spans.push(TextSpan::plain(" ".repeat(pad)));
                        marked.spans.push(TextSpan::new(note, StyleRole::HelpText));
                        marked.pick = Some(StyleRole::Selected);
                        marked
                    }
                    _ if self.found == Some(row) => TextRow {
                        pick: Some(StyleRole::SelectedUnfocused),
                        ..diff_row(text, tokens, width, &found)
                    },
                    _ => diff_row(text, tokens, width, &found),
                }
            })
            .collect()
    }

    /// The files view's list title: `Files (n)` once loaded.
    pub fn files_label(&self) -> Option<String> {
        let view = self.files.as_ref()?;
        Some(if self.loading {
            "Files".to_string()
        } else {
            format!("Files ({})", view.files.len())
        })
    }

    /// The columns the list beside the diff wants in a body `width` wide:
    /// its longest file, between 20 and two fifths of the width.
    pub fn files_list_width(&self, width: usize) -> usize {
        let Some(view) = &self.files else { return 0 };
        let longest = view.files.iter().map(|f| file_label(f).width() + 3).max().unwrap_or(0);
        (longest + 1).clamp(20, (width * 2 / 5).max(20))
    }

    /// The file rows the list below the diff wants in a body `height`
    /// high: every file, up to a third of the height. One more for the
    /// rule over it.
    pub fn files_list_rows(&self, height: usize) -> usize {
        let Some(view) = &self.files else { return 0 };
        view.files.len().clamp(1, (height / 3).max(2))
    }

    /// The commit header's rows over the diff in a space `height` rows
    /// high: up to a third of it. Above the bottom list the space can be
    /// tiny; under three rows the header (and its rule) give way so the
    /// diff still shows.
    pub fn files_header_rows(&self, height: usize) -> usize {
        let Some(view) = &self.files else { return 0 };
        let max = height / 3;
        let max = if self.files_beside { max.max(1) } else { max };
        view.header.len().min(max)
    }

    /// The files view's layout in a body `width` columns by `height` rows,
    /// or `None` outside it.
    pub fn files_layout(&self, width: usize, height: usize) -> Option<FilesLayout> {
        self.files.as_ref()?;
        let layout = if self.files_beside {
            let list_width = self.files_list_width(width).min(width);
            let header_rows = self.files_header_rows(height).min(height);
            let rule = usize::from(header_rows > 0);
            FilesLayout {
                beside: true,
                list_width,
                list_rows: height.saturating_sub(1),
                header_rows,
                diff_rows: height.saturating_sub(header_rows + rule),
                // A column of space after the list's border.
                diff_width: width.saturating_sub(list_width + 1),
            }
        } else {
            let list_rows = self.files_list_rows(height).min(height.saturating_sub(1));
            let top = height.saturating_sub(list_rows + 1);
            let header_rows = self.files_header_rows(top);
            let rule = usize::from(header_rows > 0);
            FilesLayout {
                beside: false,
                list_width: width,
                list_rows,
                header_rows,
                diff_rows: top.saturating_sub(header_rows + rule),
                diff_width: width,
            }
        };
        Some(layout)
    }

    /// The commit header's first `rows` lines, in the files view.
    pub fn header_rows(&self, rows: usize) -> Vec<Vec<TextSpan<'_>>> {
        let Some(view) = &self.files else { return Vec::new() };
        view.header
            .iter()
            .take(rows)
            .map(|l| diff_row(l, None, 0, &[]).spans)
            .collect()
    }

    /// The files on screen in a list `rows` lines and `width` columns,
    /// from its scroll offset. Call
    /// [`set_files_viewport`](Self::set_files_viewport) first.
    pub fn file_rows(&self, rows: usize, width: usize) -> Vec<FileRow<'_>> {
        let Some(view) = &self.files else { return Vec::new() };
        let start = view.cursor.offset.min(view.files.len());
        let end = (start + rows).min(view.files.len());
        (start..end)
            .map(|index| {
                let file = &view.files[index];
                FileRow {
                    index,
                    picked: index == view.cursor.selected,
                    spans: vec![
                        TextSpan::new(file.state.code().to_string(), state_role(file.state)),
                        TextSpan::plain(" "),
                        TextSpan::plain(truncate(&file_label(file), width.saturating_sub(2))),
                    ],
                }
            })
            .collect()
    }
}

/// A file as the list names it: `old → new` when renamed.
pub fn file_label(file: &CommitFile) -> Cow<'_, str> {
    match &file.orig_path {
        Some(orig) => format!("{orig} → {}", file.path).into(),
        None => file.path.as_str().into(),
    }
}

/// How a changed file's state letter looks.
pub fn state_role(state: FileState) -> StyleRole {
    match state {
        FileState::Added | FileState::Copied => StyleRole::DiffAdd,
        FileState::Deleted => StyleRole::DiffDel,
        FileState::Renamed => StyleRole::DiffHunk,
        _ => StyleRole::LabelTag,
    }
}

/// A line of diff or `git show` output. A hunk line with `tokens` is in
/// code colors, an added or deleted one on a tinted background (padded out
/// to `width` columns) with its `+` or `-` in the usual green or red. The
/// byte ranges `found` are a search's matches.
pub fn diff_row<'a>(
    line: &'a str,
    tokens: Option<&Tokens>,
    width: usize,
    found: &[std::ops::Range<usize>],
) -> TextRow<'a> {
    if let Some(tokens) = tokens
        && let Some(code) = line.get(1..)
    {
        let (sign, back) = match line.as_bytes().first() {
            Some(b'+') => (StyleRole::DiffAdd, Some(StyleRole::DiffAddLine)),
            Some(b'-') => (StyleRole::DiffDel, Some(StyleRole::DiffDelLine)),
            _ => (StyleRole::Plain, None),
        };
        let mut sign = TextSpan::new(&line[..1], sign);
        if found.first().is_some_and(|m| m.start == 0) {
            sign = sign.with_over(StyleRole::SearchMatch);
        }
        let mut spans = vec![sign];
        // The matches in the code, after the sign.
        let in_code: Vec<_> = found
            .iter()
            .filter(|m| m.end > 1)
            .map(|m| m.start.max(1) - 1..m.end - 1)
            .collect();
        spans.extend(code_spans(
            code,
            Some(tokens),
            StyleRole::Plain,
            usize::MAX,
            false,
            &in_code,
        ));
        let pad = width.saturating_sub(line.width());
        if pad > 0 && back.is_some() {
            spans.push(TextSpan::plain(" ".repeat(pad)));
        }
        return TextRow {
            spans,
            back,
            pick: None,
        };
    }
    let role = if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff --git") {
        StyleRole::DiffHeader
    } else if line.starts_with('+') {
        StyleRole::DiffAdd
    } else if line.starts_with('-') {
        StyleRole::DiffDel
    } else if line.starts_with("@@") {
        StyleRole::DiffHunk
    } else if line.starts_with("commit ") {
        StyleRole::Sha
    } else {
        StyleRole::Plain
    };
    TextRow {
        spans: code_spans(line, None, role, usize::MAX, false, found),
        back: None,
        pick: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::Component;
    use crate::jobs::Ctx;
    use crate::keymap::{Command, KeyCode, KeyCombo};
    use crate::panes::DetailsSubject;
    use crate::syntax::Class;

    fn ctx() -> Ctx {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        Ctx::new(tx)
    }

    fn text(spans: &[TextSpan]) -> String {
        spans.iter().map(|s| s.text.as_ref()).collect()
    }

    fn file(path: &str, state: FileState) -> CommitFile {
        CommitFile {
            state,
            path: path.into(),
            orig_path: None,
        }
    }

    /// A pane showing `text` as a commit.
    fn showing(text: &str) -> DetailsPane {
        let subject = DetailsSubject::Commit("a".into());
        let mut pane = DetailsPane::default();
        pane.request(subject.clone(), "Commit a".into());
        pane.set_content(&subject, text, Vec::new());
        pane
    }

    /// A files view of commit `a` with `files`, the first one's diff `diff`.
    fn files_view(files: Vec<CommitFile>, diff: &str) -> DetailsPane {
        let ctx = ctx();
        let mut pane = DetailsPane::default();
        pane.request(DetailsSubject::CommitFiles("a".into()), "Commit a files".into());
        let first = files[0].path.clone();
        pane.set_commit_files("a", "subject\nAuthor: me\n\nbody", files, &ctx);
        pane.set_file_diff("a", &first, diff, Vec::new(), &ctx);
        pane
    }

    fn search(pane: &mut DetailsPane, pattern: &str) {
        let ctx = ctx();
        pane.on_command(Command::Search, &ctx);
        for c in pattern.chars() {
            pane.on_text_key(KeyCombo::char(c), &ctx);
        }
        pane.on_text_key(KeyCombo::plain(KeyCode::Enter), &ctx);
    }

    #[test]
    fn diff_lines_take_the_role_their_start_says() {
        let role = |line: &str| diff_row(line, None, 40, &[]).spans[0].role;
        assert_eq!(role("diff --git a/x b/x"), StyleRole::DiffHeader);
        assert_eq!(role("--- a/x"), StyleRole::DiffHeader);
        assert_eq!(role("+++ b/x"), StyleRole::DiffHeader);
        assert_eq!(role("+added"), StyleRole::DiffAdd);
        assert_eq!(role("-gone"), StyleRole::DiffDel);
        assert_eq!(role("@@ -1 +1 @@"), StyleRole::DiffHunk);
        assert_eq!(role("commit abc"), StyleRole::Sha);
        assert_eq!(role("Author: me"), StyleRole::Plain);
        // Without code, nothing behind the row and no padding.
        let row = diff_row("+added", None, 40, &[]);
        assert_eq!((row.back, text(&row.spans)), (None, "+added".into()));
        assert!(diff_row("", None, 40, &[]).spans.is_empty());
    }

    #[test]
    fn changed_code_is_highlighted_on_a_tinted_row_padded_to_the_width() {
        let tokens = vec![(0..2, Class::Keyword)];
        let row = diff_row("+fn x", Some(&tokens), 10, &[]);
        assert_eq!(row.back, Some(StyleRole::DiffAddLine));
        assert_eq!(row.spans[0], TextSpan::new("+", StyleRole::DiffAdd));
        assert_eq!(row.spans[1], TextSpan::new("fn", StyleRole::Code(Class::Keyword)));
        assert_eq!(text(&row.spans), "+fn x     ");
        let row = diff_row("-fn x", Some(&tokens), 10, &[]);
        assert_eq!(row.back, Some(StyleRole::DiffDelLine));
        assert_eq!(row.spans[0].role, StyleRole::DiffDel);
        // Context lines keep their colors but no tint or padding.
        let row = diff_row(" fn x", Some(&tokens), 10, &[]);
        assert_eq!(row.back, None);
        assert_eq!(row.spans[0].role, StyleRole::Plain);
        assert_eq!(text(&row.spans), " fn x");
    }

    #[test]
    fn search_matches_lie_over_the_sign_and_the_code() {
        let tokens = vec![(0..2, Class::Keyword)];
        let row = diff_row("+fn x", Some(&tokens), 0, &[0..2, 4..5]);
        let hit = Some(StyleRole::SearchMatch);
        let shown: Vec<_> = row.spans.iter().map(|s| (s.text.as_ref(), s.role, s.over)).collect();
        assert_eq!(
            shown,
            vec![
                ("+", StyleRole::DiffAdd, hit),
                ("f", StyleRole::Code(Class::Keyword), hit),
                ("n", StyleRole::Code(Class::Keyword), None),
                (" ", StyleRole::Plain, None),
                ("x", StyleRole::Plain, hit),
            ]
        );
        let row = diff_row("-gone", None, 0, std::slice::from_ref(&(1..3)));
        assert_eq!(
            row.spans[1],
            TextSpan::new("go", StyleRole::DiffDel).with_over(StyleRole::SearchMatch)
        );
    }

    #[test]
    fn text_rows_scroll_and_pick_out_the_match_gone_to() {
        let mut pane = showing("one\nhay\nneedle\nhay\nneedle");
        pane.set_viewport(3);
        let rows = pane.text_rows(3, 20);
        assert_eq!(rows.len(), 3);
        assert_eq!(text(&rows[0].spans), "one");
        assert!(rows.iter().all(|r| r.pick.is_none()));

        search(&mut pane, "needle");
        pane.on_command(Command::SearchNext, &ctx());
        let found = pane.found.expect("went to a match");
        pane.set_viewport(3);
        let start = pane.cursor.offset;
        let rows = pane.text_rows(3, 20);
        let picked = &rows[found - start];
        assert_eq!(picked.pick, Some(StyleRole::SelectedUnfocused));
        assert_eq!(picked.spans[0].over, Some(StyleRole::SearchMatch));
        assert!(rows.iter().filter(|r| r.pick.is_some()).count() == 1);

        // Scrolled to the end, only what's left is shown.
        pane.on_command(Command::Bottom, &ctx());
        pane.set_viewport(3);
        assert_eq!(pane.text_rows(3, 20).len(), 3);
        assert_eq!(text(&pane.text_rows(3, 20)[2].spans), "needle");
        assert_eq!(pane.text_rows(10, 20).len(), 3, "from the offset to the end");
    }

    #[test]
    fn the_line_blame_came_from_is_marked_with_a_note() {
        let ctx = ctx();
        let mut pane = DetailsPane::default();
        pane.request(DetailsSubject::CommitFiles("a".into()), "Commit a files".into());
        pane.focus_file("b.txt".into(), 2);
        pane.set_commit_files(
            "a",
            "subject",
            vec![file("a.txt", FileState::Added), file("b.txt", FileState::Added)],
            &ctx,
        );
        pane.set_file_diff("a", "b.txt", "@@ -0,0 +1,3 @@\n+one\n+two\n+three", Vec::new(), &ctx);
        pane.set_files_viewport(5, 1, 10);
        pane.set_viewport(10);
        let rows = pane.text_rows(10, 30);
        let marked: Vec<_> = rows.iter().filter(|r| r.pick.is_some()).collect();
        assert_eq!(marked.len(), 1);
        assert_eq!(marked[0].pick, Some(StyleRole::Selected));
        let line = text(&marked[0].spans);
        assert!(
            line.starts_with("+two") && line.ends_with("◂ line 2 in blame "),
            "{line:?}"
        );
        assert_eq!(line.width(), 30, "padded out to the width");
        assert_eq!(marked[0].spans.last().unwrap().role, StyleRole::HelpText);
    }

    #[test]
    fn the_file_list_shows_each_state_and_the_selection() {
        let mut renamed = file("new.rs", FileState::Renamed);
        renamed.orig_path = Some("old.rs".into());
        let files = vec![
            file("a.txt", FileState::Added),
            file("b.txt", FileState::Deleted),
            renamed,
            file("d.txt", FileState::Modified),
        ];
        let mut pane = files_view(files, "+a");
        assert_eq!(pane.files_label().as_deref(), Some("Files (4)"));
        pane.set_files_viewport(3, 1, 10);
        let rows = pane.file_rows(3, 40);
        assert_eq!(rows.iter().map(|r| r.index).collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(rows.iter().map(|r| r.picked).collect::<Vec<_>>(), [true, false, false]);
        let roles: Vec<_> = rows.iter().map(|r| r.spans[0].role).collect();
        assert_eq!(roles, [StyleRole::DiffAdd, StyleRole::DiffDel, StyleRole::DiffHunk]);
        assert_eq!(text(&rows[2].spans), "R old.rs → new.rs");
        // Cut to the width.
        assert_eq!(text(&pane.file_rows(3, 8)[2].spans), "R old.r…");

        // The last file: the list scrolls to keep it in view.
        pane.on_command(Command::Bottom, &ctx());
        pane.set_files_viewport(3, 1, 10);
        let rows = pane.file_rows(3, 40);
        assert_eq!(rows.iter().map(|r| r.index).collect::<Vec<_>>(), [1, 2, 3]);
        assert!(rows[2].picked);
        assert_eq!(rows[2].spans[0].role, StyleRole::LabelTag);

        // Not in the files view: nothing.
        let pane = showing("text");
        assert_eq!(pane.files_label(), None);
        assert!(pane.file_rows(3, 40).is_empty());
        assert_eq!(pane.files_layout(80, 20), None);
    }

    #[test]
    fn a_loading_files_view_has_no_count() {
        let mut pane = DetailsPane::default();
        pane.request(DetailsSubject::CommitFiles("a".into()), "Commit a files".into());
        assert_eq!(pane.files_label().as_deref(), Some("Files"));
    }

    #[test]
    fn the_files_view_shares_the_space() {
        let files: Vec<_> = (0..10)
            .map(|i| file(&format!("f{i}.txt"), FileState::Modified))
            .collect();
        let mut pane = files_view(files, "+a");
        // Along the bottom: up to a third of the height, under a rule.
        let below = pane.files_layout(80, 30).unwrap();
        assert_eq!(
            below,
            FilesLayout {
                beside: false,
                list_width: 80,
                list_rows: 10,
                header_rows: 4,
                diff_rows: 14,
                diff_width: 80,
            }
        );
        assert_eq!((below.diff_top(), below.list_top(30)), (5, 20));
        // In a short pane the header gives way.
        let tiny = pane.files_layout(80, 5).unwrap();
        assert_eq!((tiny.list_rows, tiny.header_rows, tiny.diff_rows), (2, 0, 2));

        // Beside: the longest name, at least 20 columns, at most two fifths.
        pane.set_files_beside(true);
        let beside = pane.files_layout(100, 30).unwrap();
        assert_eq!(
            beside,
            FilesLayout {
                beside: true,
                list_width: 20,
                list_rows: 29,
                header_rows: 4,
                diff_rows: 25,
                diff_width: 79,
            }
        );
        assert_eq!(beside.list_top(30), 1);
        // The header keeps a row however short the pane.
        assert_eq!(pane.files_layout(100, 2).unwrap().header_rows, 1);
        assert_eq!(pane.files_list_width(200), 20);

        let header = pane.header_rows(2);
        assert_eq!(header.len(), 2);
        assert_eq!(text(&header[1]), "Author: me");
    }

    #[test]
    fn the_title_says_what_is_shown() {
        let mut pane = DetailsPane::default();
        assert_eq!(pane.frame_title(), "Details");
        assert_eq!(pane.empty_message(), Some(NOTHING_SELECTED));
        pane.request(DetailsSubject::Commit("a".into()), "Commit a".into());
        assert_eq!(pane.frame_title(), "Commit a loading…");
        assert_eq!(pane.empty_message(), None);
        pane.set_content(&DetailsSubject::Commit("a".into()), "x\nneedle", Vec::new());
        pane.set_trail(Some("Diff a.txt".into()));
        assert_eq!(pane.frame_title(), "Commit a  ‹ Diff a.txt");
        search(&mut pane, "needle");
        assert!(
            pane.frame_title().starts_with("Commit a  ‹ Diff a.txt /needle"),
            "{}",
            pane.frame_title()
        );
    }
}
