//! The app icon, `assets/icon.png` (from `assets/icon.svg`, the site's
//! favicon).
//!
//! - X11: set on each window through `WindowOptions::icon`.
//! - Windows: the executable's icon resource (`build.rs`), which GPUI gives
//!   every window.
//! - Wayland: the compositor finds the icon from the window's app id
//!   (`tt-git`) in an installed `tt-git.desktop`, which packaging provides.
//! - macOS: the `.app` bundle's icon, which packaging provides.

use std::sync::Arc;

static PNG: &[u8] = include_bytes!("../../assets/icon.png");

/// The icon for `WindowOptions::icon`, or `None` if it can't be decoded.
pub fn window_icon() -> Option<Arc<image::RgbaImage>> {
    match image::load_from_memory_with_format(PNG, image::ImageFormat::Png) {
        Ok(img) => Some(Arc::new(img.into_rgba8())),
        Err(e) => {
            tracing::warn!("couldn't decode the app icon: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_icon_decodes() {
        let icon = super::window_icon().expect("icon");
        assert_eq!(icon.dimensions(), (256, 256));
    }
}
