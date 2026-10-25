//! Staging one file by hunk or by line: its unstaged and staged changes, one
//! side active, with a hunk or a range of changed lines selected.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tt_git_core::patch::{FileDiff, LineKind};
use tt_git_core::{PatchOp, StageSide, StatusEntry};

use crate::action::Action;
use crate::agent::AgentHunk;
use crate::component::EventState;
use crate::editor::EditTarget;
use crate::jobs::Ctx;
use crate::keymap::Command;
use crate::modal::{GitOp, Modal};
use crate::syntax::{self, HunkTokens};
use crate::theme::Theme;
use crate::widgets::{code_spans, truncate};

/// Below this width the two sides stack instead of sitting side by side.
const SIDE_BY_SIDE_WIDTH: u16 = 140;

/// Which of a file's changes are active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageSideId {
    Unstaged,
    Staged,
}

impl StageSideId {
    fn other(self) -> Self {
        match self {
            StageSideId::Unstaged => StageSideId::Staged,
            StageSideId::Staged => StageSideId::Unstaged,
        }
    }

    fn name(self) -> &'static str {
        match self {
            StageSideId::Unstaged => "Unstaged",
            StageSideId::Staged => "Staged",
        }
    }
}

/// A position in a diff: hunk and line within it.
type Pos = (usize, usize);

/// One row drawn for a side: a hunk's `@@` heading or one of its lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Heading(usize),
    Line(usize, usize),
}

#[derive(Debug, Default)]
struct SideView {
    diff: Option<StageSide>,
    /// The code in the diff's hunks.
    syntax: HunkTokens,
    rows: Vec<Row>,
    offset: usize,
    height: usize,
}

impl SideView {
    fn set(&mut self, diff: StageSide, syntax: HunkTokens) {
        self.rows.clear();
        if let Ok(d) = &diff {
            for (h, hunk) in d.hunks.iter().enumerate() {
                self.rows.push(Row::Heading(h));
                self.rows.extend((0..hunk.lines.len()).map(|l| Row::Line(h, l)));
            }
        }
        self.offset = self.offset.min(self.rows.len().saturating_sub(1));
        self.diff = Some(diff);
        self.syntax = syntax;
    }

    fn file_diff(&self) -> Option<&FileDiff> {
        self.diff.as_ref()?.as_ref().ok()
    }

    fn hunk_count(&self) -> usize {
        self.file_diff().map_or(0, |d| d.hunks.len())
    }

    /// Something to show: hunks, or a reason there are none to pick.
    fn has_content(&self) -> bool {
        matches!(&self.diff, Some(Err(_))) || self.hunk_count() > 0
    }

    /// Every changed line, in order.
    fn changes(&self) -> Vec<Pos> {
        let Some(d) = self.file_diff() else { return Vec::new() };
        d.hunks
            .iter()
            .enumerate()
            .flat_map(|(h, hunk)| {
                hunk.lines
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.is_change())
                    .map(move |(l, _)| (h, l))
            })
            .collect()
    }

    fn row_of(&self, row: Row) -> usize {
        self.rows.iter().position(|r| *r == row).unwrap_or(0)
    }
}

/// The staging view for one file.
#[derive(Debug)]
pub struct StageView {
    entry: StatusEntry,
    unstaged: SideView,
    staged: SideView,
    side: StageSideId,
    /// Picking single lines rather than whole hunks.
    line_mode: bool,
    hunk: usize,
    /// The line cursor, on a changed line, in line mode.
    cursor: Pos,
    /// The other end of a range of lines being picked.
    anchor: Option<Pos>,
    /// Keep the selection on screen (off after paging the view by hand).
    follow: bool,
    /// A patch built from what's shown is being applied: what's shown is
    /// out of date until the next diffs arrive.
    applying: bool,
}

impl StageView {
    pub fn new(entry: StatusEntry) -> Self {
        Self {
            entry,
            unstaged: SideView::default(),
            staged: SideView::default(),
            side: StageSideId::Unstaged,
            line_mode: false,
            hunk: 0,
            cursor: (0, 0),
            anchor: None,
            follow: true,
            applying: false,
        }
    }

    pub fn path(&self) -> &str {
        &self.entry.path
    }

    pub fn side(&self) -> StageSideId {
        self.side
    }

    pub fn line_mode(&self) -> bool {
        self.line_mode
    }

    /// The selected hunk's index on the active side.
    pub fn selected_hunk(&self) -> usize {
        if self.line_mode { self.cursor.0 } else { self.hunk }
    }

    /// The active side's diff, once loaded.
    pub fn diff(&self, side: StageSideId) -> Option<&FileDiff> {
        self.view(side).file_diff()
    }

    pub fn loaded(&self) -> bool {
        self.unstaged.diff.is_some() && self.staged.diff.is_some()
    }

    fn view(&self, side: StageSideId) -> &SideView {
        match side {
            StageSideId::Unstaged => &self.unstaged,
            StageSideId::Staged => &self.staged,
        }
    }

    fn active(&self) -> &SideView {
        self.view(self.side)
    }

    fn active_mut(&mut self) -> &mut SideView {
        match self.side {
            StageSideId::Unstaged => &mut self.unstaged,
            StageSideId::Staged => &mut self.staged,
        }
    }

    /// Takes newly loaded diffs, keeping the selection near where it was
    /// (after staging a hunk, on the hunk that followed it). If the active
    /// side has nothing left, the other side becomes active.
    /// Shows `entry`'s changes, with `syntax` the code in each side's hunks
    /// (unstaged, staged).
    pub fn set_diffs(
        &mut self,
        entry: StatusEntry,
        unstaged: StageSide,
        staged: StageSide,
        syntax: (HunkTokens, HunkTokens),
    ) {
        let first = !self.loaded();
        self.entry = entry;
        self.unstaged.set(unstaged, syntax.0);
        self.staged.set(staged, syntax.1);
        self.applying = false;
        self.anchor = None;
        if first && !self.active().has_content()
            || self.active().hunk_count() == 0 && self.view(self.side.other()).hunk_count() > 0
        {
            self.side = self.side.other();
            self.hunk = 0;
            self.cursor = (0, 0);
        }
        self.clamp();
        self.follow = true;
    }

    /// Puts the selection back on a hunk, or a changed line, that exists.
    fn clamp(&mut self) {
        let hunks = self.active().hunk_count();
        self.hunk = self.hunk.min(hunks.saturating_sub(1));
        if self.line_mode {
            let changes = self.active().changes();
            self.cursor = changes
                .iter()
                .copied()
                .find(|p| *p >= self.cursor)
                .or(changes.last().copied())
                .unwrap_or((0, 0));
        }
    }

    fn picked(&self, pos: Pos) -> bool {
        if !self.line_mode {
            return pos.0 == self.hunk;
        }
        let (a, b) = match self.anchor {
            Some(anchor) if anchor <= self.cursor => (anchor, self.cursor),
            Some(anchor) => (self.cursor, anchor),
            None => (self.cursor, self.cursor),
        };
        a <= pos && pos <= b
    }

    /// The rows the selection covers on the active side, first to last.
    fn selected_rows(&self) -> (usize, usize) {
        let side = self.active();
        if !self.line_mode {
            let start = side.row_of(Row::Heading(self.hunk));
            let len = side
                .file_diff()
                .and_then(|d| d.hunks.get(self.hunk))
                .map_or(0, |h| h.lines.len());
            return (start, start + len);
        }
        let row = side.row_of(Row::Line(self.cursor.0, self.cursor.1));
        (row, row)
    }

    pub fn title(&self) -> String {
        let mode = if self.line_mode { "lines" } else { "hunks" };
        format!("Stage {} · {mode}", self.entry.path)
    }

    pub fn on_command(&mut self, cmd: Command, ctx: &Ctx) -> EventState {
        match cmd {
            Command::MoveUp | Command::MoveDown | Command::Top | Command::Bottom => self.step(cmd),
            Command::PageUp | Command::PageDown => {
                let side = self.active_mut();
                let page = side.height.max(1);
                let max = side.rows.len().saturating_sub(page);
                side.offset = if cmd == Command::PageUp {
                    side.offset.saturating_sub(page)
                } else {
                    (side.offset + page).min(max)
                };
                self.follow = false;
            }
            Command::SwitchSide => {
                if self.view(self.side.other()).has_content() {
                    self.side = self.side.other();
                    self.hunk = 0;
                    self.cursor = (0, 0);
                    self.anchor = None;
                    self.clamp();
                    self.follow = true;
                }
            }
            Command::ToggleLineMode => {
                if self.line_mode {
                    self.hunk = self.cursor.0;
                    self.anchor = None;
                } else {
                    self.cursor = (self.hunk, 0);
                }
                self.line_mode = !self.line_mode;
                self.clamp();
                self.follow = true;
            }
            Command::SelectRange => {
                if !self.line_mode {
                    self.line_mode = true;
                    self.cursor = (self.hunk, 0);
                    self.clamp();
                    self.anchor = Some(self.cursor);
                } else if self.anchor.is_some() {
                    self.anchor = None;
                } else {
                    self.anchor = Some(self.cursor);
                }
                self.follow = true;
            }
            Command::StageToggle => {
                let op = match self.side {
                    StageSideId::Unstaged => PatchOp::Stage,
                    StageSideId::Staged => PatchOp::Unstage,
                };
                if let Some(op) = self.patch_op(op, ctx) {
                    ctx.emit(Action::Run(op));
                }
            }
            Command::Discard => {
                if self.side == StageSideId::Staged {
                    ctx.emit(Action::Notify(
                        "discard works on unstaged changes (tab switches)".into(),
                    ));
                } else if let Some(op) = self.patch_op(PatchOp::Discard, ctx) {
                    let what = if self.line_mode {
                        "selected lines"
                    } else {
                        "selected hunk"
                    };
                    ctx.emit(Action::OpenModal(Modal::confirm(
                        "Discard changes",
                        format!("Discard the {what} of {}? This cannot be undone.", self.entry.path),
                        op,
                    )));
                }
            }
            Command::OpenInEditor => ctx.emit(Action::OpenInEditor(EditTarget {
                path: self.entry.path.clone(),
                line: self.selected_line(),
                rev: None,
            })),
            _ => return EventState::NotConsumed,
        }
        EventState::Consumed
    }

    /// The working-tree line the selection is on: the selected line, or the
    /// selected hunk's first change. The staged side's lines are the
    /// index's, so they're carried through the unstaged changes.
    pub fn selected_line(&self) -> Option<u32> {
        let diff = self.active().file_diff()?;
        let (h, l) = if self.line_mode {
            self.cursor
        } else {
            (self.hunk, diff.hunks.get(self.hunk)?.first_change())
        };
        let line = diff.hunks.get(h)?.new_line_of(l);
        Some(match self.side {
            StageSideId::Unstaged => line,
            StageSideId::Staged => self.unstaged.file_diff().map_or(line, |u| u.old_to_new(line)),
        })
    }

    /// The selected hunk, or the lines from the first picked to the last,
    /// to ask an agent about.
    pub fn agent_hunk(&self) -> Option<AgentHunk> {
        let diff = self.active().file_diff()?;
        let (first, last) = if self.line_mode {
            let anchor = self.anchor.unwrap_or(self.cursor);
            (anchor.min(self.cursor), anchor.max(self.cursor))
        } else {
            let len = diff.hunks.get(self.hunk)?.lines.len();
            ((self.hunk, 0), (self.hunk, len.saturating_sub(1)))
        };
        let picks: Vec<_> = (first.0..=last.0)
            .filter_map(|h| {
                let hunk = diff.hunks.get(h)?;
                let from = if h == first.0 { first.1 } else { 0 };
                let to = if h == last.0 {
                    last.1
                } else {
                    hunk.lines.len().saturating_sub(1)
                };
                Some((hunk, from, to))
            })
            .collect();
        if picks.is_empty() {
            return None;
        }
        let source = match self.side {
            StageSideId::Unstaged => "the unstaged changes (the working tree against the index)",
            StageSideId::Staged => "the staged changes (the index against HEAD)",
        };
        Some(AgentHunk::new(&self.entry.path, source.into(), &picks))
    }

    /// Moves the selection to the next or previous hunk, or changed line.
    fn step(&mut self, cmd: Command) {
        self.follow = true;
        if !self.line_mode {
            let last = self.active().hunk_count().saturating_sub(1);
            self.hunk = match cmd {
                Command::MoveUp => self.hunk.saturating_sub(1),
                Command::MoveDown => (self.hunk + 1).min(last),
                Command::Top => 0,
                _ => last,
            };
            return;
        }
        let changes = self.active().changes();
        let Some(i) = changes.iter().position(|p| *p == self.cursor) else {
            return;
        };
        let i = match cmd {
            Command::MoveUp => i.saturating_sub(1),
            Command::MoveDown => (i + 1).min(changes.len() - 1),
            Command::Top => 0,
            _ => changes.len() - 1,
        };
        self.cursor = changes[i];
    }

    /// The git op applying the selection as `op`, or `None` (with a notice)
    /// when there's nothing to apply yet.
    fn patch_op(&mut self, op: PatchOp, ctx: &Ctx) -> Option<GitOp> {
        if self.applying {
            ctx.emit(Action::Notify("still applying the last change…".into()));
            return None;
        }
        let side = self.active();
        let diff = match &side.diff {
            Some(Ok(d)) => d,
            Some(Err(why)) => {
                ctx.emit(Action::Notify(format!("{}: {why}", self.entry.path)));
                return None;
            }
            None => return None,
        };
        let untracked = self.entry.is_untracked();
        let patch = diff.patch(|h, l| self.picked((h, l)), op.direction(), untracked)?;
        Some(GitOp::ApplyPatch {
            path: self.entry.path.clone(),
            patch,
            op,
            untracked,
            lines: self.line_mode,
        })
    }

    /// A patch built from what's shown is being applied, so what's shown is
    /// out of date until the next diffs arrive.
    pub fn mark_applying(&mut self) {
        self.applying = true;
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
        if !self.loaded() {
            frame.render_widget(Paragraph::new("loading…").style(theme.muted), area);
            return;
        }
        let (u, s) = (self.unstaged.has_content(), self.staged.has_content());
        if !u && !s {
            frame.render_widget(Paragraph::new("no changes").style(theme.muted), area);
            return;
        }
        let (first, second) = if !u || !s {
            // The empty side shrinks to its title along the bottom.
            let [main, rest] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
            if u { (main, rest) } else { (rest, main) }
        } else if area.width >= SIDE_BY_SIDE_WIDTH {
            let [left, right] = Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]).areas(area);
            let block = Block::new().borders(Borders::RIGHT).border_style(theme.border);
            let inner = block.inner(left);
            frame.render_widget(block, left);
            (
                inner,
                Rect {
                    x: right.x + 1,
                    width: right.width.saturating_sub(1),
                    ..right
                },
            )
        } else {
            let [top, bottom] = Layout::vertical([Constraint::Fill(1), Constraint::Fill(1)]).areas(area);
            (top, bottom)
        };
        self.draw_side(frame, StageSideId::Unstaged, first, focused, theme);
        self.draw_side(frame, StageSideId::Staged, second, focused, theme);
    }

    fn draw_side(&mut self, frame: &mut Frame, id: StageSideId, area: Rect, focused: bool, theme: &Theme) {
        if area.height == 0 {
            return;
        }
        let active = id == self.side;
        let [title_area, body] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
        let side = self.view(id);
        let summary = match &side.diff {
            Some(Ok(d)) if d.is_empty() => "no changes".to_string(),
            Some(Ok(d)) if d.hunks.len() == 1 => "1 hunk".to_string(),
            Some(Ok(d)) => format!("{} hunks", d.hunks.len()),
            _ => String::new(),
        };
        let title_style = match (active, focused) {
            (true, true) => theme.title_focused,
            (true, false) => theme.title,
            _ => theme.muted,
        };
        let marker = if active { "▌" } else { " " };
        let title = Line::from(vec![
            Span::styled(format!("{marker}{}", id.name()), title_style),
            Span::styled(format!("  {summary}"), theme.muted),
        ]);
        frame.render_widget(Paragraph::new(title), title_area);

        if let Some(Err(why)) = &side.diff {
            frame.render_widget(Paragraph::new(format!(" {why}")).style(theme.muted), body);
            return;
        }
        let height = body.height as usize;
        let selection = (active && self.follow).then(|| self.selected_rows());
        let view = match id {
            StageSideId::Unstaged => &mut self.unstaged,
            StageSideId::Staged => &mut self.staged,
        };
        view.height = height;
        if let Some((first, last)) = selection
            && height > 0
        {
            if last + 1 - first > height || first < view.offset {
                view.offset = first;
            } else if last >= view.offset + height {
                view.offset = last + 1 - height;
            }
        }
        let view = self.view(id);
        let Some(diff) = view.diff.as_ref().and_then(|d| d.as_ref().ok()) else {
            return;
        };
        let width = body.width as usize;
        let highlight = if focused {
            theme.selected
        } else {
            theme.selected_unfocused
        };
        let start = view.offset.min(view.rows.len());
        let end = (start + height).min(view.rows.len());
        let lines: Vec<Line> = view.rows[start..end]
            .iter()
            .map(|row| {
                let (line, picked) = match *row {
                    Row::Heading(h) => (
                        Line::from(Span::styled(truncate(&diff.hunks[h].heading(), width), theme.diff_hunk)),
                        !self.line_mode && h == self.hunk,
                    ),
                    Row::Line(h, l) => {
                        let line = &diff.hunks[h].lines[l];
                        let (prefix, style, back) = match line.kind {
                            LineKind::Context => (" ", Style::new(), Style::new()),
                            LineKind::Add => ("+", theme.diff_add, theme.diff_add_line),
                            LineKind::Del => ("-", theme.diff_del, theme.diff_del_line),
                        };
                        let body = syntax::shown(&line.text);
                        let picked = if self.line_mode {
                            line.is_change() && self.picked((h, l))
                        } else {
                            h == self.hunk
                        };
                        let tokens = view.syntax.get(h).and_then(|t| t.get(l)).and_then(Option::as_ref);
                        let line = match tokens {
                            Some(tokens) if width > 0 => {
                                let code =
                                    code_spans(&body, Some(tokens), Style::new(), theme, width - 1, line.is_change());
                                let mut spans = vec![Span::styled(prefix, style)];
                                spans.extend(code.into_iter().map(|s| Span::styled(s.content.into_owned(), s.style)));
                                Line::from(spans).style(back)
                            }
                            _ => Line::from(Span::styled(truncate(&format!("{prefix}{body}"), width), style)),
                        };
                        (line, picked)
                    }
                };
                if active && picked {
                    line.patch_style(highlight)
                } else {
                    line
                }
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), body);
    }
}
