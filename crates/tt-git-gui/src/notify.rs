//! Desktop notifications (`[ui] notifications`, off by default): new
//! commits a fetch or pull brought in, shown through GPUI's system
//! notifications (a toast on Windows) while the repository's window isn't
//! focused.
//!
//! Each notification is tagged with its repository's root, so a newer one
//! replaces the last, and a click on it (or its Open button) brings that
//! window forward. Pull, offered when the commits are on what the
//! checked-out branch tracks, runs the pull command in it as `p` does.
//! GPUI hands the response to [`init`]'s handler on the main thread.
//!
//! On Linux they go through the desktop's Notification portal instead when
//! the desktop knows the app (`portal.rs`, which [`install_portal`] starts).

use std::path::{Path, PathBuf};

use gpui::component::Root;
use gpui::{
    AnyWindowHandle, App, Entity, SystemNotification, SystemNotificationAction, SystemNotificationResponse, Window,
};
use tt_git_app::Action;
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

    #[test]
    fn only_commits_on_heads_upstream_offer_pull() {
        let new = NewCommits {
            branches: vec![tt_git_app::new_commits::BranchCommits {
                branch: "origin/feature".into(),
                count: 1,
                head_upstream: false,
            }],
        };
        let shown = notification("web", Path::new("/src/web"), &new);
        assert_eq!(shown.body.as_ref(), "1 new commit on origin/feature");
        let actions: Vec<&str> = shown.actions.iter().map(|a| a.id.as_ref()).collect();
        assert_eq!(actions, [OPEN]);
        assert_eq!(tagged_root(&shown.tag), Some(PathBuf::from("/src/web")));
    }
}
