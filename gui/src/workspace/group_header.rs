//! The **group header** and the **target marker** (ADR-0058). While the
//! shown tab holds two or more editor groups, each group draws a header
//! over its document: the title of what it shows (the note's name as the
//! Footer names it, `Empty` for an empty group), the Footer's unsaved dot
//! while its text is unsaved, and a close button. A click on the header
//! makes its group the active one; the close button closes the group
//! ([`Workspace::close_editor_group`]). A tab with one group draws no
//! header, so a window that never splits looks as it always did.
//!
//! The header reads like a slot header (the same strip, background and
//! hairline, the title in the focus colour while its group has the
//! keyboard), and its controls light up under the pointer as the tab
//! strip's do. It is the group's drag handle: dragged past the pointer
//! threshold it carries the group ([`super::group_drop`]), its id stable
//! so a press that activates the group keeps the half-made drag.
//!
//! The **target marker**: while the keyboard is outside the editor (in a
//! slot or on the rail) and the tab shows two or more groups, the active
//! group wears a bar in the focus colour along its top. It says which note
//! a plain open (from FILES, the finder, a link) replaces. With one group
//! there is no doubt, and nothing is drawn.

use gpui_kit::base::TestSupportExt;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, ElementId, InteractiveElement, IntoElement,
    ParentElement, Role, SharedString, StatefulInteractiveElement, Styled, div, px, rems,
};
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::layout_tree::{Focus, GroupId, Leaf, UnitRect};

use super::Workspace;
use super::drop_zones::GroupDrag;
use super::tab_strip::{EMPTY_LABEL, group_name};
use super::tree_view::{LEAF_BORDER, LEAF_RADIUS_REMS};
use crate::theme;

/// A group's element id prefix (with its position in the shown tab).
pub(crate) const GROUP_ID: &str = "editor-group";
/// A header's id prefix.
pub(crate) const GROUP_HEADER_ID: &str = "group-header";
/// A header's unsaved dot's id prefix.
pub(crate) const GROUP_UNSAVED_ID: &str = "group-unsaved";
/// A header's close button's id prefix.
pub(crate) const GROUP_CLOSE_ID: &str = "group-close";
/// A header's note-menu button's id prefix.
pub(crate) const GROUP_MENU_ID: &str = "group-menu";
/// The floating note-menu button of a lone group (no header).
pub(crate) const CORNER_MENU_ID: &str = "group-menu-corner";
/// The hover group of a lone group's document: the corner button lights up
/// while the pointer is anywhere over it.
pub(crate) const LONE_GROUP_HOVER: &str = "lone-group";
/// The corner button's opacity until the group or the button is hovered.
pub(crate) const CORNER_DIM_OPACITY: f32 = 0.35;
/// The target marker's id prefix.
pub(crate) const GROUP_TARGET_ID: &str = "group-target";
/// Every glyph a header draws, for the bundle test: an icon the asset
/// source does not carry paints nothing.
#[cfg(test)]
pub(crate) const ICONS: [gpui_kit::assets::IconName; 1] = [super::chrome::CLOSE_ICON];

/// `prefix` with the group's position in the shown tab (reading order).
pub(crate) fn group_element(prefix: &'static str, at: usize) -> ElementId {
    ElementId::NamedInteger(prefix.into(), at as u64)
}

/// Which top corners of a group at `rect` (in the region's unit square)
/// are corners of the editor Panel too, and so rounded: (left, right).
/// None while the tab strip is drawn above the groups (`strip`), nor for a
/// group below another.
pub(crate) fn marker_corners(rect: UnitRect, strip: bool) -> (bool, bool) {
    const EPS: f64 = 1e-6;
    if strip || rect.y() > EPS {
        return (false, false);
    }
    (rect.x() <= EPS, rect.right() >= 1.0 - EPS)
}

/// What `group`'s header calls it: the name of what it shows, as the
/// Footer names it, or `Empty`.
pub(crate) fn group_title(workspace: &Workspace, group: GroupId, cx: &App) -> SharedString {
    group_name(workspace, group, cx)
        .unwrap_or_else(|| EMPTY_LABEL.to_string())
        .into()
}

/// The corner button's opacity: full while the pointer is `near` (over
/// the group or the button), dim otherwise.
pub(crate) fn corner_opacity(near: bool) -> f32 {
    if near { 1.0 } else { CORNER_DIM_OPACITY }
}

impl Workspace {
    /// The `⋯` button that opens the note menu for `group`. Its click acts
    /// on the group it belongs to (making it active first) and stops there:
    /// the header's own click and drag do not follow. The menu opens with
    /// its top-left at the pointer (the click position: the button's
    /// bounds are not recorded).
    fn menu_button(
        &self,
        id: impl Into<ElementId>,
        group: GroupId,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(id)
            .ghost()
            .xsmall()
            .label("⋯")
            .tooltip("Note menu")
            // A press here is the button's, not the header's drag handle.
            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation()
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                cx.stop_propagation();
                if this.layout.active_group() != group {
                    this.focus_group(group, cx);
                }
                this.open_note_menu(group, event.position(), window, cx);
            }))
    }

    /// The floating corner button of a lone group: top-right of its
    /// document, dim until the group or the button is hovered, drawn over
    /// the document so it reserves no space. The find bar docks at the
    /// bottom of the editor, so the two never meet.
    pub(super) fn render_corner_menu_button(
        &self,
        group: GroupId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id("group-menu-corner-wrap")
            .absolute()
            .top_1()
            // Clear of a scrollbar down the document's right edge.
            .right_4()
            .opacity(corner_opacity(false))
            .group_hover(LONE_GROUP_HOVER, |s| s.opacity(corner_opacity(true)))
            .hover(|s| s.opacity(corner_opacity(true)))
            .child(self.menu_button(CORNER_MENU_ID, group, cx))
            .into_any_element()
    }

    /// Whether `group` wears the target marker: it is the active group of
    /// a shown tab with two or more groups, and the keyboard is outside the
    /// editor.
    pub(crate) fn wears_target_marker(&self, group: GroupId) -> bool {
        let region = self.layout.region();
        group == region.active_group()
            && region.groups(region.shown()).len() > 1
            && self.layout.zoomed().is_none()
            && self.layout.focused() != Focus::Leaf(Leaf::Editor)
    }

    /// The target marker over `group`, at `at` in the shown tab: a line in
    /// the focus colour along its top. It is the top border of a box over
    /// the whole group, so where the group's top corner is a corner of the
    /// editor Panel the line bends with the Panel's rounded corner instead
    /// of sticking out past it ([`marker_corners`]). Laid over the group,
    /// it takes no room and no pointer. `rem` is the window's rem in px.
    pub(super) fn render_target_marker(
        &self,
        at: usize,
        group: GroupId,
        rem: f64,
        cx: &App,
    ) -> AnyElement {
        let region = self.layout.region();
        let rect = region
            .regions()
            .into_iter()
            .find(|(g, _)| *g == group)
            .map(|(_, rect)| rect);
        let strip = region.tabs().len() > 1 || self.dragged_group(cx).is_some();
        let (left, right) = rect.map_or((false, false), |rect| marker_corners(rect, strip));
        // The Panel's corner (`rounded_md`) less its border: the curve of
        // the Panel's inner edge.
        let radius = px((LEAF_RADIUS_REMS * rem) as f32) - LEAF_BORDER;
        div()
            .id(group_element(GROUP_TARGET_ID, at))
            .test_support()
            .absolute()
            .inset_0()
            .border_t_2()
            .border_color(theme::focus_border(&theme::current(cx)))
            .when(left, |el| el.rounded_tl(radius))
            .when(right, |el| el.rounded_tr(radius))
            .into_any_element()
    }

    /// The header of `group`, at `at` in the shown tab.
    pub(super) fn render_group_header(
        &self,
        at: usize,
        group: GroupId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme::current(cx);
        let title = group_title(self, group, cx);
        let dirty = self.editor_dirty_in(group, cx);
        let keyboard = group == self.layout.active_group()
            && self.layout.focused() == Focus::Leaf(Leaf::Editor);
        let spoken: SharedString = if dirty {
            format!("{title}, unsaved").into()
        } else {
            title.clone()
        };
        let menu = self.menu_button(group_element(GROUP_MENU_ID, at), group, cx);
        let close = super::chrome::close_button(
            group_element(GROUP_CLOSE_ID, at),
            "Close group",
            super::chrome::tooltip_with_chord("Close group", &ActionShortcuts::CloseSlot, cx),
            // The header's own click (activate the group) does not follow.
            cx.listener(move |this, _: &ClickEvent, _, cx| this.close_editor_group(group, cx)),
            cx,
        )
        .flex_none()
        .size(rems(1.125))
        .rounded_sm();
        div()
            .id(group_element(GROUP_HEADER_ID, at))
            .test_support()
            .role(Role::Group)
            .aria_label(spoken)
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap_1()
            .w_full()
            .pl_3()
            .pr_1()
            .py_0p5()
            .text_sm()
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .bg(theme::slot_header_background(&theme))
            .border_b_1()
            .border_color(theme::divider(&theme))
            .text_color(if keyboard {
                theme::focus_border(&theme)
            } else {
                theme::foreground(&theme)
            })
            .cursor_grab()
            .on_drag(
                GroupDrag { group },
                self.group_drag_ghost(title.clone(), cx),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if this.layout.active_group() != group {
                    this.focus_group(group, cx);
                }
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(title),
            )
            .when(dirty, |row| {
                row.child(
                    super::chrome::unsaved_dot(group_element(GROUP_UNSAVED_ID, at), cx).flex_none(),
                )
            })
            .child(menu)
            .child(close)
            .into_any_element()
    }
}
