//! The `[layout]` tree from config, solved the way the TUI's
//! `draw/layout.rs` solves it (same [`View`] and [`Zoom`]), as nested flex
//! containers with a resize handle between neighbours.
//!
//! Dragging a handle resizes the panes beside it. Each split keeps the
//! sizes it was dragged to for as long as the window is open, even while
//! the layout shows something else (details open, a pane zoomed): see
//! [`KeptSplit`].

use std::collections::HashMap;

use gpui::component::{ResizablePanel, ResizablePanelGroup, ResizableState, h_resizable, resizable_panel, v_resizable};
use gpui::{
    AnyElement, AppContext, Axis, Entity, Global, IntoElement, ParentElement, Pixels, RenderOnce, SharedString, Size,
    Styled, Window, WindowId, div, px,
};
use tt_git_app::App;
use tt_git_app::app::Zoom;
use tt_git_app::layout::{Placed, Split, View};
use tt_git_app::panes::PaneId;

/// What the main area shows this frame: the solved layout tree, or `None`
/// when the layout shows nothing.
pub fn placed(app: &App) -> Option<Placed> {
    match app.zoom() {
        Zoom::Full(pane) => Some(Placed::Pane(pane)),
        _ => app.config().layout.resolve(app.view()),
    }
}

/// Where details would go once opened, for sizing a split plugin pane while
/// it's closed: the layout with details open.
pub fn placed_with_details(app: &App) -> Option<Placed> {
    let open = View {
        details: true,
        ..app.view()
    };
    match app.zoom() {
        Zoom::Full(pane) => Some(Placed::Pane(pane)),
        _ => app.config().layout.resolve(open),
    }
}

/// Every pane in `placed`, in drawing order.
pub fn panes(placed: &Placed) -> Vec<PaneId> {
    let mut out = Vec::new();
    collect(placed, &mut out);
    out
}

fn collect(placed: &Placed, out: &mut Vec<PaneId>) {
    match placed {
        Placed::Pane(p) => out.push(*p),
        Placed::Split(_, children) => children.iter().for_each(|(_, c)| collect(c, out)),
    }
}

/// `placed` as elements filling `size` (the main area), each pane drawn
/// by `pane`. Splits are resizable; a split's handles keep their place
/// while the same panes show in it, and start again from the config's
/// sizes when what it shows changes.
///
/// Each pane gets `pad` of room around it: the native style's gutters.
pub fn render(
    placed: &Placed,
    size: Size<Pixels>,
    pad: Pixels,
    pane: &mut dyn FnMut(PaneId) -> AnyElement,
) -> AnyElement {
    node(placed, size, pad, String::new(), pane)
}

fn node(
    placed: &Placed,
    size: Size<Pixels>,
    pad: Pixels,
    path: String,
    pane: &mut dyn FnMut(PaneId) -> AnyElement,
) -> AnyElement {
    match placed {
        Placed::Pane(p) => div().size_full().p(pad).child(pane(*p)).into_any_element(),
        Placed::Split(split, children) => {
            let axis = match split {
                Split::Rows => Axis::Vertical,
                Split::Columns => Axis::Horizontal,
            };
            let total: u32 = children.iter().map(|(w, _)| u32::from(*w)).sum::<u32>().max(1);
            let length = match axis {
                Axis::Horizontal => size.width,
                Axis::Vertical => size.height,
            };
            let id = SharedString::from(format!("split{path}:{}", signature(placed)));
            let panels = children.iter().enumerate().map(|(i, (weight, child))| {
                let share = length * (f32::from(*weight) / total as f32);
                let child_size = match axis {
                    Axis::Horizontal => Size {
                        width: share,
                        height: size.height,
                    },
                    Axis::Vertical => Size {
                        width: size.width,
                        height: share,
                    },
                };
                let element = node(child, child_size, pad, format!("{path}.{i}"), pane);
                resizable_panel().size(share.max(px(1.))).child(element)
            });
            KeptSplit {
                id,
                axis,
                panels: panels.collect(),
            }
            .into_any_element()
        }
    }
}

/// A split whose sizes outlive its frames: its state is kept by window
/// and id (the split's place and what it shows) in [`SplitStates`], where
/// GPUI's own element state would be dropped as soon as a frame didn't
/// draw it.
#[derive(IntoElement)]
struct KeptSplit {
    id: SharedString,
    axis: Axis,
    panels: Vec<ResizablePanel>,
}

impl RenderOnce for KeptSplit {
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let state = split_state(window, &self.id, cx);
        let group: ResizablePanelGroup = match self.axis {
            Axis::Horizontal => h_resizable(self.id),
            Axis::Vertical => v_resizable(self.id),
        };
        group.with_state(&state).children(self.panels)
    }
}

/// Every split's sizes, by window and split id, for the session.
#[derive(Default)]
struct SplitStates(HashMap<(WindowId, SharedString), Entity<ResizableState>>);

impl Global for SplitStates {}

/// The state of split `id` in `window`, made the first time it's drawn.
/// The window draws again whenever it changes (a drag), as it would for
/// state GPUI keeps itself.
fn split_state(window: &mut Window, id: &SharedString, cx: &mut gpui::App) -> Entity<ResizableState> {
    let key = (window.window_handle().window_id(), id.clone());
    if let Some(state) = cx.default_global::<SplitStates>().0.get(&key) {
        return state.clone();
    }
    let state = cx.new(|_| ResizableState::default());
    let handle = window.window_handle();
    cx.observe(&state, move |_, cx| {
        let _ = handle.update(cx, |_, window, _| window.refresh());
    })
    .detach();
    cx.global_mut::<SplitStates>().0.insert(key, state.clone());
    state
}

/// The sizes of the panes in split `id` of `window` along the split, as
/// dragged or as first laid out; `None` before it's drawn.
pub fn split_sizes(window: WindowId, id: &str, cx: &gpui::App) -> Option<Vec<Pixels>> {
    let states = cx.try_global::<SplitStates>()?;
    let state = states.0.get(&(window, SharedString::from(id.to_string())))?;
    Some(state.read(cx).sizes().clone())
}

/// What a split shows, so its handles start over when that changes.
fn signature(placed: &Placed) -> String {
    match placed {
        Placed::Pane(p) => p.name().to_string(),
        Placed::Split(split, children) => {
            let kind = match split {
                Split::Rows => "rows",
                Split::Columns => "cols",
            };
            let inner: Vec<String> = children.iter().map(|(w, c)| format!("{w}:{}", signature(c))).collect();
            format!("{kind}({})", inner.join(","))
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Modifiers, MouseButton, TestAppContext, point};
    use tt_git_testkit::Sandbox;

    use super::*;
    use crate::testing::Gui;

    fn width(gui: &Gui, pane: PaneId) -> u16 {
        gui.read(|app| app.pane_area(pane).expect("drawn").width)
    }

    #[gpui::test]
    fn dragging_a_handle_resizes_the_split_for_the_session(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        sb.commit_file("a.txt", "a", "first");
        let mut gui = Gui::open(cx, &sb);
        let status = gui.read(|app| app.pane_area(PaneId::Status).unwrap());
        let branches = gui.read(|app| app.pane_area(PaneId::Branches).unwrap());
        let before = status.width;
        // The handle between status and branches, on the boundary (in the
        // middle of the gutter between them, in the native style).
        let y = gpui::px(f32::from(status.y + status.height / 2));
        let edge = gpui::px((f32::from(status.right()) + f32::from(branches.x)) / 2.);
        let cx = &mut gui.cx;
        cx.simulate_mouse_move(point(edge, y), None, Modifiers::default());
        cx.simulate_mouse_down(point(edge, y), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(
            point(edge + gpui::px(50.), y),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.simulate_mouse_move(
            point(edge + gpui::px(120.), y),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.simulate_mouse_up(point(edge + gpui::px(120.), y), MouseButton::Left, Modifiers::default());
        gui.wait_idle();
        let after = width(&gui, PaneId::Status);
        assert!(after >= before + 100, "status grew: {before} then {after}");

        // Details takes their place, then gives it back: the drag is kept.
        gui.press_all(&["1", "space"]);
        assert!(gui.read(|app| app.pane_area(PaneId::Status).is_none()));
        gui.press("space");
        assert_eq!(width(&gui, PaneId::Status), after);
        let window = gui.window.window_id();
        let sizes = gui.cx.update(|_, cx| {
            let placed = gui
                .model
                .read(cx)
                .app()
                .config()
                .layout
                .resolve(gui.model.read(cx).app().view());
            let Some(Placed::Split(_, children)) = placed else {
                panic!("a split")
            };
            let id = format!("split.1:{}", signature(&children[1].1));
            split_sizes(window, &id, cx)
        });
        let sizes = sizes.expect("the split's sizes");
        assert_eq!(sizes.len(), 2, "{sizes:?}");
    }
}
