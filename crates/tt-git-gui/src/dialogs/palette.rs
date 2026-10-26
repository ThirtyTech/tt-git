//! The command palette and the pickers (`Modal::Palette`): what's typed,
//! and the entries that match it, the highlighted one picked out.

use gpui::{AnyElement, BoxShadow, IntoElement, ParentElement, Styled, div, point, px, rems};
use tt_git_app::view::dialogs::DialogLook;
use tt_git_app::view::{StyleRole, TextSpan};

use super::DialogCx;
use super::prompt::{blank, dialog_box, list_row, screen_cols, single_line, title};

/// The id the palette is measured as.
pub const ID: &str = "dialog:palette";

pub fn render(cx: &mut DialogCx) -> AnyElement {
    let ui = cx.ui;
    let Some(view) = cx.app.modal().and_then(|m| m.palette_view(screen_cols(ui))) else {
        return div().into_any_element();
    };
    let look = DialogLook::Normal;
    let mut list = div().flex().flex_col().h(ui.font.line_height * view.list_rows as f32);
    if let Some(empty) = view.empty {
        list = list.child(single_line(ui, &[TextSpan::new(empty, StyleRole::Muted)]));
    }
    list = list.children(view.rows.iter().map(|row| list_row(ui, row)));
    let input = single_line(ui, &view.input);
    // Native: what's typed in a field, in the focus colors.
    let input = if ui.native() {
        let s = &ui.theme.surfaces;
        div()
            .flex_none()
            .flex()
            .items_center()
            .h(ui.font.line_height * 1.5)
            .px(rems(0.5))
            .rounded(rems(0.5))
            .border_1()
            .border_color(s.focus_border)
            .bg(s.card)
            .shadow(vec![BoxShadow {
                color: s.ring,
                offset: point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(3.),
                inset: false,
            }])
            .child(input)
    } else {
        input
    };
    let dialog = dialog_box(ui, ID, look, view.width, title(ui, view.title, look).element())
        .child(input)
        .child(blank(ui))
        .child(list);
    super::prompt::layer(ui, dialog).into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use tt_git_app::modal::Modal;
    use tt_git_app::view::dialogs::line_text;
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::dialogs::prompt::tests::{drawn, drawn_cols};
    use crate::testing::Gui;

    fn labels(gui: &Gui) -> Vec<String> {
        gui.read(|app| {
            let view = app.modal().unwrap().palette_view(200).unwrap();
            view.rows.iter().map(|r| line_text(&r.spans)).collect()
        })
    }

    #[gpui::test]
    fn the_palette_narrows_to_what_is_typed(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let mut gui = Gui::open(cx, &sb);
        gui.keystrokes("ctrl-p");
        assert!(matches!(
            gui.read(|app| app.modal().cloned()),
            Some(Modal::Palette { .. })
        ));
        assert_eq!(drawn_cols(&gui, ID), 70);
        let all = labels(&gui).len();
        assert!(all > 3, "{all} commands");
        let tall = drawn(&gui, ID).unwrap().size.height;

        gui.keystrokes("f e t c h");
        let shown = labels(&gui);
        assert!(!shown.is_empty() && shown.len() < all, "{shown:?}");
        assert!(shown[0].to_lowercase().contains("fetch"), "{shown:?}");
        assert!(
            drawn(&gui, ID).unwrap().size.height < tall,
            "shorter with fewer entries"
        );

        gui.keystrokes("z z z");
        assert!(labels(&gui).is_empty());
        assert!(drawn(&gui, ID).is_some(), "still drawn, saying no matches");

        gui.keystrokes("escape");
        assert!(gui.read(|app| app.modal().is_none()));
        assert!(drawn(&gui, ID).is_none());
    }

    #[gpui::test]
    fn enter_runs_the_highlighted_command(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let mut gui = Gui::open(cx, &sb);
        gui.keystrokes("ctrl-p");
        for c in "help".chars() {
            gui.press(&c.to_string());
        }
        let picked = gui.read(|app| {
            let view = app.modal().unwrap().palette_view(200).unwrap();
            view.rows.iter().find(|r| r.picked).map(|r| line_text(&r.spans))
        });
        assert!(picked.is_some_and(|l| l.to_lowercase().contains("help")));
        gui.keystrokes("enter");
        assert!(
            gui.read(|app| app.modal().is_none() && app.shows_help()),
            "the keys popup opened"
        );
    }
}
