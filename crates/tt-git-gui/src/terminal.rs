//! Plugin and agent panes (Phase B8): a pseudo-terminal's screen, split in
//! place of details or floating over the panes, as the TUI's `draw/pty.rs`
//! draws them, from the same view model (`tt_git_app::view::terminal`).
//!
//! The screen is painted by [`TerminalGrid`](grid::TerminalGrid), which
//! measures itself: the PTY gets the rows and columns that fit there in
//! the font's cells. Keys need nothing here: the keymap sends them to the
//! focused pane as it does in the TUI. Every frame tells the app the font's
//! cell size, so it sizes a new pane's PTY from the pixel areas the shell
//! reports.

use gpui::{AnyElement, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div, px};
use tt_git_app::App;
use tt_git_app::panes::PaneId;
use tt_git_app::pty_pane::PtyPane;
use tt_git_app::view::terminal::{TerminalSpot, leave_hint};
use tt_git_app::view::{StyleRole, TextSpan};

use crate::dialogs::DialogCx;
use crate::frame::PaneFrame;
use crate::overlay;
use crate::panes::PaneCx;
use crate::shell::MAIN;
use crate::text::StyledLine;
use crate::ui::Ui;

pub mod grid;

#[cfg(test)]
mod editor_tests;

use grid::TerminalGrid;

/// The id a pane's screen is measured as.
pub fn grid_id(pane: tt_git_app::pty_pane::PaneId) -> SharedString {
    format!("terminal:{pane}").into()
}

/// The plugin pane split in place of details, if there is one.
pub fn split(cx: &mut PaneCx) -> Option<AnyElement> {
    let (ui, focused) = (cx.ui, cx.focused);
    set_cell_size(cx.app, ui);
    let hint = leave_hint(&cx.app.release_hint());
    let pane = cx.app.split_pane()?;
    let title = vec![
        TextSpan::plain(pane.frame_title()),
        TextSpan::new(format!("  {hint}"), StyleRole::Muted),
    ];
    let grid = screen(ui, pane, focused);
    Some(
        PaneFrame::new(ui, PaneId::Details, focused)
            .title_spans(title)
            .child(grid)
            .into_any_element(),
    )
}

/// The floating plugin panes (popups and modal panes), the topmost last.
/// Only the topmost has the focus, as only it gets keys.
pub fn floating(cx: &mut DialogCx) -> Vec<AnyElement> {
    let ui = cx.ui;
    set_cell_size(cx.app, ui);
    let main = ui.fit(MAIN);
    let release = cx.app.release_hint();
    let panes = cx.app.plugin_panes_mut();
    let top = panes.iter().rposition(PtyPane::is_modal);
    panes
        .iter_mut()
        .enumerate()
        .filter(|(_, p)| p.is_modal())
        .map(|(i, pane)| {
            let focused = Some(i) == top;
            let spot = pane.spot(cells(main.cols), cells(main.rows));
            let hint = pane.leave_text(&release);
            let body = floating_box(ui, pane, focused, &hint);
            match spot {
                TerminalSpot::Centered { cols, rows } => overlay::centered(
                    body.w(ui.font.cell_width * f32::from(cols))
                        .h(ui.font.line_height * f32::from(rows)),
                )
                .into_any_element(),
                TerminalSpot::Main | TerminalSpot::Details => div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .child(body.size_full())
                    .into_any_element(),
            }
        })
        .collect()
}

/// Tells the app how big a cell is, in the pixels the shell reports.
fn set_cell_size(app: &mut App, ui: &Ui) {
    let px = |p: gpui::Pixels| f32::from(p).round().clamp(1., f32::from(u16::MAX)) as u16;
    app.set_cell_size(px(ui.font.cell_width), px(ui.font.line_height));
}

fn cells(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// The pane's screen, at the size that fit last frame.
fn screen(ui: &Ui, pane: &mut PtyPane, focused: bool) -> TerminalGrid {
    let id = grid_id(pane.id);
    let fit = ui.fit(&id);
    let view = pane.view(cells(fit.rows), cells(fit.cols), focused);
    TerminalGrid::new(id, ui.measures.clone(), ui.font.clone(), &ui.theme, &view)
}

/// A floating pane's box: the title and how to leave on top, the screen
/// below. It takes the mouse, so nothing under it gets clicks.
fn floating_box(ui: &Ui, pane: &mut PtyPane, focused: bool, hint: &str) -> gpui::Stateful<gpui::Div> {
    let theme = &ui.theme;
    let (border, title_role) = if focused {
        (StyleRole::BorderFocused, StyleRole::TitleFocused)
    } else {
        (StyleRole::Border, StyleRole::Title)
    };
    let title = StyledLine::new(theme, &[TextSpan::plain(pane.frame_title())], Some(title_role));
    let hint = StyledLine::new(theme, &[TextSpan::new(hint.to_string(), StyleRole::Muted)], None);
    let grid = screen(ui, pane, focused);
    div()
        .id(grid_id(pane.id))
        .occlude()
        .flex()
        .flex_col()
        .overflow_hidden()
        .border_1()
        .rounded(px(4.))
        .border_color(theme.fg(border))
        .bg(theme.background)
        .shadow_lg()
        .child(
            div()
                .flex_none()
                .flex()
                .justify_between()
                .gap(ui.font.cell_width * 2.)
                .h(ui.font.line_height)
                .px(ui.font.cell_width)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(theme.fg(title_role))
                .child(title.element())
                .child(hint.element()),
        )
        .child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .px(ui.font.cell_width / 2.)
                .overflow_hidden()
                .child(grid),
        )
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use gpui::TestAppContext;
    use tt_git_app::Config;
    use tt_git_app::app::{PaneRequest, PluginOptions};
    use tt_git_app::panes::PaneId;
    use tt_git_testkit::{PluginFixture, Sandbox};

    use super::*;
    use crate::font::Fit;
    use crate::testing::Gui;

    /// A plugin whose pane `cat` runs `git cat-file --batch-check` on the
    /// sandbox repo: it echoes what's typed and answers each line, on
    /// every OS. `extra` is more TOML for the `[[panes]]` entry.
    fn sandbox(placement: &str, extra: &str) -> Sandbox {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let repo = sb.path().display().to_string().replace('\\', "\\\\");
        PluginFixture::new(&sb, "acme.term", Path::new("git"))
            .raw(&format!(
                "[[panes]]\nid = \"cat\"\ntitle = \"Cat\"\nplacement = \"{placement}\"\n{extra}\
                 command = [\"git\", \"-C\", \"{repo}\", \"cat-file\", \"--batch-check\"]\n"
            ))
            .install(&sb);
        sb
    }

    /// The GUI on `sb` with its plugins loaded and the pane open; returns
    /// the pane's id.
    fn open(cx: &mut TestAppContext, sb: &Sandbox) -> (Gui, u64) {
        let mut gui = Gui::open(cx, sb);
        let id = gui.model.update(&mut gui.cx, |model, _| {
            let app = model.app_mut();
            app.enable_plugins(PluginOptions {
                dirs: sb.plugin_dirs(),
                bin_path: None,
                socket: false,
                extra_env: sb.env(),
            });
            app.open_pane(
                "acme.term",
                "cat",
                PaneRequest {
                    focus: true,
                    ..Default::default()
                },
            )
            .expect("pane opens")
        });
        gui.draw();
        (gui, id)
    }

    fn until(gui: &mut Gui, what: &str, mut done: impl FnMut(&App) -> bool) {
        let start = Instant::now();
        loop {
            gui.draw();
            if gui.read(&mut done) {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn pane(app: &App) -> &PtyPane {
        app.plugin_panes().last().expect("a pane is open")
    }

    fn fit(gui: &Gui, id: u64) -> Fit {
        gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(&grid_id(id)))
    }

    fn type_line(gui: &mut Gui, text: &str) {
        for c in text.chars() {
            gui.press(&c.to_string());
        }
        // Through the window's own key dispatch.
        gui.keystrokes("enter");
    }

    #[gpui::test]
    fn a_split_pane_runs_in_place_of_details(cx: &mut TestAppContext) {
        let sb = sandbox("split", "");
        let (mut gui, id) = open(cx, &sb);
        assert_eq!(gui.read(|app| app.focus()), PaneId::Details);
        let areas: Vec<PaneId> = gui.read(|app| app.pane_areas().iter().map(|(p, _)| *p).collect());
        assert!(areas.contains(&PaneId::Details), "{areas:?}");

        // The PTY is as big as the screen's place, in the font's cells.
        let fit = fit(&gui, id);
        assert!(fit.rows > 5 && fit.cols > 20, "{fit:?}");
        assert_eq!(gui.read(|app| pane(app).size()), (fit.rows as u16, fit.cols as u16));

        // Keys go to the process.
        type_line(&mut gui, "HEAD");
        until(&mut gui, "the answer", |app| {
            pane(app).screen_text().contains(" commit ")
        });
        let screen = gui.read(|app| pane(app).screen_text());
        assert!(screen.contains("HEAD"), "the terminal echoes: {screen:?}");
        assert!(!gui.read(App::should_quit));

        // The release key leaves it open and focuses the graph.
        gui.press("f12");
        assert_eq!(gui.read(|app| app.focus()), PaneId::Graph);
        assert_eq!(gui.read(|app| app.plugin_panes().len()), 1);
        gui.press("4");
        assert_eq!(gui.read(|app| app.focus()), PaneId::Details);
    }

    #[gpui::test]
    fn a_new_pty_is_sized_in_cells(cx: &mut TestAppContext) {
        let sb = sandbox("split", "");
        let mut gui = Gui::open(cx, &sb);
        // Opened before any frame has drawn it: sized from the pixel area
        // the shell reported, in cells.
        let (size, main) = gui.model.update(&mut gui.cx, |model, _| {
            let app = model.app_mut();
            app.enable_plugins(PluginOptions {
                dirs: sb.plugin_dirs(),
                bin_path: None,
                socket: false,
                extra_env: sb.env(),
            });
            app.open_pane("acme.term", "cat", PaneRequest::default()).unwrap();
            (pane(app).size(), app.main_area())
        });
        let font = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().font.clone());
        let cols = (f32::from(main.width) / f32::from(font.cell_width)) as u16;
        assert!(size.1 < cols && size.1 > 20, "{size:?} in {main:?}");
    }

    #[gpui::test]
    fn a_popup_gets_the_manifest_size(cx: &mut TestAppContext) {
        let sb = sandbox("popup", "width = 40\nheight = 12\n");
        let (mut gui, id) = open(cx, &sb);
        // 40 by 12 cells with the border, the title line and the padding.
        assert_eq!(fit(&gui, id), Fit { cols: 38, rows: 10 });
        assert_eq!(gui.read(|app| pane(app).size()), (10, 38));
        // The topmost floating pane takes the keys.
        type_line(&mut gui, "HEAD");
        until(&mut gui, "the answer", |app| {
            pane(app).screen_text().contains(" commit ")
        });
        gui.press("f12");
        assert!(gui.read(|app| app.plugin_panes().is_empty()));
        assert_eq!(gui.read(|app| app.focus()), PaneId::Graph);
    }

    #[gpui::test]
    fn an_overlay_fills_the_main_area(cx: &mut TestAppContext) {
        let sb = sandbox("overlay", "");
        let (mut gui, id) = open(cx, &sb);
        let main = gui.shell.read_with(&gui.cx, |shell, _| shell.ui().fit(MAIN));
        let fit = fit(&gui, id);
        assert!(
            fit.cols + 2 >= main.cols && fit.rows + 2 >= main.rows,
            "{fit:?} in {main:?}"
        );
        // The focused pane shows the cursor where the process left it.
        let cursor = gui.model.update(&mut gui.cx, |model, _| {
            let pane = model.app_mut().plugin_panes_mut().last_mut().unwrap();
            let (rows, cols) = pane.size();
            pane.view(rows, cols, true).cursor
        });
        assert!(cursor.is_some(), "a running pane has a cursor");
        gui.press("f12");
        assert!(gui.read(|app| app.plugin_panes().is_empty()));
    }

    #[gpui::test]
    fn the_agent_pane_shows_how_it_ended(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("f.txt", "a\n", "first");
        let cfg = Config::parse(
            "[agent]\ncommand = \"git hash-object -w --no-filters {prompt_file}\"\nplacement = \"details\"\n",
        )
        .unwrap();
        let mut gui = Gui::with_config(cx, &sb, cfg);
        gui.press_all(&["A", "tab", "enter"]);
        until(&mut gui, "the agent to finish", |app| {
            app.plugin_panes().first().is_some_and(|p| p.exited.is_some())
        });
        let (title, screen) = gui.read(|app| (pane(app).frame_title(), pane(app).screen_text()));
        assert!(
            title.starts_with("agent: ") && title.contains("[process exited"),
            "{title}"
        );
        assert_eq!(
            screen.trim().lines().next().unwrap_or_default().trim().len(),
            40,
            "{screen:?}"
        );
        // Any key dismisses it, and details is back.
        gui.press("x");
        assert!(gui.read(|app| app.plugin_panes().is_empty()));
    }
}
