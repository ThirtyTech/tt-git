//! The app's Dock icon on macOS: with `[ui] notifications` on, a badge
//! over it.
//!
//! The Dock has one icon for every window, so each window's
//! [`Taskbar`](crate::taskbar::Taskbar) hands its [`Badge`] here and the
//! icon shows them together ([`label`]): "!" while any repository has
//! conflicts or a stopped merge or rebase (the Dock's badge is text on red,
//! so the two can't differ by color as on Windows), or else the new
//! commits that came in while their windows weren't focused, added up.
//!
//! Only `main` calls [`install`], so tests never touch the Dock; the badge
//! is worked out and unit-tested here on every OS.

use std::collections::HashMap;

use gpui::{App, Global, WindowId};

use crate::taskbar::Badge;

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

/// Starts showing the badge.
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

    #[gpui::test]
    fn without_install_nothing_is_badged(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            set_badge(id(1), Some(Badge::Conflicts), cx);
            assert!(!cx.has_global::<Dock>());
        });
    }
}
