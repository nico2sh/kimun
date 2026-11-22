//! The overlay the pointer-anchored menus share: the **note menu**
//! ([`super::note_menu`]) and the **Link menu** ([`super::link_menu`]).
//!
//! Both are drawn by hand rather than with the kit's `PopupMenu`, on
//! purpose: they pick by a single letter (`R`, `M`, `D`), hide an item that
//! cannot act, hand a key they do not use to their parent, and anchor at the
//! pointer or at a group's corner. The kit menu has none of that. What they
//! share with it is the surface: [`overlay`] draws the kit's popover look
//! (its fill, ring shadow and radius), so a menu from the editor and one
//! from a list row (the kit's `ContextMenu`) read as one family.

use gpui_kit::component::ThemeStyled as _;
use gpui_kit::{
    Anchor, AnyElement, App, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    ParentElement as _, Pixels, Point, Styled as _, Window, anchored, deferred, div, point, px,
    rems,
};

use gpui_kit::base::TestSupportExt as _;

/// How far the menu keeps from the window's edges, in rems.
const EDGE_MARGIN_REMS: f32 = 0.5;

/// Where and how [`overlay`] draws a menu.
pub(super) struct Placement {
    /// The element id of the window-wide layer under the menu.
    pub(super) layer_id: &'static str,
    /// The menu's width, in rems.
    pub(super) width_rems: f32,
    /// The point the menu hangs from, in window coordinates.
    pub(super) at: Point<Pixels>,
    /// The corner of the menu that sits at `at`.
    pub(super) anchor: Anchor,
}

/// `view` as a menu at `place`, over a window-wide layer that runs
/// `dismiss` on any press and keeps the wheel from scrolling what is under
/// it. It is kept inside the window.
pub(super) fn overlay(
    place: Placement,
    view: impl IntoElement,
    window: &Window,
    cx: &App,
    dismiss: impl Fn(&MouseDownEvent, &mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let Placement {
        layer_id,
        width_rems,
        at,
        anchor,
    } = place;
    let size = window.viewport_size();
    let menu = div()
        .occlude()
        .w(rems(width_rems))
        .p_1()
        .popover_style(cx)
        .child(view);
    deferred(
        anchored().position(point(px(0.), px(0.))).child(
            div()
                .id(layer_id)
                .test_support()
                .occlude()
                .w(size.width)
                .h(size.height)
                .on_mouse_down(MouseButton::Left, dismiss.clone())
                .on_mouse_down(MouseButton::Right, dismiss.clone())
                .on_mouse_down(MouseButton::Middle, dismiss)
                .on_scroll(|_, _, cx| cx.stop_propagation())
                .child(
                    anchored()
                        .position(at)
                        .anchor(anchor)
                        .snap_to_window_with_margin(
                            rems(EDGE_MARGIN_REMS).to_pixels(window.rem_size()),
                        )
                        .child(menu),
                ),
        ),
    )
    .with_priority(gpui_kit::base::POPUP_PRIORITY)
    .into_any_element()
}
