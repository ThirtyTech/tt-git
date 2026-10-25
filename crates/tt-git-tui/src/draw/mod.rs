//! Drawing the app into a ratatui frame.

use std::ops::Range;

use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, StatusLook, Zoom};
use crate::convert;
use crate::layout::View;
use crate::panes::{BranchTab, PaneId, StageSideId};
use crate::tui_theme::Theme;
use spans::truncate_spans;

pub mod blame;
mod branches;
mod details;
pub mod filter;
mod graph;
pub mod layout;
mod modal;
mod pty;
pub mod spans;
pub mod stage;
pub mod status;

/// The terminal screen: draws the app and turns terminal input into the
/// app's, keeping what the mouse needs from one frame to the next.
#[derive(Debug, Default)]
pub struct Screen {
    hits: Hits,
}

impl Screen {
    /// Draws a frame of `app`.
    pub fn draw(&mut self, app: &mut App, frame: &mut Frame) {
        self.hits = Hits::default();
        draw_app(app, frame, &mut self.hits);
    }

    /// Handles a terminal event.
    pub fn handle_event(&self, app: &mut App, event: Event) {
        crate::input::handle_event(app, event, &self.hits);
    }
}

fn draw_app(app: &mut App, frame: &mut Frame, hits: &mut Hits) {
    app.begin_frame();
    let [main, help] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());
    let pane_areas = match app.zoom() {
        Zoom::Full(pane) => vec![(pane, main)],
        _ => layout::place(&app.config().layout, main, app.view()),
    };
    // Where details would open, for sizing split plugin panes while it's closed.
    let details_area = pane_areas
        .iter()
        .find(|(p, _)| *p == PaneId::Details)
        .map(|(_, r)| *r)
        .unwrap_or_else(|| {
            let open = View {
                details: true,
                ..app.view()
            };
            let placed = layout::place(&app.config().layout, main, open);
            placed
                .into_iter()
                .find(|(p, _)| *p == PaneId::Details)
                .map_or(main, |(_, r)| r)
        });
    app.set_layout(
        pane_areas.iter().map(|(p, r)| (*p, convert::geom_rect(*r))).collect(),
        convert::geom_rect(main),
        convert::geom_rect(details_area),
    );
    let focus = app.focus();
    let theme = &Theme::from(app.theme());
    for &(pane, area) in &pane_areas {
        match pane {
            PaneId::Status => app.status_mut().draw(frame, area, focus == pane, theme, hits),
            PaneId::Branches => app.branches_mut().draw(frame, area, focus == pane, theme, hits),
            PaneId::Graph => app.graph_mut().draw(frame, area, focus == pane, theme, hits),
            PaneId::Details => {}
        }
    }
    if app.pane_area(PaneId::Details).is_some() && !draw_split_pane(app, frame, details_area, theme) {
        // Fullscreen has room for the file list beside the diff.
        let beside = app.zoom() == Zoom::Full(PaneId::Details);
        let details = app.details_mut();
        details.set_files_beside(beside);
        details.draw(frame, details_area, focus == PaneId::Details, theme, hits);
    }

    draw_help_bar(app, frame, help, theme);
    if app.shows_help() {
        modal::draw_help_popup(app, frame, main, theme);
    }
    draw_modal_panes(app, frame, main, details_area, theme);
    if let Some(modal) = app.modal() {
        modal::draw_modal(modal, frame, main, theme);
    }
    if let Some(err) = app.error() {
        modal::draw_error_popup(frame, main, err, theme);
    }
    if app.config().theme_background
        && let (Some(bg), Some(fg)) = (theme.background, theme.foreground)
    {
        paint_background(frame, bg, fg);
    }
}

/// Draws the plugin pane split in place of details into `details`, if
/// there is one. Returns whether it did.
fn draw_split_pane(app: &mut App, frame: &mut Frame, details: Rect, theme: &Theme) -> bool {
    let focused = app.focus() == PaneId::Details;
    let hint = app.release_hint();
    let Some(pane) = app.split_pane() else {
        return false;
    };
    if pty::draw(pane, frame, details, focused, &hint, theme) {
        app.set_cursor_shown();
    }
    true
}

/// Draws the floating plugin panes, the topmost focused.
fn draw_modal_panes(app: &mut App, frame: &mut Frame, main: Rect, details: Rect, theme: &Theme) {
    let hint = app.release_hint();
    let panes = app.plugin_panes_mut();
    let top = panes.iter().rposition(|p| p.is_modal());
    let mut cursor = false;
    for (i, pane) in panes.iter_mut().enumerate().filter(|(_, p)| p.is_modal()) {
        let area = pty::area(pane, main, details);
        cursor |= pty::draw(pane, frame, area, Some(i) == top, &hint, theme);
    }
    if cursor {
        app.set_cursor_shown();
    }
}

/// The key hints along the bottom, and on the right the notice and any
/// git operation running.
fn draw_help_bar(app: &App, frame: &mut Frame, area: Rect, theme: &Theme) {
    // The notice and any running git operation get their own
    // right-aligned slot (up to half the width) so a long list of key
    // hints can't push them off screen.
    let mut keys_area = area;
    let parts: Vec<(String, Style)> = app
        .help_status()
        .into_iter()
        .map(|(text, look)| {
            let style = match look {
                StatusLook::Muted => theme.muted,
                StatusLook::Success => theme.diff_add,
                StatusLook::Busy => theme.help_key.add_modifier(Modifier::BOLD),
            };
            (text, style)
        })
        .collect();
    if !parts.is_empty() {
        let text: String = parts.iter().map(|(t, _)| t.as_str()).collect();
        let width = (text.width() as u16).min(area.width / 2);
        let [left, right] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(width)]).areas(area);
        keys_area = left;
        frame.render_widget(Paragraph::new(truncate_spans(parts, width as usize)), right);
    }

    // Global hints always show; pane hints fill what's left, in order,
    // skipping any that don't fit (the full list is under `?`).
    let (pane_hints, globals) = app.help_hints();
    let width_of = |(k, l): &(&str, &str)| k.width() + 1 + l.width() + 2;
    let mut budget = (keys_area.width as usize).saturating_sub(globals.iter().map(width_of).sum());
    let mut items = Vec::new();
    for item in pane_hints {
        if width_of(&item) <= budget {
            budget -= width_of(&item);
            items.push(item);
        }
    }
    items.extend(globals);
    let mut spans = Vec::new();
    for (key, label) in items {
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(key, theme.help_key));
        spans.push(Span::styled(format!(" {label}"), theme.help_text));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), keys_area);
}

/// Gives every cell left in the terminal's own colors the theme's instead.
/// Done to the finished frame, so it also covers popups that cleared their
/// area and plugin panes' default colors.
fn paint_background(frame: &mut Frame, bg: Color, fg: Color) {
    for cell in frame.buffer_mut().content.iter_mut() {
        if cell.bg == Color::Reset {
            cell.bg = bg;
        }
        if cell.fg == Color::Reset {
            cell.fg = fg;
        }
    }
}

/// A pane that draws itself into a frame. Drawing first tells the pane how
/// many rows it shows (its `set_viewport`), so it may change scroll state.
pub trait Draw {
    /// Draws into `area`, with the focus look when `focused`, noting in
    /// `hits` what the mouse needs to know about what was drawn.
    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme, hits: &mut Hits);
}

/// Where things the mouse can click were drawn in the last frame, beyond
/// the panes themselves (which the app keeps).
#[derive(Debug, Clone, Default)]
pub struct Hits {
    /// Each branches tab's name in the pane's title: its row and columns.
    pub tabs: Vec<(BranchTab, u16, Range<u16>)>,
    /// The details files view's file rows.
    pub files: Option<Rect>,
    /// The details files view's diff.
    pub diff: Option<Rect>,
    /// Each staging side's rows, under its title.
    pub stage: Vec<(StageSideId, Rect)>,
}

/// The column where a pane drawn in `area` starts its title text, after the
/// corner and the `[n] ` [`pane_block`] puts first.
pub(crate) fn title_x(pane: PaneId, area: Rect) -> u16 {
    area.x + 1 + format!("[{}] ", pane.number()).len() as u16
}

/// The bordered block every pane draws inside, titled `[n] title`.
pub fn pane_block<'a>(pane: PaneId, title: impl Into<Line<'a>>, focused: bool, theme: &Theme) -> Block<'a> {
    let (border, title_style) = if focused {
        (theme.border_focused, theme.title_focused)
    } else {
        (theme.border, theme.title)
    };
    let mut title: Line<'a> = title.into();
    title
        .spans
        .insert(0, Span::styled(format!("[{}] ", pane.number()), title_style));
    for span in title.spans.iter_mut().skip(1) {
        span.style = title_style.patch(span.style);
    }
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(title)
}

/// A `width` by `height` area in the middle of `area`.
pub(crate) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}
