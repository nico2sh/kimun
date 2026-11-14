//! The GPUI half of GUI motion (ADR-0060): the presence store. This file
//! is the only one that names the kit's motion types
//! (`gpui_kit::base::motion`), so a change in the kit reaches one place.
//! The speeds, the reveal order and the snap rule are the toolkit-free
//! half, [`crate::motion`].
//!
//! Each thing that opens and closes has a [`MotionKey`]: a slot, the rail,
//! the tab strip, a tab label. Its presence follows whether the layout
//! model shows it. The model changes at once, and the presence catches up
//! over a [`Speed`]. The drawing code reads the presence to size the thing
//! and to fade its content.
//!
//! # To animate a new widget
//!
//! 1. Add a variant to [`MotionKey`]. Say in [`MotionKey::lingers`] whether
//!    it moves out after the model dropped it (a label does, a slot does
//!    not).
//! 2. Add a [`Spec`] in `Workspace::motion_specs`: when the model shows it,
//!    at which [`Speed`], and how it starts ([`Enter`]). Put a parent
//!    before what comes with it.
//! 3. Where it draws, read `Motion::presence`. The frame value sizes it.
//!    The content value fades what is inside.
//! 4. Build it with [`MotionBox`](super::motion_box::MotionBox):
//!    `faded`, `clipped_while` and `covered_while`. Keep its content
//!    pinned to a size only while it moves. At rest it fills its box, to
//!    the pixel.
//! 5. If it must be drawn after the model dropped it, `Motion::remember` a
//!    snapshot type of its own where it draws, and `Motion::recall` it
//!    when it leaves. The store does not change.
//!
//! The drawing code never changes the model, and the model never reads the
//! presence.

use std::any::Any;
use std::collections::{HashMap, HashSet};

use gpui_kit::base::motion::{Transition, TransitionState};
use gpui_kit::{App, Context, Pixels, Size, Window};
use kimun_notes::logic::editor_region::TabId;
use kimun_notes::logic::layout_tree::{Leaf, RailPosition, SlotId};

use super::Workspace;
use crate::motion::{ResizePolicy, SnapFacts, Speed, Stage, must_snap, timing};

/// What opens and closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum MotionKey {
    Slot(SlotId),
    Rail,
    TabStrip,
    Tab(TabId),
}

impl MotionKey {
    /// Whether a key that is no longer asked for stays, moving out, until
    /// its frame is 0 (a tab label: it is drawn from a snapshot while it
    /// leaves). Any other key is dropped at once: what it moved is gone
    /// from the model, and the drawing code does not draw it.
    fn lingers(self) -> bool {
        matches!(self, MotionKey::Tab(_))
    }
}

/// How a key that has no state yet starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Enter {
    /// At its target: it was there before anyone looked.
    AtRest,
    /// From 0: it just appeared, and moves in.
    FromZero,
    /// From 0 if the parent key is at rest, else at its target: a thing that
    /// appears with its parent comes with it (a tab label while the strip
    /// opens). The parent is sampled earlier in the same call.
    AfterParent(MotionKey),
}

/// What one key is asked to do in one frame.
#[derive(Debug, Clone)]
pub(super) struct Spec {
    pub key: MotionKey,
    /// Whether the layout model shows it.
    pub present: bool,
    pub speed: Speed,
    pub enter: Enter,
}

/// How much of itself a thing shows: its frame (0 gone, 1 full size) and
/// its content (0 invisible, 1 opaque). They move one after the other, in
/// the order of [`crate::motion::timing`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct Presence {
    pub frame: f32,
    pub content: f32,
}

#[derive(Default)]
struct Entry {
    frame: TransitionState<f32>,
    content: TransitionState<f32>,
    /// What was sampled in the current frame.
    now: Presence,
    /// The speed of the last sample: a lingering key leaves at it.
    speed: Speed,
    /// What the drawing code remembered of the thing, to draw it after the
    /// model dropped it. Its type belongs to the widget (see
    /// [`Motion::remember`]).
    snapshot: Option<Box<dyn Any>>,
}

/// The presence of everything the drawing code has met.
#[derive(Default)]
pub(super) struct Motion {
    entries: HashMap<MotionKey, Entry>,
    snap: bool,
    snap_next: bool,
    /// What the last frame had, to tell what changed: the window size, the
    /// zoomed leaf and the rail's edge.
    last_viewport: Option<Size<Pixels>>,
    last_zoom: Option<Option<Leaf>>,
    last_rail: Option<RailPosition>,
}

impl Motion {
    /// What this frame knows that decides a snap, given the window size, the
    /// zoomed leaf, whether a divider is being dragged and the rail's edge.
    /// Remembers what changes for the next frame.
    pub(super) fn frame_facts(
        &mut self,
        viewport: Size<Pixels>,
        zoom: Option<Leaf>,
        divider_drag: bool,
        rail: RailPosition,
    ) -> SnapFacts {
        let facts = SnapFacts {
            first_frame: self.last_viewport.is_none(),
            viewport_changed: self.last_viewport.is_some_and(|last| last != viewport),
            divider_drag,
            zoom_changed: self.last_zoom.is_some_and(|last| last != zoom),
            rail_moved: self.last_rail.is_some_and(|last| last != rail),
        };
        self.last_viewport = Some(viewport);
        self.last_zoom = Some(zoom);
        self.last_rail = Some(rail);
        facts
    }

    /// Start a frame. A snapping frame resets everything, so each key
    /// adopts its target at its next sample.
    pub(super) fn begin_frame(&mut self, facts: SnapFacts) {
        self.snap = must_snap(facts) || std::mem::take(&mut self.snap_next);
    }

    /// Snap the next frame whatever its facts say: a restore landed, and
    /// what it brought is drawn in place.
    pub(super) fn snap_next_frame(&mut self) {
        self.snap_next = true;
    }

    /// Sample every spec in order, then let the lingering keys that are not
    /// in `specs` move out, then drop the keys that are done: one not in
    /// `specs` that does not linger, or whose frame is 0.
    pub(super) fn sample_all(&mut self, specs: &[Spec], window: &mut Window, cx: &mut App) {
        for spec in specs {
            self.sample(spec.key, spec.present, spec.speed, spec.enter, window, cx);
        }
        let asked: HashSet<MotionKey> = specs.iter().map(|spec| spec.key).collect();
        let leaving: Vec<(MotionKey, Speed)> = self
            .entries
            .iter()
            .filter(|(key, _)| !asked.contains(key) && key.lingers())
            .map(|(key, entry)| (*key, entry.speed))
            .collect();
        for (key, speed) in leaving {
            self.sample(key, false, speed, Enter::AtRest, window, cx);
        }
        self.entries
            .retain(|key, entry| asked.contains(key) || (key.lingers() && entry.now.frame > 0.0));
    }

    /// The presence of `key` toward `present`, for this frame. Asks for the
    /// next frame while it moves, and for none once it rests. A key with no
    /// state yet starts as `enter` says, unless the frame snaps.
    pub(super) fn sample(
        &mut self,
        key: MotionKey,
        present: bool,
        speed: Speed,
        enter: Enter,
        window: &mut Window,
        cx: &mut App,
    ) -> Presence {
        let enter = match enter {
            Enter::AfterParent(parent) if self.presence(parent).frame >= 1.0 => Enter::FromZero,
            Enter::AfterParent(_) => Enter::AtRest,
            other => other,
        };
        let snap = self.snap;
        let fresh = !self.entries.contains_key(&key);
        let entry = self.entries.entry(key).or_default();
        entry.speed = speed;
        if snap {
            entry.frame = TransitionState::default();
            entry.content = TransitionState::default();
        }
        let policy = |stage, opening| {
            let timing = timing(speed, stage, opening);
            Transition::new(timing.duration).delay(timing.delay)
        };
        if fresh && !snap && present && enter == Enter::FromZero {
            // Adopt 0 first, so the target below is a move from 0.
            entry
                .frame
                .sample(0.0, policy(Stage::Frame, false), window, cx);
            entry
                .content
                .sample(0.0, policy(Stage::Content, false), window, cx);
        }
        let target = if present { 1.0 } else { 0.0 };
        entry.now = Presence {
            frame: entry
                .frame
                .sample(target, policy(Stage::Frame, present), window, cx)
                .value,
            content: entry
                .content
                .sample(target, policy(Stage::Content, present), window, cx)
                .value,
        };
        entry.now
    }

    /// What was sampled for `key` in this frame: nothing for a key never
    /// sampled.
    pub(super) fn presence(&self, key: MotionKey) -> Presence {
        self.entries
            .get(&key)
            .map_or(Presence::default(), |entry| entry.now)
    }

    /// Remember `value` of the thing `key` moves, to draw it after the
    /// model dropped it. One value per key. Its type is the widget's own
    /// (the slot's view and size, the label's text), so a new widget adds
    /// no field here.
    pub(super) fn remember<T: 'static>(&mut self, key: MotionKey, value: T) {
        self.entries.entry(key).or_default().snapshot = Some(Box::new(value));
    }

    /// What was remembered for `key`: `None` when nothing was, or when it
    /// was a value of another type.
    pub(super) fn recall<T: Clone + 'static>(&self, key: MotionKey) -> Option<T> {
        let snapshot = self.entries.get(&key)?.snapshot.as_ref()?;
        snapshot.downcast_ref::<T>().cloned()
    }

    /// The keys the store holds, the ones that are leaving included.
    pub(super) fn keys(&self) -> Vec<MotionKey> {
        self.entries.keys().copied().collect()
    }

    /// Forget every key `drop` names. For a wholesale change of what they
    /// stand for: the layout tree was replaced, and its slots are new ones
    /// that may reuse the old ids.
    pub(super) fn forget(&mut self, drop: impl Fn(&MotionKey) -> bool) {
        self.entries.retain(|key, _| !drop(key));
    }
}

/// How a slot's content behaves while its frame changes size: pinned, laid
/// out once at the final size and clipped by the frame (ADR-0060).
pub(super) const fn slot_policy() -> ResizePolicy {
    ResizePolicy::Pinned
}

impl Workspace {
    /// Sample everything that opens and closes for this frame, before it is
    /// drawn: each thing is present while the layout model makes it
    /// visible, and its presence catches up over time. Reads the model,
    /// never writes it.
    pub(super) fn motion_frame(&self, window: &mut Window, cx: &mut Context<Self>) {
        let specs = self.motion_specs(cx);
        let mut motion = self.motion.borrow_mut();
        let facts = motion.frame_facts(
            window.viewport_size(),
            self.layout.zoomed(),
            self.resizing,
            self.rail,
        );
        motion.begin_frame(facts);
        motion.sample_all(&specs, window, cx);
    }

    /// What each thing is asked to do this frame, in the order it is
    /// sampled: a parent before what comes with it.
    fn motion_specs(&self, cx: &App) -> Vec<Spec> {
        let strip = self.layout.region().tabs().len() >= 2 || self.dragged_group(cx).is_some();
        let mut specs = vec![
            Spec {
                key: MotionKey::Rail,
                present: !self.rail_hidden,
                speed: Speed::Standard,
                enter: Enter::AtRest,
            },
            Spec {
                key: MotionKey::TabStrip,
                present: strip,
                speed: Speed::Standard,
                enter: Enter::AtRest,
            },
        ];
        // A label that appears while the strip is at rest grows in from
        // nothing. One that appears as the strip opens comes with it.
        specs.extend(self.layout.region().tabs().iter().map(|tab| Spec {
            key: MotionKey::Tab(*tab),
            present: true,
            speed: Speed::Quick,
            enter: Enter::AfterParent(MotionKey::TabStrip),
        }));
        specs.extend(self.layout.slots().into_iter().map(|slot| Spec {
            key: MotionKey::Slot(slot),
            present: self.leaf_visible(Leaf::Slot(slot)),
            speed: Speed::Standard,
            enter: Enter::AtRest,
        }));
        specs
    }
}
