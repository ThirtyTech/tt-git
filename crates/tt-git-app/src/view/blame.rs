//! The view model for blame in the details pane: one row per line of code
//! on screen, reading right to left from the code: when (the age stripe and
//! the date), who, and why (the commit and its summary, where its lines
//! start). Below the code, the selected line's commit when `d` shows it,
//! and the title's right-hand part: when the file changed and whether it's
//! the latest version. [`code`](super::code) makes the code spans.

use std::ops::Range;

use tt_git_core::Recency;
use tt_git_core::time::{clock_time, relative};

use super::code::code_spans;
use super::{StyleRole, TextSpan};
use crate::panes::DateStyle;
use crate::panes::blame::{BlameView, COMMIT_ROWS, WHO_WIDTH, WHY_MIN_WIDTH};
use crate::widgets::{expand_tabs, fit, fit_right};

/// What the code part of the blame shows in a viewport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameLines<'a> {
    /// Shown instead of rows while the blame loads, or when it failed (the
    /// error, which may need wrapping).
    pub message: Option<TextSpan<'static>>,
    /// The lines on screen, by index into the blame (0 is line 1).
    pub range: Range<usize>,
    /// One per line in `range`, top to bottom.
    pub rows: Vec<BlameRow<'a>>,
}

/// One line of code with its commit, author and age.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameRow<'a> {
    /// The line's index into the blame (0 is line 1).
    pub index: usize,
    pub look: RowLook,
    pub spans: Vec<TextSpan<'a>>,
}

/// How a blame row stands out from the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowLook {
    Normal,
    /// The selected line.
    Selected,
    /// Another line from the selected line's commit.
    SameCommit,
}

impl RowLook {
    /// The look of the whole row in a pane that is or isn't `focused`:
    /// the selection's, or the unfocused selection's for the other lines of
    /// its commit.
    pub fn role(self, focused: bool) -> Option<StyleRole> {
        match self {
            RowLook::Normal => None,
            RowLook::Selected if focused => Some(StyleRole::Selected),
            RowLook::Selected | RowLook::SameCommit => Some(StyleRole::SelectedUnfocused),
        }
    }
}

/// The selected line's commit, shown below the code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameCommitView {
    /// On the rule above the details: `Commit 1234567` or `Not committed yet`.
    pub title: TextSpan<'static>,
    /// The subject, who and when, then the message; a line may be empty.
    pub lines: Vec<Vec<TextSpan<'static>>>,
}

impl BlameView {
    /// How many of `height` rows the commit details take below the code:
    /// none unless they're showing and the file has lines.
    pub fn commit_rows(&self, height: usize) -> usize {
        if self.show_commit && self.levels[self.shown()].len() > 0 {
            (COMMIT_ROWS as usize).min(height / 2)
        } else {
            0
        }
    }

    /// The code rows a viewport of `rows` lines and `width` columns shows,
    /// the selection picked out. Call
    /// [`set_viewport`](BlameView::set_viewport) first.
    pub fn lines_view(&self, rows: usize, width: usize) -> BlameLines<'_> {
        let level = &self.levels[self.shown()];
        let message = match &level.blame {
            None => Some(TextSpan::new("loading…", StyleRole::Muted)),
            Some(Err(e)) => Some(TextSpan::new(
                format!("could not blame {}: {e}", level.path),
                StyleRole::Error,
            )),
            Some(Ok(_)) => None,
        };
        let Some(blame) = level.blame().filter(|_| message.is_none()) else {
            return BlameLines {
                message,
                range: 0..0,
                rows: Vec::new(),
            };
        };
        let now = self.clock.now();
        let selected = level.cursor.selected;
        let selected_commit = blame.lines.get(selected).map(|l| l.commit);

        let num_width = blame.lines.len().to_string().len().max(3);
        let when_width = self.dates.width();
        let summary_width = if width >= WHY_MIN_WIDTH as usize {
            (width / 6).clamp(12, 30)
        } else {
            0
        };
        // " 1234567  summary" before the author.
        let why_width = if summary_width > 0 {
            1 + 7 + 2 + summary_width
        } else {
            0
        };
        let gutter = why_width + 1 + WHO_WIDTH + 1 + when_width + 1 + 1 + num_width + 1;
        let code_width = width.saturating_sub(gutter);

        let start = level.cursor.offset.min(blame.lines.len());
        let end = (start + rows).min(blame.lines.len());
        let mut out = Vec::with_capacity(end - start);
        for i in start..end {
            let line = &blame.lines[i];
            let commit = &blame.commits[line.commit];
            let uncommitted = commit.is_uncommitted();
            let first = i == 0 || blame.lines[i - 1].commit != line.commit;
            let mut spans = Vec::with_capacity(12);
            if why_width > 0 {
                if !first {
                    spans.push(TextSpan::plain(" ".repeat(why_width)));
                } else if uncommitted {
                    spans.push(TextSpan::plain(" "));
                    spans.push(TextSpan::new(
                        fit("Not committed yet", why_width - 1),
                        StyleRole::DiffAdd,
                    ));
                } else {
                    spans.push(TextSpan::plain(" "));
                    spans.push(TextSpan::new(fit(commit.short(), 7), StyleRole::Sha));
                    spans.push(TextSpan::plain("  "));
                    spans.push(TextSpan::new(fit(&commit.summary, summary_width), StyleRole::Author));
                }
            }
            spans.push(TextSpan::plain(" "));
            if uncommitted {
                spans.push(TextSpan::new(fit_right("You", WHO_WIDTH), StyleRole::BlameUncommitted));
            } else {
                let slot = level.authors.get(line.commit).copied().unwrap_or(0);
                spans.push(TextSpan::new(
                    fit_right(&commit.author, WHO_WIDTH),
                    StyleRole::BlameAuthor(slot.min(u8::MAX as usize) as u8),
                ));
            }
            spans.push(TextSpan::plain(" "));
            let when = match self.dates {
                DateStyle::Relative => relative(now, commit.author_time),
                DateStyle::Clock => clock_time(commit.author_time, self.clock.utc_offset(commit.author_time)),
            };
            spans.push(TextSpan::new(fit_right(&when, when_width), StyleRole::Date));
            spans.push(TextSpan::plain(" "));
            let stripe = if uncommitted {
                StyleRole::BlameUncommitted
            } else {
                // Unknown is the oldest: the role takes past the end as the last.
                let bucket = level.heat.get(line.commit).copied().unwrap_or(usize::MAX);
                StyleRole::BlameHeat(bucket.min(u8::MAX as usize) as u8)
            };
            spans.push(TextSpan::new("▌", stripe));
            spans.push(TextSpan::new(format!("{:>num_width$}", i + 1), StyleRole::Muted));
            spans.push(TextSpan::plain(" "));
            let tokens = level.syntax.get(i).and_then(Option::as_ref);
            let text = expand_tabs(&line.text);
            let marks = self.search.ranges(&text);
            let code = code_spans(&text, tokens, StyleRole::Plain, code_width, true, &marks);
            // The code borrows `text`, which may be a copy with its tabs
            // expanded, so it's owned.
            spans.extend(code.into_iter().map(|s| TextSpan {
                text: s.text.into_owned().into(),
                ..s
            }));
            let look = if i == selected {
                RowLook::Selected
            } else if Some(line.commit) == selected_commit {
                RowLook::SameCommit
            } else {
                RowLook::Normal
            };
            out.push(BlameRow { index: i, look, spans });
        }
        BlameLines {
            message: None,
            range: start..end,
            rows: out,
        }
    }

    /// The selected line's commit: subject, who and when, and its message
    /// (or `loading…` while it loads). `None` with no line selected.
    pub fn commit_view(&self) -> Option<BlameCommitView> {
        let level = &self.levels[self.shown()];
        let (commit, _, _) = level.selected()?;
        let blame = level.blame()?;
        let index = blame.commits.iter().position(|c| c.oid == commit.oid).unwrap_or(0);
        let count = blame.lines_from(index);
        let title = if commit.is_uncommitted() {
            "Not committed yet".to_string()
        } else {
            format!("Commit {}", commit.short())
        };
        let lines_here = |what: &str| {
            let s = if count == 1 { "" } else { "s" };
            format!("{count} line{s} in this file {what}")
        };
        let mut lines = Vec::new();
        if commit.is_uncommitted() {
            lines.push(vec![TextSpan::new(
                "Changed in the working tree",
                StyleRole::DiffHeader,
            )]);
            lines.push(vec![TextSpan::new(
                lines_here("aren't committed yet"),
                StyleRole::Author,
            )]);
        } else {
            let when = format!(
                "{} ({} ago)",
                clock_time(commit.author_time, self.clock.utc_offset(commit.author_time)),
                relative(self.clock.now(), commit.author_time)
            );
            lines.push(vec![TextSpan::new(commit.summary.clone(), StyleRole::DiffHeader)]);
            lines.push(vec![
                TextSpan::new(commit.short().to_string(), StyleRole::Sha),
                TextSpan::new(
                    format!(
                        " · {} <{}> · {when} · {}",
                        commit.author,
                        commit.author_mail,
                        lines_here("come from it")
                    ),
                    StyleRole::Author,
                ),
            ]);
            match self.messages.get(&commit.oid) {
                Some(Some(body)) if !body.is_empty() => {
                    lines.push(Vec::new());
                    lines.extend(body.lines().map(|l| vec![TextSpan::plain(expand_tabs(l).into_owned())]));
                }
                Some(None) => {
                    lines.push(Vec::new());
                    lines.push(vec![TextSpan::new("loading…", StyleRole::Muted)]);
                }
                _ => {}
            }
        }
        Some(BlameCommitView {
            title: TextSpan::new(title, StyleRole::Muted),
            lines,
        })
    }

    /// Whether the version shown is the newest, for the title: `latest`,
    /// or how many commits have changed the file since, in a role that
    /// stands out. `None` until it's loaded.
    pub fn recency_span(&self) -> Option<TextSpan<'static>> {
        let (text, role) = match self.level().recency? {
            Recency::Latest => ("latest".to_string(), StyleRole::BlameLatest),
            Recency::Behind(1) => ("1 newer commit".to_string(), StyleRole::BlameOlder),
            Recency::Behind(n) => (format!("{n} newer commits"), StyleRole::BlameOlder),
            Recency::Elsewhere => ("not on HEAD".to_string(), StyleRole::BlameOlder),
        };
        Some(TextSpan::new(text, role))
    }

    /// What can go at the right of the title, best first: when the file
    /// changed and whether it's the latest version, then that alone. The
    /// front end shows the first that fits.
    pub fn title_right(&self) -> Vec<Vec<TextSpan<'static>>> {
        match (self.changed(), self.recency_span()) {
            (Some(changed), Some(recency)) => vec![
                vec![
                    TextSpan::new(format!(" {changed} · "), StyleRole::Date),
                    recency.clone(),
                    TextSpan::plain(" "),
                ],
                vec![TextSpan::plain(" "), recency, TextSpan::plain(" ")],
            ],
            (Some(changed), None) => vec![vec![TextSpan::new(format!(" {changed} "), StyleRole::Date)]],
            (None, Some(recency)) => vec![vec![TextSpan::plain(" "), recency, TextSpan::plain(" ")]],
            (None, None) => Vec::new(),
        }
    }

    /// The details pane's title while blaming, after `[n] `: the file and
    /// version, what `esc` goes back to (`trail`, until `[` steps back), the
    /// position, a count being typed, the search, and `loading…`.
    pub fn pane_title(&self, trail: Option<&str>) -> String {
        let mut title = self.title();
        if let Some(trail) = trail
            && self.depth() == 1
        {
            title.push_str(&format!("  ‹ {trail}"));
        }
        if let Some(position) = self.position() {
            title.push_str(&format!(" {position}"));
        }
        if let Some(count) = self.count() {
            title.push_str(&format!(" go to {count}…"));
        }
        if let Some(search) = self.search_title() {
            title.push_str(&search);
        }
        if self.loading() {
            title.push_str(" loading…");
        }
        title
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tt_git_core::time::{FixedClock, SharedClock};
    use tt_git_core::{Blame, BlameCommit, BlameLine};

    use super::*;
    use crate::jobs::Ctx;

    const NOW: i64 = 1_000_000_000;

    fn ctx() -> Ctx {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        Ctx::new(tx)
    }

    fn commit(oid: char, author: &str, summary: &str, time: i64) -> BlameCommit {
        BlameCommit {
            oid: oid.to_string().repeat(40),
            author: author.into(),
            author_mail: format!("{}@example.com", author.to_lowercase()),
            summary: summary.into(),
            author_time: time,
            ..BlameCommit::default()
        }
    }

    fn line(commit: usize, text: &str) -> BlameLine {
        BlameLine {
            commit,
            orig_line: 1,
            path: 0,
            text: text.into(),
        }
    }

    /// A blame of `a.rs`: lines 1-2 from Ada's older commit, 3 from Sam's
    /// newer one, 4 not committed.
    fn view() -> (BlameView, Ctx) {
        let ctx = ctx();
        let clock: SharedClock = Arc::new(FixedClock(NOW));
        let mut view = BlameView::open(
            "a.rs".into(),
            None,
            "working tree".into(),
            1,
            DateStyle::Relative,
            clock,
            &ctx,
        );
        let blame = Blame {
            commits: vec![
                commit('a', "Ada", "Add a", NOW - 7 * 86_400),
                commit('b', "Sam", "Fix b", NOW - 3600),
                commit('0', "Not Committed Yet", "", NOW),
            ],
            paths: vec!["a.rs".into()],
            lines: vec![line(0, "fn a() {"), line(0, "\tx"), line(1, "}"), line(2, "// new")],
        };
        let id = view.levels[0].id_for_tests();
        assert!(view.set_blame(id, Ok(blame), Vec::new(), Some(NOW - 3600), Some(Recency::Latest), &ctx));
        (view, ctx)
    }

    fn text(spans: &[TextSpan]) -> String {
        spans.iter().map(|s| s.text.as_ref()).collect()
    }

    #[test]
    fn says_loading_and_errors_instead_of_rows() {
        let ctx = ctx();
        let clock: SharedClock = Arc::new(FixedClock(NOW));
        let mut view = BlameView::open("a.rs".into(), None, "HEAD".into(), 1, DateStyle::Relative, clock, &ctx);
        let lines = view.lines_view(10, 80);
        assert_eq!(lines.message, Some(TextSpan::new("loading…", StyleRole::Muted)));
        assert!(lines.rows.is_empty());
        assert!(view.commit_view().is_none());
        assert_eq!(view.commit_rows(20), 0);
        let id = view.levels[0].id_for_tests();
        view.set_blame(id, Err("no such path".into()), Vec::new(), None, None, &ctx);
        let lines = view.lines_view(10, 80);
        assert_eq!(
            lines.message,
            Some(TextSpan::new("could not blame a.rs: no such path", StyleRole::Error))
        );
        assert!(view.title_right().is_empty());
    }

    #[test]
    fn narrow_rows_have_who_when_stripe_number_and_code() {
        let (mut view, _ctx) = view();
        view.set_viewport(10);
        let lines = view.lines_view(10, 60);
        assert_eq!(lines.message, None);
        assert_eq!(lines.range, 0..4);
        let shown: Vec<String> = lines.rows.iter().map(|r| text(&r.spans)).collect();
        // Every row fills the width; tabs are expanded.
        assert!(
            shown
                .iter()
                .all(|r| unicode_width::UnicodeWidthStr::width(r.as_str()) == 60),
            "{shown:#?}"
        );
        assert!(
            shown[0].starts_with("            Ada   1w ▌  1 fn a() {"),
            "{}",
            shown[0]
        );
        assert!(shown[1].contains("▌  2     x"), "{}", shown[1]);
        assert!(shown[3].starts_with("            You  now ▌  4 // new"), "{}", shown[3]);

        let roles = |r: &BlameRow| r.spans.iter().map(|s| s.role).collect::<Vec<_>>();
        // Sam's newer commit is the hottest; Ada's is colder.
        assert!(roles(&lines.rows[2]).contains(&StyleRole::BlameHeat(0)));
        assert!(!roles(&lines.rows[0]).contains(&StyleRole::BlameHeat(0)));
        assert!(
            roles(&lines.rows[0])
                .iter()
                .any(|r| matches!(r, StyleRole::BlameAuthor(_)))
        );
        assert_eq!(
            roles(&lines.rows[3])
                .iter()
                .filter(|r| **r == StyleRole::BlameUncommitted)
                .count(),
            2,
            "the author and the stripe"
        );
    }

    #[test]
    fn wide_rows_say_why_where_a_commit_starts() {
        let (mut view, _ctx) = view();
        view.set_viewport(10);
        let lines = view.lines_view(10, 120);
        let shown: Vec<String> = lines.rows.iter().map(|r| text(&r.spans)).collect();
        assert!(shown[0].starts_with(" aaaaaaa  Add a "), "{}", shown[0]);
        assert!(shown[1].starts_with("                              "), "{}", shown[1]);
        assert!(shown[2].starts_with(" bbbbbbb  Fix b "), "{}", shown[2]);
        assert!(shown[3].starts_with(" Not committed yet "), "{}", shown[3]);
        assert_eq!(lines.rows[0].spans[1].role, StyleRole::Sha);
    }

    #[test]
    fn the_selected_line_and_its_commit_stand_out() {
        let (mut view, ctx) = view();
        view.set_viewport(10);
        let looks: Vec<_> = view.lines_view(10, 60).rows.iter().map(|r| r.look).collect();
        assert_eq!(
            looks,
            vec![RowLook::Selected, RowLook::SameCommit, RowLook::Normal, RowLook::Normal]
        );
        view.go_to_line(3, &ctx);
        let looks: Vec<_> = view.lines_view(10, 60).rows.iter().map(|r| r.look).collect();
        assert_eq!(
            looks,
            vec![RowLook::Normal, RowLook::Normal, RowLook::Selected, RowLook::Normal]
        );
        assert_eq!(RowLook::Selected.role(true), Some(StyleRole::Selected));
        assert_eq!(RowLook::Selected.role(false), Some(StyleRole::SelectedUnfocused));
        assert_eq!(RowLook::SameCommit.role(true), Some(StyleRole::SelectedUnfocused));
        assert_eq!(RowLook::Normal.role(true), None);
    }

    #[test]
    fn shows_only_the_rows_scrolled_to() {
        let (mut view, ctx) = view();
        view.set_viewport(2);
        view.go_to_line(4, &ctx);
        view.set_viewport(2);
        let lines = view.lines_view(2, 60);
        assert_eq!(lines.range, 2..4);
        assert_eq!(lines.rows.iter().map(|r| r.index).collect::<Vec<_>>(), vec![2, 3]);
    }

    #[test]
    fn commit_details_take_up_to_half_the_height() {
        let (mut view, ctx) = view();
        assert_eq!(view.commit_rows(40), 0);
        view.on_command(crate::keymap::Command::ToggleDetails, &ctx);
        assert_eq!(view.commit_rows(40), COMMIT_ROWS as usize);
        assert_eq!(view.commit_rows(8), 4);

        let details = view.commit_view().expect("a line is selected");
        assert_eq!(details.title, TextSpan::new("Commit aaaaaaa", StyleRole::Muted));
        assert_eq!(text(&details.lines[0]), "Add a");
        assert_eq!(details.lines[0][0].role, StyleRole::DiffHeader);
        assert!(
            text(&details.lines[1]).starts_with("aaaaaaa · Ada <ada@example.com> · "),
            "{}",
            text(&details.lines[1])
        );
        assert!(text(&details.lines[1]).ends_with(" · 2 lines in this file come from it"));
        // The message is on its way.
        assert_eq!(text(&details.lines[3]), "loading…");
        view.set_message(&"a".repeat(40), "Why a\n\tbecause".into());
        let details = view.commit_view().unwrap();
        assert_eq!(text(&details.lines[3]), "Why a");
        assert_eq!(text(&details.lines[4]), "    because");

        view.go_to_line(4, &ctx);
        let details = view.commit_view().unwrap();
        assert_eq!(text(&[details.title]), "Not committed yet");
        assert_eq!(text(&details.lines[1]), "1 line in this file aren't committed yet");
    }

    #[test]
    fn the_title_says_where_and_how_new() {
        let (view, _ctx) = view();
        assert_eq!(
            view.pane_title(Some("Status")),
            "Blame a.rs at working tree  ‹ Status 1/4"
        );
        let right = view.title_right();
        assert_eq!(right.len(), 2);
        assert!(text(&right[0]).starts_with(" changed "), "{}", text(&right[0]));
        assert!(text(&right[0]).ends_with(" + uncommitted changes · latest "));
        assert_eq!(text(&right[1]), " latest ");
        assert_eq!(right[1][1].role, StyleRole::BlameLatest);
    }

    #[test]
    fn search_matches_lie_over_the_code() {
        let (mut view, ctx) = view();
        view.on_command(crate::keymap::Command::Search, &ctx);
        for c in "new".chars() {
            view.on_search_key(crate::keymap::KeyCombo::plain(crate::keymap::KeyCode::Char(c)), &ctx);
        }
        view.set_viewport(10);
        let lines = view.lines_view(10, 60);
        let hit = lines.rows[3]
            .spans
            .iter()
            .find(|s| s.over == Some(StyleRole::SearchMatch))
            .expect("a match");
        assert_eq!(hit.text, "new");
    }
}
