//! What the desktop app remembers between runs, in `<state_dir>/gui.json`:
//! where the last window was (its bounds, and whether it was maximized or
//! fullscreen) and the repositories opened most recently.
//!
//! One window state for the whole app, not one per repository: a new
//! window opens where the user last left one, whichever repository it
//! shows, and the file never fills up with places for repositories that
//! are gone. A second window opened while one is open is moved down and
//! right by [`CASCADE`] so it doesn't hide the first.
//!
//! The file is read once, when [`install`] is called at startup, and written
//! shortly after the window moves or resizes and when a repository opens.
//! Tests don't call [`install`], so they never touch the state folder.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{App, Bounds, Global, Pixels, Task, Window, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};

/// The file in the state folder.
pub const FILE: &str = "gui.json";

/// How many recent repositories are kept.
pub const RECENT_MAX: usize = 10;

/// The size of the first window, before any is remembered.
pub const DEFAULT_SIZE: (f32, f32) = (1280., 800.);

/// The smallest window size restored.
const MIN_SIZE: (f32, f32) = (480., 320.);

/// How far each window opened while others are open is moved down and right.
pub const CASCADE: f32 = 28.;

/// How long after the last move or resize the file is written.
const SAVE_DELAY: Duration = Duration::from_millis(500);

/// What `gui.json` holds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GuiState {
    #[serde(default)]
    pub window: Option<SavedWindow>,
    /// Repository roots, most recent first.
    #[serde(default)]
    pub recent: Vec<PathBuf>,
}

/// A window's bounds (in logical pixels, on the global display layout) and
/// mode. For a maximized or fullscreen window the bounds are the ones it
/// goes back to.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SavedWindow {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub mode: WindowMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowMode {
    #[default]
    Windowed,
    Maximized,
    Fullscreen,
}

impl GuiState {
    /// Reads `path`. A missing file is an empty state; so is one that
    /// can't be read or parsed, which is logged and later overwritten.
    pub fn load(path: &Path) -> Self {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                tracing::warn!("couldn't read {}: {e}", path.display());
                return Self::default();
            }
        };
        serde_json::from_str(&text).unwrap_or_else(|e| {
            tracing::warn!("ignoring {}: {e}", path.display());
            Self::default()
        })
    }

    /// Writes `path` (through a temporary file, so a crash mid-write leaves
    /// the old one), making its folder if needed.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)
    }

    /// Puts `root` first in the recent repositories.
    pub fn add_recent(&mut self, root: &Path) {
        self.recent.retain(|p| p != root);
        self.recent.insert(0, root.to_path_buf());
        self.recent.truncate(RECENT_MAX);
    }

    /// Drops `root` from the recent repositories (it's gone).
    pub fn remove_recent(&mut self, root: &Path) {
        self.recent.retain(|p| p != root);
    }
}

impl SavedWindow {
    pub fn new(bounds: WindowBounds) -> Self {
        let (b, mode) = match bounds {
            WindowBounds::Windowed(b) => (b, WindowMode::Windowed),
            WindowBounds::Maximized(b) => (b, WindowMode::Maximized),
            WindowBounds::Fullscreen(b) => (b, WindowMode::Fullscreen),
        };
        Self {
            x: b.origin.x.into(),
            y: b.origin.y.into(),
            width: b.size.width.into(),
            height: b.size.height.into(),
            mode,
        }
    }

    pub fn bounds(&self) -> Bounds<Pixels> {
        Bounds::new(point(px(self.x), px(self.y)), size(px(self.width), px(self.height)))
    }
}

/// Where a new window goes: `saved` if it's still on one of `displays`
/// (kept inside that display, and no bigger than it), else the saved size
/// centered on `primary` (or the first display). With nothing saved, the
/// default size centered. `cascade` is how many windows are open already.
///
/// A window counts as on a display when at least a strip of its top
/// (where the title bar is) shows on it, so a window dragged half off an
/// edge stays where it was.
pub fn place(
    saved: Option<&SavedWindow>,
    displays: &[Bounds<Pixels>],
    primary: Option<Bounds<Pixels>>,
    cascade: usize,
) -> WindowBounds {
    let fallback = primary.or_else(|| displays.first().copied());
    let wanted = saved.map(SavedWindow::bounds);
    let mode = saved.map_or(WindowMode::Windowed, |s| s.mode);
    let size_of = |b: Option<Bounds<Pixels>>| {
        b.map_or(size(px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1)), |b| {
            size(b.size.width.max(px(MIN_SIZE.0)), b.size.height.max(px(MIN_SIZE.1)))
        })
    };
    let home = wanted.and_then(|w| {
        // The top strip: the full width, at most 40 px tall.
        let top = Bounds::new(w.origin, size(w.size.width, w.size.height.min(px(40.))));
        displays
            .iter()
            .map(|d| (d, area(&top.intersect(d))))
            .filter(|(_, a)| *a > 0.)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(d, _)| *d)
    });
    let bounds = match (home, fallback) {
        (Some(display), _) => {
            let size = fit(size_of(wanted), &display);
            let origin = wanted.map_or(display.origin, |w| w.origin);
            keep_inside(Bounds::new(cascaded(origin, cascade), size), &display)
        }
        (None, Some(display)) => {
            let size = fit(size_of(wanted), &display);
            let centered = Bounds::centered_at(display.center(), size);
            keep_inside(Bounds::new(cascaded(centered.origin, cascade), size), &display)
        }
        // No displays reported: let the platform decide where.
        (None, None) => Bounds::new(cascaded(point(px(0.), px(0.)), cascade), size_of(wanted)),
    };
    match mode {
        WindowMode::Windowed => WindowBounds::Windowed(bounds),
        WindowMode::Maximized => WindowBounds::Maximized(bounds),
        WindowMode::Fullscreen => WindowBounds::Fullscreen(bounds),
    }
}

fn area(b: &Bounds<Pixels>) -> f32 {
    f32::from(b.size.width).max(0.) * f32::from(b.size.height).max(0.)
}

fn cascaded(origin: gpui::Point<Pixels>, n: usize) -> gpui::Point<Pixels> {
    let step = px(CASCADE * n as f32);
    point(origin.x + step, origin.y + step)
}

/// `wanted`, no bigger than `display`.
fn fit(wanted: gpui::Size<Pixels>, display: &Bounds<Pixels>) -> gpui::Size<Pixels> {
    size(
        wanted.width.min(display.size.width),
        wanted.height.min(display.size.height),
    )
}

/// `b` moved so it lies inside `display` (it's no bigger already).
fn keep_inside(b: Bounds<Pixels>, display: &Bounds<Pixels>) -> Bounds<Pixels> {
    let max_x = display.origin.x + display.size.width - b.size.width;
    let max_y = display.origin.y + display.size.height - b.size.height;
    let x = b.origin.x.min(max_x).max(display.origin.x);
    let y = b.origin.y.min(max_y).max(display.origin.y);
    Bounds::new(point(x, y), b.size)
}

/// The state file and what it holds, once [`install`]ed.
struct Store {
    path: PathBuf,
    state: GuiState,
    /// `state` changed since the file was written.
    dirty: bool,
    /// Writes the file after [`SAVE_DELAY`]; replaced (so put off) by every
    /// change.
    pending: Option<Task<()>>,
}

impl Global for Store {}

/// Reads `path` and keeps what the app remembers there from now on.
pub fn install(path: PathBuf, cx: &mut App) {
    let state = GuiState::load(&path);
    cx.set_global(Store {
        path,
        state,
        dirty: false,
        pending: None,
    });
    // However the app ends (the last window closing, Quit), what's waiting
    // is written.
    cx.on_app_quit(|cx| {
        flush(cx);
        async {}
    })
    .detach();
}

/// `<state_dir>/gui.json`.
pub fn default_path() -> PathBuf {
    tt_git_core::paths::state_dir().join(FILE)
}

/// What's remembered (empty without [`install`]).
pub fn state(cx: &App) -> GuiState {
    cx.try_global::<Store>().map(|s| s.state.clone()).unwrap_or_default()
}

/// Where the next window goes, with `open` windows open already. Without
/// [`install`], the default size centered, as before anything was saved.
pub fn next_bounds(open: usize, cx: &App) -> WindowBounds {
    let displays: Vec<Bounds<Pixels>> = cx.displays().iter().map(|d| d.bounds()).collect();
    let primary = cx.primary_display().map(|d| d.bounds());
    let saved = cx.try_global::<Store>().and_then(|s| s.state.window);
    place(saved.as_ref(), &displays, primary, open)
}

/// Remembers `window`'s bounds and mode (call when they change).
pub fn remember_window(window: &Window, cx: &mut App) {
    let saved = SavedWindow::new(window.window_bounds());
    // A window that's never been placed (or one on Wayland, which keeps
    // positions from apps) can report an empty size; keep the old state.
    if saved.width < 1. || saved.height < 1. {
        return;
    }
    update(cx, |state| {
        if state.window == Some(saved) {
            return false;
        }
        state.window = Some(saved);
        true
    });
}

/// Puts `root` first in the recent repositories.
pub fn remember_repo(root: &Path, cx: &mut App) {
    update(cx, |state| {
        if state.recent.first().is_some_and(|p| p == root) {
            return false;
        }
        state.add_recent(root);
        true
    });
}

/// Drops `root` from the recent repositories.
pub fn forget_repo(root: &Path, cx: &mut App) {
    update(cx, |state| {
        let before = state.recent.len();
        state.remove_recent(root);
        state.recent.len() != before
    });
}

/// Changes the state with `f` (which says whether it changed anything)
/// and writes it after [`SAVE_DELAY`].
fn update(cx: &mut App, f: impl FnOnce(&mut GuiState) -> bool) {
    if !cx.has_global::<Store>() {
        return;
    }
    let changed = f(&mut cx.global_mut::<Store>().state);
    if !changed {
        return;
    }
    let task = cx.spawn(async move |cx| {
        cx.background_executor().timer(SAVE_DELAY).await;
        cx.update(flush);
    });
    let store = cx.global_mut::<Store>();
    store.dirty = true;
    store.pending = Some(task);
}

/// Writes the file now if anything is waiting to be written (call before
/// quitting).
pub fn flush(cx: &mut App) {
    let Some(store) = cx.try_global::<Store>() else {
        return;
    };
    if !store.dirty {
        return;
    }
    if let Err(e) = store.state.save(&store.path) {
        tracing::warn!("couldn't write {}: {e}", store.path.display());
    } else {
        tracing::debug!("saved {}", store.path.display());
    }
    cx.global_mut::<Store>().dirty = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    fn saved(x: f32, y: f32, w: f32, h: f32, mode: WindowMode) -> SavedWindow {
        SavedWindow {
            x,
            y,
            width: w,
            height: h,
            mode,
        }
    }

    fn temp_file(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join(name);
        (dir, path)
    }

    #[test]
    fn the_state_file_round_trips() {
        let (_dir, path) = temp_file(FILE);
        let mut state = GuiState {
            window: Some(saved(100., 50., 1000., 700., WindowMode::Maximized)),
            recent: Vec::new(),
        };
        state.add_recent(Path::new("/a"));
        state.add_recent(Path::new("/b"));
        state.save(&path).unwrap();
        assert_eq!(GuiState::load(&path), state);
        assert_eq!(state.recent, [PathBuf::from("/b"), PathBuf::from("/a")]);
    }

    #[test]
    fn a_missing_or_corrupt_file_is_an_empty_state() {
        let (_dir, path) = temp_file(FILE);
        assert_eq!(GuiState::load(&path), GuiState::default());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(GuiState::load(&path), GuiState::default());
        // Written over the next time.
        GuiState::default().save(&path).unwrap();
        assert_eq!(GuiState::load(&path), GuiState::default());
    }

    #[test]
    fn missing_fields_take_their_defaults() {
        let (_dir, path) = temp_file(FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"window":{"x":1,"y":2,"width":900,"height":600}}"#).unwrap();
        let state = GuiState::load(&path);
        assert_eq!(state.window, Some(saved(1., 2., 900., 600., WindowMode::Windowed)));
        assert!(state.recent.is_empty());
    }

    #[test]
    fn recent_repos_move_to_the_front_without_repeats_and_are_capped() {
        let mut state = GuiState::default();
        for i in 0..15 {
            state.add_recent(Path::new(&format!("/r{i}")));
        }
        state.add_recent(Path::new("/r10"));
        assert_eq!(state.recent.len(), RECENT_MAX);
        assert_eq!(state.recent[0], PathBuf::from("/r10"));
        assert_eq!(state.recent.iter().filter(|p| **p == Path::new("/r10")).count(), 1);
        state.remove_recent(Path::new("/r10"));
        assert_eq!(state.recent[0], PathBuf::from("/r14"));
    }

    #[test]
    fn saved_bounds_round_trip_through_window_bounds() {
        let b = rect(10., 20., 800., 600.);
        for (bounds, mode) in [
            (WindowBounds::Windowed(b), WindowMode::Windowed),
            (WindowBounds::Maximized(b), WindowMode::Maximized),
            (WindowBounds::Fullscreen(b), WindowMode::Fullscreen),
        ] {
            let s = SavedWindow::new(bounds);
            assert_eq!(s.mode, mode);
            assert_eq!(s.bounds(), b);
            assert_eq!(place(Some(&s), &[rect(0., 0., 1920., 1080.)], None, 0), bounds);
        }
    }

    #[test]
    fn nothing_saved_centers_the_default_size_on_the_primary_display() {
        let displays = [rect(0., 0., 1920., 1080.), rect(1920., 0., 2560., 1440.)];
        let placed = place(None, &displays, Some(displays[1]), 0);
        assert_eq!(placed, WindowBounds::Windowed(rect(1920. + 640., 320., 1280., 800.)));
    }

    #[test]
    fn a_window_on_a_second_display_stays_there() {
        let displays = [rect(0., 0., 1920., 1080.), rect(1920., 0., 2560., 1440.)];
        let s = saved(2000., 100., 1200., 900., WindowMode::Windowed);
        assert_eq!(
            place(Some(&s), &displays, Some(displays[0]), 0),
            WindowBounds::Windowed(rect(2000., 100., 1200., 900.))
        );
    }

    #[test]
    fn a_window_on_a_display_that_is_gone_moves_to_the_primary_one() {
        let displays = [rect(0., 0., 1920., 1080.)];
        // It was on a display to the right that's unplugged now.
        let s = saved(2500., 200., 1000., 600., WindowMode::Maximized);
        assert_eq!(
            place(Some(&s), &displays, Some(displays[0]), 0),
            WindowBounds::Maximized(rect(460., 240., 1000., 600.))
        );
    }

    #[test]
    fn a_window_bigger_than_its_display_shrinks_to_fit() {
        let displays = [rect(0., 0., 1366., 768.)];
        let s = saved(-20., 10., 2560., 1400., WindowMode::Windowed);
        assert_eq!(
            place(Some(&s), &displays, None, 0),
            WindowBounds::Windowed(rect(0., 0., 1366., 768.))
        );
    }

    #[test]
    fn a_window_partly_off_its_display_is_pulled_back_on() {
        let displays = [rect(0., 0., 1920., 1080.)];
        let s = saved(1500., 900., 800., 600., WindowMode::Windowed);
        assert_eq!(
            place(Some(&s), &displays, None, 0),
            WindowBounds::Windowed(rect(1120., 480., 800., 600.))
        );
    }

    #[test]
    fn a_tiny_saved_window_opens_at_the_smallest_size() {
        let displays = [rect(0., 0., 1920., 1080.)];
        let s = saved(100., 100., 10., 10., WindowMode::Windowed);
        assert_eq!(
            place(Some(&s), &displays, None, 0),
            WindowBounds::Windowed(rect(100., 100., MIN_SIZE.0, MIN_SIZE.1))
        );
    }

    #[test]
    fn more_windows_cascade_and_stay_on_the_display() {
        let displays = [rect(0., 0., 1920., 1080.)];
        let s = saved(100., 100., 800., 600., WindowMode::Windowed);
        assert_eq!(
            place(Some(&s), &displays, None, 2),
            WindowBounds::Windowed(rect(100. + 2. * CASCADE, 100. + 2. * CASCADE, 800., 600.))
        );
        // Far enough that it would leave the display: kept on it.
        assert_eq!(
            place(Some(&s), &displays, None, 40),
            WindowBounds::Windowed(rect(1120., 480., 800., 600.))
        );
    }

    #[test]
    fn no_displays_keeps_the_saved_size() {
        let s = saved(100., 100., 800., 600., WindowMode::Windowed);
        assert_eq!(
            place(Some(&s), &[], None, 0),
            WindowBounds::Windowed(rect(0., 0., 800., 600.))
        );
    }
}
