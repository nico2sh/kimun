//! The GUI's **layout tree** (ADR-0052, ADR-0058): one recursive `Node`
//! whose leaves are **slots** (holding **drawer views**) and the one
//! **editor region** leaf, to the right of the Activity rail. The region's
//! tabs and editor groups live in its [`EditorRegion`], which the tree
//! owns. This module owns the structure and placement rules — which views
//! a slot holds, which one is shown, splitting and closing leaves — plus
//! the stored focus, geometric focus moves and zoom. Toolkit-free; the TUI
//! does not use it.
//!
//! Placement rules: a view is in at most one slot; a slot shows at most one
//! of its views. Showing a view hides the one sharing its slot, which stays
//! placed there. Hiding only clears the shown flag. Closing a slot never
//! loses a view: its views join a neighbouring slot, hidden.

use std::collections::BTreeMap;

pub mod rail;
pub mod spec;

pub use rail::RailPosition;

use crate::logic::editor_region::EditorRegion;

pub use crate::logic::layout::node::Fit;
pub(crate) use crate::logic::layout::node::Node;
pub use crate::logic::layout::{Axis, Dir, GroupId, Side, UnitRect};

/// Tolerance for comparing unit-square coordinates.
const EPS: f64 = 1e-4;

/// Length of the overlap of `[a0, a1]` and `[b0, b1]` (0 when disjoint).
fn overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

/// `inner`, a rectangle in the unit square, placed inside `outer`.
fn within(outer: UnitRect, inner: UnitRect) -> UnitRect {
    UnitRect::new(
        outer.x() + inner.x() * outer.width(),
        outer.y() + inner.y() * outer.height(),
        inner.width() * outer.width(),
        inner.height() * outer.height(),
    )
}

/// The nearest region among `regions` in `dir` from `cur`, skipping
/// `from`. Candidates lie wholly on that side and overlap `cur` across
/// the axis. The nearest wins; ties go to the largest overlap, then the
/// `remembered` one, then the top-left one.
fn nearest<T: Copy + PartialEq>(
    regions: &[(T, UnitRect)],
    from: T,
    cur: UnitRect,
    dir: Dir,
    remembered: &dyn Fn(T) -> bool,
) -> Option<T> {
    let across_x = |c: &UnitRect| overlap(c.x(), c.right(), cur.x(), cur.right());
    let across_y = |c: &UnitRect| overlap(c.y(), c.bottom(), cur.y(), cur.bottom());
    let candidates: Vec<(T, f64, f64, UnitRect)> = regions
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
                .then(remembered(a.0).cmp(&remembered(b.0)))
                .then(b.3.y().total_cmp(&a.3.y()))
                .then(b.3.x().total_cmp(&a.3.x()))
        })
        .map(|(l, ..)| *l)
}

/// The leaf of `regions` that touches `rail`'s edge first: nearest the
/// edge, then leftmost (for a row) or topmost (for a column).
fn first_at_rail(regions: &[(Leaf, UnitRect)], rail: RailPosition) -> Option<Leaf> {
    use std::cmp::Ordering;
    // Distance from the rail's edge, and the position along it.
    let key = |r: &UnitRect| -> (f64, f64) {
        match rail {
            RailPosition::Left => (r.x(), r.y()),
            RailPosition::Right => (1.0 - (r.x() + r.width()), r.y()),
            RailPosition::Top => (r.y(), r.x()),
            RailPosition::Bottom => (1.0 - (r.y() + r.height()), r.x()),
        }
    };
    regions
        .iter()
        .min_by(|a, b| {
            let (ad, aa) = key(&a.1);
            let (bd, ba) = key(&b.1);
            let d = if (ad - bd).abs() <= EPS {
                Ordering::Equal
            } else {
                ad.total_cmp(&bd)
            };
            d.then(aa.total_cmp(&ba))
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

/// A leaf of the layout tree: a slot, or the one editor region (whose
/// tabs and groups are the tree's [`EditorRegion`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Leaf {
    Slot(SlotId),
    Editor,
}

/// What holds focus: the rail (outside the tree) or one leaf.
/// `Focus::Leaf(Leaf::Editor)` is the region's active group.
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
    /// The only slot cannot close: its views would have nowhere to go.
    LastSlot,
    /// No such slot in the tree (or a slot entry with no leaf).
    NoSuchSlot,
    /// A view is not where it was claimed to be (a `shown` view not among
    /// its slot's views).
    NoSuchView,
    /// The tree has no editor leaf.
    NoEditor,
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

/// The GUI layout: a tree of slots and the one editor leaf, the views
/// placed in each slot, the editor region's tabs and groups, the stored
/// focus and the zoomed leaf. The editor leaf always exists.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutTree<V> {
    root: Node<Leaf>,
    slots: BTreeMap<SlotId, SlotState<V>>,
    /// The tabs and groups drawn in the editor leaf.
    region: EditorRegion,
    focus: Focus,
    /// The slot focused last, the tie-breaker for geometric moves.
    last_slot: Option<SlotId>,
    /// The leaf taking over the area right of the rail (session-only; not
    /// a placement).
    zoomed: Option<Leaf>,
    next_slot: u32,
}

impl<V: Clone + PartialEq> LayoutTree<V> {
    /// The built-in arrangement: a left slot showing `files`, the editor
    /// region (one tab, one group), and an empty right slot, at weights 0.2 / 0.6 / 0.2.
    pub fn default_layout(files: V) -> Self {
        let (left, right) = (SlotId(0), SlotId(1));
        let root = Node::split(
            Axis::Horizontal,
            vec![
                (0.2, Node::leaf(Leaf::Slot(left))),
                (0.6, Node::leaf(Leaf::Editor)),
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
    /// checks are the tree's invariants: leaf ids are distinct (so there is
    /// at most one editor leaf), the editor leaf exists, every slot leaf has
    /// exactly one entry (and every entry a leaf), every view is in at most
    /// one slot, and `shown` is among its slot's views. The region starts
    /// with one tab of one group; focus starts on the editor.
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
        if !leaves.contains(&Leaf::Editor) {
            return Err(TreeError::NoEditor);
        }
        let slot_leaves: Vec<SlotId> = leaves
            .iter()
            .filter_map(|l| match l {
                Leaf::Slot(s) => Some(*s),
                Leaf::Editor => None,
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
            region: EditorRegion::new(),
            focus: Focus::Leaf(Leaf::Editor),
            last_slot: None,
            zoomed: None,
            next_slot,
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

    /// Focus the rail or a leaf of the tree. Focusing the editor focuses
    /// the region's active group. False (nothing changes) for a leaf not in
    /// the tree.
    pub fn focus(&mut self, focus: Focus) -> bool {
        if let Focus::Leaf(leaf) = focus {
            if !self.root.contains(&leaf) {
                return false;
            }
            if let Leaf::Slot(s) = leaf {
                self.last_slot = Some(s);
            }
        }
        self.focus = focus;
        true
    }

    // ── Geometry ──────────────────────────────────────────────────────

    /// Every leaf with its rectangle in the unit square (the area right of
    /// the rail; the rail itself is not a region), in reading order. The
    /// editor leaf appears once, whole; [`Self::group_regions`] subdivides
    /// it.
    pub fn regions(&self) -> Vec<(Leaf, UnitRect)> {
        self.root.layout(UnitRect::new(0.0, 0.0, 1.0, 1.0))
    }

    /// The editor leaf's rectangle in the unit square.
    fn editor_rect(&self) -> Option<UnitRect> {
        self.regions()
            .into_iter()
            .find(|(leaf, _)| *leaf == Leaf::Editor)
            .map(|(_, rect)| rect)
    }

    /// The shown tab's groups with their rectangles in the unit square of
    /// [`Self::regions`]: the editor leaf's rectangle subdivided by the
    /// region's [`EditorRegion::regions`], in reading order.
    pub fn group_regions(&self) -> Vec<(GroupId, UnitRect)> {
        let Some(editor) = self.editor_rect() else {
            return Vec::new();
        };
        self.region
            .regions()
            .into_iter()
            .map(|(g, r)| (g, within(editor, r)))
            .collect()
    }

    /// Whether `leaf` is the one focus last rested on: the editor (its
    /// active group) or the slot focused last.
    fn is_remembered(&self, leaf: Leaf) -> bool {
        match leaf {
            Leaf::Editor => true,
            Leaf::Slot(s) => self.last_slot == Some(s),
        }
    }

    /// Move focus geometrically to the nearest leaf in `dir` (see
    /// `nearest`). From the editor, the shown tab's groups come first: the
    /// nearest group that way becomes active and focus stays on the editor
    /// leaf; only with none does focus leave the region, measured from the
    /// active group's rectangle. No wrap: with no candidate, Left/Right report
    /// [`TreeFocusMove::AtEdge`], and so do Up/Down when something is
    /// stacked in the same column — otherwise [`TreeFocusMove::NotHandled`].
    /// The rail sits at the far left here (the GUI uses [`Self::focus_dir_in`]
    /// for the other edges): Left from a leaf with nothing to its
    /// left reaches [`Focus::Rail`], and Right from the rail reaches the
    /// first leaf (leftmost, then topmost); Up/Down from the rail are not
    /// handled. While a leaf is zoomed, only it and the rail are visible:
    /// the zoomed leaf covers the unit square and is the only candidate.
    pub fn focus_dir(&mut self, dir: Dir) -> TreeFocusMove {
        self.focus_dir_in(dir, &|_| true, Some(RailPosition::Left))
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
    /// leaf ignores `visible`. The rail is at `rail` (ADR-0055), or
    /// `None` while it is hidden. From the rail, the direction into the
    /// layout focuses the leaf that touches the rail's edge, the direction
    /// away from it is the edge, and the two across are not the rail's. From
    /// a leaf with nothing in the way, the direction facing the rail focuses
    /// it; with no rail that is the edge.
    pub fn focus_dir_in(
        &mut self,
        dir: Dir,
        visible: &dyn Fn(Leaf) -> bool,
        rail: Option<RailPosition>,
    ) -> TreeFocusMove {
        let regions = match self.zoomed {
            Some(z) => vec![(z, UnitRect::new(0.0, 0.0, 1.0, 1.0))],
            None => self.visible_regions(visible),
        };
        let current = match self.focus {
            Focus::Leaf(l) => l,
            Focus::Rail => {
                let at = rail.unwrap_or_default();
                return if dir == at.toward() {
                    match first_at_rail(&regions, at) {
                        Some(l) => {
                            self.focus(Focus::Leaf(l));
                            TreeFocusMove::Moved(Focus::Leaf(l))
                        }
                        None => TreeFocusMove::AtEdge,
                    }
                } else if dir == at.facing() {
                    TreeFocusMove::AtEdge
                } else {
                    TreeFocusMove::NotHandled
                };
            }
        };
        let Some(mut cur) = regions
            .iter()
            .find(|(l, _)| *l == current)
            .map(|(_, rect)| *rect)
        else {
            return TreeFocusMove::NotHandled;
        };
        // Inside the editor region: another group of the shown tab first.
        let mut stacked_inside = false;
        if current == Leaf::Editor {
            let groups = self.region.regions();
            let active = self.region.active_group();
            if let Some(inner) = groups.iter().find(|(g, _)| *g == active).map(|(_, r)| *r) {
                if let Some(g) = nearest(&groups, active, inner, dir, &|_| false) {
                    self.region.set_active_group(g);
                    self.focus = Focus::Leaf(Leaf::Editor);
                    return TreeFocusMove::Moved(Focus::Leaf(Leaf::Editor));
                }
                stacked_inside = groups.iter().any(|(g, r)| {
                    *g != active && overlap(r.x(), r.right(), inner.x(), inner.right()) > EPS
                });
                cur = within(cur, inner);
            }
        }
        match nearest(&regions, current, cur, dir, &|l| self.is_remembered(l)) {
            Some(l) => {
                self.focus(Focus::Leaf(l));
                TreeFocusMove::Moved(Focus::Leaf(l))
            }
            None if rail.is_some_and(|rail| rail.facing() == dir) => {
                self.focus = Focus::Rail;
                TreeFocusMove::Moved(Focus::Rail)
            }
            None => match dir {
                Dir::Left | Dir::Right => TreeFocusMove::AtEdge,
                Dir::Up | Dir::Down => {
                    let stacked = stacked_inside
                        || regions.iter().any(|(l, c)| {
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

    /// The slots of `side` in reading order: those lying entirely left
    /// (right) of the editor region.
    pub fn side_slots(&self, side: Side) -> Vec<SlotId> {
        let Some(group) = self.editor_rect() else {
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
                Leaf::Editor => None,
            })
            .collect()
    }

    /// The slots lying entirely outside the editor region on `side`, in
    /// reading order: the window-edge counterpart of [`Self::side_slots`].
    /// With one editor leaf (ADR-0058) no slot lies between groups, so the
    /// two agree.
    pub fn edge_slots(&self, side: Side) -> Vec<SlotId> {
        self.side_slots(side)
    }

    /// The side `slot` reads as: left when its centre lies left of the
    /// editor region's, else right.
    pub fn slot_side(&self, slot: SlotId) -> Side {
        let rect = self
            .regions()
            .into_iter()
            .find(|(leaf, _)| *leaf == Leaf::Slot(slot))
            .map(|(_, rect)| rect);
        match (rect, self.editor_rect()) {
            (Some(rect), Some(group))
                if rect.x() + rect.width() / 2.0 >= group.x() + group.width() / 2.0 =>
            {
                Side::Right
            }
            _ => Side::Left,
        }
    }

    /// The slot that touches the rail's edge (ADR-0055 §7), drawn or not:
    /// for `Left` the leftmost then topmost, `Right` the rightmost then
    /// topmost, `Top` the topmost then leftmost, `Bottom` the bottommost
    /// then leftmost.
    pub fn slot_beside_rail(&self, rail: RailPosition) -> Option<SlotId> {
        let slots: Vec<_> = self
            .regions()
            .into_iter()
            .filter(|(leaf, _)| matches!(leaf, Leaf::Slot(_)))
            .collect();
        match first_at_rail(&slots, rail) {
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
            Leaf::Editor => false,
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
                Leaf::Editor => None,
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
        match nearest(&slots, from, cur, dir, &|l| self.is_remembered(l)) {
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

    /// [`Self::place_beside`] against the editor region: the new slot
    /// lands at the whole region's `dir` edge, never between its groups.
    pub fn place_beside_editor(&mut self, view: V, dir: Dir) -> SlotId {
        self.place_beside_leaf(view, Leaf::Editor, dir)
            .expect("the editor leaf is always in the tree")
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
    /// area is the editor, the nearest remaining slot in reading order on the
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
            Leaf::Editor => None,
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

    // ── Editor region ─────────────────────────────────────────────────

    /// The tabs and groups drawn in the editor leaf.
    pub fn region(&self) -> &EditorRegion {
        &self.region
    }

    /// The tabs and groups drawn in the editor leaf, to change them.
    pub fn region_mut(&mut self) -> &mut EditorRegion {
        &mut self.region
    }

    /// The group that owns save, the footer and `{note}`: the region's
    /// active group ([`EditorRegion::active_group`]).
    pub fn active_group(&self) -> GroupId {
        self.region.active_group()
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
    /// in vertical splits), never a subtree holding the editor leaf. Units
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
            // The editor is always drawn, so the root never empties.
            if *leaf != Leaf::Editor && !drawn(*leaf) {
                root.remove_leaf(leaf);
            }
        }
        root.layout_fit(area, &|leaf: &Leaf| min(*leaf), &|leaf: &Leaf| {
            *leaf == Leaf::Editor
        })
    }

    /// Change weights so [`Self::fit`] (same `area`, `min` and `drawn`)
    /// collapses none of `leaves` (ADR-0054 §8): the cell holding a split
    /// that collapses one grows along that split's axis, in the nearest
    /// split above it along that axis, taking from its siblings only what
    /// each has above its minimum, so none collapses there and the editor
    /// leaf does not go under its minimum. Repeated, a bounded number of times,
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
        let group = |leaf: &Leaf| *leaf == Leaf::Editor;
        self.root.make_room(
            leaves,
            area,
            &|leaf: &Leaf| min(*leaf),
            &group,
            &|leaf: &Leaf| group(leaf) || drawn(*leaf),
        )
    }
}

#[cfg(test)]
mod tests;
