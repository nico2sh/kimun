//! The **floating list under a field**: the frame and the placement every
//! suggestion popup shares — the query fields' completion and the query
//! builder's value suggestions. A window-level layer anchored to the field's
//! box, so it never changes the layout around it and always belongs to the
//! field it hangs from.
//!
//! What goes in the list, and what a click or a wheel turn does, is the
//! caller's: [`frame`] styles the list the caller built, [`floating_under`]
//! places it.

use gpui_kit::{
    Anchor, AnyElement, App, Bounds, Context, InteractiveElement, IntoElement, ParentElement as _,
    Pixels, ScrollEvent, Styled, Window, anchored, deferred, point, rems,
};
use kimun_notes::logic::autocomplete::PopupCommand;
use kimun_notes::settings::themes::Theme;

use crate::autocomplete::wheel_command;
use crate::theme;

/// The narrowest a popup gets, in `rem`s so it follows the interface zoom.
const MIN_WIDTH_REMS: f32 = 15.;
/// The gap between the popup and the field it hangs from.
const GAP_REMS: f32 = 0.125;
/// The room the popup keeps from the window's edge.
const MARGIN_REMS: f32 = 0.5;
/// Vertical padding of one row, and of the frame, in the height estimate.
const ROW_PAD_REMS: f32 = 0.25;
const FRAME_PAD_REMS: f32 = 0.375;

/// The popup's width: the field's, but never narrower than a suggestion is
/// readable in (a short field, such as a property key's, would otherwise get
/// a sliver). The field's box is the last frame's, so before its first paint
/// it has none and the popup takes the minimum.
pub(super) fn width_of(field: Bounds<Pixels>, window: &Window) -> Pixels {
    field
        .size
        .width
        .max(rems(MIN_WIDTH_REMS).to_pixels(window.rem_size()))
}

/// What the wheel does over a popup: `act` runs with the step the wheel's
/// direction means ([`wheel_command`]: nothing for a sideways swipe or a
/// trackpad's trailing events), and the wheel goes no further, so it never
/// scrolls what is under the popup. The one place both popups wire it.
pub(super) fn on_wheel<V: 'static>(
    cx: &Context<V>,
    act: impl Fn(&mut V, PopupCommand, &mut Window, &mut Context<V>) + 'static,
) -> impl Fn(&ScrollEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |this, event: &ScrollEvent, window, cx| {
        if let Some(command) = wheel_command(event.delta.pixel_delta(window.line_height()).y) {
            act(this, command, window, cx);
        }
        cx.stop_propagation();
    })
}

/// `list` (the caller's element, with its id and role) in the popup's frame:
/// a bordered, opaque column as wide as the field.
pub(super) fn frame<E: InteractiveElement + Styled>(list: E, width: Pixels, t: &Theme) -> E {
    list.occlude()
        .flex()
        .flex_col()
        .w(width)
        .py_0p5()
        .border_1()
        .border_color(theme::focus_border(t))
        .bg(theme::panel_background(t))
        .rounded_md()
        .overflow_hidden()
}

/// Estimated height of `rows` popup rows, for deciding which side of the
/// field has room. Only the decision uses it: the popup is laid out by its
/// content.
fn estimated_height(rows: usize, window: &Window) -> Pixels {
    let rem = window.rem_size();
    let row = rem * 1.25 + rem * ROW_PAD_REMS;
    row * rows as f32 + rem * FRAME_PAD_REMS
}

/// `list` as a layer under `field` (above it when the window has no room
/// below, so the popup never covers what is being typed). `rows` is how many
/// rows it shows, for that decision.
pub(super) fn floating_under(
    field: Bounds<Pixels>,
    rows: usize,
    list: impl IntoElement,
    window: &Window,
) -> AnyElement {
    let margin = rems(MARGIN_REMS).to_pixels(window.rem_size());
    let gap = rems(GAP_REMS).to_pixels(window.rem_size());
    let height = estimated_height(rows, window);
    let flip = field.bottom() + gap + height > window.viewport_size().height - margin
        && field.top() - gap - height >= margin;
    let (at, corner) = if flip {
        (point(field.left(), field.top() - gap), Anchor::BottomLeft)
    } else {
        (point(field.left(), field.bottom() + gap), Anchor::TopLeft)
    };
    deferred(
        anchored()
            .anchor(corner)
            .position(at)
            .snap_to_window_with_margin(margin)
            .child(list),
    )
    // The kit's popup priority: above a dialog (which draws at 10 and up,
    // so a popup inside one would otherwise be behind it), below tooltips.
    // It is the Selects' and the menus' priority too: one of these popups
    // shows only while its field has the keyboard, which opening a Select or
    // a menu takes away, so two never stack.
    .with_priority(gpui_kit::base::POPUP_PRIORITY)
    .into_any_element()
}
