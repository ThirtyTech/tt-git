//! The graph's filter panel: pick an author or a file and the graph shows
//! only the commits by them, or that changed it.

use std::collections::HashSet;

use tt_git_core::LogFilter;
use tt_git_core::branches::matches_filter;

use crate::keymap::{KeyCode, KeyCombo, KeyModifiers};
use crate::modal::ModalOutcome;
use crate::mouse::{FilterSpot, MouseKind};

/// Rows of choices shown at once.
pub const ROWS: usize = 12;
/// The row inside the border the choices start on, under the tabs, the
/// input and a blank line.
pub const FIRST_CHOICE_ROW: usize = 3;

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

    pub fn index(self) -> usize {
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
pub struct List {
    /// `None` while loading.
    pub items: Option<Result<Vec<Choice>, String>>,
    pub input: String,
    /// Indices into `items` that match `input`, best first.
    pub matches: Vec<usize>,
    pub selected: usize,
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
    pub fn first_shown(&self) -> usize {
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
    pub tab: FilterTab,
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

    pub fn list(&self) -> &List {
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
    /// moves the selection.
    pub fn handle_mouse(&mut self, kind: MouseKind, spot: FilterSpot) -> ModalOutcome {
        match (kind, spot) {
            (MouseKind::Click, FilterSpot::Outside) => ModalOutcome::Cancelled,
            (MouseKind::Click, FilterSpot::Nothing) => ModalOutcome::Open,
            (MouseKind::Click, FilterSpot::Tab(tab)) => {
                self.tab = tab;
                ModalOutcome::Open
            }
            (MouseKind::Click, FilterSpot::Choice(shown)) => {
                let list = self.list_mut();
                if shown >= ROWS {
                    return ModalOutcome::Open;
                }
                let picked = list.first_shown() + shown;
                if picked >= list.matches.len() {
                    return ModalOutcome::Open;
                }
                list.selected = picked;
                self.submit().map_or(ModalOutcome::Open, ModalOutcome::Filter)
            }
            (MouseKind::ScrollUp | MouseKind::ScrollDown, _) => {
                self.list_mut().step(kind == MouseKind::ScrollUp, 3);
                ModalOutcome::Open
            }
            (MouseKind::Other, _) => ModalOutcome::Open,
        }
    }

    /// A tab's name and what the filter has for it now.
    pub fn tab_text(&self, tab: FilterTab) -> String {
        let now = match (tab, tab.value(&self.current)) {
            (_, None) => "any",
            (FilterTab::Author, Some(ident)) => author_name(ident),
            (FilterTab::File, Some(path)) => path,
        };
        format!("{}: {now}", tab.label())
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
            message: None,
            ..LogFilter::default()
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
                message: None,
                ..LogFilter::default()
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
                message: None,
                ..LogFilter::default()
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
                message: None,
                ..LogFilter::default()
            })
        );
    }

    #[test]
    fn clear_entry_leads_when_set() {
        let current = LogFilter {
            author: Some("Bob <bob@x.io>".into()),
            path: Some("a.txt".into()),
            message: None,
            ..LogFilter::default()
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
                message: None,
                ..LogFilter::default()
            })
        );
        assert_eq!(p.handle_key(k("esc")), ModalOutcome::Cancelled);
    }
}
