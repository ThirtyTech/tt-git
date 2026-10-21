//! Blaming one file: when each line last changed, who changed it and in
//! which commit, reading right to left from the code. `[` steps back to the
//! file as it was before the selected line's commit; `]` comes forward again.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tt_git_core::time::{SharedClock, clock_time, relative};
use tt_git_core::{Blame, BlameCommit};

use super::DateStyle;
use crate::action::Action;
use crate::component::EventState;
use crate::editor::EditTarget;
use crate::jobs::Ctx;
use crate::keymap::Command;
use crate::theme::Theme;
use crate::widgets::{Cursor, fit, fit_right};

/// Ids for blame loads, unique across views so a late result can't land
/// in a view opened after it.
static NEXT_LOAD: AtomicU64 = AtomicU64::new(1);

/// Width of the author (who) column.
const WHO_WIDTH: usize = 14;
/// Below this width the commit and summary (why) column is left out.
const WHY_MIN_WIDTH: u16 = 100;
/// The most rows the commit details take below the code.
const COMMIT_ROWS: u16 = 10;

/// One version of the file: the one opened, then one more for each `[`.
#[derive(Debug)]
struct Level {
    id: u64,
    /// `None` for the working tree.
    rev: Option<String>,
    /// How the title names `rev`: `working tree`, `HEAD`, `19373b2^`.
    label: String,
    path: String,
    blame: Option<Result<Blame, String>>,
    /// Each commit's index into the theme's heat colors, by age.
    heat: Vec<usize>,
    cursor: Cursor,
    /// The line (from 1) to select once the blame arrives.
    want_line: u32,
    /// Put the selection a third of the way down at the next draw.
    center: bool,
}

impl Level {
    fn new(rev: Option<String>, label: String, path: String, line: u32) -> Self {
        Self {
            id: NEXT_LOAD.fetch_add(1, Ordering::Relaxed),
            rev,
            label,
            path,
            blame: None,
            heat: Vec::new(),
            cursor: Cursor::default(),
            want_line: line,
            center: true,
        }
    }

    fn load(&self, ctx: &Ctx) {
        ctx.emit(Action::LoadBlame {
            id: self.id,
            rev: self.rev.clone(),
            path: self.path.clone(),
        });
    }

    fn blame(&self) -> Option<&Blame> {
        self.blame.as_ref()?.as_ref().ok()
    }

    fn len(&self) -> usize {
        self.blame().map_or(0, |b| b.lines.len())
    }
}

/// The blame view for one file.
#[derive(Debug)]
pub struct BlameView {
    /// Newest first; the last is shown.
    levels: Vec<Level>,
    dates: DateStyle,
    clock: SharedClock,
    /// The selected line's commit details show below the code.
    show_commit: bool,
    /// Commit messages below the subject line, by commit; `None` while loading.
    messages: HashMap<String, Option<String>>,
}

impl BlameView {
    /// Opens a blame of `path` as of `rev` (the working tree when `None`),
    /// called `label` in the title, on line `line`, and asks for it.
    pub fn open(
        path: String,
        rev: Option<String>,
        label: String,
        line: u32,
        dates: DateStyle,
        clock: SharedClock,
        ctx: &Ctx,
    ) -> Self {
        let level = Level::new(rev, label, path, line);
        level.load(ctx);
        Self {
            levels: vec![level],
            dates,
            clock,
            show_commit: false,
            messages: HashMap::new(),
        }
    }

    fn level(&self) -> &Level {
        self.levels.last().expect("a blame view has a level")
    }

    fn level_mut(&mut self) -> &mut Level {
        self.levels.last_mut().expect("a blame view has a level")
    }

    pub fn path(&self) -> &str {
        &self.level().path
    }

    /// How the title names the version shown: `working tree`, `HEAD`, `19373b2^`.
    pub fn label(&self) -> &str {
        &self.level().label
    }

    /// How many versions deep: 1 until `[` steps back.
    pub fn depth(&self) -> usize {
        self.levels.len()
    }

    pub fn loading(&self) -> bool {
        self.level().blame.is_none()
    }

    pub fn blame(&self) -> Option<&Blame> {
        self.level().blame()
    }

    pub fn show_commit(&self) -> bool {
        self.show_commit
    }

    pub fn set_dates(&mut self, dates: DateStyle) {
        self.dates = dates;
    }

    /// The selected line's number, from 1.
    pub fn selected_line(&self) -> usize {
        self.level().cursor.selected + 1
    }

    /// The selected line's commit, the file's path in that commit and the
    /// line's number there.
    pub fn selected(&self) -> Option<(&BlameCommit, &str, u32)> {
        let level = self.level();
        let blame = level.blame()?;
        let line = blame.lines.get(level.cursor.selected)?;
        Some((&blame.commits[line.commit], &blame.paths[line.path], line.orig_line))
    }

    /// Takes blame load `id`'s result. Returns whether it was one of this
    /// view's.
    pub fn set_blame(&mut self, id: u64, result: Result<Blame, String>, ctx: &Ctx) -> bool {
        let Some(level) = self.levels.iter_mut().find(|l| l.id == id) else {
            return false;
        };
        if let Ok(blame) = &result {
            level.heat = heat(blame);
            let last = blame.lines.len().saturating_sub(1);
            level.cursor.selected = (level.want_line.max(1) as usize - 1).min(last);
            level.center = true;
        }
        level.blame = Some(result);
        self.want_message(ctx);
        true
    }

    pub fn set_message(&mut self, oid: &str, body: String) {
        if let Some(slot) = self.messages.get_mut(oid) {
            *slot = Some(body);
        }
    }

    /// Asks again for what was still loading when the view was put aside
    /// (its results were dropped meanwhile).
    pub fn reload_missing(&mut self, ctx: &Ctx) {
        if self.loading() {
            self.level().load(ctx);
        }
        self.messages.retain(|_, m| m.is_some());
        self.want_message(ctx);
    }

    pub fn title(&self) -> String {
        let level = self.level();
        let mut title = format!("Blame {} at {}", level.path, level.label);
        if let Some(newer) = self.levels.iter().rev().nth(1) {
            title.push_str(&format!("  ‹ {}", newer.label));
        }
        title
    }

    /// The selected line and how many there are, like `2/5`, once loaded.
    pub fn position(&self) -> Option<String> {
        let level = self.level();
        (level.len() > 0).then(|| format!("{}/{}", level.cursor.selected + 1, level.len()))
    }

    /// How a view opened from here names it, to come back to.
    pub fn back_label(&self) -> String {
        let level = self.level();
        format!("Blame {} at {}, line {}", level.path, level.label, self.selected_line())
    }

    /// The selected line in the editor.
    pub fn edit_target(&self) -> Result<EditTarget, String> {
        let level = self.level();
        if level.len() == 0 {
            return Err("no file to open here".into());
        }
        Ok(EditTarget {
            path: level.path.clone(),
            line: Some(self.selected_line() as u32),
            rev: level.rev.clone(),
        })
    }

    pub fn on_command(&mut self, cmd: Command, ctx: &Ctx) -> EventState {
        match cmd {
            Command::ToggleDetails => {
                self.show_commit = !self.show_commit;
                self.want_message(ctx);
            }
            Command::BlameBack => self.step_back(ctx),
            Command::BlameForward => {
                if self.levels.len() > 1 {
                    self.levels.pop();
                    self.want_message(ctx);
                } else {
                    ctx.emit(Action::Notify("this is the newest version".into()));
                }
            }
            Command::Copy => match self.selected() {
                Some((c, _, _)) if !c.is_uncommitted() => ctx.emit(Action::Copy(c.oid.clone())),
                Some(_) => ctx.emit(Action::Notify("this line isn't committed yet".into())),
                None => {}
            },
            Command::OpenInEditor => ctx.emit(match self.edit_target() {
                Ok(target) => Action::OpenInEditor(target),
                Err(why) => Action::Notify(why),
            }),
            _ => {
                let len = self.level().len();
                if !self.level_mut().cursor.apply(cmd, len) {
                    return EventState::NotConsumed;
                }
                self.want_message(ctx);
            }
        }
        EventState::Consumed
    }

    /// Selects the line on visible row `row` of the code.
    pub fn click(&mut self, row: usize, ctx: &Ctx) {
        let len = self.level().len();
        let level = self.level_mut();
        if row < level.cursor.height && level.cursor.click(row, len) {
            self.want_message(ctx);
        }
    }

    /// Blames the file as it was just before the selected line's commit,
    /// on that line's number there.
    fn step_back(&mut self, ctx: &Ctx) {
        let line = self.selected_line() as u32;
        let current = self.level().path.clone();
        let Some((commit, _, orig_line)) = self.selected() else {
            return;
        };
        let older = if commit.is_uncommitted() {
            Level::new(Some("HEAD".into()), "HEAD".into(), current, line)
        } else if let Some((oid, path)) = &commit.previous {
            Level::new(
                Some(oid.clone()),
                format!("{}^", commit.short()),
                path.clone(),
                orig_line,
            )
        } else {
            // No previous: the commit added the file, or it's the repo's
            // first (which blame calls a boundary).
            let why = format!("{} added {current}: there's nothing before it", commit.short());
            ctx.emit(Action::Notify(why));
            return;
        };
        older.load(ctx);
        self.levels.push(older);
    }

    /// Loads the selected commit's message if its details are showing.
    fn want_message(&mut self, ctx: &Ctx) {
        if !self.show_commit {
            return;
        }
        let Some(oid) = self
            .selected()
            .filter(|(c, _, _)| !c.is_uncommitted())
            .map(|(c, _, _)| c.oid.clone())
        else {
            return;
        };
        if !self.messages.contains_key(&oid) {
            self.messages.insert(oid.clone(), None);
            ctx.emit(Action::LoadCommitMessage(oid));
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
        let commit_rows = if self.show_commit && self.level().len() > 0 {
            COMMIT_ROWS.min(area.height / 2)
        } else {
            0
        };
        let [code_area, commit_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(commit_rows)]).areas(area);
        self.draw_lines(frame, code_area, focused, theme);
        if commit_rows > 0 {
            self.draw_commit(frame, commit_area, theme);
        }
    }

    /// Each line, reading right to left from the code: when (age stripe and
    /// date), who, and why (the commit and its summary, where its lines start).
    fn draw_lines(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
        let now = self.clock.now();
        let dates = self.dates;
        let clock = self.clock.clone();
        let height = area.height as usize;
        let level = self.levels.last_mut().expect("a blame view has a level");
        level.cursor.height = height;
        match &level.blame {
            None => {
                frame.render_widget(Paragraph::new("loading…").style(theme.muted), area);
                return;
            }
            Some(Err(e)) => {
                let text = format!("could not blame {}: {e}", level.path);
                frame.render_widget(Paragraph::new(text).style(theme.error).wrap(Wrap { trim: false }), area);
                return;
            }
            Some(Ok(_)) => {}
        }
        if std::mem::take(&mut level.center) {
            // A third of the way down, without scrolling past the end.
            let last_page = level.len().saturating_sub(height);
            level.cursor.offset = level.cursor.selected.saturating_sub(height / 3).min(last_page);
        }
        level.cursor.scroll_into_view(height);
        let level = &*level;
        let blame = level.blame().expect("checked above");
        let selected = level.cursor.selected;
        let selected_commit = blame.lines.get(selected).map(|l| l.commit);

        let num_width = blame.lines.len().to_string().len().max(3);
        let when_width = dates.width();
        let summary_width = if area.width >= WHY_MIN_WIDTH {
            (area.width as usize / 6).clamp(12, 30)
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
        let code_width = (area.width as usize).saturating_sub(gutter);

        let start = level.cursor.offset.min(blame.lines.len());
        let end = (start + height).min(blame.lines.len());
        let mut rows = Vec::with_capacity(end - start);
        for i in start..end {
            let line = &blame.lines[i];
            let commit = &blame.commits[line.commit];
            let uncommitted = commit.is_uncommitted();
            let first = i == 0 || blame.lines[i - 1].commit != line.commit;
            let mut spans = Vec::with_capacity(12);
            if why_width > 0 {
                if !first {
                    spans.push(Span::raw(" ".repeat(why_width)));
                } else if uncommitted {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(fit("Not committed yet", why_width - 1), theme.diff_add));
                } else {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(fit(commit.short(), 7), theme.sha));
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(fit(&commit.summary, summary_width), theme.author));
                }
            }
            spans.push(Span::raw(" "));
            if uncommitted {
                spans.push(Span::styled(fit_right("You", WHO_WIDTH), theme.blame_uncommitted));
            } else {
                let style = Style::new().fg(author_color(&commit.author, theme));
                spans.push(Span::styled(fit_right(&commit.author, WHO_WIDTH), style));
            }
            spans.push(Span::raw(" "));
            let when = match dates {
                DateStyle::Relative => relative(now, commit.author_time),
                DateStyle::Clock => clock_time(commit.author_time, clock.utc_offset(commit.author_time)),
            };
            spans.push(Span::styled(fit_right(&when, when_width), theme.date));
            spans.push(Span::raw(" "));
            let stripe = if uncommitted {
                theme.blame_uncommitted
            } else {
                Style::new().fg(heat_color(level.heat.get(line.commit).copied(), theme))
            };
            spans.push(Span::styled("▌", stripe));
            spans.push(Span::styled(format!("{:>num_width$}", i + 1), theme.muted));
            spans.push(Span::raw(" "));
            spans.push(Span::raw(fit(&expand_tabs(&line.text), code_width)));
            let row = Line::from(spans);
            rows.push(if i == selected {
                row.patch_style(if focused {
                    theme.selected
                } else {
                    theme.selected_unfocused
                })
            } else if Some(line.commit) == selected_commit {
                row.patch_style(theme.selected_unfocused)
            } else {
                row
            });
        }
        frame.render_widget(Paragraph::new(rows), area);
    }

    /// The selected line's commit: subject, who and when, and its message.
    fn draw_commit(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let Some((commit, _, _)) = self.selected() else { return };
        let blame = self.blame().expect("a line is selected");
        let index = blame.commits.iter().position(|c| c.oid == commit.oid).unwrap_or(0);
        let count = blame.lines_from(index);
        let title = if commit.is_uncommitted() {
            "Not committed yet".to_string()
        } else {
            format!("Commit {}", commit.short())
        };
        let block = Block::new()
            .borders(Borders::TOP)
            .border_style(theme.border)
            .title(Line::styled(title, theme.muted));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let lines_here = |what: &str| {
            let s = if count == 1 { "" } else { "s" };
            format!("{count} line{s} in this file {what}")
        };
        let mut lines = Vec::new();
        if commit.is_uncommitted() {
            lines.push(Line::styled("Changed in the working tree", theme.diff_header));
            lines.push(Line::styled(lines_here("aren't committed yet"), theme.author));
        } else {
            let when = format!(
                "{} ({} ago)",
                clock_time(commit.author_time, self.clock.utc_offset(commit.author_time)),
                relative(self.clock.now(), commit.author_time)
            );
            lines.push(Line::styled(commit.summary.clone(), theme.diff_header));
            lines.push(Line::from(vec![
                Span::styled(commit.short().to_string(), theme.sha),
                Span::styled(
                    format!(
                        " · {} <{}> · {when} · {}",
                        commit.author,
                        commit.author_mail,
                        lines_here("come from it")
                    ),
                    theme.author,
                ),
            ]));
            match self.messages.get(&commit.oid) {
                Some(Some(body)) if !body.is_empty() => {
                    lines.push(Line::raw(""));
                    lines.extend(body.lines().map(|l| Line::raw(expand_tabs(l))));
                }
                Some(None) => {
                    lines.push(Line::raw(""));
                    lines.push(Line::styled("loading…", theme.muted));
                }
                _ => {}
            }
        }
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

/// Each commit's heat bucket by age, newest 0: commits ranked by when they
/// were made and spread over the theme's heat colors.
fn heat(blame: &Blame) -> Vec<usize> {
    let buckets = Theme::default().blame_heat.len();
    let mut times: Vec<i64> = blame
        .commits
        .iter()
        .filter(|c| !c.is_uncommitted())
        .map(|c| c.author_time)
        .collect();
    times.sort_unstable();
    times.dedup();
    blame
        .commits
        .iter()
        .map(|c| {
            if times.len() <= 1 {
                return 0;
            }
            let older = times.partition_point(|t| *t < c.author_time).min(times.len() - 1);
            let newer = times.len() - 1 - older;
            newer * buckets / times.len()
        })
        .collect()
}

fn heat_color(bucket: Option<usize>, theme: &Theme) -> Color {
    let last = theme.blame_heat.len() - 1;
    theme.blame_heat[bucket.unwrap_or(last).min(last)]
}

/// The author's color: the same for the same name, every time (FNV-1a, not
/// the std hasher, whose output may change between Rust releases).
fn author_color(author: &str, theme: &Theme) -> Color {
    let hash = author.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    theme.blame_authors[(hash % theme.blame_authors.len() as u64) as usize]
}

fn expand_tabs(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\t') {
        text.replace('\t', "    ").into()
    } else {
        text.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_git_core::{BlameCommit, BlameLine};

    fn commit(oid: char, time: i64) -> BlameCommit {
        BlameCommit {
            oid: oid.to_string().repeat(40),
            author_time: time,
            ..BlameCommit::default()
        }
    }

    #[test]
    fn heat_runs_newest_to_oldest() {
        let blame = Blame {
            commits: vec![commit('a', 100), commit('b', 500), commit('c', 300), commit('0', 900)],
            paths: vec![String::new()],
            lines: Vec::<BlameLine>::new(),
        };
        // Three committed times: newest hot (0), oldest coldest.
        assert_eq!(heat(&blame), vec![3, 0, 1, 0]);
        let one = Blame {
            commits: vec![commit('a', 100)],
            ..Blame::default()
        };
        assert_eq!(heat(&one), vec![0]);
    }

    #[test]
    fn an_author_keeps_their_color() {
        let theme = Theme::default();
        assert_eq!(author_color("Ada Park", &theme), author_color("Ada Park", &theme));
        let colors: std::collections::HashSet<_> = ["Ada Park", "Sam Rivera", "Mia Chen", "Linus Brandt"]
            .iter()
            .map(|a| format!("{:?}", author_color(a, &theme)))
            .collect();
        assert!(colors.len() > 1);
    }
}
