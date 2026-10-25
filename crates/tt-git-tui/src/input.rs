//! Terminal input: crossterm's events turned into the app's, with mouse
//! positions hit-tested against what was drawn last frame.

use crossterm::event::{Event, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

use crate::app::App;
use crate::convert;
use crate::modal::Modal;
use crate::mouse::{MouseInput, MouseKind, MouseTarget};
use crate::panes::PaneId;

/// Handles a terminal event.
pub fn handle_event(app: &mut App, event: Event) {
    if let Event::Key(key) = &event {
        tracing::debug!(code = ?key.code, modifiers = ?key.modifiers, kind = ?key.kind, "key");
    }
    match event {
        // Windows reports releases too; only act on presses (and repeats).
        Event::Key(key) if key.kind != KeyEventKind::Release => app.key_pressed(convert::key_combo(&key)),
        Event::Mouse(m) if app.wants_mouse() => {
            let input = mouse_input(app, &m);
            app.handle_mouse(input);
        }
        Event::Resize(..) => app.resized(),
        _ => {}
    }
}

/// What a mouse event was over, from where things were drawn last frame.
fn mouse_input(app: &App, m: &MouseEvent) -> MouseInput {
    let kind = match m.kind {
        MouseEventKind::Down(MouseButton::Left) => MouseKind::Click,
        MouseEventKind::ScrollUp => MouseKind::ScrollUp,
        MouseEventKind::ScrollDown => MouseKind::ScrollDown,
        _ => MouseKind::Other,
    };
    MouseInput {
        kind,
        ctrl: m.modifiers.contains(KeyModifiers::CONTROL),
        target: target_at(app, Position::new(m.column, m.row)),
    }
}

fn target_at(app: &App, pos: Position) -> MouseTarget {
    if app.error().is_none()
        && let Some(Modal::Filter(panel)) = app.modal()
    {
        return MouseTarget::Filter(panel.spot_at(convert::rect(app.main_area()), pos));
    }
    let Some(&(pane, area)) = app.pane_areas().iter().find(|(_, r)| r.contains(pos.x, pos.y)) else {
        return MouseTarget::None;
    };
    let body_row = (pos.y > area.y && pos.y < area.bottom().saturating_sub(1)).then(|| (pos.y - area.y - 1) as usize);
    let part = match pane {
        PaneId::Branches => app.branches().part_at(pos),
        PaneId::Details => app.details().part_at(pos),
        _ => None,
    };
    MouseTarget::Pane { pane, body_row, part }
}
