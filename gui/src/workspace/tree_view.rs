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
//! (`view_registry`, in rems) and the editor group its own floor. Dividers
//! and resizing are not drawn yet.
//!
//! [shape]: kimun_notes::logic::layout_tree::LayoutTree::shape

use gpui_kit::base::TestSupportExt;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseDownEvent, ParentElement, Role,
    SharedString, StatefulInteractiveElement, Styled, Window, div, rems,
};
use kimun_notes::logic::layout_tree::{Axis, Focus, Leaf, SlotId, TreeShape};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::view_registry;

use super::{Workspace, empty_slot_id, route, slot_id};
use crate::shell_layout::EDITOR_MIN_REMS;
use crate::theme;

impl Workspace {
    /// The layout tree, right of the rail: it takes the width the rail
    /// leaves and the height under the title bar.
    pub(super) fn render_tree(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let shape = self.layout.shape();
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
                let drawn: Vec<(f64, Option<f64>, AnyElement)> = children
                    .iter()
                    .filter_map(|(weight, child)| {
                        let element = self.render_node(child, cx)?;
                        let min = self.min_width(child).filter(|_| *axis == Axis::Horizontal);
                        Some((*weight, min, element))
                    })
                    .collect();
                // The drawn children share the split by their weights: flex
                // grow factors summing below one would leave space unused.
                let total: f64 = drawn.iter().map(|(weight, ..)| weight).sum();
                let drawn: Vec<AnyElement> = drawn
                    .into_iter()
                    .enumerate()
                    .map(|(index, (weight, min, element))| {
                        div()
                            .flex()
                            // One pixel of window background between
                            // neighbours, so touching slots never merge.
                            .when(index > 0, |cell| match axis {
                                Axis::Horizontal => cell.ml_px(),
                                Axis::Vertical => cell.mt_px(),
                            })
                            .flex_basis(rems(0.))
                            .flex_grow((weight / total) as f32)
                            .min_w_0()
                            .min_h_0()
                            .when_some(min, |cell, min| cell.min_w(rems(min as f32)))
                            .child(element)
                            .into_any_element()
                    })
                    .collect();
                if drawn.is_empty() {
                    return None;
                }
                Some(
                    div()
                        .flex()
                        .map(|row| match axis {
                            Axis::Horizontal => row.flex_row(),
                            Axis::Vertical => row.flex_col(),
                        })
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .children(drawn)
                        .into_any_element(),
                )
            }
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
