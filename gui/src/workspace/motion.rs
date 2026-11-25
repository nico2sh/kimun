//! The GPUI half of GUI motion (ADR-0060): the presence store, and the
//! weights store in [`weights`]. This module is the only one that names the
//! kit's motion types (`gpui_kit::base::motion`), so a change in the kit
//! reaches one place.
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
//!    not). Say in [`MotionKey::staged`] whether it follows the reveal
//!    order. A color that fades (the focus indicator) does not: it moves at
//!    once, in both directions, and its amount is `Presence::frame`. A new
//!    focus signal is not a key of its own: add a `FocusTarget` in
//!    `motion/focus.rs`, one line in `Workspace::is_focused`, and draw its
//!    color with `Workspace::focus_color`.
//! 2. Add a [`Spec`] in `Workspace::motion_specs`: when the model shows it,
//!    at which [`Speed`], and how it starts ([`Enter`]). Put a parent
//!    before what comes with it.
//! 3. Where it draws, read `Motion::presence`. The frame value sizes it.
//!    The content value fades what is inside.
//! 4. Build it with [`MotionBox`](super::motion_box::MotionBox):
//!    `faded`, `framed` (with the `Clip` rule it follows) and
//!    `covered_while`. Keep its content
//!    pinned to a size only while it moves. At rest it fills its box, to
//!    the pixel.
//! 5. If it must be drawn after the model dropped it, `Motion::remember` a
//!    snapshot type of its own where it draws, and `Motion::recall` it
//!    when it leaves. The store does not change.
//!
//! Five more things a new widget can need:
//!
//! - A neighbor that is sized in pixels must read the frame value too. The
//!   rail needed `drawn_tree_area`, and slots need `shape_factor`.
//! - To draw a key that lingers, list `Motion::keys()` and accept `None`
//!   from `Motion::recall`: nothing was remembered if it never drew.
//! - A key that does not linger and must close over time stays in the specs
//!   with `present: false`, as a hidden slot stays in the layout tree.
//!   Dropped from the specs, it vanishes at once.
//! - An instant change needs a field in `SnapFacts` and an argument of
//!   `frame_facts`.
//! - Ids that a whole new layout can reuse need a `Motion::forget`, as in
//!   `install_layout`, and a `snap_next_frame` so the new ones draw in place.
//!
//! A cell that changes size, without opening or closing, is a weight: a
//! `ValueSpec` with a `CellKey` (the child's last leaf, its depth in the
//! tree, and whether the cell is one leaf: a subtree cell and the leaf at
//! its end are two keys, so a collapse that shifts depths cannot make one
//! take the other's weight). Weights live in their own store, `Weights`, and
//! never share a key with a presence. The layout pass draws the weight as
//! it moves. A cell that gets wrapped changes its last leaf, so its key: a spec lists
//! `seed_from` keys, and a new key starts from the first that has a value.
//! This covers wrapping only: a cell that a close unwraps adopts its weight
//! at once.
//! A slot that appears after the first frame enters from nothing
//! (`Enter::FromZero`).
//!
//! A slot that is removed from the tree leaves a ghost: its weight stays in
//! the store and decays, and the layout pass draws a spacer that holds the
//! px the cell last had (`Siblings`, `Weights::ghosts`) and gives them back,
//! so the cells next to it grow into its room over time. Only a leaf cell
//! that left the tree leaves a ghost: not one that only changed its key
//! (its split collapsed around it), and not one whose weight a wrapped cell
//! took as its seed. When the split collapses into the cell that stays (two
//! children), that cell has no parent to hold the ghost, so `render_node`
//! draws it in a split of its own, with the ghost beside it, until the ghost
//! is gone. A keyboard
//! resize step reads the cells as last painted, so two quick steps inside
//! one motion move less than two slow ones: that limit stays.
//!
//! The drawing code never changes the model, and the model never reads the
//! presence.

use std::any::Any;
use std::collections::{HashMap, HashSet};

use gpui_kit::base::motion::{MotionStatus, Transition, TransitionState};
use gpui_kit::{App, Context, Pixels, Size, Window};
use kimun_notes::logic::editor_region::TabId;
use kimun_notes::logic::layout_tree::{Leaf, RailPosition, SlotId, TreeShape};

use super::Workspace;
mod focus;
mod weights;

pub(super) use focus::FocusTarget;
use weights::Weights;
pub(super) use weights::{CellKey, Ghost, Siblings, ValueSpec};

use crate::motion::{SnapFacts, Speed, Stage, must_snap, timing};

/// What opens and closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum MotionKey {
    Slot(SlotId),
    Rail,
    TabStrip,
    Tab(TabId),
    /// How much a focus target shows the focus indicator (0 to 1): a border,
    /// a title, a bar, a marker, a cursor ring (see [`FocusTarget`]). The
    /// model says what is focused at once, and this catches up. It is not
    /// staged: see [`MotionKey::staged`].
    Focus(FocusTarget),
}

impl MotionKey {
    /// Whether a key that is no longer asked for stays, moving out, until
    /// its frame is 0 (a tab label: it is drawn from a snapshot while it
    /// leaves). Any other key is dropped at once: what it moved is gone
    /// from the model, and the drawing code does not draw it.
    fn lingers(self) -> bool {
        matches!(self, MotionKey::Tab(_))
    }

    /// Whether the key moves in the reveal order: on enter the frame first
    /// and the content second, on exit the content first and the frame
    /// second. A key that is not staged (a focus amount, which is a color,
    /// not a thing that opens) moves its frame and its content together
    /// over the speed's whole duration, with no delay, in both directions.
    /// Its amount is `Presence::frame`.
    fn staged(self) -> bool {
        !matches!(self, MotionKey::Focus(_))
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

/// A height that follows a measured value at the Quick speed: a widget whose
/// height follows its content (the facet bar). The first sample adopts its
/// target. A new target moves from the value on screen and never queues.
/// Under reduced motion it jumps to the target.
pub(crate) struct HeightFollow {
    state: TransitionState<f32>,
    /// The height a snap set. The next sample starts from it.
    held: Option<f32>,
    /// Whether the last sample was in a motion.
    moving: bool,
}

impl HeightFollow {
    pub(crate) fn new() -> Self {
        Self {
            state: TransitionState::default(),
            held: None,
            moving: false,
        }
    }

    /// The height to draw now, toward `target`. Asks for the next frame
    /// only while a motion runs.
    pub(crate) fn sample(&mut self, target: f32, window: &mut Window, cx: &mut App) -> f32 {
        let policy = || Transition::new(Speed::Quick.duration());
        if let Some(held) = self.held.take() {
            // Adopt the snapped height first, so the target below is a move
            // from it.
            self.state.sample(held, policy(), window, cx);
        }
        let sampled = self.state.sample(target, policy(), window, cx);
        self.moving = matches!(
            sampled.status,
            MotionStatus::Running | MotionStatus::Delayed
        );
        sampled.value
    }

    /// Jump to `target` with no motion.
    pub(crate) fn snap(&mut self, target: f32) {
        if self.held == Some(target) {
            // Snapped there already, and not sampled since.
            return;
        }
        self.state = TransitionState::default();
        self.held = Some(target);
        self.moving = false;
    }

    /// Whether a motion runs.
    pub(crate) fn moving(&self) -> bool {
        self.moving
    }
}

#[derive(Default)]
struct Entry {
    frame: TransitionState<f32>,
    content: TransitionState<f32>,
    /// What was sampled in the current frame.
    now: Presence,
    /// The speed of the last sample: a lingering key leaves at it.
    speed: Speed,
    /// When the entry was made: the one order that keys without an order of
    /// their own can be put in, the same in every process.
    born: u64,
    /// A lingering key that is no longer asked for: its place in the order
    /// in which keys began to leave. Cleared when it is asked for again.
    left: Option<u64>,
    /// What the drawing code remembered of the thing, to draw it after the
    /// model dropped it. Its type belongs to the widget (see
    /// [`Motion::remember`]).
    snapshot: Option<Box<dyn Any>>,
}

/// The presence of everything the drawing code has met.
#[derive(Default)]
pub(super) struct Motion {
    entries: HashMap<MotionKey, Entry>,
    weights: Weights,
    snap: bool,
    snap_next: bool,
    /// How many keys began to leave so far: what [`Entry::left`] counts.
    leaves: u64,
    /// How many entries were made so far: what [`Entry::born`] counts.
    births: u64,
    /// What the last frame had, to tell what changed: the window size, the
    /// zoomed leaf and the rail's edge.
    last_viewport: Option<Size<Pixels>>,
    last_zoom: Option<Option<Leaf>>,
    last_rail: Option<RailPosition>,
}

impl Motion {
    /// The entry of `key`, made (and numbered) if it is new.
    fn entry_mut(&mut self, key: MotionKey) -> &mut Entry {
        let births = &mut self.births;
        self.entries.entry(key).or_insert_with(|| {
            *births += 1;
            Entry {
                born: *births,
                ..Entry::default()
            }
        })
    }

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
        // Taken first: a frame that snaps on its facts uses a pending snap up
        // too, and does not leave it for a later, ordinary frame.
        let pending = std::mem::take(&mut self.snap_next);
        self.snap = must_snap(facts) || pending;
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
        for key in &asked {
            if let Some(entry) = self.entries.get_mut(key) {
                entry.left = None;
            }
        }
        let mut leaving: Vec<(MotionKey, Speed)> = self
            .entries
            .iter()
            .filter(|(key, _)| !asked.contains(key) && key.lingers())
            .map(|(key, entry)| (*key, entry.speed))
            .collect();
        // Keys that begin to leave in one frame have no order of their own:
        // they leave in the order they were made.
        leaving.sort_by_key(|(key, _)| self.entries[key].born);
        for (key, speed) in leaving {
            let order = self.leaves;
            if let Some(entry) = self.entries.get_mut(&key)
                && entry.left.is_none()
            {
                entry.left = Some(order);
                self.leaves += 1;
            }
            self.sample(key, false, speed, Enter::AtRest, window, cx);
        }
        self.entries
            .retain(|key, entry| asked.contains(key) || (key.lingers() && entry.now.frame > 0.0));
    }

    /// Move every weight toward its target for this frame (see
    /// [`Weights::sample`]). A snapping frame adopts every target at once.
    pub(super) fn sample_weights(
        &mut self,
        specs: &[ValueSpec],
        window: &mut Window,
        cx: &mut App,
    ) {
        self.weights.sample(specs, self.snap, window, cx);
    }

    /// The ghosts at nesting depth `depth`: the weights of cells removed
    /// from the tree that are still decaying.
    pub(super) fn ghosts(&self, depth: u8) -> Vec<Ghost> {
        self.weights.ghosts(depth)
    }

    /// What a weight was sampled to in this frame.
    pub(super) fn value(&self, key: CellKey) -> Option<f32> {
        self.weights.value(key)
    }

    /// Remember what stands next to the cell `key` and how many px it has,
    /// for a ghost.
    pub(super) fn remember_siblings(&mut self, key: CellKey, siblings: Siblings) {
        self.weights.remember(key, siblings);
    }

    /// Whether the weights store holds the cell `key` already.
    pub(super) fn holds_cell(&self, key: CellKey) -> bool {
        self.weights.holds(key)
    }

    /// The cells the weights store holds, the ghosts included.
    #[cfg(test)]
    pub(super) fn cell_keys(&self) -> Vec<CellKey> {
        self.weights.keys()
    }

    /// Forget every weight. For a wholesale change of the layout tree: the
    /// new cells have nothing to move from.
    pub(super) fn forget_weights(&mut self) {
        self.weights.forget_all();
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
        let entry = self.entry_mut(key);
        entry.speed = speed;
        if snap {
            entry.frame = TransitionState::default();
            entry.content = TransitionState::default();
        }
        let staged = key.staged();
        let policy = |stage, opening| {
            if !staged {
                return Transition::new(speed.duration());
            }
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

    /// The place of `key` in the order in which keys began to leave, or
    /// `None` while it is asked for.
    pub(super) fn left_order(&self, key: MotionKey) -> Option<u64> {
        self.entries.get(&key)?.left
    }

    /// What was sampled for `key` in this frame: `None` for a key the store
    /// does not hold.
    pub(super) fn try_presence(&self, key: MotionKey) -> Option<Presence> {
        self.entries.get(&key).map(|entry| entry.now)
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
        self.entry_mut(key).snapshot = Some(Box::new(value));
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

impl Workspace {
    /// Sample everything that opens and closes for this frame, before it is
    /// drawn: each thing is present while the layout model makes it
    /// visible, and its presence catches up over time. Reads the model,
    /// never writes it.
    pub(super) fn motion_frame(&self, window: &mut Window, cx: &mut Context<Self>) {
        let specs = self.motion_specs();
        let weights = {
            let motion = self.motion.borrow();
            self.weight_specs(|key| motion.holds_cell(key))
        };
        let mut motion = self.motion.borrow_mut();
        let facts = motion.frame_facts(
            window.viewport_size(),
            self.layout.zoomed(),
            self.resizing,
            self.rail,
        );
        motion.begin_frame(facts);
        motion.sample_all(&specs, window, cx);
        motion.sample_weights(&weights, window, cx);
    }

    /// The weight of every split child of the layout tree, each asked to be
    /// what the model says: a weight that changed moves over time, so a
    /// neighbor of a new column does not jump. A slot closed from the tree
    /// leaves a ghost instead (see the module doc). A zoomed layout draws
    /// one leaf and asks for nothing.
    pub(super) fn weight_specs(&self, held: impl Fn(CellKey) -> bool) -> Vec<ValueSpec> {
        fn leaves(shape: &TreeShape, out: &mut Vec<Leaf>) {
            match shape {
                TreeShape::Leaf(leaf) => out.push(*leaf),
                TreeShape::Split { children, .. } => {
                    for (_, child) in children {
                        leaves(child, out);
                    }
                }
            }
        }
        fn walk(
            shape: &TreeShape,
            depth: u8,
            held: &impl Fn(CellKey) -> bool,
            out: &mut Vec<ValueSpec>,
        ) {
            if let TreeShape::Split { children, .. } = shape {
                for (weight, child) in children {
                    let key = CellKey::of(child, depth);
                    // A cell that gets wrapped changes its last leaf, so its
                    // key: it can start from the key of any leaf inside it.
                    // A cell the store holds already starts from nothing.
                    let mut inside = Vec::new();
                    if !held(key) {
                        leaves(child, &mut inside);
                    }
                    let is_leaf = matches!(child, TreeShape::Leaf(_));
                    let seed_from = inside
                        .into_iter()
                        .flat_map(|leaf| {
                            // A subtree cell can start from a leaf cell or a
                            // subtree cell it wraps. A leaf cell starts from
                            // a leaf cell only: one that stands where a
                            // subtree cell stood must not take its weight.
                            let cell = |is_leaf| CellKey {
                                leaf,
                                depth,
                                is_leaf,
                            };
                            let subtree = (!is_leaf).then_some(cell(false));
                            std::iter::once(cell(true)).chain(subtree)
                        })
                        .filter(|seed| *seed != key)
                        .collect();
                    out.push(ValueSpec {
                        key,
                        target: *weight as f32,
                        speed: Speed::Standard,
                        seed_from,
                    });
                    walk(child, depth + 1, held, out);
                }
            }
        }
        let mut specs = Vec::new();
        if self.layout.zoomed().is_none() {
            walk(&self.layout.shape(), 1, &held, &mut specs);
        }
        specs
    }

    /// Whether the model wants the rail shown. The motion spec and the
    /// drawing code both ask this, so they never disagree.
    pub(super) fn rail_wanted(&self) -> bool {
        !self.rail_hidden
    }

    /// What each thing is asked to do this frame, in the order it is
    /// sampled: a parent before what comes with it.
    fn motion_specs(&self) -> Vec<Spec> {
        let strip = self.strip_wanted();
        let mut specs = vec![
            Spec {
                key: MotionKey::Rail,
                present: self.rail_wanted(),
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
            // A slot that is new after the first frame opens from nothing.
            // The ones a whole new layout brings snap (`install_layout`).
            enter: Enter::FromZero,
        }));
        // The focus indicator of everything that can look focused: lit while
        // the model focuses it, and a thing that is new fades in.
        specs.extend(self.focus_targets().into_iter().map(|target| Spec {
            key: MotionKey::Focus(target),
            present: self.is_focused(target),
            speed: Speed::Quick,
            enter: Enter::FromZero,
        }));
        specs
    }
}
