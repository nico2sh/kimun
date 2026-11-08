//! Drawing the **layout tree** (ADR-0052): everything right of the rail is
//! one recursive walk over the tree's [shape]. A split is a flex row
//! (horizontal) or column (vertical) whose children grow by their weights;
//! a slot shows its view, an editor group the editor Panel
//! ([`Workspace::render_editor`]).
//!
//! A slot that is not drawn ([`Workspace::slot_drawn`]: no shown view, not
//! opened empty) takes no space: it is left out, its siblings share its
//! weight, and a split left with no drawn child is left out too. In a row,
//! a slot keeps the largest minimum width of the views it holds
//! (`view_registry`, in rems) and the editor group its own floor.
//!
//! **Dividers.** One pixel of window background separates drawn
//! neighbours. Where the two are neighbours in the tree too, that pixel is
//! a divider: a [`resize_handle`] straddling it, whose drag moves only that
//! boundary ([`Workspace::on_divider_drag`], the tree's
//! [`resize_after`]) and is saved when it ends. A drawn cell and an
//! undrawn slot have no divider (Ruling 12), and neither do two drawn cells
//! with an undrawn slot between them in the tree: `resize_after` moves the
//! boundary between tree siblings, which there is not the one drawn. The
//! leader's grow and shrink steps move a divider of the focused leaf by
//! [`RESIZE_STEP`] ([`Workspace::step_focused_leaf`]).
//!
//! [shape]: kimun_notes::logic::layout_tree::LayoutTree::shape
//! [`resize_after`]: kimun_notes::logic::layout_tree::LayoutTree::resize_after

use std::cell::RefCell;
use std::collections::HashMap;
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
use kimun_notes::logic::layout_tree::{Axis, Focus, Leaf, SlotId, TreeShape};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::view_registry;

use super::{Workspace, empty_slot_id, route, slot_id};
use crate::shell_layout::EDITOR_MIN_REMS;
use crate::theme;

/// What one leader grow or shrink step moves a divider by: this share of
/// the two cells it sits between (5%), so a step feels the same in a
/// narrow pair and a wide one.
pub(super) const RESIZE_STEP: f64 = 0.05;

/// Each divider's axis and the drawn bounds of the cells before and after
/// it, keyed by the divider's leaf (the last leaf of the cell before it),
/// written as the tree is prepainted.
pub(super) type DividerCells = Rc<RefCell<HashMap<Leaf, (Axis, Bounds<Pixels>, Bounds<Pixels>)>>>;

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
    /// The layout tree, right of the rail: it takes the width the rail
    /// leaves and the height under the title bar.
    pub(super) fn render_tree(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let shape = self.layout.shape();
        // Measured afresh as this tree is laid out.
        self.divider_cells.borrow_mut().clear();
        let tree = self
            .render_node(&shape, cx)
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

    /// `shape`, or `None` when nothing in it is drawn.
    fn render_node(&self, shape: &TreeShape, cx: &mut Context<Self>) -> Option<AnyElement> {
        match shape {
            TreeShape::Leaf(Leaf::Group(_)) => Some(self.render_editor(cx).into_any_element()),
            TreeShape::Leaf(Leaf::Slot(slot)) => {
                if !self.slot_drawn(*slot) {
                    return None;
                }
                Some(match self.layout.shown(*slot) {
                    Some(view) => self.render_slot(*slot, view, cx),
                    None => self.render_empty_slot(*slot, cx),
                })
            }
            TreeShape::Split { axis, children } => {
                let axis = *axis;
                let drawn: Vec<(usize, f64, Option<f64>, Leaf, AnyElement)> = children
                    .iter()
                    .enumerate()
                    .filter_map(|(index, (weight, child))| {
                        let element = self.render_node(child, cx)?;
                        let min = self.min_width(child).filter(|_| axis == Axis::Horizontal);
                        Some((index, *weight, min, last_leaf(child), element))
                    })
                    .collect();
                if drawn.is_empty() {
                    return None;
                }
                // The drawn children share the split by their weights: flex
                // grow factors summing below one would leave space unused.
                let total: f64 = drawn.iter().map(|(_, weight, ..)| weight).sum();
                let mut elements: Vec<AnyElement> = Vec::new();
                // (element before, element after, divider leaf) per divider.
                let mut pairs: Vec<(usize, usize, Leaf)> = Vec::new();
                let mut previous: Option<(usize, usize, Leaf)> = None;
                for (index, weight, min, last, element) in drawn {
                    if let Some((prev_index, prev_element, prev_last)) = previous {
                        let adjacent = prev_index + 1 == index;
                        elements.push(self.render_gap(axis, adjacent.then_some(prev_last), cx));
                        if adjacent {
                            pairs.push((prev_element, elements.len(), prev_last));
                        }
                    }
                    previous = Some((index, elements.len(), last));
                    elements.push(
                        div()
                            .flex()
                            .flex_basis(rems(0.))
                            .flex_grow((weight / total) as f32)
                            .min_w_0()
                            .min_h_0()
                            .when_some(min, |cell, min| cell.min_w(rems(min as f32)))
                            .child(element)
                            .into_any_element(),
                    );
                }
                let cells = self.divider_cells.clone();
                Some(
                    div()
                        .on_children_prepainted(move |bounds, _, _| {
                            let mut cells = cells.borrow_mut();
                            for (before, after, leaf) in &pairs {
                                if let (Some(b), Some(a)) =
                                    (bounds.get(*before), bounds.get(*after))
                                {
                                    cells.insert(*leaf, (axis, *b, *a));
                                }
                            }
                        })
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
            Some((axis, before, _)) => {
                let (start, size) = extent(*axis, *before);
                along(*axis, at) - (start + size)
            }
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
        let Some(axis) = self.divider_cells.borrow().get(&leaf).map(|c| c.0) else {
            return;
        };
        let pixel = along(axis, event.event.position) - self.press_offset;
        self.on_divider_drag(leaf, pixel, window, cx);
    }

    /// The divider after `leaf` dragged to `pixel` (window px along its
    /// axis): the cell before it takes the share of the two cells' combined
    /// drawn extent that ends there ([`LayoutTree::resize_after`], which
    /// clamps it). Only those two cells' weights change. Saved when the
    /// drag ends ([`Self::end_resize`]), not on every move.
    ///
    /// [`LayoutTree::resize_after`]: kimun_notes::logic::layout_tree::LayoutTree::resize_after
    pub(crate) fn on_divider_drag(
        &mut self,
        leaf: Leaf,
        pixel: f64,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((axis, before, after)) = self.divider_cells.borrow().get(&leaf).copied() else {
            return;
        };
        let (start, first) = extent(axis, before);
        let (_, second) = extent(axis, after);
        let pair = first + second;
        if pair <= 0.0 {
            return;
        }
        if self.layout.resize_after(leaf, (pixel - start) / pair) {
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
    /// by [`RESIZE_STEP`] of its pair, through the drag's path, and save.
    /// The divider is the one after the leaf's branch in the innermost
    /// split where that neighbour is drawn, else the one before it. Nothing
    /// with the rail focused or no divider beside the leaf.
    pub(super) fn step_focused_leaf(&mut self, grow: bool, cx: &mut Context<Self>) {
        let Focus::Leaf(leaf) = self.layout.focused() else {
            return;
        };
        let Some((divider, share, leaf_before)) = self.divider_beside(&self.layout.shape(), leaf)
        else {
            return;
        };
        let step = if grow == leaf_before {
            RESIZE_STEP
        } else {
            -RESIZE_STEP
        };
        if self.layout.resize_after(divider, share + step) {
            self.save_layout(cx);
            cx.notify();
        }
    }

    /// The divider beside `leaf` in `shape` (see
    /// [`Self::step_focused_leaf`]): its leaf, the share of the cell before
    /// it, and whether `leaf` is in that cell.
    fn divider_beside(&self, shape: &TreeShape, leaf: Leaf) -> Option<(Leaf, f64, bool)> {
        let TreeShape::Split { children, .. } = shape else {
            return None;
        };
        let i = children.iter().position(|(_, c)| contains(c, leaf))?;
        if let Some(found) = self.divider_beside(&children[i].1, leaf) {
            return Some(found);
        }
        let share = |a: usize| {
            let (wa, wb) = (children[a].0, children[a + 1].0);
            wa / (wa + wb)
        };
        if i + 1 < children.len() && self.shape_drawn(&children[i + 1].1) {
            return Some((last_leaf(&children[i].1), share(i), true));
        }
        if i > 0 && self.shape_drawn(&children[i - 1].1) {
            return Some((last_leaf(&children[i - 1].1), share(i - 1), false));
        }
        None
    }

    /// Whether anything in `shape` is drawn.
    fn shape_drawn(&self, shape: &TreeShape) -> bool {
        match shape {
            TreeShape::Leaf(Leaf::Group(_)) => true,
            TreeShape::Leaf(Leaf::Slot(slot)) => self.slot_drawn(*slot),
            TreeShape::Split { children, .. } => children.iter().any(|(_, c)| self.shape_drawn(c)),
        }
    }

    /// The minimum width, in rems, of a leaf in a row: a slot's is the
    /// largest registry minimum of the views placed in it, an editor
    /// group's the editor's floor. `None` for a split.
    fn min_width(&self, shape: &TreeShape) -> Option<f64> {
        match shape {
            TreeShape::Leaf(Leaf::Slot(slot)) => self
                .layout
                .slot_views(*slot)
                .iter()
                .map(|view| view_registry::spec(*view).min.0)
                .reduce(f64::max),
            TreeShape::Leaf(Leaf::Group(_)) => Some(f64::from(EDITOR_MIN_REMS)),
            TreeShape::Split { .. } => None,
        }
    }

    /// `slot`, showing `view`: its title over the view (FILES, or, for a
    /// view that needs a workspace while none is open, a line saying so).
    fn render_slot(&self, slot: SlotId, view: DrawerView, cx: &mut Context<Self>) -> AnyElement {
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
            .map(|el| match self.view_content(view) {
                Some(content) => el.child(content),
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
