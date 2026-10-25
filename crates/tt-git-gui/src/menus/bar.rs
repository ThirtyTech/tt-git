//! The window's title bar: gpui-component's `TitleBar`, which moves the
//! window and draws its controls where the system doesn't (GNOME on
//! Wayland gives no server-side decorations; Windows with the title bar
//! drawn by the app), with tt-git's menus on Windows and Linux, where GPUI
//! shows no native menu bar, and the window title.
//!
//! A menu here is a gpui-component dropdown whose items dispatch the same
//! actions as the native menus, to the shell's focus handle, and show the
//! keymap's key beside the label.

use gpui::component::button::{Button, ButtonVariants as _};
use gpui::component::menu::{DropdownMenu as _, PopupMenu};
use gpui::component::{ActiveTheme as _, Sizable as _, TitleBar, h_flex};
use gpui::{
    App, Context, FocusHandle, InteractiveElement as _, IntoElement, ParentElement, SharedString, Styled,
    TitlebarOptions, Window, div,
};
use tt_git_app::view::StyleRole;

use super::{ItemSpec, MenuSpec};
use crate::ui::Ui;

/// Whether the window draws the menus: everywhere but macOS, whose menus
/// are native.
pub const DRAWS_MENUS: bool = !cfg!(target_os = "macos");

/// The window's title bar options: drawn by the app (see [`title_bar`]),
/// with `title` for the taskbar and window switcher.
pub fn titlebar_options(title: String) -> TitlebarOptions {
    TitlebarOptions {
        title: Some(title.into()),
        ..TitleBar::title_bar_options()
    }
}

/// The title bar: the app icon, the menus (unless native), then the title.
/// Picking an item gives the keys back to `focus`, the shell's handle, and
/// sends it the item's action.
pub fn title_bar(ui: &Ui, specs: &[MenuSpec], title: &str, focus: &FocusHandle) -> impl IntoElement {
    let theme = &ui.theme;
    let menus = DRAWS_MENUS.then(|| {
        h_flex()
            .gap_0p5()
            .children(specs.iter().enumerate().map(|(ix, menu)| menu_button(ix, menu, focus)))
    });
    TitleBar::new()
        .bg(theme.background)
        .border_color(theme.fg(StyleRole::Border))
        .child(
            h_flex()
                .gap_2()
                .children(super::icon::title_icon())
                .children(menus)
                .child(
                    div()
                        .text_color(theme.fg(StyleRole::Title))
                        .child(SharedString::from(title.to_string())),
                ),
        )
        // Keeps the title off the window controls.
        .child(div())
}

fn menu_button(ix: usize, menu: &MenuSpec, focus: &FocusHandle) -> impl IntoElement {
    let items = menu.items.clone();
    let focus = focus.clone();
    let refocus = focus.clone();
    let name = menu.name.clone();
    Button::new(("menu", ix))
        .debug_selector(|| format!("menu {name}"))
        .ghost()
        .small()
        .compact()
        .tab_stop(false)
        .label(menu.name.clone())
        .dropdown_menu(move |menu, _, cx| fill(menu, &items, &focus, cx))
        .on_open_change(move |open, window, cx| {
            // Back to the shell, so tt-git's keymap gets the keys again.
            if !*open {
                refocus.focus(window, cx);
            }
        })
}

fn fill(mut menu: PopupMenu, items: &[ItemSpec], focus: &FocusHandle, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let muted = cx.theme().muted_foreground;
    menu = menu.action_context(focus.clone()).min_w(gpui::px(220.));
    for item in items {
        menu = match item {
            ItemSpec::Separator => menu.separator(),
            ItemSpec::Item {
                label,
                key,
                action,
                enabled,
            } => {
                let (label, key) = (SharedString::from(label.clone()), key.clone().map(SharedString::from));
                menu.menu_element_with_disabled(action.boxed(), !enabled, move |_: &mut Window, _: &mut App| {
                    h_flex()
                        .debug_selector(|| format!("menu item {label}"))
                        .w_full()
                        .justify_between()
                        .gap_6()
                        .child(label.clone())
                        .children(key.clone().map(|k| div().text_color(muted).child(k)))
                })
            }
        };
    }
    menu
}

#[cfg(test)]
mod tests {
    use gpui::{Modifiers, TestAppContext};
    use tt_git_app::panes::PaneId;
    use tt_git_testkit::Sandbox;

    use crate::testing::Gui;

    fn click(gui: &mut Gui, selector: &'static str) {
        let bounds = gui
            .cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} drawn"));
        gui.cx.simulate_click(bounds.center(), Modifiers::none());
        gui.wait_idle();
    }

    #[gpui::test]
    fn the_title_bar_starts_with_the_app_icon(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let mut gui = Gui::open(cx, &sb);
        gui.draw();
        let icon = gui.cx.debug_bounds("title icon").expect("icon drawn");
        assert_eq!(icon.size, gpui::size(gpui::px(16.), gpui::px(16.)));
        assert!(icon.bottom() <= gpui::component::TITLE_BAR_HEIGHT, "{icon:?}");
        if super::DRAWS_MENUS {
            let menu = gui.cx.debug_bounds("menu tt-git").expect("menu drawn");
            assert!(icon.right() <= menu.left(), "{icon:?} {menu:?}");
        }
    }

    #[gpui::test]
    fn a_drawn_menu_runs_the_picked_command_and_gives_the_keys_back(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        sb.commit_file("b.txt", "b", "second");
        let mut gui = Gui::open(cx, &sb);
        click(&mut gui, "menu View");
        // The menu has the keys while it's open: j doesn't reach the graph.
        gui.keystrokes("j");
        assert_eq!(gui.read(|app| app.graph().selected_index()), 0);
        click(&mut gui, "menu item Branches");
        assert_eq!(gui.read(|app| app.focus()), PaneId::Branches);
        // And tt-git's keymap has them again.
        gui.keystrokes("tab");
        assert_eq!(gui.read(|app| app.focus()), PaneId::Graph);
    }
}
