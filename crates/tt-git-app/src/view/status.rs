//! The status pane's view model: the changed files on screen, as a list by
//! full path or a folder tree, with what the pane says when there are none
//! and the banner of a merge or rebase in progress.

use std::ops::Range;

use tt_git_core::StatusEntry;

use super::{StyleRole, TextSpan};
use crate::panes::StatusPane;
use crate::panes::status::{Row, file_name, in_folder};
use crate::widgets::truncate;

/// What the status pane shows in a viewport, below its title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusView<'a> {
    /// A merge, rebase or cherry-pick in progress, fitted to the width, on
    /// the line above the rows. In [`StyleRole::LabelTag`].
    pub banner: Option<String>,
    /// The rows on screen, by index into [`StatusPane::rows`].
    pub range: Range<usize>,
    /// One per row in `range`, top to bottom.
    pub rows: Vec<StatusRow<'a>>,
}

/// One row of the list: a file or, in the tree, a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRow<'a> {
    /// The row's index into [`StatusPane::rows`].
    pub index: usize,
    /// It's the selected row.
    pub picked: bool,
    pub spans: Vec<TextSpan<'a>>,
}

impl StatusPane {
    /// The pane's title: `Status: ` and the [summary](StatusPane::summary).
    pub fn title(&self) -> String {
        format!("Status: {}", self.summary())
    }

    /// What the pane says instead of rows when no file is listed (in
    /// [`StyleRole::Muted`]).
    pub fn empty_message(&self) -> Option<String> {
        if !self.entries().is_empty() {
            return None;
        }
        Some(match (self.status(), self.folder()) {
            (None, _) => "loading…".to_string(),
            (Some(_), Some(f)) if !self.all_entries().is_empty() => format!("no changes in {f} (. shows all)"),
            (Some(_), _) => "working tree clean".to_string(),
        })
    }

    /// The banner of a merge, rebase or cherry-pick in progress, unfitted.
    pub fn banner(&self) -> Option<String> {
        let op = self.status()?.in_progress?;
        Some(format!("{}: stage fixes · C continue · X abort", op.label()))
    }

    /// How many lines the [banner](StatusPane::banner) takes above the
    /// rows: the body row the list starts on.
    pub fn banner_rows(&self) -> usize {
        usize::from(self.banner().is_some())
    }

    /// The rows a viewport of `rows` lines (below any banner) and `width`
    /// columns shows. Call [`set_viewport`](StatusPane::set_viewport) first.
    pub fn view(&self, rows: usize, width: usize) -> StatusView<'_> {
        let all = self.rows();
        let cursor = self.cursor();
        let start = cursor.offset.min(all.len());
        let range = start..(start + rows).min(all.len());
        StatusView {
            banner: self.banner().map(|b| truncate(&b, width)),
            rows: range
                .clone()
                .map(|index| StatusRow {
                    index,
                    picked: index == cursor.selected,
                    spans: self.row_spans(&all[index], width),
                })
                .collect(),
            range,
        }
    }

    /// One row in `width` columns: a file's status code and path (its name
    /// alone in the tree, `old → new` for a rename), or a folder's
    /// open or closed marker and name.
    pub fn row_spans(&self, row: &Row, width: usize) -> Vec<TextSpan<'static>> {
        match row {
            Row::File { index, depth } => {
                let e = &self.entries()[*index];
                let indent = "  ".repeat(*depth);
                let name = if self.tree() { file_name(e) } else { e.path.clone() };
                let path = match &e.orig_path {
                    Some(orig) => format!("{orig} → {name}"),
                    None => name,
                };
                let room = width.saturating_sub(indent.len() + 3);
                vec![
                    TextSpan::plain(indent),
                    TextSpan::new(e.short_code(), code_role([e])),
                    TextSpan::plain(" "),
                    TextSpan::plain(truncate(&path, room)),
                ]
            }
            Row::Folder { path, name, depth } => {
                let indent = "  ".repeat(*depth);
                let marker = if self.is_collapsed(path) { "▸ " } else { "▾ " };
                let role = code_role(self.entries().iter().filter(|e| in_folder(e, path)));
                let room = width.saturating_sub(indent.len() + 2);
                vec![
                    TextSpan::plain(indent),
                    TextSpan::new(marker, role),
                    TextSpan::new(truncate(&format!("{name}/"), room), role),
                ]
            }
        }
    }

    /// Every row's text as drawn at any width (without roles), for tests.
    pub fn row_texts(&self) -> Vec<String> {
        self.rows()
            .iter()
            .map(|r| self.row_spans(r, usize::MAX).iter().map(|s| s.text.as_ref()).collect())
            .collect()
    }
}

/// The role of a file's status code, or of a folder holding `entries`:
/// [`Error`](StyleRole::Error) when conflicted, else
/// [`DiffDel`](StyleRole::DiffDel) with something left to stage, else
/// [`DiffAdd`](StyleRole::DiffAdd) (all staged).
pub fn code_role<'a>(entries: impl IntoIterator<Item = &'a StatusEntry>) -> StyleRole {
    let (mut conflicted, mut unstaged) = (false, false);
    for e in entries {
        conflicted |= e.is_conflicted();
        unstaged |= !e.has_staged() || e.has_unstaged();
    }
    if conflicted {
        StyleRole::Error
    } else if unstaged {
        StyleRole::DiffDel
    } else {
        StyleRole::DiffAdd
    }
}

#[cfg(test)]
mod tests {
    use tt_git_core::{BranchStatus, FileState, InProgress, Status};
    use unicode_width::UnicodeWidthStr;

    use super::*;
    use crate::component::Component;
    use crate::jobs::Ctx;
    use crate::keymap::Command;

    fn ctx() -> Ctx {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        Ctx::new(tx)
    }

    fn entry(path: &str, index: FileState, worktree: FileState) -> StatusEntry {
        StatusEntry {
            path: path.into(),
            orig_path: None,
            index,
            worktree,
        }
    }

    fn modified(path: &str) -> StatusEntry {
        entry(path, FileState::Unmodified, FileState::Modified)
    }

    fn staged(path: &str) -> StatusEntry {
        entry(path, FileState::Modified, FileState::Unmodified)
    }

    fn pane(entries: Vec<StatusEntry>) -> StatusPane {
        let mut pane = StatusPane::new("repo".into());
        pane.set_status(Status {
            branch: BranchStatus {
                head: Some("main".into()),
                ..BranchStatus::default()
            },
            entries,
            in_progress: None,
        });
        pane
    }

    fn text(spans: &[TextSpan]) -> String {
        spans.iter().map(|s| s.text.as_ref()).collect()
    }

    fn texts(view: &StatusView) -> Vec<String> {
        view.rows.iter().map(|r| text(&r.spans)).collect()
    }

    #[test]
    fn empty_panes_say_why() {
        let mut p = StatusPane::new("repo".into());
        assert_eq!(p.empty_message().as_deref(), Some("loading…"));
        assert_eq!(p.title(), "Status: repo");
        p.set_status(Status::default());
        assert_eq!(p.empty_message().as_deref(), Some("working tree clean"));

        let mut p = pane(vec![modified("src/a.rs")]);
        assert_eq!(p.empty_message(), None);
        p.set_folder(Some("docs/".into()));
        assert_eq!(p.empty_message().as_deref(), Some("no changes in docs/ (. shows all)"));
        assert_eq!(p.title(), "Status: repo · main · in docs/ · +1 outside");
    }

    #[test]
    fn files_show_their_code_and_path_in_its_role() {
        let mut renamed = staged("new.rs");
        renamed.index = FileState::Renamed;
        renamed.orig_path = Some("old.rs".into());
        let p = pane(vec![
            modified("a.rs"),
            staged("b.rs"),
            entry("c.rs", FileState::Unmerged, FileState::Unmerged),
            entry("d.rs", FileState::Untracked, FileState::Untracked),
            entry("e.rs", FileState::Modified, FileState::Modified),
            renamed,
        ]);
        let view = p.view(10, 80);
        assert_eq!(view.banner, None);
        assert_eq!(
            texts(&view),
            [
                " M a.rs",
                "M  b.rs",
                "UU c.rs",
                "?? d.rs",
                "MM e.rs",
                "R  old.rs → new.rs"
            ]
        );
        let roles: Vec<StyleRole> = view.rows.iter().map(|r| r.spans[1].role).collect();
        use StyleRole::*;
        assert_eq!(roles, [DiffDel, DiffAdd, Error, DiffDel, DiffDel, DiffAdd]);
        assert!(view.rows.iter().all(|r| r.spans[3].role == Plain));
        assert_eq!(p.row_texts(), texts(&view));
    }

    #[test]
    fn rows_are_cut_to_the_width() {
        let p = pane(vec![modified("a/very/long/path/to/a/file.rs")]);
        for width in [30, 12, 5, 3, 0] {
            let row = text(&p.view(1, width).rows[0].spans);
            assert!(row.width() <= width.max(3), "{row:?} in {width}");
        }
        assert_eq!(text(&p.view(1, 12).rows[0].spans), " M a/very/l…");
    }

    #[test]
    fn the_tree_shows_folders_with_the_files_role() {
        let mut p = pane(vec![
            modified("src/app/a.rs"),
            staged("src/app/b.rs"),
            staged("docs/x.md"),
            modified("top.txt"),
        ]);
        p.set_tree(true);
        let view = p.view(10, 80);
        assert_eq!(
            texts(&view),
            [
                "▾ docs/",
                "  M  x.md",
                "▾ src/app/",
                "   M a.rs",
                "  M  b.rs",
                " M top.txt"
            ]
        );
        // A folder is all staged, or has something to stage.
        assert_eq!(view.rows[0].spans[1].role, StyleRole::DiffAdd);
        assert_eq!(view.rows[0].spans[2].role, StyleRole::DiffAdd);
        assert_eq!(view.rows[2].spans[1].role, StyleRole::DiffDel);

        // The selection stayed on the file; enter on a folder collapses it.
        assert!(view.rows[3].picked);
        let ctx = ctx();
        p.on_command(Command::Top, &ctx);
        p.on_command(Command::OpenStaging, &ctx);
        let view = p.view(10, 80);
        assert_eq!(texts(&view)[..2], ["▸ docs/", "▾ src/app/"]);
        assert!(view.rows[0].picked);
    }

    #[test]
    fn view_scrolls_with_the_selection() {
        let mut p = pane((0..10).map(|i| modified(&format!("f{i}"))).collect());
        p.set_viewport(3);
        let view = p.view(3, 40);
        assert_eq!(view.range, 0..3);
        assert_eq!(
            view.rows.iter().map(|r| r.picked).collect::<Vec<_>>(),
            [true, false, false]
        );

        let ctx = ctx();
        for _ in 0..5 {
            p.on_command(Command::MoveDown, &ctx);
        }
        p.set_viewport(3);
        let view = p.view(3, 40);
        assert_eq!(view.range, 3..6);
        assert_eq!(view.rows.iter().map(|r| r.index).collect::<Vec<_>>(), [3, 4, 5]);
        assert!(view.rows[2].picked);

        // Past the end there are only the rows left.
        p.on_command(Command::Bottom, &ctx);
        p.set_viewport(3);
        assert_eq!(p.view(5, 40).range, 7..10);
    }

    #[test]
    fn an_operation_in_progress_shows_a_banner() {
        let mut p = pane(vec![entry("c.rs", FileState::Unmerged, FileState::Unmerged)]);
        assert_eq!(p.banner_rows(), 0);
        let mut status = p.status().unwrap().clone();
        status.in_progress = Some(InProgress::Merge);
        p.set_status(status);
        assert_eq!(p.banner_rows(), 1);
        let view = p.view(5, 80);
        assert_eq!(
            view.banner.as_deref(),
            Some("MERGING: stage fixes · C continue · X abort")
        );
        assert_eq!(p.view(5, 10).banner.as_deref(), Some("MERGING: …"));
        assert!(p.title().contains("MERGING"), "{}", p.title());
    }
}
