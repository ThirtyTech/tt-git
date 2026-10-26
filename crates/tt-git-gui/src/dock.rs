//! The app's Dock icon on macOS: with `[ui] notifications` on, a badge
//! over it, and its right-click menu.
//!
//! The Dock has one icon for every window, so each window's
//! [`Taskbar`](crate::taskbar::Taskbar) hands its [`Badge`] here and the
//! icon shows them together ([`label`]): "!" while any repository has
//! conflicts or a stopped merge or rebase (the Dock's badge is text on red,
//! so the two can't differ by color as on Windows), or else the new
//! commits that came in while their windows weren't focused, added up.
//!
//! The menu is the recent repositories (as the Windows jump list has them,
//! [`jump_list::entries`]) and Open Repository…; macOS adds the open
//! windows itself. [`jump_list::refresh`] builds it whenever the recent
//! repositories change. Its items run in this process, through
//! [`OpenRecent`] and [`PickRepositories`].
//!
//! Only `main` calls [`install`], so tests never touch the Dock; the badge
//! and the menu are worked out and unit-tested here on every OS.

use std::collections::HashMap;
use std::path::PathBuf;

use gpui::{App, Global, MenuItem, WindowId};

use crate::jump_list::{self, Entry};
use crate::taskbar::Badge;

/// How many recent repositories the menu lists.
pub const MENU_SLOTS: usize = 10;

/// Opens a recent repository, or brings its window forward.
#[derive(Debug, Clone, PartialEq, gpui::Action)]
#[action(namespace = tt_git, no_json)]
pub struct OpenRecent(pub PathBuf);

gpui::actions!(
    tt_git,
    [
        /// Asks for repositories with the folder picker and opens them.
        PickRepositories
    ]
);

/// Handles the menu's items. Call once, from [`crate::init`].
pub fn init(cx: &mut App) {
    cx.on_action(|open: &OpenRecent, cx| {
        let path = open.0.clone();
        // Deferred, as Quit is: it can come while a window is being updated.
        cx.defer(move |cx| {
            if let Err(e) = crate::windows::open_path(&path, cx) {
                show_error(e, cx);
            }
        });
    });
    cx.on_action(|_: &PickRepositories, cx| {
        crate::windows::pick_repos(cx, |result, cx| {
            if let Err(e) = result {
                show_error(e, cx);
            }
        });
    });
}

/// Shows what went wrong in the start window, as starting in a folder that
/// isn't a repository does.
fn show_error(e: String, cx: &mut App) {
    tracing::warn!("{e}");
    if let Err(e) = crate::start::open(Some(e), cx) {
        tracing::error!("couldn't open the start window: {e}");
    }
}

/// The Dock menu for the recent repositories `entries`.
pub fn menu(entries: &[Entry]) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = entries
        .iter()
        .map(|e| MenuItem::action(e.title.clone(), OpenRecent(e.path.clone())))
        .collect();
    if !items.is_empty() {
        items.push(MenuItem::separator());
    }
    items.push(MenuItem::action("Open Repository…", PickRepositories));
    items
}

/// Sets the Dock menu to the recent repositories `recent` (most recent
/// first) that still exist. Does nothing without [`install`].
pub fn set_menu(recent: &[PathBuf], cx: &mut App) {
    if !cx.has_global::<Dock>() {
        return;
    }
    let entries = jump_list::entries(recent, MENU_SLOTS, |p| p.is_dir());
    tracing::debug!("dock menu: {} recent repositories", entries.len());
    cx.set_dock_menu(menu(&entries));
}

/// The badge over the Dock icon for the windows' `badges`: "!" for any
/// alert, or the new commits added up.
pub fn label(badges: impl IntoIterator<Item = Badge>) -> Option<String> {
    let mut commits = 0;
    for badge in badges {
        match badge {
            Badge::Conflicts | Badge::Stopped(_) => return Some("!".to_string()),
            Badge::NewCommits(n) => commits += n,
        }
    }
    (commits > 0).then(|| commits.to_string())
}

/// Each window's badge, and the label the Dock was last given.
#[derive(Debug, Default)]
pub struct Badges {
    windows: HashMap<WindowId, Badge>,
    shown: Option<String>,
}

impl Badges {
    /// `window`'s badge is now `badge` (`None` once it has none, or has
    /// closed). The Dock's new label, if it changed.
    pub fn set(&mut self, window: WindowId, badge: Option<Badge>) -> Option<Option<String>> {
        match badge {
            Some(badge) => self.windows.insert(window, badge),
            None => self.windows.remove(&window),
        };
        let now = label(self.windows.values().copied());
        if now == self.shown {
            return None;
        }
        self.shown = now.clone();
        Some(now)
    }
}

/// The Dock, once [`install`]ed.
#[derive(Default)]
struct Dock(Badges);

impl Global for Dock {}

/// Starts showing the badge and the menu (call before
/// [`jump_list::install`], which builds the menu).
pub fn install(cx: &mut App) {
    cx.set_global(Dock::default());
}

/// Badges the Dock icon with `window`'s `badge`, along with the other
/// windows'. Does nothing without [`install`].
pub fn set_badge(window: WindowId, badge: Option<Badge>, cx: &mut App) {
    if !cx.has_global::<Dock>() {
        return;
    }
    if let Some(label) = cx.global_mut::<Dock>().0.set(window, badge) {
        tracing::debug!("dock badge: {label:?}");
        #[cfg(target_os = "macos")]
        macos::set_badge_label(label.as_deref());
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSString;

    pub fn set_badge_label(label: Option<&str>) {
        let Some(mtm) = MainThreadMarker::new() else {
            tracing::warn!("the Dock badge is only set on the main thread");
            return;
        };
        let label = label.map(NSString::from_str);
        NSApplication::sharedApplication(mtm)
            .dockTile()
            .setBadgeLabel(label.as_deref());
    }
}

#[cfg(test)]
mod tests {
    use tt_git_core::InProgress;

    use super::*;

    fn id(n: u64) -> WindowId {
        WindowId::from(n)
    }

    #[test]
    fn an_alert_anywhere_shows_a_mark_over_new_commits() {
        assert_eq!(label([]), None);
        assert_eq!(label([Badge::NewCommits(1)]), Some("1".into()));
        assert_eq!(label([Badge::NewCommits(2), Badge::NewCommits(3)]), Some("5".into()));
        assert_eq!(label([Badge::NewCommits(2), Badge::Conflicts]), Some("!".into()));
        assert_eq!(label([Badge::Stopped(InProgress::Rebase)]), Some("!".into()));
    }

    #[test]
    fn the_dock_is_told_only_when_its_label_changes() {
        let mut badges = Badges::default();
        assert_eq!(badges.set(id(1), Some(Badge::NewCommits(2))), Some(Some("2".into())));
        assert_eq!(badges.set(id(2), Some(Badge::NewCommits(1))), Some(Some("3".into())));
        assert_eq!(badges.set(id(2), Some(Badge::Conflicts)), Some(Some("!".into())));
        assert_eq!(badges.set(id(1), Some(Badge::NewCommits(4))), None, "still !");
        // The window with conflicts closed.
        assert_eq!(badges.set(id(2), None), Some(Some("4".into())));
        assert_eq!(badges.set(id(1), None), Some(None));
        assert_eq!(badges.set(id(1), None), None);
    }

    /// What each item says, and the repository it opens.
    fn items(menu: &[MenuItem]) -> Vec<(String, Option<PathBuf>)> {
        menu.iter()
            .map(|item| match item {
                MenuItem::Separator => ("-".into(), None),
                MenuItem::Action { name, action, .. } => (
                    name.to_string(),
                    action.as_any().downcast_ref::<OpenRecent>().map(|o| o.0.clone()),
                ),
                _ => panic!("unexpected item"),
            })
            .collect()
    }

    #[test]
    fn the_menu_lists_the_recent_repositories_then_open_repository() {
        let recent = [
            PathBuf::from("/work/app"),
            PathBuf::from("/gone"),
            PathBuf::from("/home/app"),
        ];
        let entries = jump_list::entries(&recent, MENU_SLOTS, |p| p != std::path::Path::new("/gone"));
        assert_eq!(
            items(&menu(&entries)),
            [
                ("app (work)".into(), Some(PathBuf::from("/work/app"))),
                ("app (home)".into(), Some(PathBuf::from("/home/app"))),
                ("-".into(), None),
                ("Open Repository…".into(), None),
            ]
        );
        let MenuItem::Action { action, .. } = &menu(&[])[0] else {
            panic!("no Open Repository…");
        };
        assert!(action.as_any().is::<PickRepositories>(), "alone with none recent");
    }

    #[gpui::test]
    fn without_install_nothing_is_badged(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            set_badge(id(1), Some(Badge::Conflicts), cx);
            assert!(!cx.has_global::<Dock>());
        });
    }
}
