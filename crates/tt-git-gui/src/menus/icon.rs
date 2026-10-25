//! The app icon, `assets/icon.png` (from `assets/icon.svg`, the site's
//! favicon).
//!
//! - X11: set on each window through `WindowOptions::icon`.
//! - Windows: the executable's icon resource (`build.rs`), which GPUI gives
//!   every window.
//! - Wayland: the compositor finds the icon from the window's app id
//!   (`tt-git`) in an installed `tt-git.desktop`, which packaging provides.
//! - macOS: the `.app` bundle's icon, which packaging provides.
//!
//! The title bar draws it too, at its left end ([`title_icon`]).

use std::sync::{Arc, LazyLock};

use gpui::{InteractiveElement as _, IntoElement, RenderImage, Styled as _, img, px};

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

/// The icon's size in the title bar, in logical pixels.
const TITLE_ICON_SIZE: f32 = 16.;

/// The title bar's icon, scaled down once from the 256px PNG: GPUI samples
/// an image without mipmaps, so drawing the big one at 16px would alias.
/// Twice the size it's drawn at, for high-DPI displays.
static TITLE_ICON: LazyLock<Option<Arc<RenderImage>>> = LazyLock::new(|| {
    let icon = window_icon()?;
    let side = (TITLE_ICON_SIZE * 2.) as u32;
    let mut small = image::imageops::resize(icon.as_ref(), side, side, image::imageops::FilterType::Lanczos3);
    // GPUI wants BGRA.
    for px in small.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new([image::Frame::new(small)])))
});

/// The app icon for the left end of a title bar, or nothing if it can't be
/// decoded.
pub fn title_icon() -> Option<impl IntoElement> {
    let icon = TITLE_ICON.clone()?;
    Some(
        img(icon)
            .debug_selector(|| "title icon".into())
            .flex_none()
            .size(px(TITLE_ICON_SIZE)),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_icon_decodes() {
        let icon = super::window_icon().expect("icon");
        assert_eq!(icon.dimensions(), (256, 256));
    }
}
