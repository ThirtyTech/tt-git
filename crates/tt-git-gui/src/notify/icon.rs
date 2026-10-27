//! Windows: the app icon on the toasts.
//!
//! GPUI sends the toasts as [`APP_ID`] and registers it under
//! `HKCU\Software\Classes\AppUserModelId\<APP_ID>` with a `DisplayName`
//! only. Without an `IconUri` there, and with no Start menu shortcut
//! carrying the id (`cargo run`), Windows shows them with no icon, and GPUI's
//! toasts have no icon of their own. So [`install`] puts the app icon in the
//! state folder and names it as the `IconUri`, every launch, so a moved
//! state folder or a newer icon is picked up.

use std::io;
use std::path::{Path, PathBuf};

use crate::app_id::{APP_ID, APP_NAME};

/// The icon's file in the state folder.
pub const FILE: &str = "notification-icon.png";

/// The key the toasts' app id is registered under, below `HKEY_CURRENT_USER`.
pub fn registry_key() -> String {
    format!(r"Software\Classes\AppUserModelId\{APP_ID}")
}

/// The values [`install`] sets on [`registry_key`], for the icon at `icon`.
pub fn registry_values(icon: &Path) -> [(&'static str, String); 2] {
    [
        ("DisplayName", APP_NAME.to_string()),
        ("IconUri", icon.display().to_string()),
    ]
}

/// Writes the app icon to [`FILE`] in `dir`, unless it's there as it is,
/// and returns its path.
pub fn write(dir: &Path) -> io::Result<PathBuf> {
    let path = dir.join(FILE);
    if std::fs::read(&path).is_ok_and(|have| have == crate::menus::icon::PNG) {
        return Ok(path);
    }
    std::fs::create_dir_all(dir)?;
    std::fs::write(&path, crate::menus::icon::PNG)?;
    Ok(path)
}

/// Writes the icon to `state_dir` and registers it for the toasts.
#[cfg(windows)]
pub fn install(state_dir: &Path) {
    let icon = match write(state_dir) {
        Ok(icon) => icon,
        Err(e) => {
            tracing::warn!("couldn't write the notification icon to {}: {e}", state_dir.display());
            return;
        }
    };
    let set = windows_registry::CURRENT_USER.create(registry_key()).and_then(|key| {
        registry_values(&icon)
            .iter()
            .try_for_each(|(name, value)| key.set_string(*name, value))
    });
    match set {
        Ok(()) => tracing::debug!("notifications show {}", icon.display()),
        Err(e) => tracing::warn!("couldn't register the notification icon: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_is_written_once_and_rewritten_when_it_differs() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let path = write(&state).unwrap();
        assert_eq!(path, state.join(FILE));
        assert_eq!(std::fs::read(&path).unwrap(), crate::menus::icon::PNG);
        assert!(crate::menus::icon::PNG.starts_with(b"\x89PNG"), "Windows wants a PNG");

        std::fs::write(&path, b"old").unwrap();
        write(&state).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), crate::menus::icon::PNG);
    }

    #[test]
    fn the_registry_names_the_app_and_its_icon() {
        assert_eq!(registry_key(), r"Software\Classes\AppUserModelId\ThirtyTech.tt-git");
        let icon = Path::new(r"C:\Users\u\AppData\Local\tt-git\notification-icon.png");
        assert_eq!(
            registry_values(icon),
            [
                ("DisplayName", "tt-git".to_string()),
                ("IconUri", icon.display().to_string()),
            ]
        );
    }

    /// The installer removes the key on uninstall, and names it for
    /// [`APP_ID`].
    #[test]
    fn the_installer_removes_the_key() {
        let path = format!("{}/packaging/windows/tt-git.iss", env!("CARGO_MANIFEST_DIR"));
        let iss = std::fs::read_to_string(&path).unwrap();
        let line = format!(r#"Subkey: "{}"; Flags: uninsdeletekey dontcreatekey"#, registry_key());
        assert!(iss.contains(&line), "{line} isn't in {path}");
    }
}
