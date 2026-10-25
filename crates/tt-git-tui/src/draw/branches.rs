//! The branches pane: branches, tags, stashes and worktrees, a tab each.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tt_git_core::time::relative;
use tt_git_core::{BaseBranches, BranchEntry, BranchState, GitRef, Stash, Worktree};
use unicode_width::UnicodeWidthStr;

use super::{Draw, Hits, pane_block, title_x};
use crate::panes::branches::{AUTHOR_WIDTH, DATE_WIDTH, STATE_WIDTH, SUBJECT_MIN_WIDTH};
use crate::panes::{BranchTab, BranchesPane, PaneId};
use crate::tui_theme::Theme;
use crate::widgets::{fit, fit_right, truncate};

impl Draw for BranchesPane {
    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme, hits: &mut Hits) {
        // Past the right corner the title is cut off.
        let start = title_x(PaneId::Branches, area);
        let end = area.right().saturating_sub(1);
        let (mut title, mut cells) = title_line(self, theme, true);
        if start as usize + title.width() > end as usize {
            (title, cells) = title_line(self, theme, false);
        }
        hits.tabs = cells
            .into_iter()
            .map(|(tab, xs)| (tab, area.y, (start + xs.start).min(end)..(start + xs.end).min(end)))
            .filter(|(_, _, xs)| !xs.is_empty())
            .collect();
        let mut block = pane_block(PaneId::Branches, title, focused, theme);
        let mut room = area.width.saturating_sub(2) as usize;
        if self.tab == BranchTab::Worktrees {
            let total = format!(" total {} ", self.worktrees_total());
            room = room.saturating_sub(total.width() + 1);
            block = block.title_bottom(Line::from(Span::styled(total, theme.muted)).right_aligned());
        }
        if let Some(summary) = self.conflict_summary()
            && room > 4
        {
            let text = format!(" {} ", truncate(&summary, room - 2));
            block = block.title_bottom(Line::from(Span::styled(text, theme.error)));
        }
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let now = self.clock.now();
        if self.visible.is_empty() {
            self.note_drawn(now, 0..0);
            frame.render_widget(Paragraph::new(self.empty_message()).style(theme.muted), inner);
            return;
        }
        let height = inner.height as usize;
        self.set_viewport(height);
        let cursor = *self.cursor();
        let start = cursor.offset.min(self.visible.len());
        let end = (start + height).min(self.visible.len());
        let width = inner.width as usize;
        let list = self.list(self.tab);
        let conflict_width = self.conflict_width();
        let name_width = self.compared_name_width(
            &self.visible[start..end],
            width.saturating_sub(if conflict_width > 0 { conflict_width + 1 } else { 0 }),
        );
        // Only the tags tab draws distances (and it can list hundreds).
        let distance_width = if matches!(self.tab, BranchTab::Tags) {
            tag_distance_width(self, theme)
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
        let lines: Vec<Line> = (start..end)
            .map(|pos| {
                let i = self.visible[pos];
                let line = match self.tab {
                    BranchTab::All => render_entry(self, &list[i], &columns, conflict_width, width, theme),
                    BranchTab::Tags => render_tag(self, &self.tags[i], name_width, distance_width, width, theme),
                    BranchTab::Stash => render_stash(self, &self.stashes[i], width, theme),
                    BranchTab::Worktrees => render_worktree(self, i, name_width, state_width, size_width, width, theme),
                    _ => render_compared(self, &list[i], name_width, conflict_width, width, theme),
                };
                if pos == cursor.selected {
                    line.patch_style(if focused {
                        theme.selected
                    } else {
                        theme.selected_unfocused
                    })
                } else {
                    line
                }
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
        self.note_drawn(now, start..end);
    }
}

/// A stash row: its name, message and age.
fn render_stash<'a>(pane: &BranchesPane, stash: &'a Stash, width: usize, theme: &Theme) -> Line<'a> {
    let name = stash.name();
    let message_width = width.saturating_sub(2 + name.width() + 1 + 1 + DATE_WIDTH);
    Line::from(vec![
        Span::raw("  "),
        Span::styled(name, theme.sha),
        Span::raw(" "),
        Span::raw(fit(&stash.message, message_width)),
        Span::raw(" "),
        Span::styled(
            fit_right(&relative(pane.clock.now(), stash.date), DATE_WIDTH),
            theme.date,
        ),
    ])
}

/// A worktree row: its path, what it has checked out and its state.
fn render_worktree<'a>(
    pane: &BranchesPane,
    i: usize,
    name_width: usize,
    state_width: usize,
    size_width: usize,
    width: usize,
    theme: &Theme,
) -> Line<'a> {
    let wt = &pane.worktrees[i];
    let (marker, name_style) = if wt.current {
        (Span::styled("* ", theme.label_local), theme.label_local)
    } else {
        (Span::raw("  "), Style::new())
    };
    let checked_out = match (&wt.branch, &wt.head) {
        _ if wt.bare => "(bare)".to_string(),
        (Some(branch), _) => branch.clone(),
        (None, Some(oid)) => format!("(detached {})", &oid[..oid.len().min(7)]),
        (None, None) => "(no commit)".to_string(),
    };
    let branch_style = if wt.branch.is_some() {
        theme.label_local
    } else {
        theme.sha
    };
    let state = pane.worktree_state(wt);
    let state_style = if wt.prunable.is_some() || state.contains('✗') {
        theme.error
    } else if wt.locked.is_some() || state.ends_with("changed") || state.contains('↑') {
        theme.label_tag
    } else {
        theme.muted
    };
    let branch_width = width.saturating_sub(2 + name_width + 1 + 1 + state_width + 1 + size_width);
    Line::from(vec![
        marker,
        Span::styled(fit(&pane.worktree_names[i], name_width), name_style),
        Span::raw(" "),
        Span::styled(fit(&checked_out, branch_width), branch_style),
        Span::raw(" "),
        Span::styled(fit_right(&state, state_width), state_style),
        Span::raw(" "),
        Span::styled(fit_right(&pane.worktree_size(wt), size_width), theme.muted),
    ])
}

fn render_entry<'a>(
    pane: &BranchesPane,
    entry: &'a BranchEntry,
    columns: &[(&str, usize)],
    conflict_width: usize,
    width: usize,
    theme: &Theme,
) -> Line<'a> {
    let (state, state_style) = state_text(&entry.state(), theme);
    let decorations: usize = columns.iter().map(|(_, w)| w + 1).sum();
    let conflicts = if conflict_width > 0 { conflict_width + 1 } else { 0 };
    let name_width = width.saturating_sub(2 + 1 + STATE_WIDTH + 1 + DATE_WIDTH + decorations + conflicts);
    let mut spans = vec![
        marker(entry, theme),
        Span::styled(fit(&entry.name, name_width), name_style(entry, theme)),
    ];
    if conflict_width > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            fit(&pane.conflict_mark(entry.target()), conflict_width),
            theme.error,
        ));
    }
    for (plugin, w) in columns {
        let text = pane.decorations[*plugin].get(&entry.name).map_or("", String::as_str);
        spans.push(Span::raw(" "));
        spans.push(Span::styled(fit(text, *w), theme.sha));
    }
    spans.extend([
        Span::raw(" "),
        Span::styled(fit(&state, STATE_WIDTH), state_style),
        Span::raw(" "),
        Span::styled(
            fit_right(&relative(pane.clock.now(), entry.activity), DATE_WIDTH),
            theme.date,
        ),
    ]);
    Line::from(spans)
}

/// A merged/unmerged row: name, tip subject, author and commit date,
/// like a `git branch -r --no-merged` alias prints them.
fn render_compared<'a>(
    pane: &'a BranchesPane,
    entry: &'a BranchEntry,
    name_width: usize,
    conflict_width: usize,
    width: usize,
    theme: &Theme,
) -> Line<'a> {
    let tip = BaseBranches::tip(entry);
    let author = tip
        .and_then(|t| pane.base.as_ref()?.authors.get(&t.full_name))
        .map_or("", String::as_str);
    let mut spans = vec![
        marker(entry, theme),
        Span::styled(fit(&entry.name, name_width), name_style(entry, theme)),
    ];
    let mut used = 2 + name_width;
    if conflict_width > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            fit(&pane.conflict_mark(tip.map(|t| t.target.as_str())), conflict_width),
            theme.error,
        ));
        used += 1 + conflict_width;
    }
    let rest = width.saturating_sub(used + 1 + DATE_WIDTH);
    let mut author_width = AUTHOR_WIDTH.min(rest.saturating_sub(1));
    let subject_width = rest.saturating_sub(author_width + 2);
    if subject_width < SUBJECT_MIN_WIDTH {
        author_width = rest.saturating_sub(1);
    } else {
        spans.push(Span::raw(" "));
        spans.push(Span::raw(fit(tip.map_or("", |t| t.subject.as_str()), subject_width)));
    }
    if author_width > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(fit(author, author_width), theme.muted));
    }
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        fit_right(&relative(pane.clock.now(), entry.activity), DATE_WIDTH),
        theme.date,
    ));
    Line::from(spans)
}

/// How far a tag is from the base: `↓12` when the base has moved on 12
/// commits, `↑3` for commits the base doesn't have, `✓` at its tip.
/// Only the newest tags are compared.
fn tag_distance(pane: &BranchesPane, tag: &GitRef, theme: &Theme) -> Option<(String, Style)> {
    let d = pane.base.as_ref()?.tags.get(&tag.full_name)?;
    let state = if d.ahead == 0 && d.behind == 0 {
        BranchState::InSync
    } else {
        BranchState::Diverged {
            ahead: d.ahead,
            behind: d.behind,
        }
    };
    Some(state_text(&state, theme))
}

/// Width of the tag distance column: the widest of the compared tags.
fn tag_distance_width(pane: &BranchesPane, theme: &Theme) -> usize {
    pane.tags
        .iter()
        .filter_map(|t| tag_distance(pane, t, theme))
        .map(|(text, _)| text.width())
        .max()
        .unwrap_or(0)
}

/// A tag row: name, distance from the base, subject (the tag message's
/// for annotated tags, else the commit's) and date.
fn render_tag<'a>(
    pane: &BranchesPane,
    tag: &'a GitRef,
    name_width: usize,
    distance_width: usize,
    width: usize,
    theme: &Theme,
) -> Line<'a> {
    let mut spans = vec![
        Span::raw("  "),
        Span::styled(fit(&tag.short_name, name_width), theme.label_tag),
    ];
    let mut used = 2 + name_width;
    if distance_width > 0 {
        let (text, style) = tag_distance(pane, tag, theme).unwrap_or_default();
        spans.push(Span::raw(" "));
        spans.push(Span::styled(fit(&text, distance_width), style));
        used += 1 + distance_width;
    }
    let subject_width = width.saturating_sub(used + 1 + 1 + DATE_WIDTH);
    if subject_width > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::raw(fit(&tag.subject, subject_width)));
    }
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        fit_right(&relative(pane.clock.now(), tag.date), DATE_WIDTH),
        theme.date,
    ));
    Line::from(spans)
}

/// `Branches 12 · Tags 3 · Unmerged 4 · Merged 9` with the current tab
/// underlined, then the base everywhere but the full list.
/// The title, and the columns each tab's name takes in it.
/// Without room for every tab's count (`counts` false), only the
/// current tab's is shown.
fn title_line(
    pane: &BranchesPane,
    theme: &Theme,
    counts: bool,
) -> (Line<'static>, Vec<(BranchTab, std::ops::Range<u16>)>) {
    let mut spans = Vec::new();
    let mut cells = Vec::new();
    let mut x = 0;
    for tab in BranchTab::ALL.into_iter().filter(|t| pane.shows_tab(*t)) {
        if !spans.is_empty() {
            spans.push(Span::styled(" · ", theme.muted));
            x += 3;
        }
        let count = if (tab.compares() && pane.base.is_none()) || (!counts && tab != pane.tab) {
            String::new()
        } else {
            format!(" {}", pane.len(tab))
        };
        let style = if tab == pane.tab {
            Style::new().add_modifier(Modifier::UNDERLINED)
        } else {
            theme.muted
        };
        let text = format!("{}{count}", tab.label());
        let width = text.width() as u16;
        cells.push((tab, x..x + width));
        x += width;
        spans.push(Span::styled(text, style));
    }
    if !matches!(pane.tab, BranchTab::All | BranchTab::Stash | BranchTab::Worktrees)
        && let Some(base) = &pane.base
    {
        spans.push(Span::styled(format!(" ({})", base.base), theme.muted));
    }
    if pane.editing_filter || !pane.filter.is_empty() {
        let cursor = if pane.editing_filter { "█" } else { "" };
        spans.push(Span::raw(format!(" /{}{cursor}", pane.filter)));
    }
    (Line::from(spans), cells)
}

fn name_style(entry: &BranchEntry, theme: &Theme) -> Style {
    if entry.is_head {
        theme.label_local
    } else if entry.local.is_none() {
        theme.label_remote
    } else {
        Style::new()
    }
}

fn marker(entry: &BranchEntry, theme: &Theme) -> Span<'static> {
    if entry.is_head {
        Span::styled("* ", theme.label_local)
    } else {
        Span::raw("  ")
    }
}

/// Short state badge and its style.
fn state_text(state: &BranchState, theme: &Theme) -> (String, Style) {
    match state {
        BranchState::LocalOnly => ("local".into(), theme.muted),
        BranchState::RemoteOnly { remotes } => (remotes.join(","), theme.label_remote),
        BranchState::InSync => ("✓".into(), theme.diff_add),
        BranchState::Diverged { ahead, behind } => {
            let mut s = String::new();
            if *ahead > 0 {
                s.push_str(&format!("↑{ahead}"));
            }
            if *behind > 0 {
                s.push_str(&format!("↓{behind}"));
            }
            (s, theme.label_tag)
        }
        BranchState::Gone => ("gone".into(), theme.error),
        BranchState::Untracked { remotes, same_commit } => {
            let sign = if *same_commit { "=" } else { "≠" };
            (format!("{sign}{}", remotes.join(",")), theme.muted)
        }
    }
}
