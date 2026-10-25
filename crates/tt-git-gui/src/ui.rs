//! [`Ui`]: what every view needs to draw part of the app (the theme, the
//! font, where things were laid out, a way to send the app mouse input),
//! and [`Measures`], which remembers where things were laid out so the next
//! frame knows how many rows and columns fit.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, Bounds, IntoElement, Pixels, SharedString, Styled, WeakEntity, canvas};
use tt_git_app::mouse::MouseInput;

use crate::font::{Fit, MonoFont};
use crate::model::AppModel;
use crate::theme::GuiTheme;

/// What a view needs to draw part of the app. Cheap to clone: clone it into
/// event handlers.
#[derive(Clone)]
pub struct Ui {
    pub theme: Rc<GuiTheme>,
    pub font: Rc<MonoFont>,
    pub measures: Measures,
    pub model: WeakEntity<AppModel>,
}

impl Ui {
    /// The columns and rows that fit where the element measured as `id`
    /// (with [`measure`](Self::measure)) was laid out last frame. Zero
    /// before its first frame; it's measured then, and the window draws
    /// again with the real size.
    pub fn fit(&self, id: &str) -> Fit {
        self.measures.get(id).map(|b| self.font.fit(b.size)).unwrap_or_default()
    }

    /// An element that fills its parent and records the parent's bounds
    /// as `id`, for [`fit`](Self::fit) and [`Measures::get`]. The parent
    /// must be `.relative()`. When the bounds change the window draws again,
    /// so whatever was sized from the old ones catches up.
    pub fn measure<I: Into<SharedString>>(&self, id: I) -> impl IntoElement + use<I> {
        let id = id.into();
        let measures = self.measures.clone();
        canvas(
            move |bounds, window, _| {
                // GPUI ignores `refresh` while it draws, so ask for the
                // next frame and refresh from there.
                if measures.record(&id, bounds) {
                    window.on_next_frame(|window, _| window.refresh());
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    /// Sends the app a mouse event, hit-tested by the caller.
    pub fn send_mouse(&self, input: MouseInput, cx: &mut App) {
        if let Some(model) = self.model.upgrade() {
            model.update(cx, |model, cx| model.mouse(input, cx));
        }
    }
}

/// Where measured elements were laid out, by id, kept from frame to frame.
#[derive(Clone, Default)]
pub struct Measures(Rc<RefCell<MeasureState>>);

#[derive(Default)]
struct MeasureState {
    bounds: HashMap<SharedString, Bounds<Pixels>>,
    /// Measured since [`Measures::begin_frame`].
    seen: HashSet<SharedString>,
    /// Wheel movement not yet turned into whole scroll steps, by id.
    wheel: HashMap<SharedString, f32>,
}

impl Measures {
    /// Where `id` was laid out, as of the last frame that drew it.
    pub fn get(&self, id: &str) -> Option<Bounds<Pixels>> {
        self.0.borrow().bounds.get(id).copied()
    }

    /// Where `id` was laid out this frame, if it was drawn in it.
    pub fn this_frame(&self, id: &str) -> Option<Bounds<Pixels>> {
        let state = self.0.borrow();
        state.seen.contains(id).then(|| state.bounds.get(id).copied()).flatten()
    }

    /// Starts a frame: forgets what was drawn in the last one, for
    /// [`this_frame`](Self::this_frame).
    pub fn begin_frame(&self) {
        self.0.borrow_mut().seen.clear();
    }

    /// Records `id` at `bounds`. Returns whether that's a change.
    pub fn record(&self, id: &SharedString, bounds: Bounds<Pixels>) -> bool {
        let mut state = self.0.borrow_mut();
        state.seen.insert(id.clone());
        state.bounds.insert(id.clone(), bounds) != Some(bounds)
    }

    /// Adds `amount` of wheel movement for `id` and takes the whole steps
    /// of `step` it adds up to: positive up, negative down.
    pub fn wheel_steps(&self, id: &SharedString, amount: f32, step: f32) -> i32 {
        let mut state = self.0.borrow_mut();
        let total = state.wheel.entry(id.clone()).or_default();
        *total += amount;
        let steps = (*total / step).trunc();
        *total -= steps * step;
        steps as i32
    }
}
