//! Linux: the notifications through the desktop's Notification portal
//! (`org.freedesktop.portal.Notification`), when the desktop knows the app.
//!
//! GPUI sends them to `org.freedesktop.Notifications`, which a sandbox
//! (Flatpak) may not reach and some GNOME sessions don't have. The portal
//! goes through the desktop instead, under the app's name and icon. It needs
//! an app id the desktop can find: a sandbox has one, and a host app
//! registers [`APP_ID`] (`org.freedesktop.host.portal.Registry`), which GNOME
//! only accepts with `<APP_ID>.desktop` installed. It refuses anything else
//! without telling the sender, so the entry is looked for first. Without it,
//! or without the portal, the notifications stay on GPUI's path.
//!
//! The notification's id is its tag, so a newer one for a repository
//! replaces the last. A click (or a button) comes back as `ActionInvoked`,
//! which goes to the same handler as GPUI's responses.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use ashpd::desktop::notification::{Button, Notification, NotificationProxy};
use futures::StreamExt as _;
use gpui::{App, Global, SystemNotification, SystemNotificationResponse};

use super::OPEN;
use crate::app_id::APP_ID;

/// The portal, once it's connected and tt-git is registered with it.
struct Portal(Rc<NotificationProxy>);

impl Global for Portal {}

/// Connects to the portal if the desktop knows the app, and from then on
/// sends the notifications there and hands its clicks to `respond`.
pub fn install(cx: &mut App) {
    if !ashpd::is_sandboxed() && !entry_installed() {
        tracing::info!("no {APP_ID}.desktop installed, so notifications go to org.freedesktop.Notifications");
        return;
    }
    cx.spawn(async move |cx| {
        let proxy = match connect().await {
            Ok(proxy) => Rc::new(proxy),
            Err(e) => {
                tracing::info!("no notification portal ({e}), so notifications go to org.freedesktop.Notifications");
                return;
            }
        };
        let actions = match proxy.receive_action_invoked().await {
            Ok(actions) => actions,
            Err(e) => {
                tracing::warn!("couldn't listen to the notification portal: {e}");
                return;
            }
        };
        tracing::debug!("notifications go to the portal");
        cx.update(|cx| cx.set_global(Portal(proxy.clone())));
        let mut actions = std::pin::pin!(actions);
        while let Some(action) = actions.next().await {
            let response = SystemNotificationResponse {
                tag: action.id().to_string().into(),
                action_id: Some(action.name().to_string().into()),
            };
            cx.update(|cx| super::respond(response, cx));
        }
    })
    .detach();
}

/// Sends `notification` through the portal. False when it isn't in use, for
/// GPUI to show it instead.
pub fn show(notification: &SystemNotification, cx: &mut App) -> bool {
    let Some(Portal(proxy)) = cx.try_global::<Portal>() else {
        return false;
    };
    let proxy = proxy.clone();
    let id = notification.tag.to_string();
    let sent = to_portal(notification);
    cx.spawn(async move |_| {
        if let Err(e) = proxy.add_notification(&id, sent).await {
            tracing::warn!("the notification portal refused a notification: {e}");
        }
    })
    .detach();
    true
}

/// A connection of tt-git's own, so registering is the first thing on it
/// (GPUI's portal calls share ashpd's).
async fn connect() -> ashpd::Result<NotificationProxy> {
    let connection = ashpd::zbus::Connection::session().await?;
    ashpd::register_host_app_with_connection(connection.clone(), APP_ID.parse()?).await?;
    NotificationProxy::with_connection(connection).await
}

/// `notification` as the portal's: a click on it is [`OPEN`], and each
/// action a button.
fn to_portal(notification: &SystemNotification) -> Notification {
    notification.actions.iter().fold(
        Notification::new(&notification.title)
            .body(notification.body.as_ref())
            .default_action(OPEN),
        |sent, action| sent.button(Button::new(&action.label, &action.id)),
    )
}

/// Whether `<APP_ID>.desktop` is in one of the desktop's application folders.
fn entry_installed() -> bool {
    let dirs = application_dirs(
        std::env::var_os("XDG_DATA_HOME"),
        std::env::var_os("HOME"),
        std::env::var_os("XDG_DATA_DIRS"),
    );
    let name = format!("{APP_ID}.desktop");
    dirs.iter().any(|dir| dir.join(&name).is_file())
}

/// The folders desktop entries are installed in, from the XDG base
/// directory variables: `$XDG_DATA_HOME` (else `~/.local/share`), then each
/// of `$XDG_DATA_DIRS` (else `/usr/local/share:/usr/share`), each with
/// `applications` added. Relative paths are ignored, as the spec says.
fn application_dirs(data_home: Option<OsString>, home: Option<OsString>, data_dirs: Option<OsString>) -> Vec<PathBuf> {
    let set = |v: Option<OsString>| v.filter(|v| !v.is_empty());
    let home_dir = set(data_home)
        .map(PathBuf::from)
        .or_else(|| set(home).map(|h| Path::new(&h).join(".local/share")));
    let data_dirs = set(data_dirs).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    home_dir
        .into_iter()
        .chain(std::env::split_paths(&data_dirs))
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("applications"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(data_home: Option<&str>, home: Option<&str>, data_dirs: Option<&str>) -> Vec<PathBuf> {
        application_dirs(
            data_home.map(Into::into),
            home.map(Into::into),
            data_dirs.map(Into::into),
        )
    }

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn the_defaults_are_the_home_folder_then_usr() {
        assert_eq!(
            dirs(None, Some("/home/u"), None),
            paths(&[
                "/home/u/.local/share/applications",
                "/usr/local/share/applications",
                "/usr/share/applications",
            ])
        );
        assert_eq!(
            dirs(Some(""), Some("/home/u"), Some("")),
            dirs(None, Some("/home/u"), None)
        );
    }

    #[test]
    fn the_variables_replace_the_defaults() {
        assert_eq!(
            dirs(
                Some("/data"),
                Some("/home/u"),
                Some("/var/lib/flatpak/exports/share:/usr/share")
            ),
            paths(&[
                "/data/applications",
                "/var/lib/flatpak/exports/share/applications",
                "/usr/share/applications",
            ])
        );
    }

    #[test]
    fn relative_folders_are_ignored() {
        assert_eq!(
            dirs(Some("data"), None, Some("share:/usr/share")),
            paths(&["/usr/share/applications"])
        );
    }
}
