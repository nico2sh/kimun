//! The motion of the facet bar's height (ADR-0060).
//!
//! It owns what the bar needs to ease to a new height after its rows
//! change, and the collapse of the first-use hint on the first pick.
//! At rest it draws nothing of its own: the bar and the hint are drawn as
//! they are. It does not own what the bar holds.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, ScrollHandle,
    Styled as _, Window, div, px, relative,
};

use super::keys::Stop;
use super::{BAR_FRAME_ID, FacetView, HINT_FRAME_ID};
use crate::workspace::motion::HeightFollow;

/// The state of the bar's motion and of the hint's collapse.
pub(super) struct BarMotion {
    /// The height the bar is drawn at while it moves.
    follow: HeightFollow,
    /// The bar's content height as last measured, inside its frame while it
    /// moves and from its own bounds at rest.
    measured: Rc<Cell<f32>>,
    /// The bar's width in the last drawn frame. `None` until a frame that
    /// drew the bar has painted it: on the bar's first frame, and on its
    /// first frame after it was hidden.
    width: Option<f32>,
    /// The picks or the rows of the bar changed. It clears when the motion
    /// ends.
    changed: bool,
    /// The next frame draws the bar in its frame at the old height, to
    /// measure the new one.
    measure: bool,
    /// The bar snapped in this frame. The hint snaps with it.
    snapped: bool,
    /// The hint's collapse after the first pick.
    hint: HintCollapse,
}

/// Where the hint is in its collapse.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HintPhase {
    /// Not collapsing.
    Idle,
    /// Drawn in its frame at its own height, to measure it.
    Measure,
    /// Measured: the next frame starts the collapse from that height.
    Start,
    /// Collapsing to 0.
    Move,
}

struct HintCollapse {
    follow: HeightFollow,
    /// The hint's height, measured in its frame.
    height: Rc<Cell<f32>>,
    /// The hint showed at rest in the last frame.
    shown: bool,
    phase: HintPhase,
}

/// How the hint is drawn in one frame.
pub(super) enum HintDraw {
    /// Not at all.
    Gone,
    /// As it is, at rest.
    Plain,
    /// In a frame that clips it: at its own height (`None`), or at a height.
    Framed(Option<f32>),
}

impl Default for BarMotion {
    fn default() -> Self {
        Self {
            follow: HeightFollow::new(),
            measured: Rc::default(),
            width: None,
            changed: false,
            measure: false,
            snapped: true,
            hint: HintCollapse {
                follow: HeightFollow::new(),
                height: Rc::default(),
                shown: false,
                phase: HintPhase::Idle,
            },
        }
    }
}

impl BarMotion {
    /// The picks or the rows of the bar changed: the bar eases to its new
    /// height.
    pub(super) fn mark(&mut self) {
        self.changed = true;
        self.measure = true;
    }
}

impl FacetView {
    /// The bar is not drawn in this frame. Its scroll handle starts again,
    /// so the bounds of a bar drawn before are not read as the last frame's.
    pub(super) fn bar_hidden(&mut self) {
        let motion = &mut self.bar_motion;
        motion.width = None;
        motion.changed = false;
        motion.measure = false;
        motion.snapped = true;
        self.bar_scroll = ScrollHandle::new();
    }

    /// The height of the bar's frame in this frame, or `None` to draw the
    /// bar as it is. Call it after the bar's stops are drawn and scrolled
    /// to.
    pub(super) fn bar_frame(&mut self, window: &mut Window, cx: &mut App) -> Option<f32> {
        let arrived = matches!(
            self.stops.borrow().arrived_drawn(),
            Some(Stop::Menu(_) | Stop::Pill(_) | Stop::Clear | Stop::Undo)
        );
        // A handle that never painted has no size: the bar was not drawn in
        // the last frame. A drawn bar is never 0 px wide, it has padding.
        let bounds = self.bar_scroll.bounds();
        let painted = (bounds.size.width > px(0.))
            .then(|| (f32::from(bounds.size.width), f32::from(bounds.size.height)));
        let capped = self.bar_scroll.max_offset().y > px(0.);
        // The cap is 40 percent of the view. The view runs from the top of
        // the bar to the bottom of the column.
        let cap = 0.4 * f32::from(self.scroll.bounds().bottom() - bounds.top());
        let motion = &mut self.bar_motion;
        let first = motion.width.is_none();
        let width_changed =
            matches!((painted, motion.width), (Some((now, _)), Some(before)) if now != before);
        motion.width = painted.map(|(width, _)| width);
        if first && let Some((_, height)) = painted {
            // The first frame that can read the bar's height. The follow
            // starts there, so a change in this frame moves from it.
            motion.follow.snap(height);
            motion.measured.set(height);
        }
        // A capped bar keeps its scroll only outside the frame. A new width
        // makes the measured height wrong. The first frame has nothing to
        // move from.
        motion.snapped = painted.is_none() || width_changed || capped || cx.reduce_motion();
        if !motion.snapped && motion.changed {
            let shown = motion.follow.sample(motion.measured.get(), window, cx);
            // A stop that took the keyboard must be seen whole. A frame that
            // measures, or that is shorter than the bar, can clip it.
            let clips = motion.measure || shown < motion.measured.get() - 0.5;
            // Past the cap the frame stops at the cap, so a motion shows
            // nothing. The bar snaps to its capped height.
            let past_cap = !motion.measure && motion.measured.get() > cap + 0.5;
            let snap = (arrived && clips) || past_cap;
            if !snap {
                if std::mem::take(&mut motion.measure) {
                    // The new height is measured in this frame. The next
                    // frame moves.
                    window.request_animation_frame();
                    return Some(shown);
                }
                if motion.follow.moving() {
                    return Some(shown);
                }
            }
            motion.snapped = snap;
        }
        motion.changed = false;
        motion.measure = false;
        if let Some((_, height)) = painted {
            motion.follow.snap(height);
            motion.measured.set(height);
        }
        None
    }

    /// How the hint is drawn in this frame, given whether it `shows`. Call
    /// it after [`Self::bar_frame`]: the hint snaps when the bar does.
    pub(super) fn hint_draw(&mut self, shows: bool, window: &mut Window, cx: &mut App) -> HintDraw {
        let motion = &mut self.bar_motion;
        let hint = &mut motion.hint;
        if shows {
            hint.shown = true;
            hint.phase = HintPhase::Idle;
            return HintDraw::Plain;
        }
        if std::mem::take(&mut hint.shown) {
            hint.phase = HintPhase::Measure;
        }
        if motion.snapped {
            hint.phase = HintPhase::Idle;
        }
        match hint.phase {
            HintPhase::Idle => HintDraw::Gone,
            HintPhase::Measure => {
                hint.phase = HintPhase::Start;
                window.request_animation_frame();
                HintDraw::Framed(None)
            }
            HintPhase::Start | HintPhase::Move => {
                if hint.phase == HintPhase::Start {
                    hint.follow.snap(hint.height.get());
                    hint.phase = HintPhase::Move;
                }
                let height = hint.follow.sample(0.0, window, cx);
                if hint.follow.moving() {
                    HintDraw::Framed(Some(height))
                } else {
                    hint.phase = HintPhase::Idle;
                    HintDraw::Gone
                }
            }
        }
    }

    /// `bar` in the frame that clips it at `height` while it moves. The bar
    /// lays out at its full height inside, and the frame measures it.
    pub(super) fn framed_bar(&self, bar: impl IntoElement, height: f32) -> AnyElement {
        let measured = self.bar_motion.measured.clone();
        div()
            .on_children_prepainted(move |bounds, _, _| {
                if let Some(bar) = bounds.first() {
                    measured.set(f32::from(bar.size.height));
                }
            })
            .id(BAR_FRAME_ID)
            .flex()
            .flex_col()
            .flex_none()
            .max_h(relative(0.4))
            .h(px(height))
            .overflow_hidden()
            .child(bar)
            .test_support()
            .into_any_element()
    }

    /// `hint` in the frame that clips it while it collapses: at its own
    /// height (`None`), where the frame measures it, or at `height`.
    pub(super) fn framed_hint(&self, hint: impl IntoElement, height: Option<f32>) -> AnyElement {
        let measured = self.bar_motion.hint.height.clone();
        div()
            .on_children_prepainted(move |bounds, _, _| {
                if let Some(hint) = bounds.first() {
                    measured.set(f32::from(hint.size.height));
                }
            })
            .id(HINT_FRAME_ID)
            .flex()
            .flex_col()
            .flex_none()
            .overflow_hidden()
            .when_some(height, |frame, height| frame.h(px(height)))
            .child(hint)
            .test_support()
            .into_any_element()
    }
}
