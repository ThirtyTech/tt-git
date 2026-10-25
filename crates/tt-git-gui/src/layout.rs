//! The `[layout]` tree from config, solved the way the TUI's
//! `draw/layout.rs` solves it (same [`View`] and [`Zoom`]), as nested flex
//! containers with a resize handle between neighbours.

use gpui::component::{ResizablePanelGroup, h_resizable, resizable_panel, v_resizable};
use gpui::{AnyElement, Axis, IntoElement, ParentElement, Pixels, SharedString, Size, Styled, div, px};
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
pub fn render(placed: &Placed, size: Size<Pixels>, pane: &mut dyn FnMut(PaneId) -> AnyElement) -> AnyElement {
    node(placed, size, String::new(), pane)
}

fn node(placed: &Placed, size: Size<Pixels>, path: String, pane: &mut dyn FnMut(PaneId) -> AnyElement) -> AnyElement {
    match placed {
        Placed::Pane(p) => div().size_full().child(pane(*p)).into_any_element(),
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
            let group: ResizablePanelGroup = match axis {
                Axis::Horizontal => h_resizable(id),
                Axis::Vertical => v_resizable(id),
            };
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
                let element = node(child, child_size, format!("{path}.{i}"), pane);
                resizable_panel().size(share.max(px(1.))).child(element)
            });
            group.children(panels.collect::<Vec<_>>()).into_any_element()
        }
    }
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
