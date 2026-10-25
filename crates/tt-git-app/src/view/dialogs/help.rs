//! The keys popup (`?`): every key for the focused pane and the global
//! ones, narrowed to what's typed.

use super::{Line, cursor};
use crate::App;
use crate::view::{StyleRole, TextSpan};

pub const HELP_TITLE: &str = "Keybindings";
/// The column the keys are right-aligned in, before what they do.
pub const HELP_KEYS_WIDTH: usize = 16;

/// What the keys popup shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpView {
    /// `tt-git` and its version, for the top right.
    pub version: String,
    /// Columns across, the border included.
    pub width: usize,
    /// Rows down, the border included: sized for the whole list, so the
    /// popup holds still while typing.
    pub height: usize,
    /// `> ` and what's typed, or a muted `type to search`.
    pub search: Line<'static>,
    /// A key and what it does, per row: the keys right-aligned in
    /// [`HELP_KEYS_WIDTH`] columns.
    pub rows: Vec<Line<'static>>,
    /// Said when nothing matches.
    pub empty: Option<&'static str>,
}

impl App {
    /// What the keys popup shows.
    pub fn help_view(&self) -> HelpView {
        let query = self.help_query();
        let search = if query.is_empty() {
            vec![
                TextSpan::new("> ", StyleRole::HelpKey),
                TextSpan::new("type to search", StyleRole::Muted),
            ]
        } else {
            vec![
                TextSpan::new("> ", StyleRole::HelpKey),
                TextSpan::plain(query.to_string()),
                cursor(),
            ]
        };
        let rows: Vec<Line<'static>> = self
            .help_rows()
            .into_iter()
            .map(|(keys, label)| {
                vec![
                    TextSpan::new(format!("{keys:>HELP_KEYS_WIDTH$}  "), StyleRole::HelpKey),
                    TextSpan::new(label, StyleRole::HelpText),
                ]
            })
            .collect();
        HelpView {
            version: concat!("tt-git ", env!("CARGO_PKG_VERSION")).into(),
            width: 50,
            height: self.all_help_row_count() + 3,
            search,
            empty: rows.is_empty().then_some("no matching keys"),
            rows,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tt_git_core::time::FixedClock;
    use tt_git_core::{GitCli, Repo};
    use tt_git_testkit::{NOW, Sandbox};

    use super::*;
    use crate::Config;
    use crate::keymap::KeyCombo;
    use crate::view::dialogs::line_text;

    fn app(sb: &Sandbox) -> App {
        let git = GitCli::new(sb.path()).with_env(sb.env());
        let repo = Repo::discover(sb.path(), &git).unwrap();
        App::new(Config::default(), repo, Arc::new(FixedClock(NOW))).unwrap().0
    }

    #[test]
    fn lists_every_key_then_narrows_to_the_typed_ones() {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let mut app = app(&sb);
        app.handle_key(KeyCombo::char('?'));
        assert!(app.shows_help());
        let view = app.help_view();
        assert_eq!(view.width, 50);
        assert_eq!(view.height, app.all_help_row_count() + 3);
        assert_eq!(view.rows.len(), app.all_help_row_count());
        assert_eq!(line_text(&view.search), "> type to search");
        assert_eq!(view.search[1].role, StyleRole::Muted);
        assert!(view.version.starts_with("tt-git "));
        let quit = view.rows.iter().find(|r| r[1].text == "quit").expect("a quit row");
        assert_eq!(quit[0].text.len(), HELP_KEYS_WIDTH + 2, "keys right-aligned");
        assert_eq!((quit[0].role, quit[1].role), (StyleRole::HelpKey, StyleRole::HelpText));

        for c in "zzqx".chars() {
            app.handle_key(KeyCombo::char(c));
        }
        let view = app.help_view();
        assert_eq!(line_text(&view.search), "> zzqx█");
        assert!(view.rows.is_empty());
        assert_eq!(view.empty, Some("no matching keys"));
        assert_eq!(view.height, app.all_help_row_count() + 3, "holds its size");
    }
}
