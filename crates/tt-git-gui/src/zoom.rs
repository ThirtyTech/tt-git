//! The zoom: how big the app draws everything, in percent, as a browser
//! zooms. It's one setting for the whole app, every window, kept as a GPUI
//! global and started from `[ui] zoom` in the global config.
//!
//! [`MonoFont::new`](crate::font::MonoFont::new) scales the font (and with
//! it every cell, so the panes' text and spacing), and
//! [`GuiTheme::apply_to_components`](crate::theme::GuiTheme::apply_to_components)
//! scales gpui-component's font size, which its `Root` makes each window's
//! rem size, so whatever is sized in rems follows.

use gpui::{App, Global};
use tt_git_app::config::{DEFAULT_ZOOM, ZoomStep, clamp_zoom, step_zoom};

/// The zoom, in percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiZoom(pub u16);

impl Global for UiZoom {}

/// Sets the zoom to `percent` (kept within the zoom levels).
pub fn set(percent: u16, cx: &mut App) {
    let percent = clamp_zoom(percent);
    if percent != self::percent(cx) || !cx.has_global::<UiZoom>() {
        cx.set_global(UiZoom(percent));
    }
}

/// The zoom in percent; the default before [`set`].
pub fn percent(cx: &App) -> u16 {
    cx.try_global::<UiZoom>().map_or(DEFAULT_ZOOM, |z| z.0)
}

/// The zoom as a factor: 1.0 at 100%.
pub fn scale(cx: &App) -> f32 {
    f32::from(percent(cx)) / 100.
}

/// Zooms in, out or back to the default, and returns the new zoom.
pub fn step(step: ZoomStep, cx: &mut App) -> u16 {
    let percent = step_zoom(percent(cx), step);
    set(percent, cx);
    percent
}

#[cfg(test)]
mod tests {
    use gpui::component::Theme as ComponentTheme;
    use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, point, px};
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::testing::Gui;

    fn font_size(gui: &Gui) -> f32 {
        gui.shell.read_with(&gui.cx, |shell, _| f32::from(shell.ui().font.size))
    }

    fn rem(gui: &Gui) -> f32 {
        gui.cx.read(|cx| f32::from(ComponentTheme::global(cx).font_size))
    }

    #[gpui::test]
    fn the_zoom_keys_scale_the_font_and_the_rem_size(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let mut gui = Gui::open(cx, &sb);
        assert_eq!((font_size(&gui), rem(&gui)), (13., 16.));

        gui.press("ctrl+=");
        assert_eq!(gui.cx.read(percent), 110);
        assert!((font_size(&gui) - 14.3).abs() < 0.01, "{}", font_size(&gui));
        assert!((rem(&gui) - 17.6).abs() < 0.01);
        assert_eq!(
            gui.read(|app| app.notice().map(str::to_string)).as_deref(),
            Some("zoom 110%")
        );

        gui.press("ctrl+-").press("ctrl+-");
        assert_eq!(gui.cx.read(percent), 90);
        gui.press("ctrl+0");
        assert_eq!(gui.cx.read(percent), 100);
        assert_eq!(font_size(&gui), 13.);
    }

    #[gpui::test]
    fn the_window_sends_ctrl_equals_to_the_keymap(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let mut gui = Gui::open(cx, &sb);
        gui.keystrokes("ctrl-=");
        assert_eq!(gui.cx.read(percent), 110);
        gui.keystrokes("ctrl-0");
        assert_eq!(gui.cx.read(percent), 100);
    }

    #[gpui::test]
    fn ctrl_wheel_zooms_instead_of_scrolling(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        for i in 0..40 {
            sb.commit_file("a.txt", &i.to_string(), &format!("commit {i}"));
        }
        let mut gui = Gui::open(cx, &sb);
        let wheel = |lines: f32| ScrollWheelEvent {
            position: point(px(200.), px(200.)),
            delta: ScrollDelta::Lines(point(0., lines)),
            modifiers: Modifiers::control(),
            touch_phase: TouchPhase::Moved,
        };
        gui.cx.simulate_event(wheel(3.));
        gui.wait_idle();
        assert_eq!(gui.cx.read(percent), 110);
        gui.cx.simulate_event(wheel(-6.));
        gui.wait_idle();
        assert_eq!(gui.cx.read(percent), 90);
        assert_eq!(gui.read(|app| app.graph().selected_index()), 0, "nothing scrolled");
    }
}
