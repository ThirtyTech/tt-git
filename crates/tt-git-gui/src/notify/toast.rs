//! Windows: the toasts, sent by tt-git rather than GPUI.
//!
//! GPUI's toasts have only text and buttons. tt-git's carry the app icon as
//! the large logo left of the text (`appLogoOverride`), from the copy
//! `icon.rs` writes, with the same text, buttons and tag as GPUI's: a newer
//! toast for a repository replaces the last, and a click (or a button) goes
//! to the same handler as GPUI's responses. Without the logo's file, or if
//! Windows won't make a notifier, the toasts stay on GPUI's path.
//!
//! A toast goes out from a task of its own, after the update that asked for
//! it. Making and showing one are calls to Windows' notification platform,
//! in another process, and the thread pumps messages while it waits: a
//! repaint of the window can come in there. Made during the update, it
//! found the app borrowed, and GPUI dropped the frame (`RefCell already
//! borrowed`); from the task, the app is free for it.

use std::path::Path;

use gpui::SystemNotification;

/// The toast for `notification`, as Windows' toast XML, with `logo` (a
/// local PNG) as its large logo. A click on it, or a button, comes back with
/// the button's id as its `arguments` (none for the toast itself), and it
/// makes no sound, as GPUI's.
pub fn xml(notification: &SystemNotification, logo: Option<&Path>) -> String {
    let mut binding = String::new();
    for text in [&notification.title, &notification.body] {
        binding.push_str(&format!("<text>{}</text>", escape(text)));
    }
    if let Some(logo) = logo {
        binding.push_str(&format!(
            r#"<image placement="appLogoOverride" src="{}"/>"#,
            escape(&file_uri(logo))
        ));
    }
    let actions = if notification.actions.is_empty() {
        String::new()
    } else {
        let buttons: String = notification
            .actions
            .iter()
            .map(|a| {
                format!(
                    r#"<action content="{}" arguments="{}"/>"#,
                    escape(&a.label),
                    escape(&a.id)
                )
            })
            .collect();
        format!("<actions>{buttons}</actions>")
    };
    format!(
        r#"<toast><visual><binding template="ToastGeneric">{binding}</binding></visual>{actions}<audio silent="true"/></toast>"#
    )
}

/// `text` safe in XML text and attribute values.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// The `file:///` URI of the absolute Windows path `path`, which the toast
/// takes for a local image.
fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file:///");
    for byte in path.to_string_lossy().replace('\\', "/").bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => uri.push(byte as char),
            byte => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

#[cfg(windows)]
pub use toaster::{install, show};

#[cfg(windows)]
mod toaster {
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    use futures::StreamExt as _;
    use futures::channel::mpsc;
    use gpui::{App, Global, SharedString, SystemNotification, SystemNotificationResponse};
    use tt_git_win::toast::Toaster;

    use crate::app_id::APP_ID;

    /// The toaster, once it's made, and the logo the toasts carry.
    struct Toasts {
        toaster: Rc<RefCell<Toaster>>,
        logo: PathBuf,
        responses: mpsc::UnboundedSender<SystemNotificationResponse>,
    }

    impl Global for Toasts {}

    /// Sends the toasts from now on, with `logo` (the icon `icon.rs`
    /// wrote) as their logo, and hands their clicks to `respond`.
    pub fn install(logo: PathBuf, cx: &mut App) {
        let toaster = match Toaster::new(APP_ID) {
            Ok(toaster) => toaster,
            Err(e) => {
                tracing::warn!("couldn't make a toast notifier, so GPUI sends the notifications: {e}");
                return;
            }
        };
        let (responses, mut received) = mpsc::unbounded();
        cx.set_global(Toasts {
            toaster: Rc::new(RefCell::new(toaster)),
            logo,
            responses,
        });
        cx.spawn(async move |cx| {
            while let Some(response) = received.next().await {
                cx.update(|cx| super::super::respond(response, cx));
            }
        })
        .detach();
    }

    /// Shows `notification` as a toast with the logo, from a task once the
    /// current update is done (see the module's notes), or through GPUI if
    /// Windows refuses it. False when tt-git's toasts aren't in use, for
    /// GPUI to show it instead.
    pub fn show(notification: &SystemNotification, cx: &mut App) -> bool {
        let Some(toasts) = cx.try_global::<Toasts>() else {
            return false;
        };
        let xml = super::xml(notification, Some(&toasts.logo));
        let responses = toasts.responses.clone();
        let toaster = toasts.toaster.clone();
        let notification = notification.clone();
        cx.spawn(async move |cx| {
            let tag: SharedString = notification.tag.clone();
            let start = std::time::Instant::now();
            let shown = toaster.borrow_mut().show(&notification.tag, &xml, move |action| {
                let response = SystemNotificationResponse {
                    tag: tag.clone(),
                    action_id: action.map(Into::into),
                };
                responses.unbounded_send(response).ok();
            });
            tracing::debug!(elapsed = ?start.elapsed(), "toast: Show");
            if let Err(e) = shown {
                tracing::warn!("Windows refused a toast, so GPUI sends it: {e}");
                cx.update(|cx| cx.show_system_notification(notification));
            }
        })
        .detach();
        true
    }
}

#[cfg(test)]
mod tests {
    use gpui::SystemNotificationAction;

    use super::*;

    fn sample(actions: &[(&str, &str)]) -> SystemNotification {
        SystemNotification {
            tag: "new-commits:/src/web".into(),
            title: "web & <co>".into(),
            body: "3 new commits on \"origin/main\"".into(),
            actions: actions
                .iter()
                .map(|(id, label)| SystemNotificationAction {
                    id: (*id).into(),
                    label: (*label).into(),
                })
                .collect(),
        }
    }

    #[test]
    fn the_toast_has_the_text_the_logo_and_the_buttons() {
        let logo = Path::new(r"C:\Users\Jo Doe\AppData\Roaming\tt-git\notification-icon.png");
        assert_eq!(
            xml(&sample(&[("open", "Open"), ("pull", "Pull")]), Some(logo)),
            concat!(
                r#"<toast><visual><binding template="ToastGeneric">"#,
                "<text>web &amp; &lt;co&gt;</text>",
                "<text>3 new commits on &quot;origin/main&quot;</text>",
                r#"<image placement="appLogoOverride" "#,
                r#"src="file:///C:/Users/Jo%20Doe/AppData/Roaming/tt-git/notification-icon.png"/>"#,
                "</binding></visual>",
                r#"<actions><action content="Open" arguments="open"/><action content="Pull" arguments="pull"/></actions>"#,
                r#"<audio silent="true"/></toast>"#,
            )
        );
    }

    #[test]
    fn without_buttons_or_a_logo_there_are_none() {
        let shown = xml(&sample(&[]), None);
        assert!(!shown.contains("<actions>"), "{shown}");
        assert!(!shown.contains("<image"), "{shown}");
    }
}
