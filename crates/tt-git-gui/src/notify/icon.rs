//! Windows: the app icon on the toasts.
//!
//! The toasts go out as [`APP_ID`], which GPUI registers under
//! `HKCU\Software\Classes\AppUserModelId\<APP_ID>` with a `DisplayName`
//! only. Without an `IconUri` there, and with no Start menu shortcut
//! carrying the id (`cargo run`), Windows shows them with no icon. So
//! [`install`] puts two copies of the app icon in the state folder, every
//! launch, so a moved state folder or a newer icon is picked up:
//!
//! - [`badge_name`], `assets/notification-badge.png`, as the `IconUri`: the
//!   small icon by the app's name at the top of the toast. Windows shows a
//!   placeholder there for the 256px icon, and for a 32px one scaled with
//!   the `image` crate, but not for ImageMagick's, so it's made ahead of
//!   time: `magick assets/icon.png -resize 32x32 -strip
//!   assets/notification-badge.png`.
//! - [`FILE`], the full icon: the large logo left of the text, which
//!   `toast.rs` puts in each toast (`appLogoOverride`).

use std::io;
use std::path::{Path, PathBuf};

use crate::app_id::{APP_ID, APP_NAME};

/// The full icon's file in the state folder.
pub const FILE: &str = "notification-icon.png";

/// The badge, 32px: what the Windows Community Toolkit registers.
static BADGE_PNG: &[u8] = include_bytes!("../../assets/notification-badge.png");

/// The start of the badge's file name in the state folder.
const BADGE_PREFIX: &str = "notification-badge-";

/// The badge's file name in the state folder, named for its contents:
/// Windows keeps showing what it first found at an `IconUri`, so a new
/// badge needs a new name.
pub fn badge_name() -> String {
    use sha2::{Digest as _, Sha256};
    let hash = Sha256::digest(BADGE_PNG);
    let hex: String = hash[..4].iter().map(|b| format!("{b:02x}")).collect();
    format!("{BADGE_PREFIX}{hex}.png")
}

/// The key the toasts' app id is registered under, below `HKEY_CURRENT_USER`.
pub fn registry_key() -> String {
    format!(r"Software\Classes\AppUserModelId\{APP_ID}")
}

/// The values [`install`] sets on [`registry_key`], for the badge at `badge`.
pub fn registry_values(badge: &Path) -> [(&'static str, String); 2] {
    [
        ("DisplayName", APP_NAME.to_string()),
        ("IconUri", badge.display().to_string()),
    ]
}

/// Writes `bytes` to `name` in `dir`, unless it's there as it is, and
/// returns its path.
fn write_file(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let path = dir.join(name);
    if std::fs::read(&path).is_ok_and(|have| have == bytes) {
        return Ok(path);
    }
    std::fs::create_dir_all(dir)?;
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Writes the full icon to [`FILE`] in `dir`, and returns its path.
pub fn write(dir: &Path) -> io::Result<PathBuf> {
    write_file(dir, FILE, crate::menus::icon::PNG)
}

/// Writes the badge to [`badge_name`] in `dir`, removes older badges there,
/// and returns its path.
pub fn write_badge(dir: &Path) -> io::Result<PathBuf> {
    let name = badge_name();
    let path = write_file(dir, &name, BADGE_PNG)?;
    for entry in std::fs::read_dir(dir)?.flatten() {
        let old = entry.file_name();
        let old = old.to_string_lossy();
        if old.starts_with(BADGE_PREFIX) && old.ends_with(".png") && old != name.as_str() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(path)
}

/// Writes both icons to `state_dir` and registers the badge for the toasts.
/// Returns the full icon's path, for the toasts' logo.
#[cfg(windows)]
pub fn install(state_dir: &Path) -> Option<PathBuf> {
    match write_badge(state_dir) {
        Ok(badge) => {
            let set = windows_registry::CURRENT_USER.create(registry_key()).and_then(|key| {
                registry_values(&badge)
                    .iter()
                    .try_for_each(|(name, value)| key.set_string(*name, value))
            });
            match set {
                Ok(()) => tracing::debug!("notifications' badge is {}", badge.display()),
                Err(e) => tracing::warn!("couldn't register the notification icon: {e}"),
            }
        }
        Err(e) => tracing::warn!("couldn't write the notification badge to {}: {e}", state_dir.display()),
    }
    match write(state_dir) {
        Ok(icon) => Some(icon),
        Err(e) => {
            tracing::warn!("couldn't write the notification icon to {}: {e}", state_dir.display());
            None
        }
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
    fn the_badge_is_a_small_png_named_for_its_contents() {
        let badge = image::load_from_memory_with_format(BADGE_PNG, image::ImageFormat::Png).unwrap();
        assert_eq!((badge.width(), badge.height()), (32, 32));
        let name = badge_name();
        assert!(name.starts_with(BADGE_PREFIX) && name.ends_with(".png"), "{name}");
        assert_eq!(name.len(), BADGE_PREFIX.len() + 8 + 4, "{name}");
    }

    #[test]
    fn writing_the_badge_removes_older_ones() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join(format!("{BADGE_PREFIX}00000000.png"));
        std::fs::write(&old, b"old").unwrap();
        std::fs::write(dir.path().join(FILE), b"icon").unwrap();
        let path = write_badge(dir.path()).unwrap();
        assert_eq!(path, dir.path().join(badge_name()));
        assert_eq!(std::fs::read(&path).unwrap(), BADGE_PNG);
        assert!(!old.exists(), "the old badge is gone");
        assert!(dir.path().join(FILE).exists(), "the full icon stays");
    }

    #[test]
    fn the_registry_names_the_app_and_its_badge() {
        assert_eq!(registry_key(), format!(r"Software\Classes\AppUserModelId\{APP_ID}"));
        let badge = Path::new(r"C:\Users\u\AppData\Roaming\tt-git\notification-badge-0123abcd.png");
        assert_eq!(
            registry_values(badge),
            [
                ("DisplayName", "tt-git".to_string()),
                ("IconUri", badge.display().to_string()),
            ]
        );
    }

    /// The installer removes the key on uninstall, and names it for
    /// [`RELEASE_APP_ID`].
    #[test]
    fn the_installer_removes_the_key() {
        let path = format!("{}/packaging/windows/tt-git.iss", env!("CARGO_MANIFEST_DIR"));
        let iss = std::fs::read_to_string(&path).unwrap();
        let key = format!(r"Software\Classes\AppUserModelId\{}", crate::app_id::RELEASE_APP_ID);
        let line = format!(r#"Subkey: "{key}"; Flags: uninsdeletekey dontcreatekey"#);
        assert!(iss.contains(&line), "{line} isn't in {path}");
    }
}
