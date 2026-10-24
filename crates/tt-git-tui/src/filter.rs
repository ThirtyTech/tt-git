//! The graph's filter panel: pick an author or a file and the graph shows
//! only the commits by them, or that changed it.

use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use tt_git_core::LogFilter;
use tt_git_core::branches::matches_filter;
use unicode_width::UnicodeWidthStr;

use crate::keymap::KeyCombo;
use crate::modal::ModalOutcome;
use crate::theme::Theme;
use crate::widgets::{fit, truncate};

/// Rows of choices shown at once.
const ROWS: usize = 12;
/// The row inside the border the choices start on, under the tabs, the
/// input and a blank line.
const FIRST_CHOICE_ROW: usize = 3;

/// The panel's two lists, switched with `tab`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterTab {
    Author,
    File,
}

impl FilterTab {
    pub const ALL: [FilterTab; 2] = [FilterTab::Author, FilterTab::File];

    fn label(self) -> &'static str {
        match self {
            FilterTab::Author => "Author",
            FilterTab::File => "File",
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    /// The part of `filter` this tab sets.
    fn value(self, filter: &LogFilter) -> Option<&String> {
        match self {
            FilterTab::Author => filter.author.as_ref(),
            FilterTab::File => filter.path.as_ref(),
        }
    }

    fn value_mut(self, filter: &mut LogFilter) -> &mut Option<String> {
        match self {
            FilterTab::Author => &mut filter.author,
            FilterTab::File => &mut filter.path,
        }
    }
}

/// One entry in a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// What the filter is set to; `None` clears that part of it.
    pub value: Option<String>,
    pub label: String,
    /// Shown dimmed on the right, e.g. a commit count.
    pub hint: String,
}

/// Authors from [`Repo::authors`](tt_git_core::Repo::authors), with their
/// commit counts as hints.
pub fn author_choices(authors: Vec<(String, usize)>) -> Vec<Choice> {
    authors
        .into_iter()
        .map(|(ident, count)| Choice {
            value: Some(ident.clone()),
            label: ident,
            hint: count.to_string(),
        })
        .collect()
}

/// Tracked files, each directory listed (with a trailing `/`) before the
/// first file in it.
pub fn file_choices(files: Vec<String>) -> Vec<Choice> {
    let mut dirs = HashSet::new();
    let mut choices = Vec::with_capacity(files.len());
    for file in files {
        for (i, _) in file.match_indices('/') {
            let dir = &file[..=i];
            if dirs.insert(dir.to_string()) {
                choices.push(Choice {
                    value: Some(dir.to_string()),
                    label: dir.to_string(),
                    hint: String::new(),
                });
            }
        }
        choices.push(Choice {
            value: Some(file.clone()),
            label: file,
            hint: String::new(),
        });
    }
    choices
}

/// `Name` from `Name <email>`.
pub fn author_name(ident: &str) -> &str {
    ident.rsplit_once(" <").map_or(ident, |(name, _)| name)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct List {
    /// `None` while loading.
    items: Option<Result<Vec<Choice>, String>>,
    input: String,
    /// Indices into `items` that match `input`, best first.
    matches: Vec<usize>,
    selected: usize,
}

impl List {
    fn items(&self) -> &[Choice] {
        match &self.items {
            Some(Ok(items)) => items,
            _ => &[],
        }
    }

    /// Labels containing the input come first, then ones that only match
    /// it as a subsequence.
    fn refilter(&mut self) {
        let needle = self.input.to_lowercase();
        let mut ranked: Vec<(bool, usize)> = self
            .items()
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                if c.label.to_lowercase().contains(&needle) {
                    Some((false, i))
                } else {
                    matches_filter(&c.label, &self.input).then_some((true, i))
                }
            })
            .collect();
        ranked.sort_by_key(|(loose, _)| *loose);
        self.matches = ranked.into_iter().map(|(_, i)| i).collect();
        self.selected = 0;
    }

    fn selected(&self) -> Option<&Choice> {
        self.matches.get(self.selected).map(|&i| &self.items()[i])
    }

    /// The position in `matches` of the first choice shown, scrolled so
    /// the selection is in view.
    fn first_shown(&self) -> usize {
        self.selected.saturating_sub(ROWS - 1)
    }

    fn step(&mut self, up: bool, by: usize) {
        let last = self.matches.len().saturating_sub(1);
        self.selected = if up {
            self.selected.saturating_sub(by)
        } else {
            (self.selected + by).min(last)
        };
    }
}

/// The filter panel's state. Lists start out loading; the app fills them
/// in with [`set_choices`](Self::set_choices).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterPanel {
    /// The graph's filter when the panel opened.
    current: LogFilter,
    /// The folder tt-git was started in, offered first on the file tab.
    folder: Option<String>,
    tab: FilterTab,
    lists: [List; 2],
}

impl FilterPanel {
    pub fn new(current: LogFilter, folder: Option<String>) -> Self {
        // Open on the file tab when only a file is filtered, to change it.
        let tab = if current.author.is_none() && current.path.is_some() {
            FilterTab::File
        } else {
            FilterTab::Author
        };
        Self {
            current,
            folder,
            tab,
            lists: Default::default(),
        }
    }

    pub fn tab(&self) -> FilterTab {
        self.tab
    }

    /// Fills in `tab`'s list, led by an entry that clears its part of the
    /// filter if that part is set, and on the file tab by the folder tt-git
    /// was started in.
    pub fn set_choices(&mut self, tab: FilterTab, choices: Result<Vec<Choice>, String>) {
        let clear = tab.value(&self.current).is_some().then(|| Choice {
            value: None,
            label: match tab {
                FilterTab::Author => "Any author".into(),
                FilterTab::File => "Any file".into(),
            },
            hint: "clear".into(),
        });
        let folder = self.folder.clone().filter(|_| tab == FilterTab::File);
        let here = folder.clone().map(|f| Choice {
            value: Some(f.clone()),
            label: f,
            hint: "current folder".into(),
        });
        let list = &mut self.lists[tab.index()];
        list.items = Some(choices.map(|c| {
            let rest = c.into_iter().filter(|c| folder.is_none() || c.value != folder);
            clear.into_iter().chain(here).chain(rest).collect()
        }));
        list.refilter();
    }

    fn list(&self) -> &List {
        &self.lists[self.tab.index()]
    }

    fn list_mut(&mut self) -> &mut List {
        &mut self.lists[self.tab.index()]
    }

    /// The filter to apply on enter, if any: the selected choice, or on the
    /// file tab a typed path nothing matches (so files not in the index,
    /// such as deleted ones, still work).
    fn submit(&self) -> Option<LogFilter> {
        let list = self.list();
        let value = match list.selected() {
            Some(choice) => choice.value.clone(),
            None if self.tab == FilterTab::File && !list.input.trim().is_empty() => Some(list.input.trim().to_string()),
            None => return None,
        };
        let mut filter = self.current.clone();
        *self.tab.value_mut(&mut filter) = value;
        Some(filter)
    }

    pub fn handle_key(&mut self, key: KeyCombo) -> ModalOutcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return ModalOutcome::Cancelled,
            KeyCode::Enter => {
                return match self.submit() {
                    Some(filter) => ModalOutcome::Filter(filter),
                    None => ModalOutcome::Open,
                };
            }
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                let n = FilterTab::ALL.len();
                self.tab = FilterTab::ALL[(self.tab.index() + 1) % n];
            }
            KeyCode::Up => self.list_mut().step(true, 1),
            KeyCode::Down => self.list_mut().step(false, 1),
            KeyCode::Char('p') if ctrl => self.list_mut().step(true, 1),
            KeyCode::Char('n') if ctrl => self.list_mut().step(false, 1),
            KeyCode::PageUp => self.list_mut().step(true, ROWS),
            KeyCode::PageDown => self.list_mut().step(false, ROWS),
            KeyCode::Char('u') if ctrl => {
                self.list_mut().input.clear();
                self.list_mut().refilter();
            }
            KeyCode::Backspace => {
                self.list_mut().input.pop();
                self.list_mut().refilter();
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.list_mut().input.push(c);
                self.list_mut().refilter();
            }
            _ => {}
        }
        ModalOutcome::Open
    }

    /// A click on a choice applies it as enter would, a click on a tab
    /// switches to it, and a click outside the panel closes it. The wheel
    /// moves the selection. `area` is what [`draw`](Self::draw) was given.
    pub fn handle_mouse(&mut self, m: MouseEvent, area: Rect) -> ModalOutcome {
        let popup = popup_area(area);
        let pos = Position::new(m.column, m.row);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if !popup.contains(pos) => ModalOutcome::Cancelled,
            MouseEventKind::Down(MouseButton::Left) => {
                let inner = Block::bordered().inner(popup);
                if !inner.contains(pos) {
                    return ModalOutcome::Open;
                }
                let row = (pos.y - inner.y) as usize;
                if row == 0 {
                    let mut x = inner.x;
                    for tab in FilterTab::ALL {
                        if tab.index() > 0 {
                            x += 3;
                        }
                        let width = self.tab_text(tab).width() as u16;
                        if (x..x + width).contains(&pos.x) {
                            self.tab = tab;
                        }
                        x += width;
                    }
                    return ModalOutcome::Open;
                }
                let list = self.list_mut();
                let Some(shown) = row.checked_sub(FIRST_CHOICE_ROW).filter(|&r| r < ROWS) else {
                    return ModalOutcome::Open;
                };
                let picked = list.first_shown() + shown;
                if picked >= list.matches.len() {
                    return ModalOutcome::Open;
                }
                list.selected = picked;
                self.submit().map_or(ModalOutcome::Open, ModalOutcome::Filter)
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                self.list_mut().step(m.kind == MouseEventKind::ScrollUp, 3);
                ModalOutcome::Open
            }
            _ => ModalOutcome::Open,
        }
    }

    /// A tab's name and what the filter has for it now.
    fn tab_text(&self, tab: FilterTab) -> String {
        let now = match (tab, tab.value(&self.current)) {
            (_, None) => "any",
            (FilterTab::Author, Some(ident)) => author_name(ident),
            (FilterTab::File, Some(path)) => path,
        };
        format!("{}: {now}", tab.label())
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let popup = popup_area(area);
        frame.render_widget(Clear, popup);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(theme.border_focused)
            .title(Span::styled(" Filter graph ", theme.title_focused));
        let inner_width = popup.width.saturating_sub(2) as usize;
        let list = self.list();

        let mut tabs = Vec::new();
        for tab in FilterTab::ALL {
            if !tabs.is_empty() {
                tabs.push(Span::styled(" · ", theme.muted));
            }
            let style = if tab == self.tab {
                theme.help_key.add_modifier(Modifier::UNDERLINED)
            } else {
                theme.muted
            };
            tabs.push(Span::styled(self.tab_text(tab), style));
        }

        let count = match &list.items {
            Some(Ok(items)) => format!("{}/{}", list.matches.len(), items.len()),
            _ => String::new(),
        };
        let input = truncate(&list.input, inner_width.saturating_sub(count.width() + 4));
        let pad = inner_width.saturating_sub(input.width() + count.width() + 3);
        let mut lines = vec![
            Line::from(tabs),
            Line::from(vec![
                Span::styled("> ", theme.help_key),
                Span::raw(input),
                Span::styled("█", theme.help_key),
                Span::raw(" ".repeat(pad)),
                Span::styled(count, theme.muted),
            ]),
            Line::raw(""),
        ];

        match &list.items {
            None => lines.push(Line::styled("loading…", theme.muted)),
            Some(Err(e)) => lines.push(Line::styled(e.clone(), theme.error)),
            Some(Ok(_)) if list.matches.is_empty() => {
                let typed = list.input.trim();
                let msg = if self.tab == FilterTab::File && !typed.is_empty() {
                    format!("no matches · enter shows commits that changed {typed}")
                } else {
                    "no matches".into()
                };
                lines.push(Line::styled(msg, theme.muted));
            }
            Some(Ok(items)) => {
                for (pos, &i) in list.matches.iter().enumerate().skip(list.first_shown()).take(ROWS) {
                    let choice = &items[i];
                    let hint_w = choice.hint.width();
                    let label_w = inner_width.saturating_sub(hint_w + 1);
                    let label_style = if choice.value.is_none() {
                        theme.muted
                    } else {
                        Style::new()
                    };
                    let line = Line::from(vec![
                        Span::styled(fit(&choice.label, label_w), label_style),
                        Span::raw(" "),
                        Span::styled(choice.hint.clone(), theme.muted),
                    ]);
                    lines.push(if pos == list.selected {
                        line.patch_style(theme.selected)
                    } else {
                        line
                    });
                }
            }
        }

        // Keep the hint on the last row.
        let body_rows = popup.height.saturating_sub(2) as usize;
        while lines.len() + 1 < body_rows {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(
            "enter filter · tab author/file · ↑↓ pick · esc cancel",
            theme.muted,
        ));
        frame.render_widget(Paragraph::new(lines).block(block), popup);
    }
}

/// Where the panel goes in `area`: a fixed height, so it doesn't jump
/// about while typing.
fn popup_area(area: Rect) -> Rect {
    let width = 70.min(area.width);
    let height = (ROWS as u16 + 7).min(area.height);
    Rect {
        y: area.y + area.height.saturating_sub(height) / 4,
        ..crate::app::centered(area, width, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> KeyCombo {
        s.parse().unwrap()
    }

    fn type_text(p: &mut FilterPanel, text: &str) {
        for c in text.chars() {
            p.handle_key(KeyCombo::char(c));
        }
    }

    #[test]
    fn files_list_their_directories_first() {
        let labels: Vec<String> = file_choices(vec!["a.txt".into(), "src/app/x.rs".into(), "src/y.rs".into()])
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert_eq!(labels, ["a.txt", "src/", "src/app/", "src/app/x.rs", "src/y.rs"]);
    }

    #[test]
    fn picks_an_author_keeping_the_file() {
        let current = LogFilter {
            author: None,
            path: Some("src/".into()),
        };
        let mut p = FilterPanel::new(current, None);
        assert_eq!(p.tab(), FilterTab::File, "opens on the part that's set");
        p.handle_key(k("tab"));
        assert_eq!(p.handle_key(k("enter")), ModalOutcome::Open, "nothing loaded yet");
        p.set_choices(
            FilterTab::Author,
            Ok(author_choices(vec![
                ("Bob <bob@x.io>".into(), 3),
                ("Alice <alice@x.io>".into(), 1),
            ])),
        );
        type_text(&mut p, "alice");
        assert_eq!(
            p.handle_key(k("enter")),
            ModalOutcome::Filter(LogFilter {
                author: Some("Alice <alice@x.io>".into()),
                path: Some("src/".into()),
            })
        );
    }

    #[test]
    fn substring_matches_rank_before_subsequences() {
        let mut p = FilterPanel::new(LogFilter::default(), None);
        p.handle_key(k("tab"));
        p.set_choices(
            FilterTab::File,
            Ok(file_choices(vec!["mxaxixn.rs".into(), "src/main.rs".into()])),
        );
        type_text(&mut p, "main");
        let labels: Vec<&str> = p
            .list()
            .matches
            .iter()
            .map(|&i| p.list().items()[i].label.as_str())
            .collect();
        assert_eq!(labels, ["src/main.rs", "mxaxixn.rs"]);
        p.handle_key(k("ctrl+u"));
        type_text(&mut p, "srcmain");
        assert_eq!(p.list().selected().unwrap().label, "src/main.rs");
    }

    #[test]
    fn typed_path_without_a_match_is_used_as_is() {
        let mut p = FilterPanel::new(LogFilter::default(), None);
        p.handle_key(k("tab"));
        p.set_choices(FilterTab::File, Ok(file_choices(vec!["a.txt".into()])));
        type_text(&mut p, "gone/old.rs");
        assert_eq!(
            p.handle_key(k("enter")),
            ModalOutcome::Filter(LogFilter {
                author: None,
                path: Some("gone/old.rs".into()),
            })
        );
    }

    #[test]
    fn current_folder_comes_first_on_the_file_tab() {
        let mut p = FilterPanel::new(LogFilter::default(), Some("src/app/".into()));
        p.handle_key(k("tab"));
        p.set_choices(
            FilterTab::File,
            Ok(file_choices(vec!["a.txt".into(), "src/app/x.rs".into()])),
        );
        let labels: Vec<(&str, &str)> = p
            .list()
            .items()
            .iter()
            .map(|c| (c.label.as_str(), c.hint.as_str()))
            .collect();
        assert_eq!(
            labels,
            [
                ("src/app/", "current folder"),
                ("a.txt", ""),
                ("src/", ""),
                ("src/app/x.rs", "")
            ],
            "listed once, first"
        );
        assert_eq!(
            p.handle_key(k("enter")),
            ModalOutcome::Filter(LogFilter {
                author: None,
                path: Some("src/app/".into()),
            })
        );
    }

    #[test]
    fn clear_entry_leads_when_set() {
        let current = LogFilter {
            author: Some("Bob <bob@x.io>".into()),
            path: Some("a.txt".into()),
        };
        let mut p = FilterPanel::new(current, None);
        p.set_choices(
            FilterTab::Author,
            Ok(author_choices(vec![("Bob <bob@x.io>".into(), 3)])),
        );
        assert_eq!(p.list().selected().unwrap().label, "Any author");
        assert_eq!(
            p.handle_key(k("enter")),
            ModalOutcome::Filter(LogFilter {
                author: None,
                path: Some("a.txt".into()),
            })
        );
        assert_eq!(p.handle_key(k("esc")), ModalOutcome::Cancelled);
    }
}
