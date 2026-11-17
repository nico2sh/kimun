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
    Anchor, AnyElement, Bounds, InteractiveElement, IntoElement, ParentElement as _, Pixels,
    Styled, Window, anchored, deferred, point, px,
};
use kimun_notes::settings::themes::Theme;

use crate::theme;

/// The popup's width: the field's. The field's box is the last frame's, so
/// before its first paint it has none and the popup takes a usable one.
pub(super) fn width_of(field: Bounds<Pixels>) -> Pixels {
    if field.size.width > px(0.) {
        field.size.width
    } else {
        px(240.)
    }
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
    let row = window.rem_size() * 1.25 + px(4.);
    row * rows as f32 + px(6.)
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
    let margin = px(8.);
    let gap = px(2.);
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
    .with_priority(2)
    .into_any_element()
}
