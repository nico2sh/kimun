//! The motion of the facet bar's height (ADR-0060).
//!
//! It owns what the bar needs to ease to a new height after its rows
//! change, and the collapse of the first-use hint on the first pick.
//! At rest it draws nothing of its own: the bar and the hint are drawn as
//! they are. It does not own what the bar holds.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, ScrollHandle,
    Styled as _, Window, div, px, relative,
};
use kimun_core::note::Facet;
use kimun_notes::logic::facet_bar::KindCounts;

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
    /// What the bar drew at the last plan. `None` before the first plan.
    shape: Option<BarShape>,
    /// How many measuring frames the bar drew (tests).
    #[cfg(test)]
    measure_frames: usize,
}

/// What the bar draws, in the terms that set its height. A plan that keeps
/// it gives the bar no motion.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct BarShape {
    /// The picks in pick order, each with the count its pill shows.
    picks: Vec<(Facet, usize)>,
    /// The counts of the menu chips.
    counts: KindCounts,
    /// The summary text. It can wrap the summary row in another number of
    /// lines. The row itself shows while there are picks.
    summary: Option<String>,
    /// The `Picks cleared` row with `Undo` shows.
    cleared: bool,
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
            shape: None,
            #[cfg(test)]
            measure_frames: 0,
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

    /// Mark the bar when `shape` differs from the shape of the last plan.
    /// The first shape is kept with no mark: the bar's first frame snaps.
    pub(super) fn mark_if_shape_changed(&mut self, shape: BarShape) {
        let changed = self.shape.as_ref().is_some_and(|old| *old != shape);
        self.shape = Some(shape);
        if changed {
            self.mark();
        }
    }

    /// How many measuring frames the bar drew (tests).
    #[cfg(test)]
    pub(super) fn measure_frames(&self) -> usize {
        self.measure_frames
    }
}

impl FacetView {
    /// What the bar draws now. It reads the counts the view keeps from the
    /// last select and counts nothing again. The summary text counts no
    /// facet either.
    pub(super) fn bar_shape(&self) -> BarShape {
        let merged = &self.members.merged;
        let picks = self.bar.picks();
        BarShape {
            picks: picks
                .iter()
                .map(|facet| (facet.clone(), self.members.in_view_of(facet)))
                .collect(),
            counts: self.bar.menus(merged, &|f| self.members.in_view_of(f)),
            summary: self.summary(),
            cleared: picks.is_empty() && self.undo.offered(),
        }
    }

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
        let mut facts = BarFacts {
            painted: painted.is_some(),
            first: motion.width.is_none(),
            width_changed: matches!(
                (painted, motion.width),
                (Some((now, _)), Some(before)) if now != before
            ),
            capped,
            reduced: cx.reduce_motion(),
            changed: motion.changed,
            measure: motion.measure,
            moving: false,
            arrived,
            clips: false,
            past_cap: false,
        };
        motion.width = painted.map(|(width, _)| width);
        if facts.seeds()
            && let Some((_, height)) = painted
        {
            // The first frame that can read the bar's height. The follow
            // starts there, so a change in this frame moves from it.
            motion.follow.snap(height);
            motion.measured.set(height);
        }
        // The follow is sampled only in a frame that can move: a sample
        // starts or steps its motion. Until then `moving`, `clips`,
        // `past_cap` and `shown` hold placeholders. `decide` reads those
        // facts, and returns Measure or Move, only when `may_move` is true.
        // Only Measure and Move draw `shown`.
        let mut shown = 0.0;
        if facts.may_move() {
            shown = motion.follow.sample(motion.measured.get(), window, cx);
            facts.moving = motion.follow.moving();
            facts.clips = shown < motion.measured.get() - 0.5;
            facts.past_cap = motion.measured.get() > cap + 0.5;
        }
        let decision = decide(&facts);
        motion.snapped = decision == Decision::Snap;
        match decision {
            Decision::Measure => {
                // The new height is measured in this frame. The next frame
                // moves.
                motion.measure = false;
                #[cfg(test)]
                {
                    motion.measure_frames += 1;
                }
                window.request_animation_frame();
                Some(shown)
            }
            Decision::Move => Some(shown),
            Decision::Rest | Decision::Snap => {
                motion.changed = false;
                motion.measure = false;
                if let Some((_, height)) = painted {
                    motion.follow.snap(height);
                    motion.measured.set(height);
                }
                None
            }
        }
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
        clip_frame(bar, measured, BAR_FRAME_ID, Some(height), true)
    }

    /// `hint` in the frame that clips it while it collapses: at its own
    /// height (`None`), where the frame measures it, or at `height`.
    pub(super) fn framed_hint(&self, hint: impl IntoElement, height: Option<f32>) -> AnyElement {
        let measured = self.bar_motion.hint.height.clone();
        clip_frame(hint, measured, HINT_FRAME_ID, height, false)
    }
}

/// The facts of one frame of the bar that its motion turns on.
#[derive(Clone, Copy)]
struct BarFacts {
    /// The bar painted in the last frame, so its size is known.
    painted: bool,
    /// No frame painted the bar since it was first drawn or hidden.
    first: bool,
    /// The bar painted at another width than in the frame before.
    width_changed: bool,
    /// The bar is taller than its cap and scrolls.
    capped: bool,
    /// The user asked for reduced motion.
    reduced: bool,
    /// The picks or the rows of the bar changed since it last rested.
    changed: bool,
    /// This frame measures the new height.
    measure: bool,
    /// The follow still moves after this frame's sample.
    moving: bool,
    /// A stop in the bar took the keyboard since the last frame.
    arrived: bool,
    /// The height drawn in this frame is shorter than the bar.
    clips: bool,
    /// The bar's measured height is taller than its cap.
    past_cap: bool,
}

impl BarFacts {
    /// Whether this frame seeds the follow with the painted height: the
    /// first frame that painted the bar.
    fn seeds(&self) -> bool {
        self.first && self.painted
    }

    /// Whether the bar can move in this frame. Only then is the follow
    /// sampled, and only then do `moving`, `clips` and `past_cap` count.
    fn may_move(&self) -> bool {
        self.changed && !self.snaps_at_once()
    }

    /// The bar snaps whatever changed. A bar that never painted has nothing
    /// to move from. A new width makes the measured height wrong. A capped
    /// bar keeps its scroll only outside the frame. Reduced motion draws no
    /// frame.
    fn snaps_at_once(&self) -> bool {
        !self.painted || self.width_changed || self.capped || self.reduced
    }
}

/// What the bar does in one frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Decision {
    /// Drawn as it is, with no frame. The follow takes the painted height.
    Rest,
    /// Drawn in its frame at the old height, to measure the new one.
    Measure,
    /// Drawn in its frame at the height the follow shows.
    Move,
    /// Drawn as it is, with no frame, and the hint snaps with it.
    Snap,
}

/// What the bar does in the frame of `facts` (ADR-0060 decision 12).
fn decide(facts: &BarFacts) -> Decision {
    if facts.snaps_at_once() {
        return Decision::Snap;
    }
    if !facts.changed {
        return Decision::Rest;
    }
    // A stop that took the keyboard must be seen whole. The measuring frame
    // holds the old height, so it can clip as well.
    let clips = facts.measure || facts.clips;
    // Past the cap the frame stops at the cap, so a motion shows nothing.
    // The bar snaps to its capped height once it is measured.
    let past_cap = !facts.measure && facts.past_cap;
    if (facts.arrived && clips) || past_cap {
        Decision::Snap
    } else if facts.measure {
        Decision::Measure
    } else if facts.moving {
        Decision::Move
    } else {
        Decision::Rest
    }
}

/// `child` in a column that clips it. The column puts the height of
/// `child` in `measured` each frame. It is `height` tall, or as tall as
/// `child` when `height` is `None`. With `cap` it is at most 40 percent of
/// its parent.
///
/// The frame has no element id. The kit keys the state of an element by
/// its path. An id here would add a step to the path of everything inside.
/// The state of the open list's popover, of its field and of the chips
/// would drop when the motion starts, and again when it ends. Tests find
/// the frame by its debug selector `selector`.
fn clip_frame(
    child: impl IntoElement,
    measured: Rc<Cell<f32>>,
    selector: &'static str,
    height: Option<f32>,
    cap: bool,
) -> AnyElement {
    div()
        .on_children_prepainted(move |bounds, _, _| {
            if let Some(child) = bounds.first() {
                measured.set(f32::from(child.size.height));
            }
        })
        .debug_selector(move || selector.into())
        .flex()
        .flex_col()
        .flex_none()
        .when(cap, |frame| frame.max_h(relative(0.4)))
        .when_some(height, |frame, height| frame.h(px(height)))
        .overflow_hidden()
        .child(child)
        .into_any_element()
}

#[cfg(test)]
mod decide_tests;
