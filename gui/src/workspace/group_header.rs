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
//! strip's do. It is also where dragging a group's note will start (a
//! later milestone): its id is stable for that, and nothing is draggable
//! yet.
//!
//! The **target marker**: while the keyboard is outside the editor (in a
//! slot or on the rail) and the tab shows two or more groups, the active
//! group wears a bar in the focus colour along its top. It says which note
//! a plain open (from FILES, the finder, a link) replaces. With one group
//! there is no doubt, and nothing is drawn.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, ElementId, InteractiveElement, IntoElement,
    ParentElement, Role, SharedString, StatefulInteractiveElement, Styled, div, rems,
};
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::layout_tree::{Focus, GroupId, Leaf};

use super::Workspace;
use super::tab_strip::{EMPTY_LABEL, group_name};
use crate::{keymap, theme};

/// A group's element id prefix (with its position in the shown tab).
pub(crate) const GROUP_ID: &str = "editor-group";
/// A header's id prefix.
pub(crate) const GROUP_HEADER_ID: &str = "group-header";
/// A header's unsaved dot's id prefix.
pub(crate) const GROUP_UNSAVED_ID: &str = "group-unsaved";
/// A header's close button's id prefix.
pub(crate) const GROUP_CLOSE_ID: &str = "group-close";
/// The target marker's id prefix.
pub(crate) const GROUP_TARGET_ID: &str = "group-target";
/// The close glyph: the kit's own close icon, as on a tab.
const CLOSE_ICON: IconName = IconName::Close;
/// Every glyph a header draws, for the bundle test: an icon the asset
/// source does not carry paints nothing.
#[cfg(test)]
pub(crate) const ICONS: [IconName; 1] = [CLOSE_ICON];

/// `prefix` with the group's position in the shown tab (reading order).
pub(crate) fn group_element(prefix: &'static str, at: usize) -> ElementId {
    ElementId::NamedInteger(prefix.into(), at as u64)
}

/// What `group`'s header calls it: the name of what it shows, as the
/// Footer names it, or `Empty`.
pub(crate) fn group_title(workspace: &Workspace, group: GroupId, cx: &App) -> SharedString {
    group_name(workspace, group, cx)
        .unwrap_or_else(|| EMPTY_LABEL.to_string())
        .into()
}

impl Workspace {
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

    /// The target marker over the group at `at`: a bar in the focus colour
    /// along its top. Laid over the group, it takes no room and no pointer.
    pub(super) fn render_target_marker(&self, at: usize, cx: &App) -> AnyElement {
        div()
            .id(group_element(GROUP_TARGET_ID, at))
            .test_support()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h_0p5()
            .bg(theme::focus_border(&theme::current(cx)))
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
        let hover = theme::hover_background(&theme);
        let spoken: SharedString = if dirty {
            format!("{title}, unsaved").into()
        } else {
            title.clone()
        };
        let close_tip: SharedString = match keymap::chord_label(cx, &ActionShortcuts::CloseSlot) {
            Some(chord) => format!("Close group ({chord})").into(),
            None => "Close group".into(),
        };
        let close = div()
            .id(group_element(GROUP_CLOSE_ID, at))
            .test_support()
            .role(Role::Button)
            .aria_label("Close group")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(rems(1.125))
            .rounded_sm()
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .child(Icon::new(CLOSE_ICON).size_3())
            .tooltip(move |window, cx| Tooltip::new(close_tip.clone()).build(window, cx))
            // The header's own click (activate the group) does not follow.
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.close_editor_group(group, cx);
            }));
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
                    div()
                        .id(group_element(GROUP_UNSAVED_ID, at))
                        .test_support()
                        .flex_none()
                        .size(rems(0.375))
                        .rounded_full()
                        .bg(theme::unsaved(&theme)),
                )
            })
            .child(close)
            .into_any_element()
    }
}
