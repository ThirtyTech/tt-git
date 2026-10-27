//! Desktop notifications (`[ui] notifications`, off by default, and
//! `[ui.alerts] popup`): new commits a fetch or pull brought in, or that
//! were committed from outside the window (another worktree, an agent),
//! shown through GPUI's system notifications (a toast on Windows) while the
//! repository's window isn't focused.
//!
//! Each notification is tagged with its repository's root, so a newer one
//! replaces the last, and a click on it (or its Open button) brings that
//! window forward. Pull, offered when the commits are on what the
//! checked-out branch tracks, runs the pull command in it as `p` does.
//! GPUI hands the response to [`init`]'s handler on the main thread.
//!
//! On Linux they go through the desktop's Notification portal instead when
//! the desktop knows the app (`portal.rs`, which [`install_portal`] starts).
//!
//! On macOS they go through `UNUserNotificationCenter`, which only works in
//! an app bundle: GPUI leaves them off when the process has no bundle
//! identifier (`cargo run`). macOS asks the user once whether tt-git may
//! notify; [`ask_permission`] does that when a window opens with the
//! setting on, rather than with the first notification.

use std::path::{Path, PathBuf};

use gpui::component::Root;
use gpui::{
    AnyWindowHandle, App, Entity, SystemNotification, SystemNotificationAction, SystemNotificationResponse, Window,
};
use tt_git_app::Action;
use tt_git_app::app::AlertSettings;
use tt_git_app::keymap::Command;
use tt_git_app::new_commits::NewCommits;

use crate::model::AppModel;
use crate::shell::Shell;

/// The action that brings the window forward, as a click on the
/// notification does.
pub const OPEN: &str = "open";
/// The action that pulls, as `p` does.
pub const PULL: &str = "pull";

const TAG_PREFIX: &str = "new-commits:";

#[cfg(target_os = "linux")]
mod portal;

/// Starts handling clicks on the notifications. Call once, from
/// [`crate::init`].
pub fn init(cx: &mut App) {
    cx.on_system_notification_response(respond);
}

/// Linux: sends the notifications through the desktop's Notification portal
/// from now on, where it can. Elsewhere it does nothing. Only `main` calls
/// it, so tests never touch D-Bus.
pub fn install_portal(cx: &mut App) {
    #[cfg(target_os = "linux")]
    portal::install(cx);
    #[cfg(not(target_os = "linux"))]
    let _ = cx;
}

/// Asks the OS for leave to notify, once per process: call when a window
/// opens with `[ui] notifications` on. On macOS the user is asked the first
/// time only; the answer is kept under the bundle's identifier and later
/// asks get it back without a prompt. Elsewhere there's nothing to ask.
pub fn ask_permission(cx: &mut App) {
    if cx.has_global::<Asked>() {
        return;
    }
    cx.set_global(Asked);
    #[cfg(target_os = "macos")]
    macos::ask_permission();
}

/// The notification settings last changed in a window, for every window
/// to use (`Shell` watches it).
pub struct UiAlerts(AlertSettings);

impl gpui::Global for UiAlerts {}

/// Hands `settings`, changed in one window, to every window, and asks
/// leave to notify if they turned notifications on.
pub fn share_settings(settings: AlertSettings, cx: &mut App) {
    if settings.notifications {
        ask_permission(cx);
    }
    cx.set_global(UiAlerts(settings));
}

/// The settings [`share_settings`] last handed out.
pub fn shared_settings(cx: &App) -> Option<AlertSettings> {
    cx.try_global::<UiAlerts>().map(|s| s.0)
}

/// [`ask_permission`] has asked.
struct Asked;

impl gpui::Global for Asked {}

#[cfg(target_os = "macos")]
mod macos {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSBundle, NSError};
    use objc2_user_notifications::{UNAuthorizationOptions, UNUserNotificationCenter};

    pub fn ask_permission() {
        // As GPUI does: the notification center aborts the process outside
        // an app bundle.
        if NSBundle::mainBundle().bundleIdentifier().is_none() {
            tracing::info!("not running from tt-git.app, so no notifications");
            return;
        }
        // GPUI logs why, when it's an error.
        let done = RcBlock::new(|granted: Bool, _: *mut NSError| {
            if granted.as_bool() {
                tracing::info!("macOS allows notifications");
            } else {
                tracing::info!("macOS doesn't allow notifications (System Settings > Notifications > tt-git)");
            }
        });
        UNUserNotificationCenter::currentNotificationCenter().requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &done,
        );
    }
}

/// Shows `new` for `model`'s repository, unless its `window` is focused.
pub fn new_commits(new: &NewCommits, model: &Entity<AppModel>, window: &Window, cx: &mut App) {
    if window.is_window_active() {
        tracing::debug!("{}: the window is focused, so no notification", new.summary());
        return;
    }
    let repo = model.read(cx).app().repo();
    let notification = notification(&repo.name(), repo.root(), new);
    tracing::debug!("notifying: {}", notification.body);
    #[cfg(target_os = "linux")]
    if portal::show(&notification, cx) {
        return;
    }
    cx.show_system_notification(notification);
}

/// The notification for `new` in the repository `name` at `root`: the
/// repository's name over `"3 new commits on origin/main"`.
pub fn notification(name: &str, root: &Path, new: &NewCommits) -> SystemNotification {
    let mut actions = vec![SystemNotificationAction {
        id: OPEN.into(),
        label: "Open".into(),
    }];
    if new.branches.iter().any(|b| b.head_upstream) {
        actions.push(SystemNotificationAction {
            id: PULL.into(),
            label: "Pull".into(),
        });
    }
    SystemNotification {
        tag: format!("{TAG_PREFIX}{}", root.display()).into(),
        title: name.to_string().into(),
        body: new.summary().into(),
        actions,
    }
}

/// The repository root a notification's tag names.
fn tagged_root(tag: &str) -> Option<PathBuf> {
    tag.strip_prefix(TAG_PREFIX).map(PathBuf::from)
}

fn respond(response: SystemNotificationResponse, cx: &mut App) {
    let Some(root) = tagged_root(&response.tag) else {
        return;
    };
    let Some(window) = crate::windows::window_for(&root, cx) else {
        tracing::info!("{} isn't open any more", root.display());
        return;
    };
    let _ = window.update(cx, |_, window, _| window.activate_window());
    if response.action_id.as_deref() == Some(PULL)
        && let Some(model) = model_of(window, cx)
    {
        model.update(cx, |m, cx| m.send(Action::Command(Command::Pull), cx));
    }
}

/// The app in a repository window.
fn model_of(window: AnyWindowHandle, cx: &mut App) -> Option<Entity<AppModel>> {
    let root = window.downcast::<Root>()?;
    root.update(cx, |root, _, cx| {
        let shell = root.view().clone().downcast::<Shell>().ok()?;
        Some(shell.read(cx).model().clone())
    })
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use tt_git_app::Config;
    use tt_git_app::config::Alert;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::testing::Gui;

    /// `main` tracking `origin/main`, which another clone has pushed `n`
    /// commits to, and the window open on it with notifications on.
    fn behind_origin(cx: &mut TestAppContext, n: usize) -> (Gui, Sandbox) {
        let sb = Sandbox::named("web");
        sb.commit_file("a.txt", "1", "base");
        let url = sb.add_remote("origin");
        sb.git(&["branch", "-q", "-u", "origin/main"]);
        let other = sb.clone_to(&url, "other");
        for i in 0..n {
            let name = format!("theirs{i}.txt");
            std::fs::write(other.join(&name), "x").unwrap();
            sb.git_in(&other, &["add", "."]);
            sb.git_in(&other, &["commit", "-q", "-m", &name]);
        }
        sb.git_in(&other, &["push", "-q", "origin", "main"]);
        let mut config = Config::default();
        config.ui.notifications = true;
        cx.update(|cx| crate::app_id::set(cx));
        (Gui::with_config(cx, &sb, config), sb)
    }

    /// Fetches outside tt-git, as `git fetch` in a terminal would, and
    /// refreshes as the watcher would.
    fn fetch(gui: &mut Gui, sb: &Sandbox) {
        sb.git(&["fetch", "-q", "origin"]);
        gui.model.update(&mut gui.cx, |m, cx| m.send(Action::Refresh, cx));
        gui.wait_idle();
    }

    #[gpui::test]
    fn new_commits_show_while_the_window_isnt_focused(cx: &mut TestAppContext) {
        let (mut gui, sb) = behind_origin(cx, 3);
        assert!(
            gui.cx.delivered_system_notifications().is_empty(),
            "none on the first load"
        );

        gui.cx.deactivate_window();
        fetch(&mut gui, &sb);
        let shown = gui.cx.delivered_system_notifications();
        assert_eq!(shown.len(), 1, "{shown:?}");
        assert_eq!(shown[0].title.as_ref(), "web");
        assert_eq!(shown[0].body.as_ref(), "3 new commits on origin/main");
        let actions: Vec<&str> = shown[0].actions.iter().map(|a| a.label.as_ref()).collect();
        assert_eq!(actions, ["Open", "Pull"]);
    }

    #[gpui::test]
    fn nothing_shows_while_the_window_is_focused(cx: &mut TestAppContext) {
        let (mut gui, sb) = behind_origin(cx, 1);
        gui.cx.update(|window, _| window.activate_window());
        gui.cx.run_until_parked();
        fetch(&mut gui, &sb);
        assert!(gui.cx.shown_system_notifications().is_empty());
    }

    #[gpui::test]
    fn pull_on_the_notification_pulls_in_its_window(cx: &mut TestAppContext) {
        let (mut gui, sb) = behind_origin(cx, 2);
        gui.cx.deactivate_window();
        fetch(&mut gui, &sb);
        let tag = gui.cx.delivered_system_notifications()[0].tag.clone();

        gui.cx
            .simulate_system_notification_response(SystemNotificationResponse {
                tag,
                action_id: Some(PULL.into()),
            });
        gui.wait_idle();
        assert_eq!(sb.rev_parse("main"), sb.rev_parse("origin/main"));
    }

    #[gpui::test]
    fn a_window_with_notifications_on_asks_leave_to_notify(cx: &mut TestAppContext) {
        let (mut gui, _sb) = behind_origin(cx, 0);
        assert!(gui.cx.update(|_, cx| cx.has_global::<Asked>()));
    }

    #[gpui::test]
    fn with_notifications_off_nothing_is_asked(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "1", "base");
        let mut gui = Gui::with_config(cx, &sb, Config::default());
        assert!(!gui.cx.update(|_, cx| cx.has_global::<Asked>()));
    }

    #[gpui::test]
    fn the_popup_can_be_off_with_the_badge_on(cx: &mut TestAppContext) {
        let sb = Sandbox::named("web");
        sb.commit_file("a.txt", "1", "base");
        sb.git(&["branch", "-q", "feature"]);
        let wt = sb.root().join("feature-wt");
        sb.git(&["worktree", "add", "-q", wt.to_str().unwrap(), "feature"]);
        let mut config = Config::default();
        config.ui.notifications = true;
        config.ui.alerts.set(Alert::Popup, false);
        let mut gui = Gui::with_config(cx, &sb, config);
        gui.cx.deactivate_window();
        std::fs::write(wt.join("b.txt"), "x").unwrap();
        sb.git_in(&wt, &["add", "."]);
        sb.git_in(&wt, &["commit", "-q", "-m", "agent"]);
        gui.model.update(&mut gui.cx, |m, cx| m.send(Action::Refresh, cx));
        gui.wait_idle();
        assert!(gui.cx.shown_system_notifications().is_empty());
        let badge = gui.shell.read_with(&gui.cx, |shell, _| shell.badge());
        assert_eq!(badge, Some(crate::taskbar::Badge::NewCommits(1)));
    }

    #[gpui::test]
    fn a_commit_in_another_worktree_notifies(cx: &mut TestAppContext) {
        let sb = Sandbox::named("web");
        sb.commit_file("a.txt", "1", "base");
        sb.git(&["branch", "-q", "feature"]);
        let wt = sb.root().join("feature-wt");
        sb.git(&["worktree", "add", "-q", wt.to_str().unwrap(), "feature"]);
        let mut config = Config::default();
        config.ui.notifications = true;
        config.ui.alerts.set(Alert::Badge, false);
        cx.update(|cx| crate::app_id::set(cx));
        let mut gui = Gui::with_config(cx, &sb, config);
        gui.cx.deactivate_window();
        std::fs::write(wt.join("b.txt"), "x").unwrap();
        sb.git_in(&wt, &["add", "."]);
        sb.git_in(&wt, &["commit", "-q", "-m", "agent"]);
        gui.model.update(&mut gui.cx, |m, cx| m.send(Action::Refresh, cx));
        gui.wait_idle();
        let shown = gui.cx.delivered_system_notifications();
        assert_eq!(shown.len(), 1, "{shown:?}");
        assert_eq!(shown[0].body.as_ref(), "1 new commit on feature (worktree feature-wt)");
        let actions: Vec<&str> = shown[0].actions.iter().map(|a| a.id.as_ref()).collect();
        assert_eq!(actions, [OPEN]);
        let badge = gui.shell.read_with(&gui.cx, |shell, _| shell.badge());
        assert_eq!(badge, None, "the badge is off");
    }

    #[gpui::test]
    fn turning_notifications_on_shares_them_and_asks_leave(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "1", "base");
        let mut gui = Gui::with_config(cx, &sb, Config::default());
        assert!(!gui.cx.update(|_, cx| cx.has_global::<Asked>()));
        gui.model
            .update(&mut gui.cx, |m, cx| m.send(Action::Command(Command::Notifications), cx));
        gui.wait_idle();
        gui.press("enter");
        assert!(gui.read(|app| app.config().ui.notifications));
        let shared = gui.cx.update(|_, cx| shared_settings(cx));
        assert_eq!(shared.map(|s| s.notifications), Some(true));
        assert!(gui.cx.update(|_, cx| cx.has_global::<Asked>()));
    }

    #[test]
    fn only_commits_on_heads_upstream_offer_pull() {
        let new = NewCommits {
            branches: vec![tt_git_app::new_commits::BranchCommits {
                branch: "origin/feature".into(),
                count: 1,
                head_upstream: false,
                local: false,
                worktree: None,
            }],
        };
        let shown = notification("web", Path::new("/src/web"), &new);
        assert_eq!(shown.body.as_ref(), "1 new commit on origin/feature");
        let actions: Vec<&str> = shown.actions.iter().map(|a| a.id.as_ref()).collect();
        assert_eq!(actions, [OPEN]);
        assert_eq!(tagged_root(&shown.tag), Some(PathBuf::from("/src/web")));
    }
}
