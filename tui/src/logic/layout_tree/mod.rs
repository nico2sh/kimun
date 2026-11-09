//! The GUI's **layout tree** (ADR-0052): one recursive `Node` whose leaves
//! are **slots** (holding **drawer views**) and **editor groups**, to the
//! right of the Activity rail. This module owns the structure and placement
//! rules — which views a slot holds, which one is shown, splitting and
//! closing leaves — plus the stored focus, geometric focus moves and zoom. Toolkit-free; the TUI does not
//! use it.
//!
//! Placement rules: a view is in at most one slot; a slot shows at most one
//! of its views. Showing a view hides the one sharing its slot, which stays
//! placed there. Hiding only clears the shown flag. Closing a slot never
//! loses a view: its views join a neighbouring slot, hidden.

use std::collections::BTreeMap;

pub mod spec;

pub use crate::logic::layout::node::Fit;
pub(crate) use crate::logic::layout::node::Node;
pub use crate::logic::layout::{Axis, Dir, GroupId, Side, UnitRect};

/// Tolerance for comparing unit-square coordinates.
const EPS: f64 = 1e-4;

/// Length of the overlap of `[a0, a1]` and `[b0, b1]` (0 when disjoint).
fn overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

/// The leaf of `regions` that reads first: leftmost, then topmost.
fn first_in_reading_order(regions: &[(Leaf, UnitRect)]) -> Option<Leaf> {
    regions
        .iter()
        .min_by(|a, b| {
            let x = if (a.1.x() - b.1.x()).abs() <= EPS {
                std::cmp::Ordering::Equal
            } else {
                a.1.x().total_cmp(&b.1.x())
            };
            x.then(a.1.y().total_cmp(&b.1.y()))
        })
        .map(|(l, _)| *l)
}

/// Identifies one slot. Stable across splits and closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SlotId(u32);

impl SlotId {
    /// The slot with id `n` (for layouts built from a spec).
    pub(crate) fn new(n: u32) -> SlotId {
        SlotId(n)
    }
}

/// A leaf of the layout tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Leaf {
    Slot(SlotId),
    Group(GroupId),
}

/// What holds focus: the rail (outside the tree) or one leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Focus {
    Rail,
    Leaf(Leaf),
}

/// What [`LayoutTree::focus_dir`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeFocusMove {
    /// Focus moved here.
    Moved(Focus),
    /// A neighbour could exist in that direction but focus is at the edge;
    /// focus is unchanged and the key is handled (no wrap).
    AtEdge,
    /// Up/Down with nothing stacked in the focused column (or from the
    /// rail): focus is unchanged and the caller should let the key through.
    NotHandled,
}

/// A read-only picture of the tree's structure, for a frontend that draws
/// it: each split with its axis and its children's weights (normalized per
/// split), each leaf by id, in reading order.
#[derive(Debug, Clone, PartialEq)]
pub enum TreeShape {
    Leaf(Leaf),
    Split {
        axis: Axis,
        children: Vec<(f64, TreeShape)>,
    },
}

impl TreeShape {
    fn of(node: &Node<Leaf>) -> TreeShape {
        match node.split_children() {
            Some((axis, children)) => TreeShape::Split {
                axis,
                children: children
                    .into_iter()
                    .map(|(w, child)| (w, TreeShape::of(child)))
                    .collect(),
            },
            None => TreeShape::Leaf(*node.as_leaf().expect("a node is a leaf or a split")),
        }
    }
}

/// Why a tree operation (or `LayoutTree::from_parts`) was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeError {
    /// The only editor group cannot close.
    LastGroup,
    /// The only slot cannot close: its views would have nowhere to go.
    LastSlot,
    /// No such slot in the tree (or a slot entry with no leaf).
    NoSuchSlot,
    /// No such editor group in the tree.
    NoSuchGroup,
    /// A view is not where it was claimed to be (a `shown` view not among
    /// its slot's views).
    NoSuchView,
    /// The tree has no editor group.
    NoGroup,
    /// A view appears more than once.
    DuplicateView,
    /// A leaf id appears more than once in the node.
    DuplicateLeaf,
    /// A slot leaf has no entry in the slot map.
    MissingSlot,
}

/// The views placed in one slot, in placement order, and which is shown.
#[derive(Debug, Clone, PartialEq)]
struct SlotState<V> {
    views: Vec<V>,
    shown: Option<usize>,
}

impl<V: Clone + PartialEq> SlotState<V> {
    fn position(&self, view: &V) -> Option<usize> {
        self.views.iter().position(|v| v == view)
    }

    /// Take `view` out, keeping the shown index pointing at the same view.
    fn remove(&mut self, view: &V) {
        let Some(i) = self.position(view) else {
            return;
        };
        self.views.remove(i);
        self.shown = match self.shown {
            Some(s) if s == i => None,
            Some(s) if s > i => Some(s - 1),
            other => other,
        };
    }
}

/// The GUI layout: a tree of slots and editor groups, the views placed in
/// each slot, the active group, the stored focus and the zoomed leaf. At
/// least one editor group always exists.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutTree<V> {
    root: Node<Leaf>,
    slots: BTreeMap<SlotId, SlotState<V>>,
    active_group: GroupId,
    focus: Focus,
    /// The slot focused last, the tie-breaker for geometric moves.
    last_slot: Option<SlotId>,
    /// The leaf taking over the area right of the rail (session-only; not
    /// a placement).
    zoomed: Option<Leaf>,
    next_slot: u32,
    next_group: u32,
}

impl<V: Clone + PartialEq> LayoutTree<V> {
    /// The built-in arrangement: a left slot showing `files`, the editor
    /// group, and an empty right slot, at weights 0.2 / 0.6 / 0.2.
    pub fn default_layout(files: V) -> Self {
        let (left, right) = (SlotId(0), SlotId(1));
        let root = Node::split(
            Axis::Horizontal,
            vec![
                (0.2, Node::leaf(Leaf::Slot(left))),
                (0.6, Node::leaf(Leaf::Group(GroupId::new(0)))),
                (0.2, Node::leaf(Leaf::Slot(right))),
            ],
        );
        let slots = BTreeMap::from([
            (left, (vec![files.clone()], Some(files))),
            (right, (Vec::new(), None)),
        ]);
        Self::from_parts(root, slots).expect("the default layout is valid")
    }

    /// Build a tree from its parts: the node and, per slot leaf, its views
    /// and the shown one. **The single constructor** every layout goes
    /// through (the default layout and preset/spec building), so its
    /// checks are the tree's invariants: leaf ids are distinct, at least
    /// one editor group exists, every slot leaf has exactly one entry (and
    /// every entry a leaf), every view is in at most one slot, and `shown`
    /// is among its slot's views. The active group and focus start on the
    /// first group in reading order.
    pub(crate) fn from_parts(
        root: Node<Leaf>,
        slots: BTreeMap<SlotId, (Vec<V>, Option<V>)>,
    ) -> Result<Self, TreeError> {
        let leaves: Vec<Leaf> = root.leaves().into_iter().copied().collect();
        for (i, leaf) in leaves.iter().enumerate() {
            if leaves[..i].contains(leaf) {
                return Err(TreeError::DuplicateLeaf);
            }
        }
        let groups: Vec<GroupId> = leaves
            .iter()
            .filter_map(|l| match l {
                Leaf::Group(g) => Some(*g),
                Leaf::Slot(_) => None,
            })
            .collect();
        let Some(&first) = groups.first() else {
            return Err(TreeError::NoGroup);
        };
        let slot_leaves: Vec<SlotId> = leaves
            .iter()
            .filter_map(|l| match l {
                Leaf::Slot(s) => Some(*s),
                Leaf::Group(_) => None,
            })
            .collect();
        if slot_leaves.iter().any(|s| !slots.contains_key(s)) {
            return Err(TreeError::MissingSlot);
        }
        if slots.keys().any(|s| !slot_leaves.contains(s)) {
            return Err(TreeError::NoSuchSlot);
        }
        let mut seen: Vec<&V> = Vec::new();
        let mut states = BTreeMap::new();
        for (id, (views, shown)) in &slots {
            for v in views {
                if seen.contains(&v) {
                    return Err(TreeError::DuplicateView);
                }
                seen.push(v);
            }
            let shown = match shown {
                None => None,
                Some(s) => Some(
                    views
                        .iter()
                        .position(|v| v == s)
                        .ok_or(TreeError::NoSuchView)?,
                ),
            };
            states.insert(
                *id,
                SlotState {
                    views: views.clone(),
                    shown,
                },
            );
        }
        let next_slot = slot_leaves.iter().map(|s| s.0 + 1).max().unwrap_or(0);
        Ok(Self {
            root,
            slots: states,
            active_group: first,
            focus: Focus::Leaf(Leaf::Group(first)),
            last_slot: None,
            zoomed: None,
            next_slot,
            next_group: next_group_after(&groups),
        })
    }

    /// The tree itself, for layout fitting and spec export.
    pub(crate) fn node(&self) -> &Node<Leaf> {
        &self.root
    }

    /// The tree's structure, for drawing it.
    pub fn shape(&self) -> TreeShape {
        TreeShape::of(&self.root)
    }

    // ── Focus ─────────────────────────────────────────────────────────

    /// What holds focus.
    pub fn focused(&self) -> Focus {
        self.focus
    }

    /// Focus the rail or a leaf of the tree. Focusing a group makes it the
    /// active group. False (nothing changes) for a leaf not in the tree.
    pub fn focus(&mut self, focus: Focus) -> bool {
        if let Focus::Leaf(leaf) = focus {
            if !self.root.contains(&leaf) {
                return false;
            }
            match leaf {
                Leaf::Group(g) => self.active_group = g,
                Leaf::Slot(s) => self.last_slot = Some(s),
            }
        }
        self.focus = focus;
        true
    }

    // ── Geometry ──────────────────────────────────────────────────────

    /// Every leaf with its rectangle in the unit square (the area right of
    /// the rail; the rail itself is not a region), in reading order.
    pub fn regions(&self) -> Vec<(Leaf, UnitRect)> {
        self.root.layout(UnitRect::new(0.0, 0.0, 1.0, 1.0))
    }

    /// Whether `leaf` is the one focus last rested on: the active group or
    /// the slot focused last.
    fn is_remembered(&self, leaf: Leaf) -> bool {
        match leaf {
            Leaf::Group(g) => g == self.active_group,
            Leaf::Slot(s) => self.last_slot == Some(s),
        }
    }

    /// The nearest region among `regions` in `dir` from `cur`, skipping
    /// `from`. Candidates lie wholly on that side and overlap `cur` across
    /// the axis. The nearest wins; ties go to the largest overlap, then the
    /// remembered leaf, then the top-left one.
    fn nearest(
        &self,
        regions: &[(Leaf, UnitRect)],
        from: Leaf,
        cur: UnitRect,
        dir: Dir,
    ) -> Option<Leaf> {
        let across_x = |c: &UnitRect| overlap(c.x(), c.right(), cur.x(), cur.right());
        let across_y = |c: &UnitRect| overlap(c.y(), c.bottom(), cur.y(), cur.bottom());
        let candidates: Vec<(Leaf, f64, f64, UnitRect)> = regions
            .iter()
            .filter(|(l, _)| *l != from)
            .filter_map(|(l, c)| {
                let (beyond, gap, ov) = match dir {
                    Dir::Left => (c.right() <= cur.x() + EPS, cur.x() - c.right(), across_y(c)),
                    Dir::Right => (c.x() >= cur.right() - EPS, c.x() - cur.right(), across_y(c)),
                    Dir::Up => (
                        c.bottom() <= cur.y() + EPS,
                        cur.y() - c.bottom(),
                        across_x(c),
                    ),
                    Dir::Down => (
                        c.y() >= cur.bottom() - EPS,
                        c.y() - cur.bottom(),
                        across_x(c),
                    ),
                };
                (beyond && ov > EPS).then_some((*l, gap, ov, *c))
            })
            .collect();
        let nearest = candidates
            .iter()
            .map(|(_, gap, _, _)| *gap)
            .fold(f64::INFINITY, f64::min);
        candidates
            .iter()
            .filter(|(_, gap, _, _)| *gap <= nearest + EPS)
            .max_by(|a, b| {
                let overlap = if (a.2 - b.2).abs() <= EPS {
                    std::cmp::Ordering::Equal
                } else {
                    a.2.total_cmp(&b.2)
                };
                overlap
                    .then(self.is_remembered(a.0).cmp(&self.is_remembered(b.0)))
                    .then(b.3.y().total_cmp(&a.3.y()))
                    .then(b.3.x().total_cmp(&a.3.x()))
            })
            .map(|(l, ..)| *l)
    }

    /// Move focus geometrically to the nearest leaf in `dir` (see
    /// `Self::nearest`). No wrap: with no candidate, Left/Right report
    /// [`TreeFocusMove::AtEdge`], and so do Up/Down when something is
    /// stacked in the same column — otherwise [`TreeFocusMove::NotHandled`].
    /// The rail sits at the far left: Left from a leaf with nothing to its
    /// left reaches [`Focus::Rail`], and Right from the rail reaches the
    /// first leaf (leftmost, then topmost); Up/Down from the rail are not
    /// handled. While a leaf is zoomed, only it and the rail are visible:
    /// the zoomed leaf covers the unit square and is the only candidate.
    pub fn focus_dir(&mut self, dir: Dir) -> TreeFocusMove {
        self.focus_dir_among(dir, &|_| true)
    }

    /// The leaves `visible` accepts with the rectangles they are drawn at:
    /// the others are removed first, so their siblings share their weight
    /// (as [`Self::fit`] draws them), not the rectangles they would have
    /// had.
    fn visible_regions(&self, visible: &dyn Fn(Leaf) -> bool) -> Vec<(Leaf, UnitRect)> {
        let mut root = self.root.clone();
        for leaf in self.root.leaves() {
            if !visible(*leaf) {
                root.remove_leaf(leaf);
            }
        }
        root.layout(UnitRect::new(0.0, 0.0, 1.0, 1.0))
            .into_iter()
            .filter(|(l, _)| visible(*l))
            .collect()
    }

    /// [`Self::focus_dir`] over only the leaves `visible` accepts (a
    /// frontend that leaves some leaves undrawn, such as a slot with no
    /// shown view): the others are neither targets nor obstacles, and keep
    /// no place in the geometry: the rest is laid out as drawn. The zoomed
    /// leaf ignores `visible`.
    pub fn focus_dir_among(&mut self, dir: Dir, visible: &dyn Fn(Leaf) -> bool) -> TreeFocusMove {
        let regions = match self.zoomed {
            Some(z) => vec![(z, UnitRect::new(0.0, 0.0, 1.0, 1.0))],
            None => self.visible_regions(visible),
        };
        let current = match self.focus {
            Focus::Leaf(l) => l,
            Focus::Rail => {
                return match dir {
                    Dir::Right => {
                        let first = first_in_reading_order(&regions);
                        match first {
                            Some(l) => {
                                self.focus(Focus::Leaf(l));
                                TreeFocusMove::Moved(Focus::Leaf(l))
                            }
                            None => TreeFocusMove::AtEdge,
                        }
                    }
                    Dir::Left => TreeFocusMove::AtEdge,
                    Dir::Up | Dir::Down => TreeFocusMove::NotHandled,
                };
            }
        };
        let Some(cur) = regions
            .iter()
            .find(|(l, _)| *l == current)
            .map(|(_, rect)| *rect)
        else {
            return TreeFocusMove::NotHandled;
        };
        match self.nearest(&regions, current, cur, dir) {
            Some(l) => {
                self.focus(Focus::Leaf(l));
                TreeFocusMove::Moved(Focus::Leaf(l))
            }
            None => match dir {
                Dir::Left => {
                    self.focus = Focus::Rail;
                    TreeFocusMove::Moved(Focus::Rail)
                }
                Dir::Right => TreeFocusMove::AtEdge,
                Dir::Up | Dir::Down => {
                    let stacked = regions.iter().any(|(l, c)| {
                        *l != current && overlap(c.x(), c.right(), cur.x(), cur.right()) > EPS
                    });
                    if stacked {
                        TreeFocusMove::AtEdge
                    } else {
                        TreeFocusMove::NotHandled
                    }
                }
            },
        }
    }

    // ── Sides ─────────────────────────────────────────────────────────

    /// The active group's rectangle.
    fn active_rect(&self) -> Option<UnitRect> {
        let active = Leaf::Group(self.active_group);
        self.regions()
            .into_iter()
            .find(|(leaf, _)| *leaf == active)
            .map(|(_, rect)| rect)
    }

    /// The slots of `side` in reading order: those lying entirely left
    /// (right) of the active group.
    pub fn side_slots(&self, side: Side) -> Vec<SlotId> {
        let Some(group) = self.active_rect() else {
            return Vec::new();
        };
        self.regions()
            .into_iter()
            .filter_map(|(leaf, rect)| match leaf {
                Leaf::Slot(slot) => {
                    let beside = match side {
                        Side::Left => rect.right() <= group.x() + EPS,
                        Side::Right => rect.x() >= group.right() - EPS,
                    };
                    beside.then_some(slot)
                }
                Leaf::Group(_) => None,
            })
            .collect()
    }

    /// The editor group nearest the window edge on `side`: the leftmost
    /// (rightmost) one, the first in reading order on a tie. Unlike
    /// [`Self::side_slots`] it does not depend on the active group.
    pub fn edge_group(&self, side: Side) -> Option<GroupId> {
        let mut best: Option<(GroupId, f64)> = None;
        for (leaf, rect) in self.regions() {
            let Leaf::Group(g) = leaf else { continue };
            // Smaller is nearer the edge.
            let key = match side {
                Side::Left => rect.x(),
                Side::Right => -rect.right(),
            };
            if best.is_none_or(|(_, k)| key < k - EPS) {
                best = Some((g, key));
            }
        }
        best.map(|(g, _)| g)
    }

    /// The slots lying entirely outside every editor group on `side`, in
    /// reading order: the window-edge counterpart of [`Self::side_slots`],
    /// the same whichever group is active.
    pub fn edge_slots(&self, side: Side) -> Vec<SlotId> {
        let regions = self.regions();
        let groups = regions.iter().filter(|(l, _)| matches!(l, Leaf::Group(_)));
        let bound = match side {
            Side::Left => groups.map(|(_, r)| r.x()).fold(f64::INFINITY, f64::min),
            Side::Right => groups
                .map(|(_, r)| r.right())
                .fold(f64::NEG_INFINITY, f64::max),
        };
        regions
            .into_iter()
            .filter_map(|(leaf, rect)| match leaf {
                Leaf::Slot(slot) => {
                    let outside = match side {
                        Side::Left => rect.right() <= bound + EPS,
                        Side::Right => rect.x() >= bound - EPS,
                    };
                    outside.then_some(slot)
                }
                Leaf::Group(_) => None,
            })
            .collect()
    }

    /// The side `slot` reads as: left when its centre lies left of the
    /// active group's, else right.
    pub fn slot_side(&self, slot: SlotId) -> Side {
        let rect = self
            .regions()
            .into_iter()
            .find(|(leaf, _)| *leaf == Leaf::Slot(slot))
            .map(|(_, rect)| rect);
        match (rect, self.active_rect()) {
            (Some(rect), Some(group))
                if rect.x() + rect.width() / 2.0 >= group.x() + group.width() / 2.0 =>
            {
                Side::Right
            }
            _ => Side::Left,
        }
    }

    /// The first slot right of the rail (leftmost, then topmost), drawn
    /// or not.
    pub fn slot_beside_rail(&self) -> Option<SlotId> {
        let slots: Vec<_> = self
            .regions()
            .into_iter()
            .filter(|(leaf, _)| matches!(leaf, Leaf::Slot(_)))
            .collect();
        match first_in_reading_order(&slots) {
            Some(Leaf::Slot(slot)) => Some(slot),
            _ => None,
        }
    }

    /// Whether another slot `drawn` accepts shares `slot`'s column
    /// (stacked above or below it).
    pub fn slot_stacked(&self, slot: SlotId, drawn: &dyn Fn(SlotId) -> bool) -> bool {
        let regions = self.regions();
        let Some(rect) = regions
            .iter()
            .find(|(leaf, _)| *leaf == Leaf::Slot(slot))
            .map(|(_, rect)| *rect)
        else {
            return false;
        };
        regions.iter().any(|(leaf, other)| match leaf {
            Leaf::Slot(s) => {
                *s != slot
                    && drawn(*s)
                    && overlap(other.x(), other.right(), rect.x(), rect.right()) > EPS
            }
            Leaf::Group(_) => false,
        })
    }

    // ── Zoom ──────────────────────────────────────────────────────────

    /// Zoom `leaf` over the area right of the rail, or leave zoom with
    /// `None`. Placements are untouched. A leaf not in the tree is ignored.
    pub fn zoom(&mut self, leaf: Option<Leaf>) {
        match leaf {
            Some(l) if !self.root.contains(&l) => {}
            other => self.zoomed = other,
        }
    }

    /// The zoomed leaf, if any. Cleared when that leaf closes.
    pub fn zoomed(&self) -> Option<Leaf> {
        self.zoomed
    }

    // ── Slots and views ───────────────────────────────────────────────

    /// Every slot in reading order (left → right, top → bottom).
    pub fn slots(&self) -> Vec<SlotId> {
        self.root
            .leaves()
            .into_iter()
            .filter_map(|l| match l {
                Leaf::Slot(s) => Some(*s),
                Leaf::Group(_) => None,
            })
            .collect()
    }

    /// The views placed in `slot`, shown or hidden, in placement order.
    /// Empty for an absent slot.
    pub fn slot_views(&self, slot: SlotId) -> Vec<V> {
        self.slots
            .get(&slot)
            .map(|s| s.views.clone())
            .unwrap_or_default()
    }

    /// The view `slot` shows, if any.
    pub fn shown(&self, slot: SlotId) -> Option<V> {
        let s = self.slots.get(&slot)?;
        s.shown.map(|i| s.views[i].clone())
    }

    /// The slot `view` is placed in, shown or hidden.
    pub fn view_slot(&self, view: &V) -> Option<SlotId> {
        self.slots
            .iter()
            .find(|(_, s)| s.position(view).is_some())
            .map(|(id, _)| *id)
    }

    /// Place `view` in `slot`, shown or hidden (restoring saved state).
    /// Refused (false) when `view` is already placed, `slot` is absent, or
    /// it should be shown and `slot` already shows a view.
    pub fn place(&mut self, view: V, slot: SlotId, shown: bool) -> bool {
        if self.view_slot(&view).is_some() {
            return false;
        }
        let Some(state) = self.slots.get_mut(&slot) else {
            return false;
        };
        if shown && state.shown.is_some() {
            return false;
        }
        state.views.push(view);
        if shown {
            state.shown = Some(state.views.len() - 1);
        }
        true
    }

    /// Show `view` and return its slot. A placed view is shown where it is
    /// placed, hiding the view its slot showed (which stays placed). An
    /// unplaced view is placed in the last slot in reading order when
    /// `home_right`, else the first. `None` only when the tree has no slot.
    pub fn show_view(&mut self, view: &V, home_right: bool) -> Option<SlotId> {
        let slot = match self.view_slot(view) {
            Some(slot) => slot,
            None => {
                let slots = self.slots();
                let slot = if home_right {
                    *slots.last()?
                } else {
                    *slots.first()?
                };
                self.slots.get_mut(&slot)?.views.push(view.clone());
                slot
            }
        };
        let state = self.slots.get_mut(&slot)?;
        state.shown = state.position(view);
        Some(slot)
    }

    /// Move `view` to the nearest slot in `dir` from its slot (the
    /// geometric rule of [`Self::focus_dir`], with only slots as
    /// candidates), shown there as by [`Self::move_view`]. False when
    /// `view` is unplaced, no slot lies that way, or a leaf is zoomed (the
    /// other slots are not visible).
    pub fn move_view_dir(&mut self, view: &V, dir: Dir) -> bool {
        if self.zoomed.is_some() {
            return false;
        }
        let Some(from) = self.view_slot(view) else {
            return false;
        };
        let from = Leaf::Slot(from);
        let regions: Vec<(Leaf, UnitRect)> = self.regions();
        let Some(cur) = regions.iter().find(|(l, _)| *l == from).map(|(_, r)| *r) else {
            return false;
        };
        let slots: Vec<(Leaf, UnitRect)> = regions
            .into_iter()
            .filter(|(l, _)| matches!(l, Leaf::Slot(_)))
            .collect();
        match self.nearest(&slots, from, cur, dir) {
            Some(Leaf::Slot(target)) => self.move_view(view.clone(), target),
            _ => false,
        }
    }

    /// Hide `view`: clear its shown flag, keeping its placement. Returns
    /// its slot, `None` when it is not shown.
    pub fn hide_view(&mut self, view: &V) -> Option<SlotId> {
        let slot = self.view_slot(view)?;
        let state = self.slots.get_mut(&slot)?;
        if state.shown != state.position(view) {
            return None;
        }
        state.shown = None;
        Some(slot)
    }

    /// Move `view` to `slot`, shown there, replacing the view `slot`
    /// showed (which stays placed there, hidden). A view placed elsewhere
    /// leaves its old slot; a view already in `slot` is just shown. False
    /// (nothing changes) when `slot` is absent.
    pub fn move_view(&mut self, view: V, slot: SlotId) -> bool {
        if !self.slots.contains_key(&slot) {
            return false;
        }
        if let Some(from) = self.view_slot(&view)
            && from != slot
            && let Some(state) = self.slots.get_mut(&from)
        {
            state.remove(&view);
        }
        let state = self.slots.get_mut(&slot).expect("checked above");
        let i = state.position(&view).unwrap_or_else(|| {
            state.views.push(view);
            state.views.len() - 1
        });
        state.shown = Some(i);
        true
    }

    /// Split `slot` along `axis`; a new empty slot comes right after it.
    /// Focus is unchanged.
    pub fn split_slot(&mut self, slot: SlotId, axis: Axis) -> Result<SlotId, TreeError> {
        let new = SlotId(self.next_slot);
        if !self.slots.contains_key(&slot)
            || !self
                .root
                .split_leaf(&Leaf::Slot(slot), axis, Leaf::Slot(new))
        {
            return Err(TreeError::NoSuchSlot);
        }
        self.next_slot += 1;
        self.slots.insert(
            new,
            SlotState {
                views: Vec::new(),
                shown: None,
            },
        );
        Ok(new)
    }

    /// Move `view` into a new slot beside `target`: split along the axis of
    /// `dir` (Up/Down vertical, Left/Right horizontal), the new slot before
    /// `target` for Up and Left, after it for Down and Right. The view
    /// leaves its old slot as by [`Self::move_view`] and is shown in the
    /// new one. Focus is unchanged; the slot the view left is not closed
    /// (see [`Self::vacate_close`]). [`TreeError::NoSuchSlot`] (nothing
    /// changes) when `target` is absent.
    pub fn place_beside(&mut self, view: V, target: SlotId, dir: Dir) -> Result<SlotId, TreeError> {
        if !self.slots.contains_key(&target) {
            return Err(TreeError::NoSuchSlot);
        }
        self.place_beside_leaf(view, Leaf::Slot(target), dir)
            .ok_or(TreeError::NoSuchSlot)
    }

    /// [`Self::place_beside`] against an editor group: the new slot lands
    /// at that group's `dir` edge, which with the group between the sides
    /// is the window-edge side for Left and Right.
    /// [`TreeError::NoSuchGroup`] (nothing changes) when `group` is absent.
    pub fn place_beside_group(
        &mut self,
        view: V,
        group: GroupId,
        dir: Dir,
    ) -> Result<SlotId, TreeError> {
        self.place_beside_leaf(view, Leaf::Group(group), dir)
            .ok_or(TreeError::NoSuchGroup)
    }

    /// The split behind [`Self::place_beside`]: `None` when `target` is not
    /// a leaf of the tree.
    fn place_beside_leaf(&mut self, view: V, target: Leaf, dir: Dir) -> Option<SlotId> {
        let (axis, before) = match dir {
            Dir::Up => (Axis::Vertical, true),
            Dir::Down => (Axis::Vertical, false),
            Dir::Left => (Axis::Horizontal, true),
            Dir::Right => (Axis::Horizontal, false),
        };
        let new = SlotId(self.next_slot);
        if !self
            .root
            .split_leaf_at(&target, axis, Leaf::Slot(new), before)
        {
            return None;
        }
        self.next_slot += 1;
        self.slots.insert(
            new,
            SlotState {
                views: Vec::new(),
                shown: None,
            },
        );
        self.move_view(view, new);
        Some(new)
    }

    /// Close `slot` when it holds no views at all and another slot reads
    /// as on its side of the active group ([`Self::slot_side`]); returns
    /// the slot that absorbed it. `None` otherwise: the last slot on a
    /// side stays, empty, so that side can be filled again.
    pub fn vacate_close(&mut self, slot: SlotId) -> Option<SlotId> {
        if !self.slots.get(&slot)?.views.is_empty() {
            return None;
        }
        let side = self.slot_side(slot);
        if !self
            .slots()
            .into_iter()
            .any(|s| s != slot && self.slot_side(s) == side)
        {
            return None;
        }
        self.close_slot(slot).ok()
    }

    /// Close `slot` and return the slot that absorbed its views, which
    /// join it hidden (its own shown view is unchanged). That is
    /// `Node::absorber` when it is a slot; when the leaf taking over the
    /// area is a group, the nearest remaining slot in reading order on the
    /// absorber's side, else on the other side. Focus on the closed slot
    /// moves to the absorbing slot. Refused for the only slot
    /// ([`TreeError::LastSlot`]).
    pub fn close_slot(&mut self, slot: SlotId) -> Result<SlotId, TreeError> {
        let leaf = Leaf::Slot(slot);
        if !self.slots.contains_key(&slot) {
            return Err(TreeError::NoSuchSlot);
        }
        let leaves: Vec<Leaf> = self.root.leaves().into_iter().copied().collect();
        let Some(area) = self.root.absorber(&leaf) else {
            return Err(TreeError::LastSlot);
        };
        let at = leaves.iter().position(|l| *l == leaf).expect("slot leaf");
        let pos = |l: &Leaf| leaves.iter().position(|x| x == l).expect("leaf");
        let forward = pos(&area) > at;
        let after = leaves[at + 1..].iter();
        let before = leaves[..at].iter().rev();
        let slot_of = |l: &Leaf| match l {
            Leaf::Slot(s) => Some(*s),
            Leaf::Group(_) => None,
        };
        let target = if forward {
            after
                .clone()
                .find_map(slot_of)
                .or_else(|| before.clone().find_map(slot_of))
        } else {
            before
                .clone()
                .find_map(slot_of)
                .or_else(|| after.clone().find_map(slot_of))
        };
        let Some(target) = target else {
            return Err(TreeError::LastSlot);
        };
        self.root.remove_leaf(&leaf);
        let closed = self.slots.remove(&slot).expect("checked above");
        self.slots
            .get_mut(&target)
            .expect("remaining slot")
            .views
            .extend(closed.views);
        if self.focus == Focus::Leaf(leaf) {
            self.focus = Focus::Leaf(Leaf::Slot(target));
        }
        if self.last_slot == Some(slot) {
            self.last_slot = Some(target);
        }
        if self.zoomed == Some(leaf) {
            self.zoomed = None;
        }
        Ok(target)
    }

    // ── Editor groups ─────────────────────────────────────────────────

    /// Every editor group in reading order.
    pub fn groups(&self) -> Vec<GroupId> {
        self.root
            .leaves()
            .into_iter()
            .filter_map(|l| match l {
                Leaf::Group(g) => Some(*g),
                Leaf::Slot(_) => None,
            })
            .collect()
    }

    /// The group that owns save, the footer and `{note}`.
    pub fn active_group(&self) -> GroupId {
        self.active_group
    }

    /// Split `group` along `axis`; the new group comes right after it. The
    /// active group and focus are unchanged.
    pub fn split_group(&mut self, group: GroupId, axis: Axis) -> Result<GroupId, TreeError> {
        let new = GroupId::new(self.next_group);
        if !self
            .root
            .split_leaf(&Leaf::Group(group), axis, Leaf::Group(new))
        {
            return Err(TreeError::NoSuchGroup);
        }
        self.next_group += 1;
        Ok(new)
    }

    /// Close `group`. The last group cannot close. When the active group
    /// closes, the group before it (or the first) becomes active, and focus
    /// on the closed group moves to the new active group.
    pub fn close_group(&mut self, group: GroupId) -> Result<(), TreeError> {
        let groups = self.groups();
        let Some(pos) = groups.iter().position(|&g| g == group) else {
            return Err(TreeError::NoSuchGroup);
        };
        if groups.len() == 1 {
            return Err(TreeError::LastGroup);
        }
        let leaf = Leaf::Group(group);
        self.root.remove_leaf(&leaf);
        if self.active_group == group {
            let groups = self.groups();
            self.active_group = groups[pos.saturating_sub(1).min(groups.len() - 1)];
        }
        if self.focus == Focus::Leaf(leaf) {
            self.focus = Focus::Leaf(Leaf::Group(self.active_group));
        }
        if self.zoomed == Some(leaf) {
            self.zoomed = None;
        }
        Ok(())
    }

    // ── Sizes ─────────────────────────────────────────────────────────

    /// Move the boundary after `leaf`'s branch so it takes `share` of the
    /// pair (see `Node::resize_after`). False when there is none.
    pub fn resize_after(&mut self, leaf: Leaf, share: f64) -> bool {
        self.root.resize_after(&leaf, share)
    }

    /// [`Self::resize_after`] between drawn cells: the boundary after
    /// `leaf`'s branch is with the next sibling holding a `visible` leaf,
    /// hidden siblings between them skipped and keeping their weights
    /// (see `Node::resize_after_visible`). False when there is none.
    pub fn resize_after_visible(
        &mut self,
        leaf: Leaf,
        share: f64,
        visible: &dyn Fn(Leaf) -> bool,
    ) -> bool {
        self.root
            .resize_after_visible(&leaf, share, &|l: &Leaf| visible(*l))
    }

    /// Which leaves fit in `area` (ADR-0052 §6): the leaves `drawn` rejects
    /// are left out first (their siblings share their weight; they are not
    /// reported as collapsed), then each overflowing split collapses its
    /// children last → first by `min` (width, height; heights count only
    /// in vertical splits), never a subtree holding an editor group. Units
    /// are the caller's (rems in the GUI). The tree itself, its weights
    /// and placements are untouched: collapse is the caller's session
    /// state.
    pub fn fit(
        &self,
        area: UnitRect,
        min: &dyn Fn(Leaf) -> (f64, f64),
        drawn: &dyn Fn(Leaf) -> bool,
    ) -> Fit<Leaf> {
        let mut root = self.root.clone();
        for leaf in self.root.leaves() {
            // Groups are always drawn, so the root never empties.
            if !matches!(leaf, Leaf::Group(_)) && !drawn(*leaf) {
                root.remove_leaf(leaf);
            }
        }
        root.layout_fit(area, &|leaf: &Leaf| min(*leaf), &|leaf: &Leaf| {
            matches!(leaf, Leaf::Group(_))
        })
    }

    /// Change weights so [`Self::fit`] (same `area`, `min` and `drawn`)
    /// collapses none of `leaves` (ADR-0054 §8): the cell holding a split
    /// that collapses one grows along that split's axis, in the nearest
    /// split above it along that axis, taking from its siblings only what
    /// each has above its minimum, so none collapses there and no editor
    /// group goes under its minimum. Repeated, a bounded number of times,
    /// while one of `leaves` still collapses. True when none collapses
    /// (already, or now); false, nothing changed, when one is absent, not
    /// drawn, or there is no room to make for it.
    pub fn make_room(
        &mut self,
        leaves: &[Leaf],
        area: UnitRect,
        min: &dyn Fn(Leaf) -> (f64, f64),
        drawn: &dyn Fn(Leaf) -> bool,
    ) -> bool {
        let group = |leaf: &Leaf| matches!(leaf, Leaf::Group(_));
        self.root.make_room(
            leaves,
            area,
            &|leaf: &Leaf| min(*leaf),
            &group,
            &|leaf: &Leaf| group(leaf) || drawn(*leaf),
        )
    }
}

/// The first group id above every id in `groups`. `GroupId` exposes no
/// number, so match candidates upward until every given id is accounted for.
fn next_group_after(groups: &[GroupId]) -> u32 {
    let mut left = groups.len();
    let mut n = 0u32;
    while left > 0 {
        if groups.contains(&GroupId::new(n)) {
            left -= 1;
        }
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests;
