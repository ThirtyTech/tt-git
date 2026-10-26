//! The branches pane's view model: its title (the tabs, the base and the
//! filter), one row of spans per entry on screen in the current tab
//! (branches, tags, merged and unmerged branches, stashes, worktrees) with
//! plugin decorations and merge-conflict marks, and the notes on its bottom
//! border.

use std::ops::Range;

use tt_git_core::time::relative;
use tt_git_core::{BaseBranches, BranchEntry, BranchState, GitRef, Stash, Worktree};
use unicode_width::UnicodeWidthStr;

use super::{StyleRole, TextSpan};
use crate::panes::branches::{AUTHOR_WIDTH, DATE_WIDTH, STATE_WIDTH, SUBJECT_MIN_WIDTH};
use crate::panes::{BranchTab, BranchesPane};
use crate::widgets::{fit, fit_right, truncate};

/// The pane's title, after `[n] `.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchesTitle {
    pub spans: Vec<TextSpan<'static>>,
    /// The columns each tab's name takes in the title, from its start, for
    /// clicks on the tabs.
    pub tabs: Vec<(BranchTab, Range<usize>)>,
}

impl BranchesTitle {
    /// How many columns the title takes.
    pub fn width(&self) -> usize {
        self.spans.iter().map(|s| s.text.width()).sum()
    }

    /// The tab at column `x` of the title, if there's one there.
    pub fn tab_at(&self, x: usize) -> Option<BranchTab> {
        self.tabs.iter().find(|(_, xs)| xs.contains(&x)).map(|(tab, _)| *tab)
    }
}

/// What the bottom border says, each with a space either side.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchesFooter {
    /// On the left: the files the selected row's merge into the base would
    /// leave in conflict.
    pub conflicts: Option<TextSpan<'static>>,
    /// On the right, on the worktrees tab: the disk space they take.
    pub total: Option<TextSpan<'static>>,
}

/// What the list shows in a viewport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchesView {
    /// What the pane says instead of rows when the tab shows none.
    pub empty: Option<String>,
    /// The rows on screen, by position among those the filter lets through
    /// ([`BranchesPane::visible`]).
    pub range: Range<usize>,
    /// One per position in `range`, top to bottom.
    pub rows: Vec<BranchRow>,
}

/// One entry's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRow {
    /// Its position among the rows shown ([`BranchesPane::visible`]).
    pub pos: usize,
    /// It's selected.
    pub picked: bool,
    pub spans: Vec<TextSpan<'static>>,
}

impl BranchesPane {
    /// The title for `room` columns: every tab's count when they fit, else
    /// only the current tab's.
    pub fn title(&self, room: usize) -> BranchesTitle {
        let title = title_with(self, true);
        if title.width() > room {
            title_with(self, false)
        } else {
            title
        }
    }

    /// What the bottom border says when it's `width` columns wide inside
    /// its corners.
    pub fn footer(&self, width: usize) -> BranchesFooter {
        let mut room = width;
        let mut footer = BranchesFooter::default();
        if self.tab == BranchTab::Worktrees {
            let total = format!(" total {} ", self.worktrees_total());
            room = room.saturating_sub(total.width() + 1);
            footer.total = Some(TextSpan::new(total, StyleRole::Muted));
        }
        if let Some(summary) = self.conflict_summary()
            && room > 4
        {
            let text = format!(" {} ", truncate(&summary, room - 2));
            footer.conflicts = Some(TextSpan::new(text, StyleRole::Error));
        }
        footer
    }

    /// The rows a viewport of `rows` lines and `width` columns shows, with
    /// relative dates as of `now`. Call
    /// [`set_viewport`](BranchesPane::set_viewport) first.
    pub fn view(&self, rows: usize, width: usize, now: i64) -> BranchesView {
        if self.visible.is_empty() {
            return BranchesView {
                empty: Some(self.empty_message()),
                range: 0..0,
                rows: Vec::new(),
            };
        }
        let cursor = *self.cursor();
        let start = cursor.offset.min(self.visible.len());
        let end = (start + rows).min(self.visible.len());
        let list = self.list(self.tab);
        let conflict_width = self.conflict_width();
        let name_width = self.compared_name_width(
            &self.visible[start..end],
            width.saturating_sub(if conflict_width > 0 { conflict_width + 1 } else { 0 }),
        );
        // Only the tags tab draws distances (and it can list hundreds).
        let distance_width = if self.tab == BranchTab::Tags {
            tag_distance_width(self)
        } else {
            0
        };
        let (state_width, size_width) = if self.tab == BranchTab::Worktrees {
            let widest =
                |text: &dyn Fn(&Worktree) -> String| self.worktrees.iter().map(|w| text(w).width()).max().unwrap_or(0);
            (widest(&|w| self.worktree_state(w)), widest(&|w| self.worktree_size(w)))
        } else {
            (0, 0)
        };
        let columns = if self.tab == BranchTab::All {
            self.decoration_columns()
        } else {
            Vec::new()
        };
        let rows = (start..end)
            .map(|pos| {
                let i = self.visible[pos];
                let spans = match self.tab {
                    BranchTab::All => entry_spans(self, &list[i], &columns, conflict_width, width, now),
                    BranchTab::Tags => tag_spans(self, &self.tags[i], name_width, distance_width, width, now),
                    BranchTab::Stash => stash_spans(&self.stashes[i], width, now),
                    BranchTab::Worktrees => worktree_spans(self, i, name_width, state_width, size_width, width),
                    BranchTab::Unmerged | BranchTab::Merged => {
                        compared_spans(self, &list[i], name_width, conflict_width, width, now)
                    }
                };
                BranchRow {
                    pos,
                    picked: pos == cursor.selected,
                    spans,
                }
            })
            .collect();
        BranchesView {
            empty: None,
            range: start..end,
            rows,
        }
    }
}

/// `Branches 12 · Tags 3 · Unmerged 4 · Merged 9` with the current tab
/// underlined, then the base everywhere but the full list, stashes and
/// worktrees, then the filter. Without room for every tab's count
/// (`counts` false), only the current tab's is shown.
fn title_with(pane: &BranchesPane, counts: bool) -> BranchesTitle {
    let mut spans = Vec::new();
    let mut tabs = Vec::new();
    let mut x = 0;
    for tab in BranchTab::ALL.into_iter().filter(|t| pane.shows_tab(*t)) {
        if !spans.is_empty() {
            spans.push(TextSpan::new(" · ", StyleRole::Muted));
            x += 3;
        }
        let count = if (tab.compares() && pane.base.is_none()) || (!counts && tab != pane.tab) {
            String::new()
        } else {
            format!(" {}", pane.len(tab))
        };
        let role = if tab == pane.tab {
            StyleRole::Underlined
        } else {
            StyleRole::Muted
        };
        let text = format!("{}{count}", tab.label());
        let width = text.width();
        tabs.push((tab, x..x + width));
        x += width;
        spans.push(TextSpan::new(text, role));
    }
    if !matches!(pane.tab, BranchTab::All | BranchTab::Stash | BranchTab::Worktrees)
        && let Some(base) = &pane.base
    {
        spans.push(TextSpan::new(format!(" ({})", base.base), StyleRole::Muted));
    }
    if pane.editing_filter || !pane.filter.is_empty() {
        let cursor = if pane.editing_filter { "█" } else { "" };
        spans.push(TextSpan::plain(format!(" /{}{cursor}", pane.filter)));
    }
    BranchesTitle { spans, tabs }
}

/// A stash row: its name, message and age.
fn stash_spans(stash: &Stash, width: usize, now: i64) -> Vec<TextSpan<'static>> {
    let name = stash.name();
    let message_width = width.saturating_sub(2 + name.width() + 1 + 1 + DATE_WIDTH);
    vec![
        TextSpan::plain("  "),
        TextSpan::new(name, StyleRole::Sha),
        TextSpan::plain(" "),
        TextSpan::plain(fit(&stash.message, message_width)),
        TextSpan::plain(" "),
        TextSpan::new(fit_right(&relative(now, stash.date), DATE_WIDTH), StyleRole::Date),
    ]
}

/// A worktree row: its path, what it has checked out, its state and size.
fn worktree_spans(
    pane: &BranchesPane,
    i: usize,
    name_width: usize,
    state_width: usize,
    size_width: usize,
    width: usize,
) -> Vec<TextSpan<'static>> {
    let wt = &pane.worktrees[i];
    let (marker, name_role) = if wt.current {
        (TextSpan::new("* ", StyleRole::LabelLocal), StyleRole::LabelLocal)
    } else {
        (TextSpan::plain("  "), StyleRole::Plain)
    };
    let checked_out = match (&wt.branch, &wt.head) {
        _ if wt.bare => "(bare)".to_string(),
        (Some(branch), _) => branch.clone(),
        (None, Some(oid)) => format!("(detached {})", &oid[..oid.len().min(7)]),
        (None, None) => "(no commit)".to_string(),
    };
    let branch_role = if wt.branch.is_some() {
        StyleRole::LabelLocal
    } else {
        StyleRole::Sha
    };
    let state = pane.worktree_state(wt);
    let state_role = if wt.prunable.is_some() || state.contains('✗') {
        StyleRole::Error
    } else if wt.locked.is_some() || state.ends_with("changed") || state.contains('↑') {
        StyleRole::LabelTag
    } else {
        StyleRole::Muted
    };
    let branch_width = width.saturating_sub(2 + name_width + 1 + 1 + state_width + 1 + size_width);
    vec![
        marker,
        TextSpan::new(fit(&pane.worktree_names[i], name_width), name_role),
        TextSpan::plain(" "),
        TextSpan::new(fit(&checked_out, branch_width), branch_role),
        TextSpan::plain(" "),
        TextSpan::new(fit_right(&state, state_width), state_role),
        TextSpan::plain(" "),
        TextSpan::new(fit_right(&pane.worktree_size(wt), size_width), StyleRole::Muted),
    ]
}

/// A row of the full list: name, conflict mark, a column per plugin's
/// decorations, its local and remote state, and its last activity.
fn entry_spans(
    pane: &BranchesPane,
    entry: &BranchEntry,
    columns: &[(&str, usize)],
    conflict_width: usize,
    width: usize,
    now: i64,
) -> Vec<TextSpan<'static>> {
    let (state, state_role) = state_text(&entry.state());
    let decorations: usize = columns.iter().map(|(_, w)| w + 1).sum();
    let conflicts = if conflict_width > 0 { conflict_width + 1 } else { 0 };
    let name_width = width.saturating_sub(2 + 1 + STATE_WIDTH + 1 + DATE_WIDTH + decorations + conflicts);
    let mut spans = vec![
        marker(entry),
        TextSpan::new(fit(&entry.name, name_width), name_role(entry)),
    ];
    if conflict_width > 0 {
        spans.push(TextSpan::plain(" "));
        spans.push(TextSpan::new(
            fit(&pane.conflict_mark(entry.target()), conflict_width),
            StyleRole::Error,
        ));
    }
    for (plugin, w) in columns {
        let text = pane.decorations[*plugin].get(&entry.name).map_or("", String::as_str);
        spans.push(TextSpan::plain(" "));
        spans.push(TextSpan::new(fit(text, *w), StyleRole::Sha));
    }
    spans.extend([
        TextSpan::plain(" "),
        TextSpan::new(fit(&state, STATE_WIDTH), state_role),
        TextSpan::plain(" "),
        TextSpan::new(fit_right(&relative(now, entry.activity), DATE_WIDTH), StyleRole::Date),
    ]);
    spans
}

/// A merged/unmerged row: name, tip subject, author and commit date,
/// like a `git branch -r --no-merged` alias prints them.
fn compared_spans(
    pane: &BranchesPane,
    entry: &BranchEntry,
    name_width: usize,
    conflict_width: usize,
    width: usize,
    now: i64,
) -> Vec<TextSpan<'static>> {
    let tip = BaseBranches::tip(entry);
    let author = tip
        .and_then(|t| pane.base.as_ref()?.authors.get(&t.full_name))
        .map_or("", String::as_str);
    let mut spans = vec![
        marker(entry),
        TextSpan::new(fit(&entry.name, name_width), name_role(entry)),
    ];
    let mut used = 2 + name_width;
    if conflict_width > 0 {
        spans.push(TextSpan::plain(" "));
        spans.push(TextSpan::new(
            fit(&pane.conflict_mark(tip.map(|t| t.target.as_str())), conflict_width),
            StyleRole::Error,
        ));
        used += 1 + conflict_width;
    }
    let rest = width.saturating_sub(used + 1 + DATE_WIDTH);
    let mut author_width = AUTHOR_WIDTH.min(rest.saturating_sub(1));
    let subject_width = rest.saturating_sub(author_width + 2);
    if subject_width < SUBJECT_MIN_WIDTH {
        author_width = rest.saturating_sub(1);
    } else {
        spans.push(TextSpan::plain(" "));
        spans.push(TextSpan::plain(fit(
            tip.map_or("", |t| t.subject.as_str()),
            subject_width,
        )));
    }
    if author_width > 0 {
        spans.push(TextSpan::plain(" "));
        spans.push(TextSpan::new(fit(author, author_width), StyleRole::Muted));
    }
    spans.push(TextSpan::plain(" "));
    spans.push(TextSpan::new(
        fit_right(&relative(now, entry.activity), DATE_WIDTH),
        StyleRole::Date,
    ));
    spans
}

/// How far a tag is from the base: `↓12` when the base has moved on 12
/// commits, `↑3` for commits the base doesn't have, `✓` at its tip.
/// Only the newest tags are compared.
fn tag_distance(pane: &BranchesPane, tag: &GitRef) -> Option<(String, StyleRole)> {
    let d = pane.base.as_ref()?.tags.get(&tag.full_name)?;
    let state = if d.ahead == 0 && d.behind == 0 {
        BranchState::InSync
    } else {
        BranchState::Diverged {
            ahead: d.ahead,
            behind: d.behind,
        }
    };
    Some(state_text(&state))
}

/// Width of the tag distance column: the widest of the compared tags.
fn tag_distance_width(pane: &BranchesPane) -> usize {
    pane.tags
        .iter()
        .filter_map(|t| tag_distance(pane, t))
        .map(|(text, _)| text.width())
        .max()
        .unwrap_or(0)
}

/// A tag row: name, distance from the base, subject (the tag message's
/// for annotated tags, else the commit's) and date.
fn tag_spans(
    pane: &BranchesPane,
    tag: &GitRef,
    name_width: usize,
    distance_width: usize,
    width: usize,
    now: i64,
) -> Vec<TextSpan<'static>> {
    let mut spans = vec![
        TextSpan::plain("  "),
        TextSpan::new(fit(&tag.short_name, name_width), StyleRole::LabelTag),
    ];
    let mut used = 2 + name_width;
    if distance_width > 0 {
        let (text, role) = tag_distance(pane, tag).unwrap_or((String::new(), StyleRole::Plain));
        spans.push(TextSpan::plain(" "));
        spans.push(TextSpan::new(fit(&text, distance_width), role));
        used += 1 + distance_width;
    }
    let subject_width = width.saturating_sub(used + 1 + 1 + DATE_WIDTH);
    if subject_width > 0 {
        spans.push(TextSpan::plain(" "));
        spans.push(TextSpan::plain(fit(&tag.subject, subject_width)));
    }
    spans.push(TextSpan::plain(" "));
    spans.push(TextSpan::new(
        fit_right(&relative(now, tag.date), DATE_WIDTH),
        StyleRole::Date,
    ));
    spans
}

fn name_role(entry: &BranchEntry) -> StyleRole {
    if entry.is_head {
        StyleRole::LabelLocal
    } else if entry.local.is_none() {
        StyleRole::LabelRemote
    } else {
        StyleRole::Plain
    }
}

fn marker(entry: &BranchEntry) -> TextSpan<'static> {
    if entry.is_head {
        TextSpan::new("* ", StyleRole::LabelLocal)
    } else {
        TextSpan::plain("  ")
    }
}

/// Short state badge and its role.
fn state_text(state: &BranchState) -> (String, StyleRole) {
    match state {
        BranchState::LocalOnly => ("local".into(), StyleRole::Muted),
        BranchState::RemoteOnly { remotes } => (remotes.join(","), StyleRole::LabelRemote),
        BranchState::InSync => ("✓".into(), StyleRole::DiffAdd),
        BranchState::Diverged { ahead, behind } => {
            let mut s = String::new();
            if *ahead > 0 {
                s.push_str(&format!("↑{ahead}"));
            }
            if *behind > 0 {
                s.push_str(&format!("↓{behind}"));
            }
            (s, StyleRole::LabelTag)
        }
        BranchState::Gone => ("gone".into(), StyleRole::Error),
        BranchState::Untracked { remotes, same_commit } => {
            let sign = if *same_commit { "=" } else { "≠" };
            (format!("{sign}{}", remotes.join(",")), StyleRole::Muted)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    use tt_git_core::time::FixedClock;
    use tt_git_core::{AheadBehind, RefKind};

    use super::*;
    use crate::component::Component;
    use crate::jobs::Ctx;
    use crate::keymap::{Command, KeyCode, KeyCombo, KeyModifiers};
    use crate::mouse::PanePart;

    const NOW: i64 = 100 * 86_400;

    fn ctx() -> Ctx {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        Ctx::new(tx)
    }

    fn git_ref(full: &str, kind: RefKind, date: i64) -> GitRef {
        GitRef {
            full_name: full.into(),
            short_name: full.splitn(3, '/').nth(2).unwrap_or(full).into(),
            kind,
            target: format!("{:0>40}", full.len()),
            date,
            upstream: None,
            ahead: 0,
            behind: 0,
            upstream_gone: false,
            is_head: false,
            symref: None,
            subject: format!("subject of {full}"),
        }
    }

    fn local(name: &str, days_ago: i64) -> BranchEntry {
        BranchEntry {
            name: name.into(),
            local: Some(git_ref(
                &format!("refs/heads/{name}"),
                RefKind::LocalBranch,
                NOW - days_ago * 86_400,
            )),
            remotes: Vec::new(),
            is_head: false,
            activity: NOW - days_ago * 86_400,
        }
    }

    fn remote(name: &str, days_ago: i64) -> BranchEntry {
        BranchEntry {
            name: name.into(),
            local: None,
            remotes: vec![git_ref(
                &format!("refs/remotes/origin/{name}"),
                RefKind::RemoteBranch {
                    remote: "origin".into(),
                },
                NOW - days_ago * 86_400,
            )],
            is_head: false,
            activity: NOW - days_ago * 86_400,
        }
    }

    fn worktree(path: &str, branch: Option<&str>, main: bool) -> Worktree {
        Worktree {
            path: PathBuf::from(path),
            head: Some("a".repeat(40)),
            branch: branch.map(str::to_string),
            main,
            bare: false,
            locked: None,
            prunable: None,
            current: main,
        }
    }

    fn base(name: &str, unmerged: Vec<BranchEntry>) -> BaseBranches {
        BaseBranches {
            base: name.into(),
            base_ref: format!("refs/remotes/{name}"),
            base_target: "b".repeat(40),
            chosen: false,
            detected: None,
            unmerged,
            merged: Vec::new(),
            authors: HashMap::new(),
            tags: HashMap::new(),
        }
    }

    /// A pane with `main` checked out, a stale local branch and a remote one.
    fn pane() -> BranchesPane {
        let mut pane = BranchesPane::new(Arc::new(FixedClock(NOW)));
        let mut main = local("main", 0);
        main.is_head = true;
        pane.set_entries(
            vec![main, local("old", 3), remote("feature", 1)],
            None,
            &[],
            Vec::new(),
            Vec::new(),
        );
        pane
    }

    fn text(spans: &[TextSpan]) -> String {
        spans.iter().map(|s| s.text.as_ref()).collect()
    }

    fn role_of(spans: &[TextSpan], text: &str) -> StyleRole {
        spans
            .iter()
            .find(|s| s.text.trim().starts_with(text))
            .unwrap_or_else(|| panic!("{text:?} in {spans:?}"))
            .role
    }

    fn rows(pane: &mut BranchesPane, height: usize, width: usize) -> BranchesView {
        pane.set_viewport(height);
        pane.view(height, width, NOW)
    }

    #[test]
    fn title_lists_the_tabs_and_where_each_is() {
        let pane = pane();
        let title = pane.title(200);
        assert_eq!(text(&title.spans), "Branches 3 · Tags 0 · Unmerged · Merged");
        assert_eq!(title.spans[0], TextSpan::new("Branches 3", StyleRole::Underlined));
        assert_eq!(title.spans[2], TextSpan::new("Tags 0", StyleRole::Muted));
        assert_eq!(title.tabs[0], (BranchTab::All, 0..10));
        assert_eq!(title.tabs[1], (BranchTab::Tags, 13..19));
        assert_eq!(title.tab_at(14), Some(BranchTab::Tags));
        assert_eq!(title.tab_at(11), None, "the separator isn't a tab");
        // Too narrow for every count: only the current tab's.
        let narrow = pane.title(20);
        assert_eq!(text(&narrow.spans), "Branches 3 · Tags · Unmerged · Merged");
        assert_eq!(narrow.tab_at(13), Some(BranchTab::Tags));
    }

    #[test]
    fn title_shows_the_base_and_the_filter() {
        let mut pane = pane();
        let ctx = ctx();
        let entries = pane.entries().to_vec();
        pane.set_entries(
            entries,
            Some(base("origin/main", vec![remote("feature", 1)])),
            &[],
            Vec::new(),
            Vec::new(),
        );
        assert!(
            !text(&pane.title(200).spans).contains("origin/main"),
            "not on the full list"
        );
        pane.on_click_part(PanePart::Tab(BranchTab::Unmerged), &ctx);
        let title = text(&pane.title(200).spans);
        assert!(
            title.contains("Unmerged 1") && title.ends_with(" (origin/main)"),
            "{title}"
        );

        pane.on_command(Command::Filter, &ctx);
        pane.filter.push_str("fe");
        assert!(text(&pane.title(200).spans).ends_with(" /fe█"));
        pane.editing_filter = false;
        assert!(text(&pane.title(200).spans).ends_with(" /fe"));
    }

    #[test]
    fn ctrl_w_in_the_filter_deletes_the_word_before_it() {
        let mut pane = pane();
        let ctx = ctx();
        pane.on_command(Command::Filter, &ctx);
        for c in "main fea".chars() {
            pane.on_text_key(KeyCombo::char(c), &ctx);
        }
        let ctrl_w = KeyCombo::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
        pane.on_text_key(ctrl_w, &ctx);
        assert_eq!(pane.filter(), "main ");
        pane.on_text_key(ctrl_w, &ctx);
        assert_eq!(pane.filter(), "");
        assert_eq!(rows(&mut pane, 10, 40).range, 0..3, "the full list again");
        assert!(pane.editing_filter, "still typing");
    }

    #[test]
    fn branch_rows_mark_the_head_and_the_state() {
        let mut pane = pane();
        let view = rows(&mut pane, 10, 40);
        assert_eq!(view.empty, None);
        assert_eq!(view.range, 0..3);
        assert_eq!(
            view.rows.iter().map(|r| r.picked).collect::<Vec<_>>(),
            [true, false, false]
        );
        for row in &view.rows {
            assert_eq!(text(&row.spans).width(), 40, "{:?}", text(&row.spans));
        }
        let head = &view.rows[0].spans;
        assert_eq!(head[0], TextSpan::new("* ", StyleRole::LabelLocal));
        assert_eq!(role_of(head, "main"), StyleRole::LabelLocal);
        assert_eq!(role_of(head, "local"), StyleRole::Muted);
        assert_eq!(role_of(head, "now"), StyleRole::Date);
        let old = &view.rows[1].spans;
        assert_eq!(role_of(old, "old"), StyleRole::Plain);
        assert!(text(old).ends_with(&fit_right(&relative(NOW, NOW - 3 * 86_400), DATE_WIDTH)));
        let feature = &view.rows[2].spans;
        assert_eq!(role_of(feature, "feature"), StyleRole::LabelRemote);
        assert_eq!(role_of(feature, "origin"), StyleRole::LabelRemote);
    }

    #[test]
    fn state_badges_say_how_a_branch_tracks_its_remote() {
        let remotes = vec!["origin".to_string()];
        assert_eq!(state_text(&BranchState::InSync), ("✓".into(), StyleRole::DiffAdd));
        assert_eq!(
            state_text(&BranchState::Diverged { ahead: 2, behind: 1 }),
            ("↑2↓1".into(), StyleRole::LabelTag)
        );
        assert_eq!(state_text(&BranchState::Gone), ("gone".into(), StyleRole::Error));
        assert_eq!(
            state_text(&BranchState::Untracked {
                remotes,
                same_commit: false
            }),
            ("≠origin".into(), StyleRole::Muted)
        );
    }

    #[test]
    fn plugin_decorations_get_a_column_each() {
        let mut pane = pane();
        pane.set_decorations(
            "ci",
            HashMap::from([("old".to_string(), "passing-and-more".to_string())]),
        );
        let view = rows(&mut pane, 10, 60);
        let old = &view.rows[1].spans;
        // Cut to the widest a decoration column gets.
        assert_eq!(role_of(old, "passing-a"), StyleRole::Sha);
        assert!(text(old).contains(&fit("passing-and-more", 10)));
        assert!(text(&view.rows[0].spans).width() == 60);
    }

    #[test]
    fn conflicts_are_marked_and_summed_up_below() {
        let mut pane = pane();
        let entries = pane.entries().to_vec();
        let tip = entries[1].target().unwrap().to_string();
        pane.set_entries(
            entries,
            Some(base("origin/main", Vec::new())),
            &[],
            Vec::new(),
            Vec::new(),
        );
        pane.set_merge_conflicts(
            &"b".repeat(40),
            vec![tip.clone()],
            Ok(Some(HashMap::from([(
                tip,
                vec!["a.txt".to_string(), "b.txt".to_string()],
            )]))),
        );
        let view = rows(&mut pane, 10, 60);
        assert_eq!(role_of(&view.rows[1].spans, "✗2"), StyleRole::Error);
        assert!(pane.footer(60).conflicts.is_none(), "the selected row merges cleanly");

        pane.on_command(Command::MoveDown, &ctx());
        let footer = pane.footer(60);
        assert_eq!(
            footer.conflicts,
            Some(TextSpan::new(
                " ✗ 2 conflicts with origin/main: a.txt, b.txt ",
                StyleRole::Error
            ))
        );
        assert_eq!(footer.total, None);
        let narrow = pane.footer(20).conflicts.unwrap();
        assert_eq!(narrow.text.width(), 20, "cut to fit: {narrow:?}");
        assert_eq!(pane.footer(4).conflicts, None);
    }

    #[test]
    fn tag_rows_show_how_far_they_are_from_the_base() {
        let mut pane = pane();
        let entries = pane.entries().to_vec();
        let tags = vec![
            git_ref("refs/tags/v2", RefKind::Tag, NOW - 86_400),
            git_ref("refs/tags/v1", RefKind::Tag, NOW - 2 * 86_400),
        ];
        let mut base = base("origin/main", Vec::new());
        base.tags
            .insert("refs/tags/v2".into(), AheadBehind { ahead: 0, behind: 0 });
        base.tags
            .insert("refs/tags/v1".into(), AheadBehind { ahead: 0, behind: 12 });
        pane.set_entries(entries, Some(base), &tags, Vec::new(), Vec::new());
        pane.on_click_part(PanePart::Tab(BranchTab::Tags), &ctx());
        let view = rows(&mut pane, 10, 60);
        let v2 = &view.rows[0].spans;
        assert_eq!(role_of(v2, "v2"), StyleRole::LabelTag);
        assert_eq!(role_of(v2, "✓"), StyleRole::DiffAdd);
        assert_eq!(role_of(&view.rows[1].spans, "↓12"), StyleRole::LabelTag);
        assert!(text(v2).contains("subject of refs/tags/v2"));
        assert_eq!(text(v2).width(), 60);
    }

    #[test]
    fn stash_and_worktree_rows() {
        let mut pane = pane();
        let entries = pane.entries().to_vec();
        let stash = Stash {
            index: 0,
            oid: "c".repeat(40),
            untracked: None,
            date: NOW - 60,
            message: "On main: wip".into(),
        };
        let worktrees = vec![worktree("/repo", Some("main"), true), worktree("/repo-b", None, false)];
        pane.set_entries(entries, None, &[], vec![stash], worktrees);
        let ctx = ctx();
        pane.on_click_part(PanePart::Tab(BranchTab::Stash), &ctx);
        let view = rows(&mut pane, 10, 40);
        let stash = &view.rows[0].spans;
        assert_eq!(role_of(stash, "stash@{0}"), StyleRole::Sha);
        assert!(text(stash).contains("On main: wip"));
        assert_eq!(text(stash).width(), 40);

        pane.on_click_part(PanePart::Tab(BranchTab::Worktrees), &ctx);
        pane.worktree_names = vec![".".into(), "../repo-b".into()];
        pane.set_worktree_changes(vec![(PathBuf::from("/repo"), Ok(2), None)]);
        let view = rows(&mut pane, 10, 60);
        let main = &view.rows[0].spans;
        assert_eq!(main[0], TextSpan::new("* ", StyleRole::LabelLocal));
        assert_eq!(role_of(main, "main"), StyleRole::LabelLocal);
        assert_eq!(role_of(main, "main · 2 changed"), StyleRole::LabelTag);
        let other = &view.rows[1].spans;
        assert_eq!(role_of(other, "../repo-b"), StyleRole::Plain);
        assert_eq!(role_of(other, "(detached aaaaaaa)"), StyleRole::Sha);
        assert_eq!(
            pane.footer(60).total,
            Some(TextSpan::new(" total … ", StyleRole::Muted))
        );
    }

    #[test]
    fn the_view_scrolls_with_the_selection() {
        let mut pane = BranchesPane::new(Arc::new(FixedClock(NOW)));
        let entries = (0..6).map(|i| local(&format!("b{i}"), i)).collect();
        pane.set_entries(entries, None, &[], Vec::new(), Vec::new());
        let ctx = ctx();
        for _ in 0..4 {
            pane.on_command(Command::MoveDown, &ctx);
        }
        let view = rows(&mut pane, 3, 40);
        assert_eq!(view.range, 2..5);
        assert_eq!(view.rows.iter().map(|r| r.pos).collect::<Vec<_>>(), [2, 3, 4]);
        assert_eq!(
            view.rows.iter().map(|r| r.picked).collect::<Vec<_>>(),
            [false, false, true]
        );
    }

    #[test]
    fn empty_tabs_say_why() {
        let mut pane = BranchesPane::new(Arc::new(FixedClock(NOW)));
        let view = rows(&mut pane, 5, 40);
        assert_eq!(view.empty.as_deref(), Some("no branches"));
        assert!(view.rows.is_empty());
        pane.on_click_part(PanePart::Tab(BranchTab::Unmerged), &ctx());
        assert_eq!(
            rows(&mut pane, 5, 40).empty.as_deref(),
            Some("no base branch found; pick one")
        );
        let mut pane = self::pane();
        pane.filter = "zzz".into();
        pane.on_click_part(PanePart::Tab(BranchTab::Tags), &ctx());
        pane.on_click_part(PanePart::Tab(BranchTab::All), &ctx());
        assert_eq!(rows(&mut pane, 5, 40).empty.as_deref(), Some("no matches"));
    }
}
