//! Drawing the **layout tree** (ADR-0052): everything right of the rail is
//! one recursive walk over the tree's [shape]. A split is a flex row
//! (horizontal) or column (vertical) whose children grow by their weights;
//! a slot shows its view, an editor group the editor Panel
//! ([`Workspace::render_editor`]).
//!
//! A slot that is not drawn ([`Workspace::slot_drawn`]: no shown view, not
//! opened empty) takes no space: it is left out, its siblings share its
//! weight, and a split left with no drawn child is left out too.
//!
//! **Minimum sizes** (ADR-0052 §6). A slot's minimum is that of the view it
//! shows (`view_registry`, 1 unit = 1 rem; an empty slot is
//! [`DRAWER_MIN_REMS`] wide), an editor group's [`EDITOR_MIN_REMS`] wide;
//! heights count only in vertical splits. Each frame [`Workspace::fit`]
//! collapses what does not fit: in the first overflowing split, children
//! last to first, never a subtree holding an editor group. A collapsed leaf
//! is left out like an undrawn slot, takes no focus, and comes back when
//! the window grows; the tree, its weights and the saved layout never
//! change. Where the minimums fit, every cell keeps its own; a slot that
//! still ends up under its view's minimum (inside the editor's subtree,
//! which cannot collapse) shows [`TOO_SMALL`] under its header.
//!
//! **Dividers.** One pixel of window background separates drawn
//! neighbours. Where the two are neighbours in the tree too, that pixel is
//! a divider: a [`resize_handle`] straddling it, whose drag moves only that
//! boundary ([`Workspace::on_divider_drag`], the tree's
//! [`resize_after`]) and is saved when it ends. The handles sit in an
//! empty copy of the split's cells and gaps laid over it, so they paint
//! above both neighbours. A drag or step stops at either cell's drawn
//! minimum (Ruling 16). A drawn cell and an
//! undrawn slot have no divider (Ruling 12), and neither do two drawn cells
//! with an undrawn slot between them in the tree: `resize_after` moves the
//! boundary between tree siblings, which there is not the one drawn. The
//! leader's grow and shrink steps move a divider of the focused leaf by
//! [`RESIZE_STEP`] ([`Workspace::step_focused_leaf`]).
//!
//! [shape]: kimun_notes::logic::layout_tree::LayoutTree::shape
//! [`resize_after`]: kimun_notes::logic::layout_tree::LayoutTree::resize_after

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui_kit::base::{
    ResizeHandleContext, ResizeHandleRenderer, ResizeHandleState, TestSupportExt, resize_handle,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Bounds, Context, DragMoveEvent, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, Pixels, Point, Render, Rgba, Role, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px, rems,
};
use kimun_notes::logic::layout_tree::{Axis, Fit, Focus, Leaf, SlotId, TreeShape, UnitRect};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::view_registry;

use super::{Workspace, empty_slot_id, route, slot_id};
use crate::shell_layout::{DRAWER_MIN_REMS, EDITOR_MIN_REMS, RAIL_REMS};
use crate::theme;

/// What one leader grow or shrink step moves a divider by: this share of
/// the two cells it sits between (5%), so a step feels the same in a
/// narrow pair and a wide one.
pub(super) const RESIZE_STEP: f64 = 0.05;

/// What a slot shows in place of its view while smaller than the view's
/// minimum.
pub(crate) const TOO_SMALL: &str = "Too small";
/// The element id of that placeholder.
pub(crate) const TOO_SMALL_ID: &str = "slot-too-small";

/// The window background between two drawn neighbours, in px.
const GAP: f64 = 1.0;

/// Share `len` px among cells of (weight, minimum px) the way flex does
/// with those minimums set: by weight, a cell that would fall under its
/// minimum held there and the rest shared again. When the minimums alone
/// exceed `len` (only cells holding an editor group are left then) they
/// are not set and the cells go by weight. The sizes, and whether the
/// minimums hold.
fn distribute(len: f64, cells: &[(f64, f64)]) -> (Vec<f64>, bool) {
    let total: f64 = cells.iter().map(|(weight, _)| weight).sum();
    let need: f64 = cells.iter().map(|(_, min)| min).sum();
    if need > len + 0.01 {
        let sizes = cells.iter().map(|(weight, _)| len * weight / total);
        return (sizes.collect(), false);
    }
    let mut held: Vec<Option<f64>> = vec![None; cells.len()];
    loop {
        let free = len - held.iter().flatten().sum::<f64>();
        let weight: f64 = cells
            .iter()
            .zip(&held)
            .filter(|(_, held)| held.is_none())
            .map(|((weight, _), _)| weight)
            .sum();
        let mut more = false;
        for (i, (w, min)) in cells.iter().enumerate() {
            if held[i].is_none() && free * w / weight < min - 1e-9 {
                held[i] = Some(*min);
                more = true;
            }
        }
        if !more {
            let sizes = cells
                .iter()
                .zip(held)
                .map(|((w, _), held)| held.unwrap_or_else(|| free * w / weight));
            return (sizes.collect(), true);
        }
    }
}

/// The two cells a divider sits between, as last drawn.
#[derive(Debug, Clone, Copy)]
pub(super) struct DividerCell {
    axis: Axis,
    before: Bounds<Pixels>,
    after: Bounds<Pixels>,
    /// The cells' drawn minimums along `axis`, in rems (zero where none).
    min_before: f64,
    min_after: f64,
}

impl DividerCell {
    /// The share of the pair that puts the boundary `size` px after the
    /// start of the cell before, kept so both cells stay at or above their
    /// drawn minimums (Ruling 16; a cell already below its floor does not
    /// shrink further). `None` when that leaves the boundary where it is.
    fn share_at(&self, size: f64, rem: f64) -> Option<f64> {
        let (_, first) = extent(self.axis, self.before);
        let (_, second) = extent(self.axis, self.after);
        let pair = first + second;
        if pair <= 0.0 {
            return None;
        }
        let low = (self.min_before * rem).min(first);
        let high = (pair - self.min_after * rem).max(first);
        let size = size.clamp(low, high);
        ((size - first).abs() > 0.01).then_some(size / pair)
    }

    fn start(&self) -> f64 {
        extent(self.axis, self.before).0
    }

    fn first(&self) -> f64 {
        extent(self.axis, self.before).1
    }

    fn pair(&self) -> f64 {
        self.first() + extent(self.axis, self.after).1
    }
}

/// Each divider's cells, keyed by the divider's leaf (the last leaf of the
/// cell before it), written as the tree is prepainted.
pub(super) type DividerCells = Rc<RefCell<HashMap<Leaf, DividerCell>>>;

/// The payload of a divider drag: the leaf whose following boundary it
/// moves ([`LayoutTree::resize_after`]).
///
/// [`LayoutTree::resize_after`]: kimun_notes::logic::layout_tree::LayoutTree::resize_after
#[derive(Debug, Clone, Copy)]
pub(crate) struct DividerDrag(Leaf);

/// What a divider drag shows under the pointer: nothing, the cells
/// themselves follow.
struct DragGhost;

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The last leaf of `shape` in reading order: in every split under it, it
/// is in the last child, so [`LayoutTree::resize_after`] on it moves the
/// boundary after `shape` itself.
///
/// [`LayoutTree::resize_after`]: kimun_notes::logic::layout_tree::LayoutTree::resize_after
fn last_leaf(shape: &TreeShape) -> Leaf {
    match shape {
        TreeShape::Leaf(leaf) => *leaf,
        TreeShape::Split { children, .. } => last_leaf(
            &children
                .last()
                .expect("a split holds at least two children")
                .1,
        ),
    }
}

fn contains(shape: &TreeShape, leaf: Leaf) -> bool {
    match shape {
        TreeShape::Leaf(l) => *l == leaf,
        TreeShape::Split { children, .. } => children.iter().any(|(_, c)| contains(c, leaf)),
    }
}

/// `point` along `axis`, and where `bounds` starts and how long it is
/// along it.
fn along(axis: Axis, point: Point<Pixels>) -> f64 {
    f64::from(match axis {
        Axis::Horizontal => point.x,
        Axis::Vertical => point.y,
    })
}

fn extent(axis: Axis, bounds: Bounds<Pixels>) -> (f64, f64) {
    match axis {
        Axis::Horizontal => (f64::from(bounds.left()), f64::from(bounds.size.width)),
        Axis::Vertical => (f64::from(bounds.top()), f64::from(bounds.size.height)),
    }
}

/// A divider's look: nothing at rest (the pixel of window background
/// between the cells is the seam), the focus colour along it while
/// hovered, pressed or dragged.
fn seam(axis: gpui_kit::Axis, active: Rgba) -> ResizeHandleRenderer {
    Rc::new(move |handle: &ResizeHandleContext, _, _| {
        let bar = div().flex_none();
        let bar = if handle.state() == ResizeHandleState::Idle {
            bar
        } else {
            bar.bg(active)
        };
        Some(
            match axis {
                gpui_kit::Axis::Vertical => bar.w_full().h(px(1.)),
                _ => bar.h_full().w(px(1.)),
            }
            .into_any_element(),
        )
    })
}

impl Workspace {
    /// The minimum (width, height) of `slot`, in rems: the shown view's
    /// (`view_registry`), [`DRAWER_MIN_REMS`] wide for a slot opened empty.
    /// Hidden views do not count.
    pub(crate) fn slot_min(&self, slot: SlotId) -> (f64, f64) {
        match self.layout.shown(slot) {
            Some(view) => view_registry::spec(view).min,
            None => (f64::from(DRAWER_MIN_REMS), 0.0),
        }
    }

    /// The minimum (width, height) of `leaf`, in rems.
    fn leaf_min(&self, leaf: Leaf) -> (f64, f64) {
        match leaf {
            Leaf::Group(_) => (f64::from(EDITOR_MIN_REMS), 0.0),
            Leaf::Slot(slot) => self.slot_min(slot),
        }
    }

    /// The px the tree has: the window's width less the rail, its height
    /// less the title bar (as last drawn).
    fn tree_area(&self, window: &Window) -> (f64, f64) {
        let viewport = window.viewport_size();
        let rem = f64::from(window.rem_size());
        (
            (f64::from(viewport.width) - f64::from(RAIL_REMS) * rem).max(0.0),
            (f64::from(viewport.height) - self.title_bar_height.get()).max(0.0),
        )
    }

    /// Which leaves the window has room for (ADR-0052 §6, Ruling 12):
    /// slots that are not drawn are left out, then what overflows
    /// collapses ([`LayoutTree::fit`], in rems; editor groups never).
    /// Each leaf's minimum carries one px for the gap beside it.
    ///
    /// [`LayoutTree::fit`]: kimun_notes::logic::layout_tree::LayoutTree::fit
    pub(crate) fn fit(&self, window: &Window) -> Fit<Leaf> {
        let rem = f64::from(window.rem_size());
        let (width, height) = self.tree_area(window);
        let gap = GAP / rem;
        self.layout.fit(
            UnitRect::new(0.0, 0.0, width / rem, height / rem),
            &|leaf| {
                let (w, h) = self.leaf_min(leaf);
                (w + gap, h + gap)
            },
            &|leaf| self.leaf_drawn(leaf),
        )
    }

    /// Fit the tree to the window for this frame: remember what collapsed
    /// (session only), and move the keyboard off a leaf that did.
    pub(super) fn refit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.collapsed = self
            .fit(window)
            .collapsed
            .into_iter()
            .collect::<HashSet<_>>();
        if let Focus::Leaf(leaf) = self.layout.focused()
            && self.collapsed.contains(&leaf)
        {
            self.fix_focus();
            cx.defer_in(window, |this, window, cx| this.apply_focus(window, cx));
        }
    }

    /// Whether `leaf` is on screen: drawn and not collapsed.
    pub(super) fn leaf_visible(&self, leaf: Leaf) -> bool {
        self.leaf_drawn(leaf) && !self.collapsed.contains(&leaf)
    }

    /// The minimum (width, height) of what is visible in `shape`, in px,
    /// gaps included.
    fn min_px(&self, shape: &TreeShape, rem: f64) -> (f64, f64) {
        match shape {
            TreeShape::Leaf(leaf) => {
                let (w, h) = self.leaf_min(*leaf);
                (w * rem, h * rem)
            }
            TreeShape::Split { axis, children } => {
                let mins: Vec<(f64, f64)> = children
                    .iter()
                    .filter(|(_, c)| self.shape_visible(c))
                    .map(|(_, c)| self.min_px(c, rem))
                    .collect();
                let gaps = mins.len().saturating_sub(1) as f64 * GAP;
                match axis {
                    Axis::Horizontal => mins
                        .iter()
                        .fold((gaps, 0.0), |(w, h), (cw, ch)| (w + cw, h.max(*ch))),
                    Axis::Vertical => mins
                        .iter()
                        .fold((0.0, gaps), |(w, h), (cw, ch)| (w.max(*cw), h + ch)),
                }
            }
        }
    }

    /// The layout tree, right of the rail: it takes the width the rail
    /// leaves and the height under the title bar.
    pub(super) fn render_tree(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let shape = self.layout.shape();
        // Measured afresh as this tree is laid out.
        self.divider_cells.borrow_mut().clear();
        let rem = f64::from(window.rem_size());
        let tree = self
            .render_node(&shape, self.tree_area(window), (false, false), rem, cx)
            // Every tree holds an editor group, which is always drawn; a
            // root left empty would be a model bug, so draw the editor.
            .unwrap_or_else(|| self.render_editor(cx).into_any_element());
        div()
            .id("layout-tree")
            .flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(tree)
            .into_any_element()
    }

    /// `shape`, drawn in `size` px, or `None` when nothing in it is
    /// visible. `across` says whether a horizontal (vertical) split holds
    /// it: only along those does a slot check its view's minimum.
    fn render_node(
        &self,
        shape: &TreeShape,
        size: (f64, f64),
        across: (bool, bool),
        rem: f64,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        match shape {
            TreeShape::Leaf(Leaf::Group(_)) => Some(self.render_editor(cx).into_any_element()),
            TreeShape::Leaf(Leaf::Slot(slot)) => {
                if !self.leaf_visible(Leaf::Slot(*slot)) {
                    return None;
                }
                Some(match self.layout.shown(*slot) {
                    Some(view) => {
                        let (w, h) = view_registry::spec(view).min;
                        let small = (across.0 && size.0 < w * rem - 0.5)
                            || (across.1 && size.1 < h * rem - 0.5);
                        self.render_slot(*slot, view, small, cx)
                    }
                    None => self.render_empty_slot(*slot, cx),
                })
            }
            TreeShape::Split { axis, children } => {
                let axis = *axis;
                let visible: Vec<(usize, f64, &TreeShape)> = children
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, child))| self.shape_visible(child))
                    .map(|(index, (weight, child))| (index, *weight, child))
                    .collect();
                if visible.is_empty() {
                    return None;
                }
                let (length, cross) = match axis {
                    Axis::Horizontal => size,
                    Axis::Vertical => (size.1, size.0),
                };
                let mins: Vec<f64> = visible
                    .iter()
                    .map(|(_, _, child)| {
                        let (w, h) = self.min_px(child, rem);
                        if axis == Axis::Horizontal { w } else { h }
                    })
                    .collect();
                let gaps = (visible.len() - 1) as f64 * GAP;
                let cells: Vec<(f64, f64)> = visible
                    .iter()
                    .zip(&mins)
                    .map(|((_, weight, _), min)| (*weight, *min))
                    .collect();
                let (sizes, held) = distribute((length - gaps).max(0.0), &cells);
                let across = match axis {
                    Axis::Horizontal => (true, across.1),
                    Axis::Vertical => (across.0, true),
                };
                let drawn: Vec<(usize, f64, Option<f64>, Leaf, AnyElement)> = visible
                    .iter()
                    .zip(sizes.iter().zip(&mins))
                    .filter_map(|((index, weight, child), (along, min))| {
                        let size = match axis {
                            Axis::Horizontal => (*along, cross),
                            Axis::Vertical => (cross, *along),
                        };
                        let element = self.render_node(child, size, across, rem, cx)?;
                        // Minimums are set only where they all fit.
                        let min = held.then_some(*min);
                        Some((*index, *weight, min, last_leaf(child), element))
                    })
                    .collect();
                // The visible children share the split by their weights:
                // flex grow factors summing below one would leave space
                // unused.
                let total: f64 = drawn.iter().map(|(_, weight, ..)| weight).sum();
                let cell = |weight: f64, min: Option<f64>| {
                    div()
                        .flex()
                        .flex_basis(rems(0.))
                        .flex_grow((weight / total) as f32)
                        .min_w_0()
                        .min_h_0()
                        .when_some(min, |cell, min| match axis {
                            Axis::Horizontal => cell.min_w(px(min as f32)),
                            Axis::Vertical => cell.min_h(px(min as f32)),
                        })
                };
                let mut elements: Vec<AnyElement> = Vec::new();
                // The same cells and gaps, empty, over the split: the
                // handles live in its gaps, so they paint (and take the
                // pointer) above both neighbours, their whole band grabbable.
                let mut handles: Vec<AnyElement> = Vec::new();
                // (element before, element after, divider leaf, minimums)
                // per divider.
                let mut pairs: Vec<(usize, usize, Leaf, f64, f64)> = Vec::new();
                let mut previous: Option<(usize, usize, Leaf, f64)> = None;
                let mut any_divider = false;
                for (index, weight, min, last, element) in drawn {
                    if let Some((prev_index, prev_element, prev_last, prev_min)) = previous {
                        let adjacent = prev_index + 1 == index;
                        elements.push(self.render_gap(axis, None, cx));
                        handles.push(self.render_gap(axis, adjacent.then_some(prev_last), cx));
                        if adjacent {
                            any_divider = true;
                            pairs.push((
                                prev_element,
                                elements.len(),
                                prev_last,
                                prev_min,
                                min.unwrap_or(0.0) / rem,
                            ));
                        }
                    }
                    previous = Some((index, elements.len(), last, min.unwrap_or(0.0) / rem));
                    elements.push(cell(weight, min).child(element).into_any_element());
                    handles.push(cell(weight, min).into_any_element());
                }
                let direction = |row: gpui_kit::Div| match axis {
                    Axis::Horizontal => row.flex_row(),
                    Axis::Vertical => row.flex_col(),
                };
                if any_divider {
                    elements.push(
                        direction(div().absolute().top_0().left_0().size_full().flex())
                            .children(handles)
                            .into_any_element(),
                    );
                }
                let cells = self.divider_cells.clone();
                Some(
                    div()
                        .on_children_prepainted(move |bounds, _, _| {
                            let mut cells = cells.borrow_mut();
                            for (before, after, leaf, min_before, min_after) in &pairs {
                                if let (Some(b), Some(a)) =
                                    (bounds.get(*before), bounds.get(*after))
                                {
                                    cells.insert(
                                        *leaf,
                                        DividerCell {
                                            axis,
                                            before: *b,
                                            after: *a,
                                            min_before: *min_before,
                                            min_after: *min_after,
                                        },
                                    );
                                }
                            }
                        })
                        .relative()
                        .flex()
                        .map(|row| match axis {
                            Axis::Horizontal => row.flex_row(),
                            Axis::Vertical => row.flex_col(),
                        })
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .children(elements)
                        .into_any_element(),
                )
            }
        }
    }

    /// The pixel of window background between two drawn neighbours, so
    /// touching cells never merge; with `divider` (the leaf before it,
    /// when the two are tree siblings), a resize handle straddling it.
    fn render_gap(&self, axis: Axis, divider: Option<Leaf>, cx: &mut Context<Self>) -> AnyElement {
        let gap = div().flex_none().relative().map(|gap| match axis {
            Axis::Horizontal => gap.w(px(1.)).h_full(),
            Axis::Vertical => gap.h(px(1.)).w_full(),
        });
        let Some(leaf) = divider else {
            return gap.into_any_element();
        };
        // The kit's axis is the one the handle resizes along.
        let handle_axis = match axis {
            Axis::Horizontal => gpui_kit::Axis::Horizontal,
            Axis::Vertical => gpui_kit::Axis::Vertical,
        };
        gap.child(
            resize_handle::<DividerDrag, DragGhost>(self.divider_id(leaf), handle_axis)
                .with_appearance(seam(handle_axis, theme::focus_border(&theme::current(cx))))
                .on_drag(DividerDrag(leaf), |_, _, _, cx| cx.new(|_| DragGhost))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, _| {
                        this.grab_divider(leaf, event.position)
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.end_resize(cx)),
                ),
        )
        .into_any_element()
    }

    /// The element id of the divider after `leaf`: `divider-slot-<n>` or
    /// `divider-group-<n>`, by the leaf's place among the tree's slots or
    /// groups in reading order.
    fn divider_id(&self, leaf: Leaf) -> SharedString {
        match leaf {
            Leaf::Slot(slot) => {
                let n = self.layout.slots().iter().position(|s| *s == slot);
                format!("divider-slot-{}", n.unwrap_or_default()).into()
            }
            Leaf::Group(group) => {
                let n = self.layout.groups().iter().position(|g| *g == group);
                format!("divider-group-{}", n.unwrap_or_default()).into()
            }
        }
    }

    /// A press on the divider after `leaf` at `at`: remember how far from
    /// the boundary it landed.
    fn grab_divider(&mut self, leaf: Leaf, at: Point<Pixels>) {
        self.press_offset = match self.divider_cells.borrow().get(&leaf) {
            Some(cell) => along(cell.axis, at) - (cell.start() + cell.first()),
            None => 0.0,
        };
    }

    /// A divider drag moved: the boundary follows the pointer.
    pub(super) fn on_divider_move(
        &mut self,
        event: &DragMoveEvent<DividerDrag>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DividerDrag(leaf) = *event.drag(cx);
        let Some(axis) = self.divider_cells.borrow().get(&leaf).map(|c| c.axis) else {
            return;
        };
        let pixel = along(axis, event.event.position) - self.press_offset;
        self.on_divider_drag(leaf, pixel, window, cx);
    }

    /// The divider after `leaf` dragged to `pixel` (window px along its
    /// axis): the cell before it takes the share of the two cells' combined
    /// drawn extent that ends there ([`LayoutTree::resize_after`]), held so
    /// neither cell goes under its drawn minimum (Ruling 16: past a floor
    /// flex would take the difference from a third cell). Only those two
    /// cells' weights change. Saved when the drag ends
    /// ([`Self::end_resize`]), not on every move.
    ///
    /// [`LayoutTree::resize_after`]: kimun_notes::logic::layout_tree::LayoutTree::resize_after
    pub(crate) fn on_divider_drag(
        &mut self,
        leaf: Leaf,
        pixel: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(cell) = self.divider_cells.borrow().get(&leaf).copied() else {
            return;
        };
        let rem = f64::from(window.rem_size());
        let Some(share) = cell.share_at(pixel - cell.start(), rem) else {
            return;
        };
        if self.layout.resize_after(leaf, share) {
            self.resizing = true;
            cx.notify();
        }
    }

    /// The pointer came up (in the window or outside it): a divider drag in
    /// progress ends and the layout is saved. Idempotent.
    pub(super) fn end_resize(&mut self, cx: &mut Context<Self>) {
        self.press_offset = 0.0;
        if std::mem::take(&mut self.resizing) {
            self.save_layout(cx);
            cx.notify();
        }
    }

    /// Leader grow (`grow`) or shrink: move a divider of the focused leaf
    /// by [`RESIZE_STEP`] of its pair's drawn extent, held at both cells'
    /// drawn minimums like a drag, and save. The divider is the one after
    /// the leaf's branch in the innermost split where that neighbour is
    /// drawn, else the one before it. Nothing with the rail focused, no
    /// divider beside the leaf, or the boundary already at a floor (no
    /// save either).
    pub(super) fn step_focused_leaf(
        &mut self,
        grow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Focus::Leaf(leaf) = self.layout.focused() else {
            return;
        };
        let Some((divider, leaf_before)) = self.divider_beside(&self.layout.shape(), leaf) else {
            return;
        };
        let Some(cell) = self.divider_cells.borrow().get(&divider).copied() else {
            return;
        };
        let step = if grow == leaf_before {
            RESIZE_STEP
        } else {
            -RESIZE_STEP
        };
        let rem = f64::from(window.rem_size());
        let Some(share) = cell.share_at(cell.first() + step * cell.pair(), rem) else {
            return;
        };
        if self.layout.resize_after(divider, share) {
            self.save_layout(cx);
            cx.notify();
        }
    }

    /// The divider beside `leaf` in `shape` (see
    /// [`Self::step_focused_leaf`]): its leaf, and whether `leaf` is in the
    /// cell before it.
    fn divider_beside(&self, shape: &TreeShape, leaf: Leaf) -> Option<(Leaf, bool)> {
        let TreeShape::Split { children, .. } = shape else {
            return None;
        };
        let i = children.iter().position(|(_, c)| contains(c, leaf))?;
        if let Some(found) = self.divider_beside(&children[i].1, leaf) {
            return Some(found);
        }
        if i + 1 < children.len() && self.shape_visible(&children[i + 1].1) {
            return Some((last_leaf(&children[i].1), true));
        }
        if i > 0 && self.shape_visible(&children[i - 1].1) {
            return Some((last_leaf(&children[i - 1].1), false));
        }
        None
    }

    /// Whether anything in `shape` is visible (drawn and not collapsed).
    fn shape_visible(&self, shape: &TreeShape) -> bool {
        match shape {
            TreeShape::Leaf(leaf) => self.leaf_visible(*leaf),
            TreeShape::Split { children, .. } => {
                children.iter().any(|(_, c)| self.shape_visible(c))
            }
        }
    }

    /// `slot`, showing `view`: its title over the view (FILES, or, for a
    /// view that needs a workspace while none is open, a line saying so;
    /// [`TOO_SMALL`] when `small`: under the view's minimum).
    fn render_slot(
        &self,
        slot: SlotId,
        view: DrawerView,
        small: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let spec = view_registry::spec(view);
        div()
            .id(slot_id(view))
            .test_support()
            .track_focus(&self.slot_handle(slot))
            .role(Role::Group)
            .aria_label(spec.name)
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(theme::panel_background(&theme::current(cx)))
            .capture_any_mouse_down(cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                this.focus_slot(slot, window, cx)
            }))
            .child(self.render_slot_header(slot, Some(view), cx))
            .map(|el| match self.view_content(view).filter(|_| !small) {
                Some(content) => el.child(content),
                None if small => el.child(
                    div()
                        .id(TOO_SMALL_ID)
                        .test_support()
                        .role(Role::Label)
                        .aria_label(TOO_SMALL)
                        .px_3()
                        .text_sm()
                        .text_color(theme::muted(&theme::current(cx)))
                        .child(TOO_SMALL),
                ),
                // Every view is built; one without content needs a vault.
                None => el.child(
                    div()
                        .px_3()
                        .text_sm()
                        .text_color(theme::muted(&theme::current(cx)))
                        .child(route::NO_WORKSPACE),
                ),
            })
            .children(self.focus_bar(Focus::Leaf(Leaf::Slot(slot)), cx))
            .into_any_element()
    }

    /// The strip at the top of a drawn slot: the title of the view it shows
    /// (`Empty` for a slot opened with nothing in it) on the header
    /// background, a hairline under it, and the accent on the title while
    /// the slot has the focus. No actions yet: the hook for a slot menu.
    fn render_slot_header(
        &self,
        slot: SlotId,
        view: Option<DrawerView>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let focused = self.layout.focused() == Focus::Leaf(Leaf::Slot(slot));
        let title: SharedString = match view {
            Some(view) => view_registry::spec(view).title.into(),
            None => "Empty".into(),
        };
        let theme = theme::current(cx);
        div()
            .id(if focused {
                "slot-header-focused"
            } else {
                "slot-header"
            })
            .test_support()
            .role(Role::Group)
            .aria_label(title.clone())
            .w_full()
            .flex_none()
            // A slot narrower than its title clips it to one line.
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .px_3()
            .py_1()
            .text_sm()
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .bg(theme::slot_header_background(&theme))
            .border_b_1()
            .border_color(theme::divider(&theme))
            .text_color(if focused {
                theme::focus_border(&theme)
            } else {
                theme::foreground(&theme)
            })
            .child(title)
    }

    /// What `view`'s slot shows, for the views built so far.
    pub(super) fn view_content(&self, view: DrawerView) -> Option<AnyElement> {
        match view {
            DrawerView::Files => self.files.clone().map(|v| v.into_any_element()),
            DrawerView::Links => self.links.clone().map(|v| v.into_any_element()),
            DrawerView::Outline => self.outline.clone().map(|v| v.into_any_element()),
            DrawerView::Find => self.find.clone().map(|v| v.into_any_element()),
            DrawerView::Tags => self.tags.clone().map(|v| v.into_any_element()),
            DrawerView::Semantic => self.semantic.clone().map(|v| v.into_any_element()),
            DrawerView::Ask => self.ask_sources.clone().map(|v| v.into_any_element()),
            DrawerView::Config => Some(self.config.clone().into_any_element()),
        }
    }

    /// An empty slot that is drawn (opened empty this session): a quiet
    /// hint, not a target. A view gets here only by a move or the rail; the
    /// hint names the move chords the keymap binds.
    fn render_empty_slot(&self, slot: SlotId, cx: &mut Context<Self>) -> AnyElement {
        let hint: SharedString = self.chord_labels.empty_slot_hint.clone();
        let index = self
            .layout
            .slots()
            .iter()
            .position(|s| *s == slot)
            .unwrap_or_default();
        div()
            .id(empty_slot_id(index))
            .test_support()
            .track_focus(&self.slot_handle(slot))
            .role(Role::Group)
            .aria_label(hint.clone())
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .items_center()
            .justify_center()
            .min_w_0()
            .min_h_0()
            .bg(theme::panel_background(&theme::current(cx)))
            .capture_any_mouse_down(cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                this.focus_slot(slot, window, cx)
            }))
            .child(self.render_slot_header(slot, None, cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .px_4()
                    .text_sm()
                    .text_center()
                    .text_color(theme::muted(&theme::current(cx)))
                    .child(hint),
            )
            .children(self.focus_bar(Focus::Leaf(Leaf::Slot(slot)), cx))
            .into_any_element()
    }
}
