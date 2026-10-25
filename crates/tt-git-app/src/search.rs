//! A regex search through lines of text, typed after `/`: blame's code and
//! the details pane's text. Typing jumps to the first match; `n` and `N` go
//! to the next and previous matching line, round past the end.

use std::ops::Range;

use regex::{Regex, RegexBuilder};
use tt_git_core::ignores_case;

use crate::action::Action;
use crate::jobs::Ctx;
use crate::keymap::{KeyCode, KeyCombo};

#[derive(Debug, Default)]
pub struct LineSearch {
    pattern: String,
    /// `pattern` compiled; `None` while it's empty or doesn't compile.
    regex: Option<Regex>,
    /// Why `pattern` doesn't compile.
    invalid: Option<String>,
    /// Keys go to the pattern.
    typing: bool,
    /// The line (index) the search was started from, to come back to.
    origin: usize,
    /// The lines (indices) that match, in order.
    lines: Vec<usize>,
}

/// What a key typed into the search did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKey {
    /// The pattern changed: find the matches again, then go to
    /// [`landing`](LineSearch::landing).
    Changed,
    /// `esc`: the search is dropped; go back to this line.
    Cancelled(usize),
    /// `enter`, or a key that does nothing.
    Done,
}

impl LineSearch {
    /// A search being typed, started from line `origin`.
    pub fn start(origin: usize) -> Self {
        Self {
            typing: true,
            origin,
            ..Self::default()
        }
    }

    /// Whether the search is being typed.
    pub fn typing(&self) -> bool {
        self.typing
    }

    /// The matching lines, in order.
    pub fn lines(&self) -> &[usize] {
        &self.lines
    }

    /// Whether there's a pattern to search for.
    pub fn active(&self) -> bool {
        self.regex.is_some()
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// The next matching line after `at`, or the one before it, without
    /// going round past the end.
    pub fn after(&self, at: usize, forward: bool) -> Option<usize> {
        let lines = &self.lines;
        if forward {
            lines.get(lines.partition_point(|&l| l <= at)).copied()
        } else {
            lines[..lines.partition_point(|&l| l < at)].last().copied()
        }
    }

    /// Compiles the pattern: ignoring case unless it has a capital letter.
    fn compile(&mut self) {
        self.regex = None;
        self.invalid = None;
        if self.pattern.is_empty() {
            return;
        }
        match RegexBuilder::new(&self.pattern)
            .case_insensitive(ignores_case(&self.pattern))
            .build()
        {
            Ok(regex) => self.regex = Some(regex),
            Err(e) => self.invalid = Some(e.to_string()),
        }
    }

    /// Where it matches `text`, leaving out empty matches.
    pub fn ranges(&self, text: &str) -> Vec<Range<usize>> {
        self.regex.as_ref().map_or_else(Vec::new, |r| {
            r.find_iter(text).filter(|m| !m.is_empty()).map(|m| m.range()).collect()
        })
    }

    /// Works out which of `texts`, the lines searched, match.
    pub fn find<'a>(&mut self, texts: impl IntoIterator<Item = &'a str>) {
        self.lines = match &self.regex {
            Some(regex) => texts
                .into_iter()
                .enumerate()
                .filter(|(_, t)| regex.is_match(t))
                .map(|(i, _)| i)
                .collect(),
            None => Vec::new(),
        };
    }

    /// A key while the search is typed. `enter` keeps the search (an
    /// invalid one is dropped, saying why; an empty one just dropped).
    pub fn on_key(&mut self, key: KeyCombo, ctx: &Ctx) -> SearchKey {
        match key.code {
            KeyCode::Enter => {
                self.typing = false;
                if let Some(why) = self.invalid.take() {
                    ctx.emit(Action::Notify(format!("invalid regex: {why}")));
                    *self = Self::default();
                } else if self.pattern.is_empty() {
                    *self = Self::default();
                }
                return SearchKey::Done;
            }
            KeyCode::Esc => {
                let origin = self.origin;
                *self = Self::default();
                return SearchKey::Cancelled(origin);
            }
            KeyCode::Backspace => {
                self.pattern.pop();
            }
            KeyCode::Char(c) => self.pattern.push(c),
            _ => return SearchKey::Done,
        }
        self.compile();
        SearchKey::Changed
    }

    /// Where typing goes: the first match from where the search started,
    /// else the first, else back where it started.
    pub fn landing(&self) -> usize {
        let lines = &self.lines;
        lines
            .get(lines.partition_point(|&l| l < self.origin))
            .or(lines.first())
            .copied()
            .unwrap_or(self.origin)
    }

    /// The next matching line after `at`, or the one before it, round past
    /// the end (saying so). `None`, having said why, when there's none.
    pub fn step(&self, at: usize, forward: bool, ctx: &Ctx) -> Option<usize> {
        if self.regex.is_none() {
            ctx.emit(Action::Notify("nothing to search for yet".into()));
            return None;
        }
        let lines = &self.lines;
        let (Some(&first), Some(&last)) = (lines.first(), lines.last()) else {
            ctx.emit(Action::Notify(format!("no match for /{}", self.pattern)));
            return None;
        };
        Some(self.after(at, forward).unwrap_or_else(|| {
            let why = if forward {
                "search wrapped to the top"
            } else {
                "search wrapped to the bottom"
            };
            ctx.emit(Action::Notify(why.into()));
            if forward { first } else { last }
        }))
    }

    /// The search for a title: ` /pattern█` while it's typed, then, once
    /// `counted` (the text is there to search), which match `at` is of how
    /// many.
    pub fn title(&self, at: Option<usize>, counted: bool) -> Option<String> {
        if !self.typing && self.pattern.is_empty() {
            return None;
        }
        let mut title = format!(" /{}", self.pattern);
        if self.typing {
            title.push('█');
        }
        if self.invalid.is_some() {
            title.push_str(" invalid regex");
        } else if self.regex.is_some() && counted {
            let count = self.lines.len();
            match at.map(|at| self.lines.binary_search(&at)) {
                _ if count == 0 => title.push_str(" no match"),
                Some(Ok(i)) => title.push_str(&format!(" {}/{count}", i + 1)),
                _ if count == 1 => title.push_str(" 1 match"),
                _ => title.push_str(&format!(" {count} matches")),
            }
        }
        Some(title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str, origin: usize, ctx: &Ctx) -> LineSearch {
        let mut search = LineSearch::start(origin);
        for c in text.chars() {
            search.on_key(KeyCombo::char(c), ctx);
        }
        search
    }

    #[test]
    fn finds_steps_and_wraps() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = Ctx::new(tx);
        let texts = ["fn a()", "let x", "FN b()", "", "fn c()"];
        let mut search = typed("fn", 2, &ctx);
        search.find(texts);
        assert_eq!(search.lines(), [0, 2, 4], "lowercase ignores case");
        assert_eq!(search.landing(), 2, "the first from where it started");
        assert_eq!(search.title(Some(2), true).as_deref(), Some(" /fn█ 2/3"));
        assert_eq!(search.title(None, true).as_deref(), Some(" /fn█ 3 matches"));
        assert_eq!(search.title(Some(2), false).as_deref(), Some(" /fn█"));
        assert_eq!(search.on_key("enter".parse().unwrap(), &ctx), SearchKey::Done);
        assert!(!search.typing());

        assert_eq!(search.step(2, true, &ctx), Some(4));
        assert_eq!(search.step(4, true, &ctx), Some(0));
        assert_eq!(search.step(0, false, &ctx), Some(4));
        assert_eq!(search.step(3, false, &ctx), Some(2), "from between matches");
        let said: Vec<String> = std::iter::from_fn(|| rx.try_recv().ok())
            .filter_map(|a| match a {
                Action::Notify(m) => Some(m),
                _ => None,
            })
            .collect();
        assert_eq!(said, ["search wrapped to the top", "search wrapped to the bottom"]);
        assert_eq!(search.ranges("fn a() fn"), [0..2, 7..9]);

        let mut capital = typed("FN", 0, &ctx);
        capital.find(texts);
        assert_eq!(capital.lines(), [2], "a capital matches case");
    }

    #[test]
    fn esc_goes_back_and_bad_patterns_are_dropped() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = Ctx::new(tx);
        let mut search = typed("x", 7, &ctx);
        assert_eq!(search.on_key("esc".parse().unwrap(), &ctx), SearchKey::Cancelled(7));
        assert_eq!(search.title(None, true), None);

        let mut search = typed("(", 0, &ctx);
        assert_eq!(search.title(None, true).as_deref(), Some(" /(█ invalid regex"));
        search.on_key("enter".parse().unwrap(), &ctx);
        assert_eq!(search.title(None, true), None);
        assert!(matches!(rx.try_recv(), Ok(Action::Notify(m)) if m.starts_with("invalid regex")));
        assert_eq!(search.step(0, true, &ctx), None);
        assert!(matches!(rx.try_recv(), Ok(Action::Notify(m)) if m == "nothing to search for yet"));
    }
}
