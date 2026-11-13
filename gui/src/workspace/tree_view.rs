//! Drawing the **layout tree** (ADR-0052): everything right of the rail is
//! one recursive walk over the tree's [shape]. A split is a flex row
//! (horizontal) or column (vertical) whose children grow by their weights;
//! a slot shows its view, the editor region the editor Panel
//! ([`Workspace::render_editor`]).
//!
//! A slot that is not drawn ([`Workspace::slot_drawn`]: no shown view)
//! takes no space: it is left out, its siblings share its
//! weight, and a split left with no drawn child is left out too.
//!
//! **Minimum sizes** (ADR-0052 §6). A slot's minimum is that of the view it
//! shows (`view_registry`, 1 unit = 1 rem), the editor region
//! [`EDITOR_MIN_REMS`] wide;
//! heights count only in vertical splits. Each frame [`Workspace::fit`]
//! collapses what does not fit: in the first overflowing split, children
//! last to first, never a subtree holding the editor region. A collapsed leaf
//! is left out like an undrawn slot, takes no focus, and comes back when
//! the window grows; the tree, its weights and the saved layout never
//! change. Where the minimums fit, every cell keeps its own; a slot that
//! still ends up under its view's minimum (inside the editor's subtree,
//! which cannot collapse) shows [`TOO_SMALL`] under its header. Only a
//! placement that makes a slot changes weights for room, once, so the new
//! slot is not collapsed ([`Workspace::make_room`], ADR-0054 §8).
//!
//! **Dividers.** One pixel of window background separates drawn
//! neighbours, and between two drawn children of one split that pixel is a
//! divider: a [`resize_handle`] straddling it, whose drag moves only that
//! boundary ([`Workspace::on_divider_drag`], the tree's
//! [`resize_after_visible`]) and is saved when it ends. Undrawn or
//! collapsed siblings between the two are skipped and keep their weights
//! (Ruling 23); an undrawn slot itself has no divider (Ruling 12). The
//! handles sit in an empty copy of the split's cells and gaps laid over it,
//! so they paint above both neighbours. A drag or step stops at either
//! cell's drawn minimum (Ruling 16). The leader's grow and shrink steps
//! move a divider of the focused leaf by [`RESIZE_STEP`]
//! ([`Workspace::step_focused_leaf`]).
//!
//! **Zoom** (ADR-0053). While a leaf is zoomed only it is drawn, filling
//! the whole area with the Footer under it (the editor region's own), and
//! only it counts as visible: no fit, nothing collapsed, no divider. Zoom
//! ends when its leaf is no longer drawn or the keyboard goes to another
//! leaf ([`Workspace::settle_zoom`]); the tree never changes, so leaving
//! zoom draws the arrangement as it was.
//!
//! [shape]: kimun_notes::logic::layout_tree::LayoutTree::shape
//! [`resize_after_visible`]: kimun_notes::logic::layout_tree::LayoutTree::resize_after_visible

/// Why leader grow / shrink does nothing: the keyboard is on the rail.
pub(crate) const FOCUS_TO_RESIZE: &str = "Focus a view or the editor to resize it";
/// ... zoom shows one leaf, with no divider.
pub(crate) const ZOOMED_NO_RESIZE: &str = "Leave zoom to resize";
/// ... the focused leaf has no divider beside it on screen.
pub(crate) const NOTHING_TO_RESIZE: &str = "Nothing to resize here";
/// ... the divider is already at both sides' minimum.
pub(crate) const AT_MINIMUM_SIZE: &str = "Already at the minimum size";

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui_kit::base::{
    ResizeHandleContext, ResizeHandleRenderer, ResizeHandleState, TestSupportExt, resize_handle,
};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Bounds, ClickEvent, Context, DragMoveEvent, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Point, Render, Rgba, Role,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px, rems,
};
use kimun_notes::logic::editor_region::GroupShape;
use kimun_notes::logic::layout_tree::{
    Axis, Fit, Focus, GroupId, LayoutTree, Leaf, SlotId, TreeShape, UnitRect,
};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::view_registry;

use super::drop_zones::{DragFrom, ViewDrag};
use super::group_header::{GROUP_ID, group_element};
use super::{Workspace, route, slot_id};
use crate::shell_layout::{EDITOR_MIN_REMS, RAIL_REMS, SLOT_MIN_REMS};
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

/// The slot menu's item that opens the move chooser.
pub(crate) const MOVE_VIEW_ITEM: &str = "Move view…";

/// The slot menu's item that closes the slot.
pub(crate) const CLOSE_SLOT_ITEM: &str = "Close slot";

/// The element id of the `⋯` in the header of the slot showing `view`
/// (`slot-move-files`).
pub(crate) fn move_button_id(view: DrawerView) -> SharedString {
    format!("slot-move-{}", view.id()).into()
}

/// The window background between two drawn neighbours, in px.
const GAP: f64 = 1.0;

/// The element id of the divider after the group at `n` in the shown tab.
pub(crate) fn group_divider_id(n: usize) -> SharedString {
    format!("group-divider-{n}").into()
}

/// Window background around each leaf's border.
pub(super) const LEAF_MARGIN: Pixels = px(4.);

/// Space between a slot header (or the border) and the view under it.
const CONTENT_PADDING: Pixels = px(8.);
/// The hairline border of a leaf's surface (`border_1`).
pub(super) const LEAF_BORDER: Pixels = px(1.);
/// The corner radius of a leaf's surface, in rems (`rounded_md`).
pub(super) const LEAF_RADIUS_REMS: f64 = 0.375;

/// Width a view gets inside a leaf cell `cell` px wide: the cell less the
/// margin, the border and the content padding on both sides.
fn view_width(cell: f64) -> f64 {
    let inset = f64::from(f32::from(LEAF_MARGIN + LEAF_BORDER + CONTENT_PADDING));
    cell - 2.0 * inset
}

/// What a slot tells the view it holds about its own place: the facts a
/// view cannot see from inside and may lay itself out by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct SlotContext {
    /// The slot's leaf is the zoomed one.
    pub zoomed: bool,
    /// Width the view gets, in px (the cell less the leaf's insets).
    pub width: f64,
    /// One rem, in px.
    pub rem: f64,
}

/// A view that lays itself out by its slot ([`SlotContext`]). The workspace
/// pushes the context each render; an implementation redraws only on a
/// change. A view opts in here and in [`Workspace::push_slot_context`].
pub(super) trait SlotAware: Sized {
    fn set_slot(&mut self, slot: SlotContext, cx: &mut Context<Self>);
}

/// Share `len` px among cells of (weight, minimum px) the way flex does
/// with those minimums set: by weight, a cell that would fall under its
/// minimum held there and the rest shared again. When the minimums alone
/// exceed `len` (only cells holding the editor region are left then) they
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

/// Where a divider sits: after a leaf of the layout tree (the last leaf of
/// the cell before it), or between two groups of the shown tab, after a
/// group (the last group of the cell before it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DividerAt {
    Leaf(Leaf),
    Group(GroupId),
}

impl From<Leaf> for DividerAt {
    fn from(leaf: Leaf) -> Self {
        DividerAt::Leaf(leaf)
    }
}

/// Each divider's cells, keyed by where it sits, written as the tree is
/// prepainted.
pub(super) type DividerCells = Rc<RefCell<HashMap<DividerAt, DividerCell>>>;

/// The payload of a divider drag: the leaf or group whose following
/// boundary it moves ([`LayoutTree::resize_after_visible`],
/// [`EditorRegion::resize_after`]).
///
/// [`LayoutTree::resize_after_visible`]: kimun_notes::logic::layout_tree::LayoutTree::resize_after_visible
/// [`EditorRegion::resize_after`]: kimun_notes::logic::editor_region::EditorRegion::resize_after
#[derive(Debug, Clone, Copy)]
pub(crate) struct DividerDrag(DividerAt);

/// What a divider drag shows under the pointer: nothing, the cells
/// themselves follow.
struct DragGhost;

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The last leaf of `shape` in reading order: in every split under it, it
/// is in the last child, so [`LayoutTree::resize_after_visible`] on it moves the
/// boundary after `shape` itself.
///
/// [`LayoutTree::resize_after_visible`]: kimun_notes::logic::layout_tree::LayoutTree::resize_after_visible
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
    /// (`view_registry`), [`SLOT_MIN_REMS`] wide for a slot showing none (not
    /// drawn, so never laid out). Hidden views do not count.
    pub(crate) fn slot_min(&self, slot: SlotId) -> (f64, f64) {
        match self.layout.shown(slot) {
            Some(view) => view_registry::spec(view).min,
            None => (f64::from(SLOT_MIN_REMS), 0.0),
        }
    }

    /// The minimum (width, height) of `leaf`, in rems. The editor region's
    /// width is [`EDITOR_MIN_REMS`] for each group in the widest row of the
    /// shown tab ([`LayoutTree::editor_min`]): groups side by side ask for
    /// room, so the slots collapse first and a group never does.
    ///
    /// [`LayoutTree::editor_min`]: kimun_notes::logic::layout_tree::LayoutTree::editor_min
    fn leaf_min(&self, leaf: Leaf) -> (f64, f64) {
        match leaf {
            Leaf::Editor => self.layout.editor_min((f64::from(EDITOR_MIN_REMS), 0.0)),
            Leaf::Slot(slot) => self.slot_min(slot),
        }
    }

    /// The px the tree has: the window less the title bar (as last drawn)
    /// and less the rail's thickness, taken from the width by a left or
    /// right rail and from the height by a top or bottom one (ADR-0055 §4).
    /// A hidden rail takes nothing.
    fn tree_area(&self, window: &Window) -> (f64, f64) {
        let viewport = window.viewport_size();
        let rem = f64::from(window.rem_size());
        let rail = if self.rail_hidden {
            0.0
        } else {
            f64::from(RAIL_REMS) * rem
        };
        let (across, down) = if self.rail.horizontal() {
            (0.0, rail)
        } else {
            (rail, 0.0)
        };
        (
            (f64::from(viewport.width) - across).max(0.0),
            (f64::from(viewport.height) - self.title_bar_height.get() - down).max(0.0),
        )
    }

    /// The frame of the layout tree: it fills what the rail leaves, in
    /// whichever direction the body runs.
    fn tree_frame(&self) -> impl IntoElement + ParentElement {
        let horizontal = self.rail.horizontal();
        div()
            .id("layout-tree")
            .test_support()
            .flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .when(horizontal, |frame| frame.w_full())
            .when(!horizontal, |frame| frame.h_full())
    }

    /// Which leaves the window has room for (ADR-0052 §6, Ruling 12):
    /// slots that are not drawn are left out, then what overflows
    /// collapses ([`LayoutTree::fit`], in rems; the editor region never).
    /// Each leaf's minimum carries one px for the gap beside it.
    ///
    /// [`LayoutTree::fit`]: kimun_notes::logic::layout_tree::LayoutTree::fit
    pub(crate) fn fit(&self, window: &Window) -> Fit<Leaf> {
        self.fit_tree(&self.layout, window)
    }

    /// [`Self::fit`] for `tree`, a copy of the layout differing from it in
    /// weights only (the minimums and what is drawn are read from the
    /// layout).
    fn fit_tree(&self, tree: &LayoutTree<DrawerView>, window: &Window) -> Fit<Leaf> {
        tree.fit(
            self.fit_area(window),
            &|leaf| self.fit_min(leaf, window),
            &|leaf| self.leaf_drawn(leaf),
        )
    }

    /// The tree's area in rems, as [`Self::fit`] measures it.
    fn fit_area(&self, window: &Window) -> UnitRect {
        let rem = f64::from(window.rem_size());
        let (width, height) = self.tree_area(window);
        UnitRect::new(0.0, 0.0, width / rem, height / rem)
    }

    /// `leaf`'s minimum in rems with the px of gap beside it, as
    /// [`Self::fit`] counts it.
    fn fit_min(&self, leaf: Leaf, window: &Window) -> (f64, f64) {
        let gap = GAP / f64::from(window.rem_size());
        let (w, h) = self.leaf_min(leaf);
        (w + gap, h + gap)
    }

    /// Make the window's room for the new slot `new` (ADR-0054 §8): while
    /// [`Self::fit`] collapses it or a leaf of `visible_before` (what was
    /// on screen before the placement, still drawn), grow the cell holding
    /// the split that collapses it, taking from its siblings only what
    /// each has above its minimum ([`LayoutTree::make_room`]). Kept only
    /// when `new` is then visible and no leaf of `visible_before`
    /// collapsed; false otherwise, the layout unchanged.
    ///
    /// [`LayoutTree::make_room`]: kimun_notes::logic::layout_tree::LayoutTree::make_room
    pub(super) fn make_room(
        &mut self,
        new: SlotId,
        visible_before: &[Leaf],
        window: &Window,
    ) -> bool {
        // A leaf gone or undrawn since (the slot the view left) needs no room.
        let leaves: Vec<Leaf> = std::iter::once(Leaf::Slot(new))
            .chain(
                visible_before
                    .iter()
                    .copied()
                    .filter(|leaf| self.leaf_drawn(*leaf)),
            )
            .collect();
        let mut grown = self.layout.clone();
        let room = grown.make_room(
            &leaves,
            self.fit_area(window),
            &|leaf| self.fit_min(leaf, window),
            &|leaf| self.leaf_drawn(leaf),
        );
        if !room {
            return false;
        }
        let fit = self.fit_tree(&grown, window);
        if leaves.iter().any(|leaf| fit.collapsed.contains(leaf)) {
            return false;
        }
        self.layout = grown;
        true
    }

    /// The leaves [`Self::fit`] has room for now (zoom aside).
    pub(super) fn fitting_leaves(&self, window: &Window) -> Vec<Leaf> {
        self.fit(window)
            .rects
            .into_iter()
            .map(|(leaf, _)| leaf)
            .collect()
    }

    /// Fit the tree to the window for this frame: remember what collapsed
    /// (session only), and move the keyboard off a leaf that did. While
    /// zoomed the zoomed leaf alone fills the area, so nothing collapses
    /// ([`Self::settle_zoom`] first ends a zoom that no longer holds).
    pub(super) fn refit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let unzoomed = self.settle_zoom();
        self.collapsed = if self.layout.zoomed().is_some() {
            HashSet::new()
        } else {
            self.fit(window).collapsed.into_iter().collect()
        };
        if let Focus::Leaf(leaf) = self.layout.focused()
            && (self.collapsed.contains(&leaf) || (unzoomed && !self.leaf_visible(leaf)))
        {
            self.fix_focus();
            cx.defer_in(window, |this, window, cx| this.apply_focus(window, cx));
        }
    }

    /// End zoom when it no longer holds (ADR-0053): its leaf is no longer
    /// drawn (a toggle or move hid its view), or a command gave another
    /// leaf the keyboard (the rail click, opening a view or a note: the
    /// user asked to see something else; the rail itself does not count).
    /// Closing the zoomed leaf already ended it in the model. Whether it
    /// ended now.
    fn settle_zoom(&mut self) -> bool {
        let Some(zoomed) = self.layout.zoomed() else {
            return false;
        };
        let elsewhere = matches!(self.layout.focused(), Focus::Leaf(leaf) if leaf != zoomed);
        if self.leaf_drawn(zoomed) && !elsewhere {
            return false;
        }
        self.layout.zoom(None);
        true
    }

    /// Whether `leaf` is on screen: while zoomed only the zoomed leaf;
    /// otherwise drawn and not collapsed.
    pub(super) fn leaf_visible(&self, leaf: Leaf) -> bool {
        match self.layout.zoomed() {
            Some(zoomed) => leaf == zoomed && self.leaf_drawn(leaf),
            None => self.leaf_drawn(leaf) && !self.collapsed.contains(&leaf),
        }
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
    /// leaves and the height under the title bar. While zoomed (ADR-0053)
    /// only the zoomed leaf is drawn there, no other slot, group or
    /// divider; a zoomed slot has the Footer under it (the editor region
    /// carries its own).
    pub(super) fn render_tree(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // Measured afresh as this tree is laid out.
        self.divider_cells.borrow_mut().clear();
        let rem = f64::from(window.rem_size());
        if let Some(zoomed) = self.layout.zoomed() {
            return self.render_zoomed(zoomed, window, rem, cx);
        }
        let shape = self.layout.shape();
        let tree = self
            .render_node(
                &shape,
                self.tree_area(window),
                (false, false),
                rem,
                window,
                cx,
            )
            // Every tree holds the editor region, which is always drawn; a
            // root left empty would be a model bug, so draw the editor.
            .unwrap_or_else(|| {
                self.render_editor(self.tree_area(window), window, cx)
                    .into_any_element()
            });
        self.tree_frame().child(tree).into_any_element()
    }

    /// The zoomed `leaf` filling the tree's area: the editor region is the
    /// editor Panel, Footer included; a slot is drawn over the Footer. No
    /// minimum applies: the leaf has the whole area.
    fn render_zoomed(
        &self,
        leaf: Leaf,
        window: &mut Window,
        rem: f64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let content = match leaf {
            Leaf::Editor => self
                .render_editor(self.tree_area(window), window, cx)
                .into_any_element(),
            Leaf::Slot(_) => {
                let slot = self
                    .render_node(
                        &TreeShape::Leaf(leaf),
                        self.tree_area(window),
                        (false, false),
                        rem,
                        window,
                        cx,
                    )
                    .unwrap_or_else(|| div().flex_1().into_any_element());
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(div().flex().flex_1().min_h_0().child(slot))
                    .child(self.render_footer(false, window, cx))
                    .into_any_element()
            }
        };
        self.tree_frame().child(content).into_any_element()
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        match shape {
            TreeShape::Leaf(Leaf::Editor) => {
                Some(self.render_editor(size, window, cx).into_any_element())
            }
            TreeShape::Leaf(Leaf::Slot(slot)) => {
                if !self.leaf_visible(Leaf::Slot(*slot)) {
                    return None;
                }
                let view = self.layout.shown(*slot)?;
                let (w, h) = view_registry::spec(view).min;
                let small =
                    (across.0 && size.0 < w * rem - 0.5) || (across.1 && size.1 < h * rem - 0.5);
                Some(self.render_slot(*slot, view, small, size.0, rem, cx))
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
                        let element = self.render_node(child, size, across, rem, window, cx)?;
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
                let mut pairs: Vec<(usize, usize, DividerAt, f64, f64)> = Vec::new();
                // Consecutive drawn children are neighbours on screen, and
                // the tree resizes between them across hidden siblings
                // (`resize_after_visible`), so every seam is a divider.
                let mut previous: Option<(usize, Leaf, f64)> = None;
                let mut any_divider = false;
                for (_, weight, min, last, element) in drawn {
                    if let Some((prev_element, prev_last, prev_min)) = previous {
                        elements.push(self.render_gap(axis, None, None, cx));
                        handles.push(self.render_gap(
                            axis,
                            Some(DividerAt::Leaf(prev_last)),
                            None,
                            cx,
                        ));
                        any_divider = true;
                        pairs.push((
                            prev_element,
                            elements.len(),
                            DividerAt::Leaf(prev_last),
                            prev_min,
                            min.unwrap_or(0.0) / rem,
                        ));
                    }
                    previous = Some((elements.len(), last, min.unwrap_or(0.0) / rem));
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
                Some(self.split_frame(axis, pairs, elements))
            }
        }
    }

    /// The row (`axis` horizontal) or column of a split's `elements`,
    /// recording each divider's cells as it is prepainted: `pairs` holds,
    /// per divider, the elements before and after it, where it sits and
    /// the two cells' minimums in rems.
    fn split_frame(
        &self,
        axis: Axis,
        pairs: Vec<(usize, usize, DividerAt, f64, f64)>,
        elements: Vec<AnyElement>,
    ) -> AnyElement {
        let cells = self.divider_cells.clone();
        div()
            .on_children_prepainted(move |bounds, _, _| {
                let mut cells = cells.borrow_mut();
                for (before, after, at, min_before, min_after) in &pairs {
                    if let (Some(b), Some(a)) = (bounds.get(*before), bounds.get(*after)) {
                        cells.insert(
                            *at,
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
            .into_any_element()
    }

    /// The shown tab's groups, laid out in the editor Panel of `size` px
    /// (its layout cell) as the tree lays out its leaves: each split a
    /// flex row or column whose children grow by their weights, a hairline
    /// between neighbours that is also a divider ([`DividerAt::Group`]).
    /// A group's minimum width is [`EDITOR_MIN_REMS`] for each group in its
    /// widest row; where the minimums do not fit (the region is never
    /// collapsed) the groups share by weight alone, so none disappears.
    pub(super) fn render_groups(
        &self,
        size: (f64, f64),
        rem: f64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let region = self.layout.region();
        let order = region.groups(region.shown());
        // The groups get the Panel less its margin and border.
        let inset = 2.0 * f64::from(f32::from(LEAF_MARGIN + LEAF_BORDER));
        let size = ((size.0 - inset).max(0.0), (size.1 - inset).max(0.0));
        self.render_group_node(&region.shape(), size, &order, rem, cx)
    }

    /// `shape`, a part of the shown tab, drawn in `size` px; `order` is the
    /// tab's groups in reading order (a group's place there names its
    /// elements).
    fn render_group_node(
        &self,
        shape: &GroupShape,
        size: (f64, f64),
        order: &[GroupId],
        rem: f64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (axis, children) = match shape {
            GroupShape::Group(group) => {
                let at = order.iter().position(|g| g == group).unwrap_or_default();
                return self.render_group(at, *group, rem, cx);
            }
            GroupShape::Split { axis, children } => (*axis, children),
        };
        let (length, cross) = match axis {
            Axis::Horizontal => size,
            Axis::Vertical => (size.1, size.0),
        };
        let per_group = f64::from(EDITOR_MIN_REMS) * rem;
        let mins: Vec<f64> = children
            .iter()
            .map(|(_, child)| match axis {
                Axis::Horizontal => child.widest_row() as f64 * per_group,
                Axis::Vertical => 0.0,
            })
            .collect();
        let gaps = children.len().saturating_sub(1) as f64 * GAP;
        let cells: Vec<(f64, f64)> = children
            .iter()
            .zip(&mins)
            .map(|((weight, _), min)| (*weight, *min))
            .collect();
        let (sizes, held) = distribute((length - gaps).max(0.0), &cells);
        let line = theme::divider(&theme::current(cx));
        // The weights are normalized: they are the grow factors.
        let cell = |weight: f64, min: Option<f64>| {
            div()
                .flex()
                .flex_basis(rems(0.))
                .flex_grow(weight as f32)
                .min_w_0()
                .min_h_0()
                .when_some(min, |cell, min| match axis {
                    Axis::Horizontal => cell.min_w(px(min as f32)),
                    Axis::Vertical => cell.min_h(px(min as f32)),
                })
        };
        let mut elements: Vec<AnyElement> = Vec::new();
        let mut handles: Vec<AnyElement> = Vec::new();
        let mut pairs: Vec<(usize, usize, DividerAt, f64, f64)> = Vec::new();
        let mut previous: Option<(usize, GroupId, f64)> = None;
        for (((weight, child), along), min) in children.iter().zip(&sizes).zip(&mins) {
            let child_size = match axis {
                Axis::Horizontal => (*along, cross),
                Axis::Vertical => (cross, *along),
            };
            let element = self.render_group_node(child, child_size, order, rem, cx);
            // Minimums are set only where they all fit.
            let min = held.then_some(*min);
            let min_rems = min.unwrap_or(0.0) / rem;
            if let Some((prev_element, prev_last, prev_min)) = previous {
                let divider = DividerAt::Group(prev_last);
                elements.push(self.render_gap(axis, None, Some(line), cx));
                handles.push(self.render_gap(axis, Some(divider), None, cx));
                pairs.push((prev_element, elements.len(), divider, prev_min, min_rems));
            }
            previous = Some((elements.len(), child.last_group(), min_rems));
            elements.push(cell(*weight, min).child(element).into_any_element());
            handles.push(cell(*weight, min).into_any_element());
        }
        // The handles over the groups, in an empty copy of the cells and
        // gaps, as the tree's are.
        elements.push(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .map(|row| match axis {
                    Axis::Horizontal => row.flex_row(),
                    Axis::Vertical => row.flex_col(),
                })
                .children(handles)
                .into_any_element(),
        );
        self.split_frame(axis, pairs, elements)
    }

    /// The group `group`, at `at` in the shown tab: its header over what it
    /// shows ([`Workspace::render_group_document`]), with the target marker
    /// while it wears it. A press in its document makes it the active
    /// group (its header's click does the same; its close button does not).
    fn render_group(
        &self,
        at: usize,
        group: GroupId,
        rem: f64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (document, _) = self.render_group_document(group, cx);
        div()
            .id(group_element(GROUP_ID, at))
            .test_support()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(self.render_group_header(at, group, cx))
            // The body, under the header: while a group is dragged its
            // zones (the inner layer) lie over it.
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .capture_any_mouse_down(cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                        if this.layout.active_group() != group {
                            this.focus_group(group, cx);
                        }
                    }))
                    .child(document)
                    .children(self.group_zone_layer(at, group, cx)),
            )
            .when(self.wears_target_marker(group), |el| {
                el.child(self.render_target_marker(at, group, rem, cx))
            })
            .into_any_element()
    }

    /// The pixel between two drawn neighbours, so touching cells never
    /// merge: window background between leaves, `line` (a hairline colour)
    /// between groups. With `divider` (where it sits), a resize handle
    /// straddling it.
    fn render_gap(
        &self,
        axis: Axis,
        divider: Option<DividerAt>,
        line: Option<Rgba>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let gap = div()
            .flex_none()
            .relative()
            .when_some(line, |gap, line| gap.bg(line))
            .map(|gap| match axis {
                Axis::Horizontal => gap.w(px(1.)).h_full(),
                Axis::Vertical => gap.h(px(1.)).w_full(),
            });
        let Some(at) = divider else {
            return gap.into_any_element();
        };
        // The kit's axis is the one the handle resizes along.
        let handle_axis = match axis {
            Axis::Horizontal => gpui_kit::Axis::Horizontal,
            Axis::Vertical => gpui_kit::Axis::Vertical,
        };
        gap.child(
            resize_handle::<DividerDrag, DragGhost>(self.divider_id(at), handle_axis)
                .with_appearance(seam(handle_axis, theme::focus_border(&theme::current(cx))))
                .on_drag(DividerDrag(at), |_, _, _, cx| cx.new(|_| DragGhost))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, _| {
                        this.grab_divider(at, event.position)
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.end_resize(cx)),
                ),
        )
        .into_any_element()
    }

    /// The element id of the divider at `at`: after a leaf,
    /// `divider-slot-<n>` by the slot's place among the tree's slots in
    /// reading order, or `divider-group-0` for the editor leaf (the id it
    /// had as the one editor group); between groups, `group-divider-<n>` by
    /// the place of the group before it in the shown tab.
    fn divider_id(&self, at: DividerAt) -> SharedString {
        match at {
            DividerAt::Leaf(Leaf::Slot(slot)) => {
                let n = self.layout.slots().iter().position(|s| *s == slot);
                format!("divider-slot-{}", n.unwrap_or_default()).into()
            }
            DividerAt::Leaf(Leaf::Editor) => "divider-group-0".into(),
            DividerAt::Group(group) => {
                let region = self.layout.region();
                let n = region
                    .groups(region.shown())
                    .iter()
                    .position(|g| *g == group);
                group_divider_id(n.unwrap_or_default())
            }
        }
    }

    /// A press on the divider `divider` at `at`: remember how far from
    /// the boundary it landed.
    fn grab_divider(&mut self, divider: DividerAt, at: Point<Pixels>) {
        self.press_offset = match self.divider_cells.borrow().get(&divider) {
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
        let DividerDrag(at) = *event.drag(cx);
        let Some(axis) = self.divider_cells.borrow().get(&at).map(|c| c.axis) else {
            return;
        };
        let pixel = along(axis, event.event.position) - self.press_offset;
        self.on_divider_drag(at, pixel, window, cx);
    }

    /// The divider at `at` dragged to `pixel` (window px along its
    /// axis): the cell before it takes the share of the two cells' combined
    /// drawn extent that ends there ([`LayoutTree::resize_after_visible`];
    /// between groups [`EditorRegion::resize_after`]), held so neither cell
    /// goes under its drawn minimum (Ruling 16: past a floor flex would
    /// take the difference from a third cell). Only those two cells'
    /// weights change. Saved when the drag ends ([`Self::end_resize`]), not
    /// on every move.
    ///
    /// [`LayoutTree::resize_after_visible`]: kimun_notes::logic::layout_tree::LayoutTree::resize_after_visible
    /// [`EditorRegion::resize_after`]: kimun_notes::logic::editor_region::EditorRegion::resize_after
    pub(crate) fn on_divider_drag(
        &mut self,
        at: impl Into<DividerAt>,
        pixel: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let at = at.into();
        let Some(cell) = self.divider_cells.borrow().get(&at).copied() else {
            return;
        };
        let rem = f64::from(window.rem_size());
        let Some(share) = cell.share_at(pixel - cell.start(), rem) else {
            return;
        };
        let moved = match at {
            DividerAt::Leaf(leaf) => self.resize_after_visible(leaf, share),
            DividerAt::Group(group) => self.layout.region_mut().resize_after(group, share),
        };
        if moved {
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
        let (divider, leaf_before) = match self.resize_target() {
            Ok(target) => target,
            Err(reason) => return self.flash(reason, cx),
        };
        let Some(cell) = self
            .divider_cells
            .borrow()
            .get(&DividerAt::Leaf(divider))
            .copied()
        else {
            return self.flash(NOTHING_TO_RESIZE, cx);
        };
        let step = if grow == leaf_before {
            RESIZE_STEP
        } else {
            -RESIZE_STEP
        };
        let rem = f64::from(window.rem_size());
        let Some(share) = cell.share_at(cell.first() + step * cell.pair(), rem) else {
            return self.flash(AT_MINIMUM_SIZE, cx);
        };
        if self.resize_after_visible(divider, share) {
            self.save_layout(cx);
            cx.notify();
        } else {
            self.flash(AT_MINIMUM_SIZE, cx);
        }
    }

    /// Why leader grow / shrink cannot act now, from what the focused leaf
    /// and the drawn layout are (the palette greys the rows with it): the
    /// rail or a zoom has no divider to move, nor a leaf alone on screen.
    /// The floors are not checked here: they depend on the window's size
    /// and the step says so when it hits one.
    pub(super) fn resize_block(&self) -> Option<&'static str> {
        self.resize_target().err()
    }

    /// The divider the focused leaf's grow / shrink moves (see
    /// [`Self::step_focused_leaf`]), or why there is none.
    fn resize_target(&self) -> Result<(Leaf, bool), &'static str> {
        let Focus::Leaf(leaf) = self.layout.focused() else {
            return Err(FOCUS_TO_RESIZE);
        };
        if self.layout.zoomed().is_some() {
            return Err(ZOOMED_NO_RESIZE);
        }
        self.divider_beside(&self.layout.shape(), leaf)
            .ok_or(NOTHING_TO_RESIZE)
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
        // The nearest drawn sibling on either side, hidden ones between
        // skipped, as the dividers are drawn.
        if children[i + 1..].iter().any(|(_, c)| self.shape_visible(c)) {
            return Some((last_leaf(&children[i].1), true));
        }
        children[..i]
            .iter()
            .rev()
            .find(|(_, c)| self.shape_visible(c))
            .map(|(_, c)| (last_leaf(c), false))
    }

    /// Move the boundary after `leaf`'s branch to `share` of it and the
    /// next drawn sibling (hidden and collapsed leaves skipped, keeping
    /// their weights), as the dividers are drawn.
    fn resize_after_visible(&mut self, leaf: Leaf, share: f64) -> bool {
        let leaves = self.layout.slots().into_iter().map(Leaf::Slot);
        let leaves = leaves.chain(std::iter::once(Leaf::Editor));
        let visible: Vec<Leaf> = leaves.filter(|l| self.leaf_visible(*l)).collect();
        self.layout
            .resize_after_visible(leaf, share, &|l| visible.contains(&l))
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

    /// The border of `focus`'s surface: the accent while it has the
    /// keyboard (it follows the rounded corners, as a bar would not), the
    /// hairline otherwise.
    pub(super) fn leaf_border(&self, focus: Focus, cx: &gpui_kit::App) -> Rgba {
        let theme = theme::current(cx);
        if self.layout.focused() == focus {
            theme::focus_border(&theme)
        } else {
            theme::divider(&theme)
        }
    }

    /// The surface of a leaf, inside the margin its root element keeps
    /// (`p(LEAF_MARGIN)` on the root: padding, so the root's bounds stay the
    /// layout cell and every minimum and divider position stays what the
    /// weights say): rounded, hairline-bordered, on `background`.
    pub(super) fn leaf_surface(background: Rgba, border: Rgba) -> gpui_kit::Div {
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .rounded(rems(LEAF_RADIUS_REMS as f32))
            .border_1()
            .border_color(border)
            .bg(background)
    }

    /// Tell `view` about its slot, if it lays itself out by it
    /// ([`SlotAware`]). Views that do not are untouched.
    fn push_slot_context(&self, view: DrawerView, context: SlotContext, cx: &mut Context<Self>) {
        match view {
            DrawerView::Ask => {
                if let Some(ask) = self.ask.clone() {
                    ask.update(cx, |ask, cx| ask.set_slot(context, cx));
                }
            }
            DrawerView::Files
            | DrawerView::Links
            | DrawerView::Outline
            | DrawerView::Find
            | DrawerView::Tags
            | DrawerView::Semantic
            | DrawerView::Config => {}
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
        width: f64,
        rem: f64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let spec = view_registry::spec(view);
        let context = SlotContext {
            zoomed: self.layout.zoomed() == Some(Leaf::Slot(slot)),
            width: view_width(width),
            rem,
        };
        self.push_slot_context(view, context, cx);
        div()
            .id(slot_id(view))
            .test_support()
            .track_focus(&self.slot_handle(slot))
            .role(Role::Group)
            .aria_label(spec.name)
            .flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .p(LEAF_MARGIN)
            .child(
                Self::leaf_surface(
                    theme::panel_background(&theme::current(cx)),
                    self.leaf_border(Focus::Leaf(Leaf::Slot(slot)), cx),
                )
                .capture_any_mouse_down(cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    this.focus_slot(slot, window, cx)
                }))
                .child(self.render_slot_header(slot, view, cx))
                // The body, under the header: while a view is dragged its
                // zones lie over it (not the header, so a drag released
                // where it started changes nothing).
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .child(self.render_slot_body(view, small, cx))
                        .children(self.zone_layer(slot, cx)),
                ),
            )
            .into_any_element()
    }

    /// What a drawn slot shows under its header: `view` (or, for a view
    /// that needs a workspace while none is open, a line saying so;
    /// [`TOO_SMALL`] when `small`: under the view's minimum).
    fn render_slot_body(
        &self,
        view: DrawerView,
        small: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.view_content(view).filter(|_| !small) {
            Some(content) => div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .p(CONTENT_PADDING)
                .child(content)
                .into_any_element(),
            None if small => div()
                .id(TOO_SMALL_ID)
                .test_support()
                .role(Role::Label)
                .aria_label(TOO_SMALL)
                .p(CONTENT_PADDING)
                .text_sm()
                .text_color(theme::muted(&theme::current(cx)))
                .child(TOO_SMALL)
                .into_any_element(),
            // Every view is built; one without content needs a vault.
            None => div()
                .p(CONTENT_PADDING)
                .text_sm()
                .text_color(theme::muted(&theme::current(cx)))
                .child(route::NO_WORKSPACE)
                .into_any_element(),
        }
    }

    /// The strip at the top of a drawn slot: the title of the view it shows
    /// on the header background, a hairline under it, and the accent on the title while
    /// the slot has the focus. It ends with a quiet
    /// `⋯` (its own id, [`move_button_id`]): a menu of the slot's actions,
    /// moving its view ([`Workspace::open_move_chooser`]) and closing it
    /// ([`Workspace::close_slot`]; greyed out for the last slot on its
    /// side of the editor).
    fn render_slot_header(
        &self,
        slot: SlotId,
        view: DrawerView,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let focused = self.layout.focused() == Focus::Leaf(Leaf::Slot(slot));
        let title: SharedString = view_registry::spec(view).title.into();
        let theme = theme::current(cx);
        let header = div()
            .id(if focused {
                "slot-header-focused"
            } else {
                "slot-header"
            })
            .test_support()
            .role(Role::Group)
            .aria_label(title.clone())
            .w_full()
            // A slot narrower than its title clips it to one line.
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .px_3()
            // Room for the `⋯` laid over the strip's right end.
            .pr(px(28.))
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
            .child(title);
        // The strip is a drag source (ADR-0054 §8) through a wrapper whose
        // id is stable: the strip's own id follows the focus, and a press
        // that focuses the slot would lose the half-made drag with it. The
        // `⋯` below is the wrapper's sibling, so it stays a plain click.
        let strip = div()
            .id(SharedString::from(format!("slot-drag-{}", view.id())))
            .w_full()
            .cursor_grab()
            .child(header)
            .on_drag(
                ViewDrag {
                    view,
                    from: DragFrom::Header,
                },
                self.view_drag_ghost(cx),
            );
        // The button is the strip's sibling, not its child: the strip's id
        // follows the focus, and a press that focuses the slot would
        // otherwise lose the button's half-made click with it.
        div().relative().w_full().flex_none().child(strip).child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .right_1()
                .flex()
                .items_center()
                .child(
                    Button::new(move_button_id(view))
                        .ghost()
                        .xsmall()
                        .label("⋯")
                        .tooltip("View actions")
                        .dropdown_menu_with_anchor(gpui_kit::Anchor::TopRight, {
                            let this = cx.entity().downgrade();
                            move |menu, _, cx| {
                                // Worked out when the menu opens, not every
                                // frame: each side reading is a layout pass.
                                let only = this
                                    .upgrade()
                                    .is_none_or(|ws| ws.read(cx).layout.last_on_side(slot));
                                let run = |action: fn(&mut Workspace, SlotId, &mut Window, &mut Context<Workspace>)| {
                                    let this = this.clone();
                                    move |_: &ClickEvent, window: &mut Window, cx: &mut gpui_kit::App| {
                                        this.update(cx, |ws, cx| action(ws, slot, window, cx)).ok();
                                    }
                                };
                                menu.item(PopupMenuItem::new(MOVE_VIEW_ITEM).on_click(run(
                                    |ws, slot, window, cx| {
                                        ws.focus_slot(slot, window, cx);
                                        ws.open_move_chooser(slot, window, cx);
                                    },
                                )))
                                .item(
                                    PopupMenuItem::new(CLOSE_SLOT_ITEM)
                                        .disabled(only)
                                        .on_click(run(|ws, slot, window, cx| {
                                            ws.close_slot(slot, window, cx);
                                        })),
                                )
                            }
                        }),
                ),
        )
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
            DrawerView::Ask => self.ask.clone().map(|v| v.into_any_element()),
            DrawerView::Config => Some(self.config.clone().into_any_element()),
        }
    }
}
