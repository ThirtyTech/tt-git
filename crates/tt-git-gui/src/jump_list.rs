//! The Windows taskbar's jump list: right-clicking tt-git on the taskbar
//! (or its Start menu entry) lists the recent repositories, each opening
//! with `tt-git-gui <path>`, and the tasks (Open repository…).
//!
//! What goes in the list is worked out here, on every OS, so it's tested
//! anywhere; `tt_git_win::jump_list` hands it to Windows. It's
//! rebuilt at startup and whenever the recent repositories change
//! ([`window_state::remember_repo`], [`window_state::forget_repo`]). Only
//! `main` calls [`install`], so tests never touch the shell. Elsewhere than
//! Windows, [`refresh`] does nothing.
//!
//! The list uses the process's AppUserModelID (set at startup), so it stays
//! with the same taskbar button as the windows.

use std::path::{Path, PathBuf};

use gpui::{App, Global};

use crate::window_state;

/// The heading over the recent repositories.
pub const RECENT_CATEGORY: &str = "Recent repositories";

/// The flag the Open repository… task starts `tt-git-gui` with.
pub const OPEN_REPOSITORY_FLAG: &str = "--open-repository";

/// A recent repository in the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    /// The folder's name, with its parent's when another entry has the same
    /// name.
    pub title: String,
    /// The whole path, shown when the pointer rests on the entry.
    pub tooltip: String,
    /// The command line after `tt-git-gui`: the path, quoted.
    pub arguments: String,
}

/// A task in the list's Tasks section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub title: &'static str,
    pub arguments: &'static str,
}

/// The tasks, in order.
pub const TASKS: &[Task] = &[Task {
    title: "Open repository…",
    arguments: OPEN_REPOSITORY_FLAG,
}];

/// The entries for `recent` (most recent first): the folders that still
/// exist, at most `slots` of them.
pub fn entries(recent: &[PathBuf], slots: usize, exists: impl Fn(&Path) -> bool) -> Vec<Entry> {
    let kept: Vec<&PathBuf> = recent.iter().filter(|p| exists(p)).take(slots).collect();
    let names: Vec<String> = kept.iter().map(|p| name(p)).collect();
    kept.iter()
        .zip(&names)
        .map(|(path, name)| {
            let shared = names.iter().filter(|n| n.eq_ignore_ascii_case(name)).count() > 1;
            let title = match plain(path).parent().and_then(Path::file_name) {
                Some(parent) if shared => format!("{name} ({})", parent.to_string_lossy()),
                _ => name.clone(),
            };
            Entry {
                path: path.to_path_buf(),
                title,
                tooltip: plain(path).display().to_string(),
                arguments: arguments(path),
            }
        })
        .collect()
}

/// The repositories in `recent` whose entries the user removed from the
/// list: Windows hands back the removed entries' `removed` command lines.
pub fn removed(recent: &[PathBuf], removed: &[String]) -> Vec<PathBuf> {
    recent
        .iter()
        .filter(|p| removed.contains(&arguments(p)))
        .cloned()
        .collect()
}

/// The command line that opens `path`.
pub fn arguments(path: &Path) -> String {
    quote(&plain(path).to_string_lossy())
}

/// `path` without Windows' `\\?\`.
fn plain(path: &Path) -> &Path {
    path.to_str()
        .and_then(|s| s.strip_prefix(r"\\?\"))
        .map_or(path, Path::new)
}

/// The folder's name, or the whole path for a drive's root.
fn name(path: &Path) -> String {
    let path = plain(path);
    path.file_name()
        .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// `arg` quoted for a Windows command line, as `CommandLineToArgvW` (and
/// Rust's `std::env::args`) reads it back: in double quotes, with a quote
/// and the backslashes before it (or before the closing quote) doubled.
pub fn quote(arg: &str) -> String {
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut slashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
                slashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', slashes));
                slashes = 0;
            }
        }
        if c != '\\' {
            out.push(c);
        }
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}

/// What was last put in the list, once [`install`]ed.
#[derive(Default)]
struct JumpList {
    /// The recent repositories the list was last built from.
    built: Option<Vec<PathBuf>>,
    /// A rebuild is under way (forgetting a removed entry asks for another).
    building: bool,
}

impl Global for JumpList {}

/// Builds the list now and keeps it up to date from now on (call after
/// [`window_state::install`]).
pub fn install(cx: &mut App) {
    cx.set_global(JumpList::default());
    refresh(cx);
}

/// Rebuilds the list if the recent repositories changed since it was last
/// built. Does nothing without [`install`], or elsewhere than Windows.
pub fn refresh(cx: &mut App) {
    let Some(list) = cx.try_global::<JumpList>() else {
        return;
    };
    let recent = window_state::state(cx).recent;
    if list.building || list.built.as_ref() == Some(&recent) {
        return;
    }
    cx.global_mut::<JumpList>().building = true;
    let built = build(recent, cx);
    let list = cx.global_mut::<JumpList>();
    list.building = false;
    list.built = built;
}

/// Hands the list to Windows. The recent repositories it was built from,
/// or `None` when it couldn't be (to try again next time).
#[cfg(windows)]
fn build(recent: Vec<PathBuf>, cx: &mut App) -> Option<Vec<PathBuf>> {
    use tt_git_win::jump_list::{Link, Session};

    let session = match Session::begin() {
        Ok(session) => session,
        Err(e) => {
            tracing::warn!("couldn't start the jump list: {e}");
            return None;
        }
    };
    let gone = removed(&recent, session.removed());
    for path in &gone {
        tracing::info!("{} was removed from the jump list", path.display());
        window_state::forget_repo(path, cx);
    }
    let recent = window_state::state(cx).recent;
    let list = entries(&recent, session.slots(), Path::is_dir);
    let links: Vec<Link> = list
        .iter()
        .map(|e| Link {
            arguments: &e.arguments,
            title: &e.title,
            tooltip: &e.tooltip,
        })
        .collect();
    let tasks: Vec<Link> = TASKS
        .iter()
        .map(|t| Link {
            arguments: t.arguments,
            title: t.title,
            tooltip: t.title,
        })
        .collect();
    match session.commit(RECENT_CATEGORY, &links, &tasks) {
        Ok(refused) => {
            if let Some(e) = refused {
                tracing::info!("the jump list's recent repositories were refused: {e}");
            }
            tracing::debug!("jump list: {} recent repositories", list.len());
            Some(recent)
        }
        Err(e) => {
            tracing::warn!("couldn't write the jump list: {e}");
            None
        }
    }
}

#[cfg(not(windows))]
fn build(recent: Vec<PathBuf>, _: &mut App) -> Option<Vec<PathBuf>> {
    Some(recent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    fn titles(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.title.as_str()).collect()
    }

    #[test]
    fn recent_repositories_keep_their_order_and_skip_missing_folders() {
        let recent = paths(&["/src/b", "/src/gone", "/src/a"]);
        let list = entries(&recent, 10, |p| !p.ends_with("gone"));
        assert_eq!(titles(&list), ["b", "a"]);
        assert_eq!(list[0].path, PathBuf::from("/src/b"));
        assert_eq!(list[0].tooltip, "/src/b");
        assert_eq!(list[0].arguments, r#""/src/b""#);
    }

    #[test]
    fn the_list_holds_no_more_than_the_slots_windows_gives() {
        let recent = paths(&["/r1", "/r2", "/r3", "/r4"]);
        assert_eq!(titles(&entries(&recent, 2, |_| true)), ["r1", "r2"]);
        assert!(entries(&recent, 0, |_| true).is_empty());
        // Missing folders don't use up a slot.
        assert_eq!(titles(&entries(&recent, 2, |p| !p.ends_with("r1"))), ["r2", "r3"]);
    }

    #[test]
    fn folders_with_the_same_name_show_their_parent() {
        let recent = paths(&["/work/app", "/home/App", "/src/tool"]);
        let list = entries(&recent, 10, |_| true);
        assert_eq!(titles(&list), ["app (work)", "App (home)", "tool"]);
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_paths_are_shown_and_opened_without_their_prefix() {
        let recent = paths(&[r"\\?\D:\Projects\tt-git", r"\\?\D:\"]);
        let list = entries(&recent, 10, |_| true);
        assert_eq!(titles(&list), ["tt-git", r"D:\"]);
        assert_eq!(list[0].tooltip, r"D:\Projects\tt-git");
        assert_eq!(list[0].arguments, r#""D:\Projects\tt-git""#);
        // The root's trailing backslash is doubled, so it isn't read as
        // escaping the closing quote.
        assert_eq!(list[1].arguments, r#""D:\\""#);
    }

    #[test]
    fn removed_entries_are_matched_back_to_their_repositories() {
        let recent = paths(&[r"C:\src\a", r"\\?\C:\src\b", r"C:\src\c"]);
        let gone = removed(&recent, &[r#""C:\src\b""#.to_string(), r#""C:\elsewhere""#.to_string()]);
        assert_eq!(gone, paths(&[r"\\?\C:\src\b"]));
    }

    #[test]
    fn arguments_are_quoted_as_windows_reads_them_back() {
        assert_eq!(quote(r"C:\My Repos\x"), r#""C:\My Repos\x""#);
        assert_eq!(quote(r"C:\x\"), r#""C:\x\\""#);
        assert_eq!(quote(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(quote(r"\\server\share"), r#""\\server\share""#);
        assert_eq!(quote(""), r#""""#);
    }

    #[test]
    fn the_open_repository_task_starts_the_folder_picker() {
        assert_eq!(TASKS.len(), 1);
        assert_eq!(TASKS[0].title, "Open repository…");
        assert_eq!(TASKS[0].arguments, OPEN_REPOSITORY_FLAG);
    }

    #[gpui::test]
    fn without_install_refresh_does_nothing(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            refresh(cx);
            assert!(!cx.has_global::<JumpList>());
        });
    }
}
