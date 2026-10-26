//! The app's launcher on Linux (the dock in GNOME with Dash to Dock or
//! Ubuntu Dock, KDE's task manager), through the Unity LauncherEntry D-Bus
//! API: the progress bar while a network job runs, and with
//! `[ui] notifications` on the count of new commits and the urgent mark.
//! What each window's [`Taskbar`](crate::taskbar::Taskbar) shows comes here
//! as a [`WindowEntry`]. The launcher belongs to the app rather than a
//! window, so the windows are added up into one [`Entry`].
//!
//! Only `main` installs it, so tests never touch the session bus.

use std::collections::HashMap;

use gpui::{App, Global, WindowId};

/// What one window puts on the launcher.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowEntry {
    /// Commits that came in while the window wasn't focused.
    pub new_commits: usize,
    /// Conflicts, or a stopped merge or rebase, that came while the window
    /// wasn't focused.
    pub urgent: bool,
    /// A network job is running.
    pub busy: bool,
}

/// What the launcher shows, for every window together.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Entry {
    /// The count on the icon, hidden at 0.
    pub count: usize,
    /// The urgent mark: Dash to Dock wiggles the icon, KDE highlights it.
    pub urgent: bool,
    /// The progress bar. git gives no percentage and the API has no
    /// "indeterminate", so it's the bar's empty track.
    pub busy: bool,
}

impl Entry {
    /// The windows' entries added up.
    pub fn of<'a>(windows: impl IntoIterator<Item = &'a WindowEntry>) -> Self {
        windows.into_iter().fold(Entry::default(), |entry, w| Entry {
            count: entry.count + w.new_commits,
            urgent: entry.urgent || w.urgent,
            busy: entry.busy || w.busy,
        })
    }
}

// Only Linux makes one.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct LauncherEntry {
    windows: HashMap<WindowId, WindowEntry>,
    shown: Entry,
    /// To the thread that talks to the session bus.
    #[cfg(target_os = "linux")]
    sender: std::sync::mpsc::Sender<Entry>,
}

impl Global for LauncherEntry {}

/// Starts showing the windows' entries on the launcher (Linux).
pub fn install(cx: &mut App) {
    #[cfg(target_os = "linux")]
    cx.set_global(LauncherEntry {
        windows: HashMap::new(),
        shown: Entry::default(),
        sender: dbus::spawn(),
    });
    #[cfg(not(target_os = "linux"))]
    let _ = cx;
}

/// `window` now shows `entry`. Does nothing without [`install`].
pub fn set(window: WindowId, entry: WindowEntry, cx: &mut App) {
    if cx.has_global::<LauncherEntry>() {
        cx.global_mut::<LauncherEntry>().windows.insert(window, entry);
        show(cx);
    }
}

/// `window` closed.
pub fn forget(window: WindowId, cx: &mut App) {
    if cx.has_global::<LauncherEntry>() {
        cx.global_mut::<LauncherEntry>().windows.remove(&window);
        show(cx);
    }
}

fn show(cx: &mut App) {
    let launcher = cx.global_mut::<LauncherEntry>();
    let entry = Entry::of(launcher.windows.values());
    if entry == launcher.shown {
        return;
    }
    launcher.shown = entry;
    #[cfg(target_os = "linux")]
    if launcher.sender.send(entry).is_err() {
        tracing::debug!("the launcher's D-Bus thread has stopped");
    }
}

#[cfg(target_os = "linux")]
mod dbus {
    use std::collections::HashMap;
    use std::sync::mpsc;

    use zbus::blocking::Connection;
    use zbus::names::BusName;
    use zbus::zvariant::Value;

    use super::Entry;

    /// The launcher the entry is for, by its `.desktop` file.
    fn app_uri() -> String {
        format!("application://{}.desktop", crate::app_id::DESKTOP_ID)
    }

    /// Where the signal comes from. Any path will do; docks go by the app
    /// URI.
    const PATH: &str = "/com/thirtytech/tt_git/LauncherEntry";

    /// A thread with a session bus connection, sending each entry it's
    /// handed (only the latest, when several are waiting).
    pub(super) fn spawn() -> mpsc::Sender<Entry> {
        let (tx, rx) = mpsc::channel::<Entry>();
        let started = std::thread::Builder::new()
            .name("launcher-entry".into())
            .spawn(move || {
                let conn = match Connection::session() {
                    Ok(conn) => conn,
                    Err(e) => {
                        tracing::info!("no session bus, so no launcher badge: {e}");
                        return;
                    }
                };
                let uri = app_uri();
                while let Ok(mut entry) = rx.recv() {
                    while let Ok(later) = rx.try_recv() {
                        entry = later;
                    }
                    if let Err(e) = update(&conn, &uri, entry) {
                        tracing::warn!("couldn't update the launcher: {e}");
                    }
                }
            });
        if let Err(e) = started {
            tracing::warn!("couldn't start the launcher's D-Bus thread: {e}");
        }
        tx
    }

    /// Sends `com.canonical.Unity.LauncherEntry.Update` with every property,
    /// so a dock that starts later still gets the whole entry with the next
    /// one.
    fn update(conn: &Connection, uri: &str, entry: Entry) -> zbus::Result<()> {
        let props: HashMap<&str, Value> = HashMap::from([
            ("count", Value::from(i64::try_from(entry.count).unwrap_or(i64::MAX))),
            ("count-visible", Value::from(entry.count > 0)),
            ("urgent", Value::from(entry.urgent)),
            ("progress", Value::from(0.0)),
            ("progress-visible", Value::from(entry.busy)),
        ]);
        conn.emit_signal(
            None::<BusName>,
            PATH,
            "com.canonical.Unity.LauncherEntry",
            "Update",
            &(uri, props),
        )
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn the_entry_is_for_the_desktop_file() {
            assert_eq!(super::app_uri(), "application://tt-git.desktop");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_add_up() {
        assert_eq!(Entry::of(&[]), Entry::default());
        let windows = [
            WindowEntry {
                new_commits: 2,
                urgent: false,
                busy: true,
            },
            WindowEntry {
                new_commits: 3,
                urgent: true,
                busy: false,
            },
            WindowEntry::default(),
        ];
        assert_eq!(
            Entry::of(&windows),
            Entry {
                count: 5,
                urgent: true,
                busy: true
            }
        );
    }
}
