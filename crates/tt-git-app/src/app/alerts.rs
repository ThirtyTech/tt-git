//! The notifications picker, for a front end that tells the desktop about
//! the repository (the desktop app): `[ui] notifications`, the switch for
//! them all, and each alert of `[ui.alerts]` on its own. `enter` flips the
//! highlighted one, saves it in config.toml and leaves the picker open.

use super::App;
use crate::config::{Alert, AlertsConfig};
use crate::modal::{Modal, PaletteItem, PaletteTarget};

/// The notification settings, as one window hands them to the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlertSettings {
    /// `[ui] notifications`.
    pub notifications: bool,
    /// `[ui.alerts]`.
    pub alerts: AlertsConfig,
}

impl App {
    /// Says the front end notifies, which offers
    /// [`Command::Notifications`](crate::keymap::Command::Notifications) in
    /// the palette.
    pub fn enable_notifications(&mut self) {
        self.notifies = true;
    }

    pub fn notifies(&self) -> bool {
        self.notifies
    }

    /// The notification settings in use.
    pub fn alert_settings(&self) -> AlertSettings {
        AlertSettings {
            notifications: self.config.ui.notifications,
            alerts: self.config.ui.alerts,
        }
    }

    /// Uses `settings`, changed in another window (already saved there).
    pub fn set_alert_settings(&mut self, settings: AlertSettings) {
        if settings == self.alert_settings() {
            return;
        }
        self.config.ui.notifications = settings.notifications;
        self.config.ui.alerts = settings.alerts;
        if matches!(&self.modal, Some(Modal::Palette { title, .. }) if title == PICKER_TITLE) {
            self.open_alerts_picker();
        }
        self.dirty = true;
    }

    /// The settings changed in this window since the last call, for the
    /// front end to hand the other windows.
    pub fn take_alert_settings(&mut self) -> Option<AlertSettings> {
        self.alerts_changed.then(|| {
            self.alerts_changed = false;
            self.alert_settings()
        })
    }

    /// Opens the notifications picker, on the entry highlighted if it's
    /// open already, else on the first.
    pub(super) fn open_alerts_picker(&mut self) {
        let on = match self.modal.as_ref().and_then(Modal::palette_selection) {
            Some(PaletteTarget::Alert(alert)) => *alert,
            _ => None,
        };
        let ui = &self.config.ui;
        let state = |on: bool| if on { "on" } else { "off" };
        let items = std::iter::once(PaletteItem {
            label: "Notifications".into(),
            hint: state(ui.notifications).into(),
            target: PaletteTarget::Alert(None),
        })
        .chain(Alert::ALL.iter().map(|a| PaletteItem {
            label: a.label().into(),
            hint: match (ui.alerts.get(*a), ui.notifications) {
                (true, false) => "on (notifications off)".into(),
                (on, _) => state(on).into(),
            },
            target: PaletteTarget::Alert(Some(*a)),
        }))
        .collect::<Vec<_>>();
        let selected = items
            .iter()
            .position(|i| i.target == PaletteTarget::Alert(on))
            .unwrap_or(0);
        self.modal = Some(Modal::picker(PICKER_TITLE, items, selected));
        self.dirty = true;
    }

    /// Flips `alert` (`None` for `[ui] notifications`), picked in the
    /// picker, saves it in config.toml, and shows the picker again on it.
    pub(super) fn toggle_alert(&mut self, alert: Option<Alert>) {
        let ui = &mut self.config.ui;
        let (on, table, key, name) = match alert {
            None => {
                ui.notifications = !ui.notifications;
                (ui.notifications, "ui", "notifications", "notifications".to_string())
            }
            Some(a) => {
                let on = !ui.alerts.get(a);
                ui.alerts.set(a, on);
                (on, "ui.alerts", a.key(), a.label().to_lowercase())
            }
        };
        self.alerts_changed = true;
        let name = format!("{name} {}", if on { "on" } else { "off" });
        match self.save_setting(table, key, &on.to_string()) {
            None => self.notice = Some(name),
            Some(Ok(path)) => self.notice = Some(format!("{name}, saved to {}", path.display())),
            Some(Err(e)) => self.error = Some(format!("couldn't save the notifications to {e}")),
        }
        self.open_alerts_picker();
    }
}

const PICKER_TITLE: &str = "Notifications";
