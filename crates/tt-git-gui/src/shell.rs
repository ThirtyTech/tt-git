//! [`Shell`]: the window's view. It draws the app as the TUI's `draw_app`
//! does (the panes in their layout, the help bar, then what opens over
//! them), sends it the keys, and reports where the panes went.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Pixels, PromptLevel, Render, Styled, Subscription, Window, canvas, div,
};
use tt_git_app::geom::Rect;
use tt_git_app::layout::{Placed, Split};
use tt_git_app::panes::PaneId;

use crate::dialogs::{self, DialogCx};
use crate::font::MonoFont;
use crate::frame::frame_id;
use crate::model::{AppEvent, AppModel};
use crate::panes::{self, PaneCx};
use crate::theme::GuiTheme;
use crate::ui::{Measures, Ui};
use crate::{CONTEXT, help_bar, keys, layout, terminal};

/// The id the area under the help bar is measured as.
pub const MAIN: &str = "main";

/// The window's root view, around an [`AppModel`].
pub struct Shell {
    model: Entity<AppModel>,
    ui: Ui,
    focus: FocusHandle,
    /// The window title last set.
    title: String,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(model: Entity<AppModel>, font: MonoFont, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let theme = GuiTheme::new(model.read(cx).app().theme());
        theme.apply_to_components(&font, cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe_in(&model, window, |_, model, event, window, cx| match event {
                AppEvent::Quit => {
                    // Opening another worktree quits the app and starts a
                    // new one there.
                    if let Some(repo) = model.update(cx, |m, _| m.app_mut().take_reopen())
                        && let Err(e) = crate::windows::open_repo(repo, cx)
                    {
                        tracing::error!("{e}");
                    }
                    window.remove_window();
                }
                AppEvent::OpenRepository => {
                    let model = model.downgrade();
                    crate::windows::pick_repos(cx, move |result, cx| {
                        if let Err(e) = result {
                            let _ = model.update(cx, |m, cx| m.send(tt_git_app::Action::Error(e), cx));
                        }
                    });
                }
            }),
            cx.observe_window_bounds(window, |shell, window, cx| {
                crate::window_state::remember_window(window, cx);
                shell.model.update(cx, |m, cx| m.resized(cx));
            }),
        ];
        ask_before_closing(&model, window, cx);
        Self {
            ui: Ui {
                theme: Rc::new(theme),
                font: Rc::new(font),
                measures: Measures::default(),
                model: model.downgrade(),
            },
            model,
            focus,
            title: String::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn model(&self) -> &Entity<AppModel> {
        &self.model
    }

    /// What the views draw with.
    pub fn ui(&self) -> &Ui {
        &self.ui
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = keys::key_combo(&event.keystroke, event.prefer_character_input) {
            cx.stop_propagation();
            self.model.update(cx, |m, cx| m.key(key, cx));
        }
    }

    /// Picks up a theme the app switched to (the theme picker, a theme
    /// file changing).
    fn sync_theme(&mut self, cx: &mut Context<Self>) {
        let app_theme = self.model.read(cx).app().theme();
        if app_theme != self.ui.theme.app_theme() {
            let theme = GuiTheme::new(app_theme);
            theme.apply_to_components(&self.ui.font, cx);
            self.ui.theme = Rc::new(theme);
        }
    }
}

/// Closing the window while a terminal editor runs in it (which would
/// end the editor and lose what it hasn't saved) asks first.
fn ask_before_closing(model: &Entity<AppModel>, window: &mut Window, cx: &mut Context<Shell>) {
    let model = model.downgrade();
    let asking = Rc::new(Cell::new(false));
    window.on_window_should_close(cx, move |window, cx| {
        let Some(model) = model.upgrade() else {
            return true;
        };
        if !model.read(cx).app().editor_running() {
            return true;
        }
        if asking.replace(true) {
            return false;
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            "An editor is still running",
            Some("Closing the window stops it, and anything it hasn't saved is lost."),
            &[CLOSE, "Cancel"],
            cx,
        );
        let (asking, handle) = (asking.clone(), window.window_handle());
        cx.spawn(async move |cx| {
            let close = answer.await == Ok(0);
            asking.set(false);
            if close {
                tracing::info!("closing the window with its editor running");
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        })
        .detach();
        false
    });
}

/// The button that closes the window anyway.
pub const CLOSE: &str = "Close";

impl Focusable for Shell {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_theme(cx);
        let ui = self.ui.clone();
        ui.measures.begin_frame();
        let title = self.model.read(cx).app().terminal_title();
        if title != self.title {
            window.set_window_title(&title);
            self.title = title;
        }

        let (main, help, overlays, shown) = self.model.update(cx, |model, _| {
            let app = model.app_mut();
            app.begin_frame();
            let focus = app.focus();
            let placed = layout::placed(app);
            let shown = placed.as_ref().map(layout::panes).unwrap_or_default();
            let main = match (&placed, ui.measures.get(MAIN)) {
                (Some(placed), Some(bounds)) => Some(layout::render(placed, bounds.size, &mut |pane| {
                    let mut cx = PaneCx {
                        app: &mut *app,
                        ui: &ui,
                        pane,
                        focused: focus == pane,
                    };
                    if pane == PaneId::Details
                        && let Some(split) = terminal::split(&mut cx)
                    {
                        return split;
                    }
                    panes::render(&mut cx)
                })),
                // Measured first, so the panes start at the right size.
                _ => None,
            };
            let help = help_bar::render(app, &ui);
            let mut cx = DialogCx { app, ui: &ui };
            let mut overlays: Vec<AnyElement> = Vec::new();
            overlays.extend(dialogs::help(&mut cx));
            overlays.extend(terminal::floating(&mut cx));
            overlays.extend(dialogs::modal(&mut cx));
            overlays.extend(dialogs::error(&mut cx));
            (main, help, overlays, shown)
        });

        let report = {
            let (model, measures) = (self.model.clone(), ui.measures.clone());
            canvas(
                move |_, _, cx| report_layout(&model, &measures, &shown, cx),
                |_, _, _, _| {},
            )
            .absolute()
            .size_0()
        };
        let theme = &ui.theme;
        div()
            .id("tt-git")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(ui.font.family.clone())
            .text_size(ui.font.size)
            .line_height(ui.font.line_height)
            .child(
                div()
                    .id(MAIN)
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(ui.measure(MAIN))
                    .children(main)
                    .children(overlays),
            )
            .child(help)
            // Last, so the panes have measured themselves by the time it
            // runs.
            .child(report)
    }
}

/// Tells the app where this frame drew each pane, the main area, and
/// where details is (or would be once opened), as `draw_app` does.
fn report_layout(model: &Entity<AppModel>, measures: &Measures, shown: &[PaneId], cx: &mut App) {
    let Some(main) = measures.this_frame(MAIN) else {
        return;
    };
    let panes: Vec<(PaneId, Rect)> = shown
        .iter()
        .filter_map(|&p| measures.this_frame(&frame_id(p)).map(|b| (p, rect(b))))
        .collect();
    model.update(cx, |model, _| {
        let app = model.app_mut();
        let details = panes
            .iter()
            .find(|(p, _)| *p == PaneId::Details)
            .map(|(_, r)| *r)
            .or_else(|| {
                let placed = layout::placed_with_details(app)?;
                area_of(&placed, main, PaneId::Details).map(rect)
            })
            .unwrap_or_else(|| rect(main));
        app.set_layout(panes, rect(main), details);
    });
}

/// Where `pane` goes in `placed` laid out in `bounds` at the config's
/// sizes (for a pane that isn't drawn, so has no measured place).
fn area_of(placed: &Placed, bounds: Bounds<Pixels>, pane: PaneId) -> Option<Bounds<Pixels>> {
    match placed {
        Placed::Pane(p) => (*p == pane).then_some(bounds),
        Placed::Split(split, children) => {
            let total: f32 = children.iter().map(|(w, _)| f32::from(*w)).sum::<f32>().max(1.);
            let mut origin = bounds.origin;
            for (weight, child) in children {
                let share = f32::from(*weight) / total;
                let size = match split {
                    Split::Rows => gpui::size(bounds.size.width, bounds.size.height * share),
                    Split::Columns => gpui::size(bounds.size.width * share, bounds.size.height),
                };
                if let Some(found) = area_of(child, Bounds { origin, size }, pane) {
                    return Some(found);
                }
                match split {
                    Split::Rows => origin.y += size.height,
                    Split::Columns => origin.x += size.width,
                }
            }
            None
        }
    }
}

/// `bounds` in the app's terms: whole logical pixels.
pub fn rect(bounds: Bounds<Pixels>) -> Rect {
    let px = |p: Pixels| f32::from(p).round().clamp(0., f32::from(u16::MAX)) as u16;
    Rect::new(
        px(bounds.origin.x),
        px(bounds.origin.y),
        px(bounds.size.width),
        px(bounds.size.height),
    )
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use tt_git_app::panes::PaneId;
    use tt_git_testkit::Sandbox;

    use crate::testing::Gui;

    fn sandbox() -> Sandbox {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        sb.commit_file("b.txt", "b", "second");
        sb
    }

    #[gpui::test]
    fn the_window_opens_on_the_repo(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        assert!(gui.is_open());
        assert_eq!(gui.cx.window_title().as_deref(), Some("repo - tt-git"));
        assert_eq!(gui.read(|app| app.focus()), PaneId::Graph);
    }

    #[gpui::test]
    fn a_key_typed_at_once_lands_in_the_dialog_the_last_one_opened(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        // `n` opens the new-branch prompt through an action; `x` follows
        // before any task runs, as fast typing does.
        for key in ["n", "x"] {
            let combo = key.parse().unwrap();
            gui.model.update(&mut gui.cx, |model, cx| model.key(combo, cx));
        }
        let input = gui.read(|app| match app.modal() {
            Some(tt_git_app::modal::Modal::Prompt { input, .. }) => Some(input.clone()),
            _ => None,
        });
        assert_eq!(input.as_deref(), Some("x"), "x went to the prompt, not the graph");
    }

    #[gpui::test]
    fn keys_reach_the_keymap(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        // Through GPUI's own key dispatch: tab isn't taken by gpui-component.
        gui.keystrokes("tab");
        assert_eq!(gui.read(|app| app.focus()), PaneId::Status);
        gui.keystrokes("tab");
        assert_eq!(gui.read(|app| app.focus()), PaneId::Branches);
        gui.keystrokes("shift-tab shift-tab");
        assert_eq!(gui.read(|app| app.focus()), PaneId::Graph);
        gui.keystrokes("j");
        assert_eq!(gui.read(|app| app.graph().selected_index()), 1);
        gui.keystrokes("?");
        assert!(gui.read(|app| app.shows_help()), "? opens the keys");
        gui.keystrokes("escape");
        assert!(!gui.read(|app| app.shows_help()));
    }

    #[gpui::test]
    fn focus_moves_by_where_panes_were_drawn(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        // ctrl+j/l move by the rects the shell reported.
        gui.press("ctrl+j");
        assert_eq!(gui.read(|app| app.focus()), PaneId::Status);
        gui.press("ctrl+l");
        assert_eq!(gui.read(|app| app.focus()), PaneId::Branches);
        gui.press("ctrl+k");
        assert_eq!(gui.read(|app| app.focus()), PaneId::Graph);
    }

    #[gpui::test]
    fn set_layout_gets_the_pane_rects(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        let (areas, main) = gui.read(|app| (app.pane_areas().to_vec(), app.main_area()));
        let panes: Vec<PaneId> = areas.iter().map(|(p, _)| *p).collect();
        assert_eq!(panes, [PaneId::Graph, PaneId::Status, PaneId::Branches]);
        let rect = |p| areas.iter().find(|(q, _)| *q == p).unwrap().1;
        let (graph, status, branches) = (rect(PaneId::Graph), rect(PaneId::Status), rect(PaneId::Branches));
        // The default layout: graph over status and branches, side by side,
        // in pixels inside the area above the help bar.
        assert!(main.width > 1000 && main.height > 500, "{main:?}");
        assert!(
            graph.width >= main.width - 2 && graph.height > status.height,
            "{graph:?}"
        );
        assert!(
            status.y >= graph.bottom() - 1 && branches.y == status.y,
            "{status:?} {branches:?}"
        );
        assert!(status.right() <= branches.x + 1, "{status:?} {branches:?}");
        assert!(main.bottom() >= status.bottom(), "{main:?} {status:?}");

        // Opening details puts it in their place.
        gui.press("space");
        let panes: Vec<PaneId> = gui.read(|app| app.pane_areas().iter().map(|(p, _)| *p).collect());
        assert_eq!(panes, [PaneId::Graph, PaneId::Details]);
    }

    #[gpui::test]
    fn quitting_closes_the_window(cx: &mut TestAppContext) {
        let sb = sandbox();
        let mut gui = Gui::open(cx, &sb);
        gui.press("q");
        assert!(!gui.is_open());
    }
}
