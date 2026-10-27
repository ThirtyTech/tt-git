//! The window's taskbar button on Windows: the progress of a network job
//! ([`App::take_job_progress`](tt_git_app::App::take_job_progress)), and
//! with `[ui] notifications` on a badge over the icon, for each alert
//! `[ui.alerts]` lets through: conflicts, a stopped merge or rebase
//! ([`App::take_repo_alert`](tt_git_app::App::take_repo_alert)),
//! or new commits that came in while the window wasn't focused, until it is
//! again. The Win32 calls are in `tt-git-win`. On Linux the same goes to the
//! app's launcher ([`launcher_entry`](crate::launcher_entry)). On macOS the
//! badge goes to the Dock icon, which every window shares ([`crate::dock`]);
//! progress shows nowhere there.
//!
//! The Windows taskbar stacks the app's windows on one button, which shows
//! the overlay set last, whichever window set it, and keeps showing it after
//! that window closes. So every window's overlay is sent again, in
//! [`send_order`], whenever one changes, a window closes ([`forget`]) or one
//! is focused.

use gpui::{App, Window, WindowId};
use tt_git_app::app::{JobProgress, RepoAlert};
use tt_git_app::new_commits::NewCommits;
use tt_git_core::InProgress;

use crate::launcher_entry::{self, WindowEntry};

/// The badge's size in pixels: twice the taskbar's 16, so it stays sharp
/// at 200%.
#[cfg_attr(not(windows), allow(dead_code))]
const ICON_SIZE: u32 = 32;

/// What the badge over a window's taskbar button says, most pressing first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    Conflicts,
    Stopped(InProgress),
    NewCommits(usize),
}

impl Badge {
    /// What a screen reader says for it.
    pub fn description(self) -> String {
        match self {
            Badge::Conflicts => "Conflicts".to_string(),
            Badge::Stopped(op) => format!("A {} is stopped part way", op.command()),
            Badge::NewCommits(1) => "1 new commit".to_string(),
            Badge::NewCommits(n) => format!("{n} new commits"),
        }
    }

    /// The dot's color: red, amber, blue.
    #[cfg_attr(not(windows), allow(dead_code))]
    fn color(self) -> [u8; 3] {
        match self {
            Badge::Conflicts => [0xcf, 0x22, 0x2e],
            Badge::Stopped(_) => [0xd2, 0x8a, 0x00],
            Badge::NewCommits(_) => [0x1f, 0x6f, 0xeb],
        }
    }
}

/// One window's taskbar button: what it was told, so it's only told again
/// when something changed.
#[derive(Debug, Default)]
pub struct Taskbar {
    alert: RepoAlert,
    /// The alert came while the window wasn't focused: urgent on the Linux
    /// launcher until it is.
    alert_unseen: bool,
    /// Commits that came in while the window wasn't focused.
    new_commits: usize,
    busy: bool,
    shown: Option<Badge>,
}

impl Taskbar {
    /// Shows `progress` on `window`'s taskbar button.
    pub fn progress(&mut self, window: &Window, progress: JobProgress, cx: &mut App) {
        self.busy = progress == JobProgress::Busy;
        self.report(window, cx);
        #[cfg(windows)]
        {
            use tt_git_win::taskbar::Progress;
            let progress = match progress {
                JobProgress::Idle => Progress::None,
                JobProgress::Busy => Progress::Indeterminate,
                JobProgress::Failed => Progress::Error,
            };
            if let Some(hwnd) = hwnd(window)
                && let Err(e) = tt_git_win::taskbar::set_progress(hwnd, progress)
            {
                tracing::warn!("couldn't show progress on the taskbar: {e}");
            }
        }
        #[cfg(not(windows))]
        let _ = (window, progress);
    }

    /// The repository's alert changed.
    pub fn alert(&mut self, window: &Window, alert: RepoAlert, cx: &mut App) {
        self.set_alert(alert, window.is_window_active());
        self.show(window, cx);
        self.report(window, cx);
    }

    /// `new` came in. It's badged only while the window isn't focused, as
    /// the notification is.
    pub fn new_commits(&mut self, window: &Window, new: &NewCommits, cx: &mut App) {
        tracing::debug!(
            active = window.is_window_active(),
            foreground = foreground(window),
            commits = new.total(),
            "taskbar: new commits"
        );
        if !window.is_window_active() {
            self.new_commits += new.total();
            self.show(window, cx);
            self.report(window, cx);
        }
    }

    /// The window was focused or left: focused, the new commits and the
    /// alert are seen.
    pub fn activated(&mut self, window: &Window, cx: &mut App) {
        let active = window.is_window_active();
        tracing::debug!(
            active,
            foreground = foreground(window),
            new_commits = self.new_commits,
            "taskbar: activation"
        );
        if !active {
            return;
        }
        if self.new_commits > 0 || self.alert_unseen {
            self.seen();
            self.show(window, cx);
            self.report(window, cx);
        }
        // Even with nothing new here: the button may still show the badge
        // of a window that's closed since. A window that's never had a
        // badge is recorded here, when it's first focused.
        #[cfg(windows)]
        if let Some(hwnd) = hwnd(window) {
            overlays::set(window.window_handle().window_id(), hwnd, self.shown, cx);
        }
    }

    fn set_alert(&mut self, alert: RepoAlert, focused: bool) {
        if alert != self.alert {
            self.alert_unseen = alert != RepoAlert::None && !focused;
        }
        self.alert = alert;
    }

    fn seen(&mut self) {
        self.new_commits = 0;
        self.alert_unseen = false;
    }

    /// What the window puts on the Linux launcher.
    pub fn launcher_entry(&self) -> WindowEntry {
        WindowEntry {
            new_commits: self.new_commits,
            urgent: self.alert_unseen,
            busy: self.busy,
        }
    }

    fn report(&self, window: &Window, cx: &mut App) {
        launcher_entry::set(window.window_handle().window_id(), self.launcher_entry(), cx);
    }

    /// The badge for what's known now, if any.
    pub fn badge(&self) -> Option<Badge> {
        match self.alert {
            RepoAlert::Conflicts => Some(Badge::Conflicts),
            RepoAlert::Stopped(op) => Some(Badge::Stopped(op)),
            RepoAlert::None if self.new_commits > 0 => Some(Badge::NewCommits(self.new_commits)),
            RepoAlert::None => None,
        }
    }

    fn show(&mut self, window: &Window, cx: &mut App) {
        let badge = self.badge();
        if badge == self.shown {
            return;
        }
        self.shown = badge;
        tracing::debug!(?badge, "taskbar: badge");
        let id = window.window_handle().window_id();
        crate::dock::set_badge(id, badge, cx);
        #[cfg(windows)]
        if let Some(hwnd) = hwnd(window) {
            overlays::set(id, hwnd, badge, cx);
        }
    }
}

/// `window` closed: on Windows the other windows' overlays are sent again,
/// as the button they shared keeps showing the closed window's.
pub fn forget(window: WindowId, cx: &mut App) {
    #[cfg(windows)]
    overlays::forget(window, cx);
    #[cfg(not(windows))]
    let _ = (window, cx);
}

/// The order to send the windows' overlays in, as `(hwnd, badge)`, so the
/// button they share ends on the most pressing: the windows without a
/// badge first, then new commits, a stopped merge or rebase, conflicts.
/// Windows that tie go by `hwnd`, so the order holds from one send to the
/// next.
#[cfg_attr(not(windows), allow(dead_code))]
fn send_order(windows: impl IntoIterator<Item = (isize, Option<Badge>)>) -> Vec<(isize, Option<Badge>)> {
    let urgency = |badge: Option<Badge>| match badge {
        None => 0,
        Some(Badge::NewCommits(_)) => 1,
        Some(Badge::Stopped(_)) => 2,
        Some(Badge::Conflicts) => 3,
    };
    let mut windows: Vec<_> = windows.into_iter().collect();
    windows.sort_by_key(|&(hwnd, badge)| (urgency(badge), hwnd));
    windows
}

/// Every window's badge, with its `HWND`, for the button they share.
#[cfg(windows)]
mod overlays {
    use std::collections::HashMap;

    use gpui::{App, Global, WindowId};
    use tt_git_win::taskbar::Icon;

    use super::{Badge, ICON_SIZE, dot, send_order};

    #[derive(Default)]
    struct Overlays(HashMap<WindowId, (isize, Option<Badge>)>);

    impl Global for Overlays {}

    /// `window`, at `hwnd`, shows `badge` now.
    pub fn set(window: WindowId, hwnd: isize, badge: Option<Badge>, cx: &mut App) {
        cx.default_global::<Overlays>().0.insert(window, (hwnd, badge));
        send(cx);
    }

    /// `window` closed.
    pub fn forget(window: WindowId, cx: &mut App) {
        if cx.has_global::<Overlays>() && cx.global_mut::<Overlays>().0.remove(&window).is_some() {
            send(cx);
        }
    }

    /// Sends every window's overlay again.
    pub fn send(cx: &App) {
        let Some(overlays) = cx.try_global::<Overlays>() else {
            return;
        };
        for (hwnd, badge) in send_order(overlays.0.values().copied()) {
            overlay(hwnd, badge);
        }
    }

    fn overlay(hwnd: isize, badge: Option<Badge>) {
        let icon = badge.map(|b| {
            let icon = Icon {
                size: ICON_SIZE,
                bgra: dot(b.color(), ICON_SIZE),
            };
            (icon, b.description())
        });
        let overlay = icon.as_ref().map(|(icon, text)| (icon, text.as_str()));
        let start = std::time::Instant::now();
        if let Err(e) = tt_git_win::taskbar::set_overlay(hwnd, overlay) {
            tracing::warn!("couldn't badge the taskbar button: {e}");
        }
        tracing::debug!(hwnd, ?badge, elapsed = ?start.elapsed(), "taskbar: SetOverlayIcon");
    }
}

/// The badge's picture: a dot of `color` in a white ring, so it reads on a
/// light taskbar and a dark one, as `size`×`size` BGRA pixels with
/// premultiplied alpha. The edges are anti-aliased.
pub fn dot(color: [u8; 3], size: u32) -> Vec<u8> {
    let center = size as f32 / 2.;
    let outer = center - 0.5;
    let inner = outer - (size as f32 / 16.).max(1.);
    let mut bgra = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let d = (x as f32 + 0.5 - center).hypot(y as f32 + 0.5 - center);
            // How much of the pixel each disc covers.
            let all = (outer - d + 0.5).clamp(0., 1.);
            let fill = (inner - d + 0.5).clamp(0., 1.);
            let ring = all - fill;
            let [r, g, b] = color.map(|c| ring * 255. + fill * f32::from(c));
            bgra.extend([b, g, r, all * 255.].map(|v| v.round() as u8));
        }
    }
    bgra
}

/// Whether `window` is the one the user is in, as the OS has it now, for
/// the debug log to set beside GPUI's `is_window_active`. On Windows that's
/// asked of Windows.
fn foreground(window: &Window) -> bool {
    #[cfg(windows)]
    return hwnd(window).is_some_and(tt_git_win::taskbar::is_foreground);
    #[cfg(not(windows))]
    window.is_window_active()
}

/// The window's `HWND`.
#[cfg(windows)]
fn hwnd(window: &Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    // `Window` has a `window_handle` of its own (GPUI's handle).
    let handle = HasWindowHandle::window_handle(window)
        .inspect_err(|e| tracing::warn!("no window handle: {e}"))
        .ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(win) => Some(win.hwnd.get()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(bgra: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        bgra[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn the_dot_is_its_color_in_a_white_ring_on_nothing() {
        let bgra = dot([0x1f, 0x6f, 0xeb], 32);
        assert_eq!(bgra.len(), 32 * 32 * 4);
        assert_eq!(pixel(&bgra, 32, 16, 16), [0xeb, 0x6f, 0x1f, 255], "the middle");
        assert_eq!(pixel(&bgra, 32, 0, 0), [0, 0, 0, 0], "a corner");
        assert_eq!(pixel(&bgra, 32, 16, 1), [255, 255, 255, 255], "the ring");
        // Premultiplied: no channel above the alpha.
        assert!(bgra.chunks(4).all(|p| p[..3].iter().all(|&c| c <= p[3])));
    }

    #[test]
    fn the_most_pressing_badge_is_sent_last() {
        let stopped = Some(Badge::Stopped(InProgress::Merge));
        let order = send_order([
            (7, Some(Badge::NewCommits(2))),
            (3, Some(Badge::Conflicts)),
            (9, None),
            (5, stopped),
            (1, None),
            (2, Some(Badge::NewCommits(1))),
        ]);
        assert_eq!(
            order,
            [
                (1, None),
                (9, None),
                (2, Some(Badge::NewCommits(1))),
                (7, Some(Badge::NewCommits(2))),
                (5, stopped),
                (3, Some(Badge::Conflicts)),
            ]
        );
    }

    #[test]
    fn conflicts_come_before_a_stopped_rebase_before_new_commits() {
        let mut taskbar = Taskbar::default();
        assert_eq!(taskbar.badge(), None);
        taskbar.new_commits = 3;
        assert_eq!(taskbar.badge(), Some(Badge::NewCommits(3)));
        taskbar.alert = RepoAlert::Stopped(InProgress::Rebase);
        assert_eq!(taskbar.badge(), Some(Badge::Stopped(InProgress::Rebase)));
        taskbar.alert = RepoAlert::Conflicts;
        assert_eq!(taskbar.badge(), Some(Badge::Conflicts));
        taskbar.alert = RepoAlert::None;
        taskbar.new_commits = 0;
        assert_eq!(taskbar.badge(), None);
    }

    #[test]
    fn an_alert_is_urgent_on_the_launcher_until_the_window_is_focused() {
        let mut taskbar = Taskbar::default();
        taskbar.set_alert(RepoAlert::Conflicts, true);
        assert!(!taskbar.launcher_entry().urgent, "it came while focused");
        taskbar.set_alert(RepoAlert::None, false);
        taskbar.set_alert(RepoAlert::Stopped(InProgress::Merge), false);
        taskbar.new_commits = 2;
        assert_eq!(
            taskbar.launcher_entry(),
            WindowEntry {
                new_commits: 2,
                urgent: true,
                busy: false
            }
        );
        taskbar.set_alert(RepoAlert::Stopped(InProgress::Merge), false);
        assert!(taskbar.launcher_entry().urgent, "the same alert again");
        taskbar.seen();
        assert_eq!(taskbar.launcher_entry(), WindowEntry::default());
        assert_eq!(
            taskbar.badge(),
            Some(Badge::Stopped(InProgress::Merge)),
            "the badge stays"
        );
    }

    #[test]
    fn the_badge_says_what_it_means() {
        assert_eq!(Badge::NewCommits(1).description(), "1 new commit");
        assert_eq!(Badge::NewCommits(4).description(), "4 new commits");
        assert_eq!(
            Badge::Stopped(InProgress::CherryPick).description(),
            "A cherry-pick is stopped part way"
        );
    }
}
