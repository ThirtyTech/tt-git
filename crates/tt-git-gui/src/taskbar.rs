//! The window's taskbar button on Windows: the progress of a network job
//! ([`App::take_job_progress`](tt_git_app::App::take_job_progress)), and
//! with `[ui] notifications` on a badge over the icon: conflicts, a stopped
//! merge or rebase ([`App::take_repo_alert`](tt_git_app::App::take_repo_alert)),
//! or new commits that came in while the window wasn't focused, until it is
//! again. The Win32 calls are in `tt-git-win`. On Linux the same goes to the
//! app's launcher ([`launcher_entry`](crate::launcher_entry)). On macOS the
//! badge goes to the Dock icon, which every window shares ([`crate::dock`]);
//! progress shows nowhere there.

use gpui::{App, Window};
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
        if !window.is_window_active() {
            self.new_commits += new.total();
            self.show(window, cx);
            self.report(window, cx);
        }
    }

    /// The window was focused or left: focused, the new commits and the
    /// alert are seen.
    pub fn activated(&mut self, window: &Window, cx: &mut App) {
        if window.is_window_active() && (self.new_commits > 0 || self.alert_unseen) {
            self.seen();
            self.show(window, cx);
            self.report(window, cx);
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
        crate::dock::set_badge(window.window_handle().window_id(), badge, cx);
        #[cfg(windows)]
        {
            use tt_git_win::taskbar::Icon;
            let Some(hwnd) = hwnd(window) else {
                return;
            };
            let icon = badge.map(|b| {
                let icon = Icon {
                    size: ICON_SIZE,
                    bgra: dot(b.color(), ICON_SIZE),
                };
                (icon, b.description())
            });
            let overlay = icon.as_ref().map(|(icon, text)| (icon, text.as_str()));
            if let Err(e) = tt_git_win::taskbar::set_overlay(hwnd, overlay) {
                tracing::warn!("couldn't badge the taskbar button: {e}");
            }
        }
        #[cfg(not(windows))]
        let _ = window;
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
