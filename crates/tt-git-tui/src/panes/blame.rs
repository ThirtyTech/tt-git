//! Blaming one file: when each line last changed, who changed it and in
//! which commit, reading right to left from the code. `[` steps back to the
//! file as it was before the selected line's commit; `]` comes forward again.
//! `{` and `}` step one commit of the file at a time, back and forward. All
//! of them keep the selected line by its text, on the same row of the screen.
//! Digits count, vim style: `42G` goes to line 42, `5j` down five lines.
//! `/` searches the code for a regex, jumping as you type; `n` and `N` go to
//! the next and previous matching line.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tt_git_core::time::{SharedClock, clock_time, relative};
use tt_git_core::{Blame, BlameCommit, Recency, follow_line};

use super::DateStyle;
use crate::action::Action;
use crate::component::EventState;
use crate::editor::EditTarget;
use crate::jobs::Ctx;
use crate::keymap::{Command, KeyCode, KeyCombo, KeyModifiers};
use crate::search::{LineSearch, SearchKey};
use crate::syntax::LineTokens;
use crate::theme::Theme;
use crate::widgets::{Cursor, expand_tabs, fit, fit_right, marked_code_spans};

/// Ids for blame loads, unique across views so a late result can't land
/// in a view opened after it.
static NEXT_LOAD: AtomicU64 = AtomicU64::new(1);

/// Width of the author (who) column.
const WHO_WIDTH: usize = 14;
/// Below this width the commit and summary (why) column is left out.
const WHY_MIN_WIDTH: u16 = 100;
/// The most rows the commit details take below the code.
const COMMIT_ROWS: u16 = 10;
/// The largest count digits can type.
const MAX_COUNT: usize = 9_999_999;

/// One version of the file: the one opened, then one more for each `[`
/// or `{`.
#[derive(Debug)]
struct Level {
    id: u64,
    /// Which version this is, still to be found from another one's.
    step: Option<Step>,
    /// Takes the place of the version below it once loaded, for `}`.
    replaces: bool,
    /// `None` for the working tree.
    rev: Option<String>,
    /// How the title names `rev`: `working tree`, `HEAD`, `19373b2^`.
    label: String,
    path: String,
    blame: Option<Result<Blame, String>>,
    /// When the file last changed in a commit, as of `rev`.
    changed: Option<i64>,
    /// How this version stands against `HEAD`'s.
    recency: Option<Recency>,
    /// The code, line by line.
    syntax: LineTokens,
    /// Each commit's index into the theme's heat colors, by age.
    heat: Vec<usize>,
    /// Each commit's index into the theme's author colors.
    authors: Vec<usize>,
    cursor: Cursor,
    /// The line (from 1) to select once the blame arrives, when there's no
    /// newer version to follow it from.
    want_line: u32,
    /// Where to put the selection on screen at the next draw.
    place: Option<Place>,
}

/// How to find a version from the one it steps from, for `{` and `}`.
#[derive(Debug, Clone)]
struct Step {
    rev: Option<String>,
    path: String,
    /// For `}`: the newest version to step toward, its rev and path.
    newest: Option<(Option<String>, String)>,
}

/// Where the next draw puts the selected line.
#[derive(Debug, Clone, Copy)]
enum Place {
    /// A third of the way down.
    Center,
    /// On this row, where it was in the version it came from.
    Row(usize),
}

impl Level {
    fn new(rev: Option<String>, label: String, path: String, line: u32) -> Self {
        Self {
            id: NEXT_LOAD.fetch_add(1, Ordering::Relaxed),
            step: None,
            replaces: false,
            rev,
            label,
            path,
            blame: None,
            changed: None,
            recency: None,
            syntax: Vec::new(),
            heat: Vec::new(),
            authors: Vec::new(),
            cursor: Cursor::default(),
            want_line: line,
            place: Some(Place::Center),
        }
    }

    /// A version stepped to from `from` with `{`, or with `}` toward
    /// `newest`, found before it's loaded.
    fn step(from: &Level, newest: Option<&Level>) -> Self {
        let step = Step {
            rev: from.rev.clone(),
            path: from.path.clone(),
            newest: newest.map(|n| (n.rev.clone(), n.path.clone())),
        };
        let label = if newest.is_some() {
            "next commit"
        } else {
            "previous commit"
        };
        let mut level = Self::new(None, label.into(), from.path.clone(), 1);
        level.replaces = step.newest.is_some();
        level.step = Some(step);
        level
    }

    fn load(&self, ctx: &Ctx) {
        if let Some(step) = &self.step {
            ctx.emit(Action::StepBlame {
                id: self.id,
                rev: step.rev.clone(),
                path: step.path.clone(),
                newest: step.newest.clone(),
            });
            return;
        }
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

    fn texts(&self) -> Vec<&str> {
        self.blame()
            .map_or_else(Vec::new, |b| b.lines.iter().map(|l| l.text.as_str()).collect())
    }

    fn selected(&self) -> Option<(&BlameCommit, &str, u32)> {
        let blame = self.blame()?;
        let line = blame.lines.get(self.cursor.selected)?;
        Some((&blame.commits[line.commit], &blame.paths[line.path], line.orig_line))
    }

    /// The selected line's row on screen as of the last draw.
    fn row(&self) -> usize {
        self.cursor.selected.saturating_sub(self.cursor.offset)
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
    /// A count being typed, vim style.
    count: Option<usize>,
    /// The count for the command the last key resolves to.
    pending: Option<usize>,
    search: LineSearch,
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
            count: None,
            pending: None,
            search: LineSearch::default(),
        }
    }

    fn level(&self) -> &Level {
        self.levels.last().expect("a blame view has a level")
    }

    fn level_mut(&mut self) -> &mut Level {
        self.levels.last_mut().expect("a blame view has a level")
    }

    /// The version on screen: the newest, or while an older one `[` asked
    /// for is loading, the one it steps back from.
    fn shown(&self) -> usize {
        let last = self.levels.len() - 1;
        if self.levels[last].blame.is_some() {
            return last;
        }
        self.levels.iter().rposition(|l| l.blame().is_some()).unwrap_or(last)
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

    /// How many lines the file has, once loaded.
    pub fn line_count(&self) -> usize {
        self.level().len()
    }

    /// The count typed so far, before the key it applies to.
    pub fn count(&self) -> Option<usize> {
        self.count
    }

    /// Digits build a count for the next key, vim style (a first `0`
    /// doesn't start one); `esc` drops it. Returns whether the key was one
    /// of those. Any other key takes the count along to its command.
    pub fn on_key(&mut self, key: KeyCombo) -> bool {
        let typed = self.count.take();
        self.pending = None;
        match key.code {
            KeyCode::Char(c @ '0'..='9') if key.modifiers == KeyModifiers::NONE && (typed.is_some() || c != '0') => {
                let digit = c as usize - '0' as usize;
                self.count = Some((typed.unwrap_or(0) * 10 + digit).min(MAX_COUNT));
                true
            }
            KeyCode::Esc if typed.is_some() => true,
            _ => {
                self.pending = typed;
                false
            }
        }
    }

    /// Selects line `line` (from 1; past the end is the last line),
    /// scrolling it a third of the way down if it's out of view. Before the
    /// blame arrives, it's the line to open on.
    pub fn go_to_line(&mut self, line: usize, ctx: &Ctx) {
        let len = self.level().len();
        let level = self.level_mut();
        if len == 0 {
            level.want_line = line.min(u32::MAX as usize) as u32;
            return;
        }
        let cursor = &mut level.cursor;
        cursor.selected = (line.max(1) - 1).min(len - 1);
        if cursor.selected < cursor.offset || cursor.selected >= cursor.offset + cursor.height {
            level.place = Some(Place::Center);
        }
        self.want_message(ctx);
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
        self.level().selected()
    }

    /// Takes blame load `id`'s result, when the file last changed in a
    /// commit as of that version, and how that version stands against
    /// `HEAD`'s. Returns whether it was one of this view's.
    pub fn set_blame(
        &mut self,
        id: u64,
        result: Result<Blame, String>,
        syntax: LineTokens,
        changed: Option<i64>,
        recency: Option<Recency>,
        ctx: &Ctx,
    ) -> bool {
        let Some(index) = self.levels.iter().position(|l| l.id == id) else {
            return false;
        };
        let (newer, rest) = self.levels.split_at_mut(index);
        let level = &mut rest[0];
        if let Ok(blame) = &result {
            level.heat = heat(blame);
            level.authors = author_colors(blame);
            let last = blame.lines.len().saturating_sub(1);
            level.cursor.selected = (level.want_line.max(1) as usize - 1).min(last);
            level.place = Some(Place::Center);
            // Stepped back: follow the newer version's selected line by its
            // text, and keep it where it was on screen.
            if let Some(newer) = newer.last().filter(|n| n.blame().is_some()) {
                let to: Vec<&str> = blame.lines.iter().map(|l| l.text.as_str()).collect();
                level.cursor.selected = follow_line(&newer.texts(), &to, newer.cursor.selected);
                level.place = Some(Place::Row(newer.row()));
            }
        }
        level.blame = Some(result);
        level.changed = changed;
        level.recency = recency;
        level.syntax = syntax;
        if level.replaces && index > 0 {
            self.levels.remove(index - 1);
        }
        self.find_matches();
        self.want_message(ctx);
        true
    }

    /// Takes the version blame load `id` stepped to, and loads it. With
    /// none that way, `{` says so and `}` comes forward to the version
    /// below. Returns whether it was one of this view's.
    pub fn set_version(&mut self, id: u64, version: Result<Option<(String, String)>, String>, ctx: &Ctx) -> bool {
        let Some(index) = self.levels.iter().position(|l| l.id == id) else {
            return false;
        };
        let level = &mut self.levels[index];
        match version {
            Ok(Some((rev, path))) => {
                level.label = if rev.len() == 40 {
                    rev[..7].to_string()
                } else {
                    rev.clone()
                };
                level.rev = Some(rev);
                level.path = path;
                level.step = None;
                level.load(ctx);
            }
            Ok(None) if level.replaces => {
                // The next version is the one below: come forward to it.
                self.levels.remove(index);
                self.forward(ctx);
            }
            Ok(None) => {
                let path = level.path.clone();
                self.levels.remove(index);
                ctx.emit(Action::Notify(format!("no older commit changed {path}")));
            }
            Err(why) => {
                self.levels.remove(index);
                ctx.emit(Action::Notify(why));
            }
        }
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

    /// When the version shown last changed in a commit, like `changed
    /// 2026-10-17 01:30 PM (3w ago)`, and in the working tree whether it has
    /// changed since. `None` until it's loaded, or when no commit has the file.
    pub fn changed(&self) -> Option<String> {
        let level = self.level();
        let changed = level.changed?;
        let mut text = format!(
            "changed {} ({} ago)",
            clock_time(changed, self.clock.utc_offset(changed)),
            relative(self.clock.now(), changed)
        );
        if level.rev.is_none() && level.blame()?.commits.iter().any(BlameCommit::is_uncommitted) {
            text.push_str(" + uncommitted changes");
        }
        Some(text)
    }

    /// Whether the version shown is the newest, for the title: `latest`,
    /// or how many commits have changed the file since, in a color that
    /// stands out. `None` until it's loaded.
    pub fn recency(&self, theme: &Theme) -> Option<Span<'static>> {
        let (text, style) = match self.level().recency? {
            Recency::Latest => ("latest".to_string(), theme.blame_latest),
            Recency::Behind(1) => ("1 newer commit".to_string(), theme.blame_older),
            Recency::Behind(n) => (format!("{n} newer commits"), theme.blame_older),
            Recency::Elsewhere => ("not on HEAD".to_string(), theme.blame_older),
        };
        Some(Span::styled(text, style))
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
        let count = self.pending.take();
        match cmd {
            Command::Top | Command::Bottom if count.is_some() => self.go_to_line(count.unwrap_or(1), ctx),
            Command::MoveUp | Command::MoveDown if count.is_some() => {
                let (n, line) = (count.unwrap_or(1), self.selected_line());
                let line = if cmd == Command::MoveUp {
                    line.saturating_sub(n)
                } else {
                    line + n
                };
                self.go_to_line(line, ctx);
            }
            Command::ToggleDetails => {
                self.show_commit = !self.show_commit;
                self.want_message(ctx);
            }
            Command::BlameBack => self.step_back(ctx),
            Command::BlameForward => {
                if self.levels.len() > 1 {
                    self.forward(ctx);
                } else {
                    ctx.emit(Action::Notify("this is the newest version".into()));
                }
            }
            Command::BlameOlder | Command::BlameNewer if self.loading() => {}
            Command::BlameOlder => {
                let older = Level::step(self.level(), None);
                older.load(ctx);
                self.levels.push(older);
            }
            Command::BlameNewer => {
                if let [.., newest, from] = &self.levels[..] {
                    let newer = Level::step(from, Some(newest));
                    newer.load(ctx);
                    self.levels.push(newer);
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
            Command::Search => self.search = LineSearch::start(self.level().cursor.selected),
            Command::SearchNext => self.next_match(true, ctx),
            Command::SearchPrev => self.next_match(false, ctx),
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

    /// The search for the title: ` /pattern█` while it's typed, then which
    /// matching line is selected, of how many.
    pub fn search_title(&self) -> Option<String> {
        self.search.title(Some(self.level().cursor.selected), !self.loading())
    }

    /// Whether the search is being typed.
    pub fn searching(&self) -> bool {
        self.search.typing()
    }

    /// A key while the search is typed: each change jumps to the first
    /// match from where the search started.
    pub fn on_search_key(&mut self, key: KeyCombo, ctx: &Ctx) {
        match self.search.on_key(key, ctx) {
            SearchKey::Changed => {
                self.find_matches();
                let to = self.search.landing();
                self.select(to, ctx);
            }
            SearchKey::Cancelled(origin) => self.select(origin, ctx),
            SearchKey::Done => {}
        }
    }

    /// Works out which lines of the version shown match the search.
    fn find_matches(&mut self) {
        let shown = self.shown();
        let texts = self.levels[shown]
            .blame()
            .into_iter()
            .flat_map(|b| b.lines.iter().map(|l| l.text.as_str()));
        self.search.find(texts);
    }

    /// Goes to the next matching line after the selected one, or the one
    /// before it, round past the end.
    fn next_match(&mut self, forward: bool, ctx: &Ctx) {
        if let Some(to) = self.search.step(self.level().cursor.selected, forward, ctx) {
            self.select(to, ctx);
        }
    }

    /// Selects line `index`, bringing it a third of the way down when it's
    /// off screen.
    fn select(&mut self, index: usize, ctx: &Ctx) {
        let level = self.level_mut();
        if level.len() == 0 {
            return;
        }
        let cursor = &mut level.cursor;
        cursor.selected = index;
        if index < cursor.offset || index >= cursor.offset + cursor.height {
            level.place = Some(Place::Center);
        }
        self.want_message(ctx);
    }

    /// Selects the line on visible row `row` of the code.
    pub fn click(&mut self, row: usize, ctx: &Ctx) {
        let len = self.level().len();
        let level = self.level_mut();
        if row < level.cursor.height && level.cursor.click(row, len) {
            self.want_message(ctx);
        }
    }

    /// Drops the version shown for the one below it, bringing the selected
    /// line with it.
    fn forward(&mut self, ctx: &Ctx) {
        let older = self.levels.pop().expect("more than one level");
        let newer = self.level_mut();
        if older.blame().is_some() && newer.blame().is_some() {
            newer.cursor.selected = follow_line(&older.texts(), &newer.texts(), older.cursor.selected);
            newer.place = Some(Place::Row(older.row()));
        }
        self.find_matches();
        self.want_message(ctx);
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
        self.find_matches();
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
        let commit_rows = if self.show_commit && self.levels[self.shown()].len() > 0 {
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
        let shown = self.shown();
        let search = &self.search;
        let level = &mut self.levels[shown];
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
        match level.place.take() {
            Some(Place::Center) => {
                // A third of the way down, without scrolling past the end.
                let last_page = level.len().saturating_sub(height);
                level.cursor.offset = level.cursor.selected.saturating_sub(height / 3).min(last_page);
            }
            // Where it was, even if that leaves room below the end.
            Some(Place::Row(row)) => level.cursor.offset = level.cursor.selected.saturating_sub(row),
            None => {}
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
                let style = Style::new().fg(author_color(level.authors.get(line.commit).copied(), theme));
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
            let tokens = level.syntax.get(i).and_then(Option::as_ref);
            let text = expand_tabs(&line.text);
            let marks = search.ranges(&text);
            let code = marked_code_spans(&text, tokens, Style::new(), theme, code_width, true, &marks);
            spans.extend(code.into_iter().map(|s| Span::styled(s.content.into_owned(), s.style)));
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
        let level = &self.levels[self.shown()];
        let Some((commit, _, _)) = level.selected() else { return };
        let blame = level.blame().expect("a line is selected");
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

/// Each commit's index into the theme's author colors. Every author starts
/// from a slot hashed from their name, so they usually get the same color in
/// every file, and moves on to the next free one if another author in this
/// file already has it. Authors are placed in name order, so the result
/// doesn't depend on where in the file they appear. Only with more authors
/// than colors do two share one.
fn author_colors(blame: &Blame) -> Vec<usize> {
    let slots = Theme::default().blame_authors.len();
    let mut names: Vec<&str> = blame
        .commits
        .iter()
        .filter(|c| !c.is_uncommitted())
        .map(|c| c.author.as_str())
        .collect();
    names.sort_unstable();
    names.dedup();
    let mut taken = vec![false; slots];
    let mut slot_of = HashMap::with_capacity(names.len());
    for name in names {
        let home = author_slot(name, slots);
        let slot = (0..slots)
            .map(|step| (home + step) % slots)
            .find(|s| !taken[*s])
            .unwrap_or(home);
        taken[slot] = true;
        slot_of.insert(name, slot);
    }
    blame
        .commits
        .iter()
        .map(|c| slot_of.get(c.author.as_str()).copied().unwrap_or(0))
        .collect()
}

/// The slot an author starts from: the same for the same name, every time
/// (FNV-1a, not the std hasher, whose output may change between Rust
/// releases).
fn author_slot(author: &str, slots: usize) -> usize {
    let hash = author.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    (hash % slots as u64) as usize
}

fn author_color(slot: Option<usize>, theme: &Theme) -> Color {
    theme.blame_authors[slot.unwrap_or(0) % theme.blame_authors.len()]
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

    fn by(oid: char, author: &str) -> BlameCommit {
        BlameCommit {
            author: author.to_string(),
            ..commit(oid, 100)
        }
    }

    #[test]
    fn authors_in_a_file_get_different_colors() {
        let slots = Theme::default().blame_authors.len();
        let names: Vec<String> = (0..slots).map(|i| format!("Author {i}")).collect();
        let oids = "abcdefghijklmnopqrstuvwxyz";
        // Each author twice, so their commits share a color.
        let commits: Vec<_> = names
            .iter()
            .chain(&names)
            .zip(oids.chars().cycle())
            .map(|(name, oid)| by(oid, name))
            .collect();
        let blame = Blame {
            commits,
            ..Blame::default()
        };
        let colors = author_colors(&blame);
        let distinct: std::collections::HashSet<_> = colors[..slots].iter().collect();
        assert_eq!(distinct.len(), slots);
        assert_eq!(colors[..slots], colors[slots..]);
        assert_eq!(author_colors(&blame), colors);
    }

    #[test]
    fn an_author_keeps_their_color_without_a_clash() {
        let alone = Blame {
            commits: vec![by('a', "Ada Park")],
            ..Blame::default()
        };
        let slots = Theme::default().blame_authors.len();
        assert_eq!(author_colors(&alone), vec![author_slot("Ada Park", slots)]);
    }
}
