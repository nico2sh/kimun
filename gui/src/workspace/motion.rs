//! The GPUI half of GUI motion (ADR-0060): the per-slot presence store.
//! This file is the only one that names the kit's motion types
//! (`gpui_kit::base::motion`), so a change in the kit reaches one place.
//! The speeds, the reveal order and the snap rule are the toolkit-free
//! half, [`crate::motion`].
//!
//! A slot's presence is a value from 0 to 1 that follows whether the slot
//! is visible in the layout model. The model changes at once, and the
//! presence catches up over [`Speed::Standard`]. The layout pass reads it
//! to scale the slot's share of its split.

use std::collections::HashMap;

use gpui_kit::base::motion::{Transition, TransitionState};
use gpui_kit::{App, Context, Pixels, Size, Window};
use kimun_notes::logic::layout_tree::{Leaf, SlotId};
use kimun_notes::logic::nav::DrawerView;

use super::Workspace;
use crate::motion::{ResizePolicy, SnapFacts, Speed, Stage, must_snap, timing};

/// How much of itself a slot shows: its frame (0 gone, 1 full size) and its
/// content (0 invisible, 1 opaque). They move one after the other, in the
/// order of [`crate::motion::timing`].
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
    /// The view the slot last showed and the final size it had, in px
    /// (width, height): what a leaving slot is drawn from after the model
    /// dropped its view.
    last: Option<(DrawerView, (f64, f64))>,
}

/// The presence of every slot the layout pass has met.
#[derive(Default)]
pub(super) struct SlotMotion {
    slots: HashMap<SlotId, Entry>,
    snap: bool,
    /// What the last frame had, to tell what changed: the window size and
    /// the zoomed leaf.
    last_viewport: Option<Size<Pixels>>,
    last_zoom: Option<Option<Leaf>>,
}

impl SlotMotion {
    /// What this frame knows that decides a snap, given the window size, the
    /// zoomed leaf and whether a divider is being dragged. Remembers the
    /// size and the zoom for the next frame.
    pub(super) fn frame_facts(
        &mut self,
        viewport: Size<Pixels>,
        zoom: Option<Leaf>,
        divider_drag: bool,
    ) -> SnapFacts {
        let facts = SnapFacts {
            first_frame: self.last_viewport.is_none(),
            viewport_changed: self.last_viewport.is_some_and(|last| last != viewport),
            divider_drag,
            zoom_changed: self.last_zoom.is_some_and(|last| last != zoom),
        };
        self.last_viewport = Some(viewport);
        self.last_zoom = Some(zoom);
        facts
    }

    /// Start a frame. A snapping frame resets every slot, so each adopts
    /// its target at its next sample.
    pub(super) fn begin_frame(&mut self, facts: SnapFacts) {
        self.snap = must_snap(facts);
    }

    /// The presence of `slot` toward `present`, for this frame. Asks for the
    /// next frame while it moves, and for none once it rests.
    pub(super) fn sample(
        &mut self,
        slot: SlotId,
        present: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Presence {
        let snap = self.snap;
        let entry = self.slots.entry(slot).or_default();
        if snap {
            entry.frame = TransitionState::default();
            entry.content = TransitionState::default();
        }
        let target = if present { 1.0 } else { 0.0 };
        let policy = |stage| {
            let timing = timing(Speed::Standard, stage, present);
            Transition::new(timing.duration).delay(timing.delay)
        };
        entry.now = Presence {
            frame: entry
                .frame
                .sample(target, policy(Stage::Frame), window, cx)
                .value,
            content: entry
                .content
                .sample(target, policy(Stage::Content), window, cx)
                .value,
        };
        entry.now
    }

    /// What was sampled for `slot` in this frame: nothing for a slot never
    /// sampled.
    pub(super) fn presence(&self, slot: SlotId) -> Presence {
        self.slots.get(&slot).map_or(Presence::default(), |e| e.now)
    }

    /// Remember that `slot` shows `view`, `size` px (width, height) at rest.
    pub(super) fn remember(&mut self, slot: SlotId, view: DrawerView, size: (f64, f64)) {
        self.slots.entry(slot).or_default().last = Some((view, size));
    }

    /// The view and final size `slot` last showed.
    pub(super) fn last(&self, slot: SlotId) -> Option<(DrawerView, (f64, f64))> {
        self.slots.get(&slot).and_then(|entry| entry.last)
    }

    /// Forget every slot: the layout tree was replaced as a whole, and its
    /// slots are new ones that may reuse the old ids. Every slot adopts its
    /// state at its next sample.
    pub(super) fn reset(&mut self) {
        self.slots.clear();
    }

    /// Forget the slots that left the layout tree.
    pub(super) fn retain(&mut self, keep: &[SlotId]) {
        self.slots.retain(|slot, _| keep.contains(slot));
    }
}

/// How a slot's content behaves while its frame changes size: pinned, laid
/// out once at the final size and clipped by the frame (ADR-0060).
pub(super) const fn slot_policy() -> ResizePolicy {
    ResizePolicy::Pinned
}

impl Workspace {
    /// Sample every slot's presence for this frame, before the layout is
    /// drawn: a slot is present while the layout model makes it visible, and
    /// the presence catches up over time. Reads the model, never writes it.
    pub(super) fn motion_frame(&self, window: &mut Window, cx: &mut Context<Self>) {
        let slots = self.layout.slots();
        let mut motion = self.motion.borrow_mut();
        let facts = motion.frame_facts(window.viewport_size(), self.layout.zoomed(), self.resizing);
        motion.begin_frame(facts);
        motion.retain(&slots);
        for slot in slots {
            let present = self.leaf_visible(Leaf::Slot(slot));
            motion.sample(slot, present, window, cx);
        }
    }
}
