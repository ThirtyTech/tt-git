//! The window's style (`[ui] style`): native, with the panes as rounded
//! cards on a darker ground, headers and a status bar in the OS's UI font,
//! and dialogs with shadows; or terminal, drawn as the TUI draws it. One
//! setting for the whole app, every window, kept as a GPUI global as the
//! [zoom](crate::zoom) is, and switched by
//! [`Command::WindowStyle`](tt_git_app::keymap::Command::WindowStyle).
//!
//! The style changes where the theme's colors go, never the colors: the
//! native surfaces are mixed from the theme's background and text
//! ([`Surfaces`](crate::theme::Surfaces)). The panes' text stays in the
//! [content font](crate::font), so view models still fit it in columns.
//!
//! Native sizes are in rems, so they zoom with everything else.

use gpui::{App, Global, Rems, SharedString, rems};
pub use tt_git_app::config::UiStyle;

/// The style every window draws in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UiLook(pub UiStyle);

impl Global for UiLook {}

/// Sets the style every window draws in.
pub fn set(style: UiStyle, cx: &mut App) {
    if cx.try_global::<UiLook>().map(|l| l.0) != Some(style) {
        cx.set_global(UiLook(style));
    }
}

/// The style; native before [`set`].
pub fn style(cx: &App) -> UiStyle {
    cx.try_global::<UiLook>().map(|l| l.0).unwrap_or_default()
}

/// Switches to the other style, and returns it.
pub fn toggle(cx: &mut App) -> UiStyle {
    let style = style(cx).toggled();
    set(style, cx);
    style
}

/// A line's height over the content font's size: native rows get a little
/// more air.
pub fn line_spacing(style: UiStyle) -> f32 {
    match style {
        UiStyle::Native => 1.6,
        UiStyle::Terminal => 1.4,
    }
}

/// The family GPUI resolves to the OS's UI font on every platform (Segoe
/// UI on Windows, SF Pro on macOS, the desktop's sans on Linux), for the
/// native style's headers, tabs, status bar and dialog text.
pub fn ui_font_family() -> SharedString {
    ".SystemUIFont".into()
}

/// Between the panes, and between them and the window's edges.
pub const GUTTER: Rems = Rems(0.5);
/// A pane card's corners.
pub const CARD_RADIUS: Rems = Rems(0.625);
/// A pane card's header.
pub const HEADER_HEIGHT: Rems = Rems(2.25);
/// The room either side of a card's header.
pub const HEADER_PAD: Rems = Rems(0.75);
/// The room between a card's edges and its body.
pub const BODY_PAD: Rems = Rems(0.375);
/// A selected row's corners.
pub const ROW_RADIUS: Rems = Rems(0.375);
/// The room either side of a row's text, inside its highlight.
pub const ROW_INSET: Rems = Rems(0.375);
/// A dialog's corners.
pub const DIALOG_RADIUS: Rems = Rems(0.75);
/// A key chip's or a number chip's corners.
pub const CHIP_RADIUS: Rems = Rems(0.3125);
/// The focus ring around the focused card.
pub const RING: Rems = Rems(0.1875);
/// The status bar along the bottom.
pub const STATUS_BAR_HEIGHT: Rems = Rems(2.125);

/// Half of `r`, for sizes made from these.
pub fn half(r: Rems) -> Rems {
    rems(r.0 / 2.)
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use tt_git_app::keymap::Command;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::testing::Gui;

    fn drawn(gui: &Gui) -> (UiStyle, f32) {
        gui.shell.read_with(&gui.cx, |shell, _| {
            let ui = shell.ui();
            (ui.style, f32::from(ui.font.line_height))
        })
    }

    #[gpui::test]
    fn the_style_command_switches_between_native_and_terminal(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let mut gui = Gui::open(cx, &sb);
        // Native by default, with its taller rows (13 px at 1.6).
        assert_eq!(drawn(&gui), (UiStyle::Native, 21.));
        // In the palette, which runs it.
        gui.keystrokes("ctrl-p s t y l e");
        let labels = gui.read(|app| {
            let view = app.modal().unwrap().palette_view(200).unwrap();
            view.rows
                .iter()
                .map(|r| tt_git_app::view::dialogs::line_text(&r.spans))
                .collect::<Vec<_>>()
        });
        assert!(labels.iter().any(|l| l.contains("Native/terminal style")), "{labels:?}");
        gui.keystrokes("enter");
        assert_eq!(gui.cx.read(style), UiStyle::Terminal);
        assert_eq!(drawn(&gui), (UiStyle::Terminal, 18.));
        assert_eq!(
            gui.read(|app| app.notice().map(str::to_string)).as_deref(),
            Some("terminal style")
        );
        let fit = gui
            .shell
            .read_with(&gui.cx, |shell, _| shell.ui().fit(crate::panes::graph::ROWS));
        assert!(fit.rows > 0, "the graph drawn again: {fit:?}");

        gui.model.update(&mut gui.cx, |m, cx| {
            m.send(tt_git_app::Action::Command(Command::WindowStyle), cx)
        });
        gui.wait_idle();
        assert_eq!(drawn(&gui), (UiStyle::Native, 21.));
    }

    #[gpui::test]
    fn the_config_can_start_it_in_the_terminal_style(cx: &mut TestAppContext) {
        cx.update(|cx| set(UiStyle::Terminal, cx));
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let gui = Gui::open(cx, &sb);
        assert_eq!(drawn(&gui), (UiStyle::Terminal, 18.));
    }
}
