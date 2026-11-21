//! Drawing helpers of the **Facet view** ([`super::facet`]): the chips,
//! pills, list rows and text buttons its pinned bar, its lists and its
//! pieces are drawn with. What can be pressed has its label in the text
//! colour ([`label_color`]); counts and other meta text stay muted.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Div, InteractiveElement as _, ParentElement as _, Pixels, Rgba, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use kimun_notes::settings::themes::Theme;

use super::facet::rows::{self, button};
use crate::theme;

/// The focus ring's width: a physical stroke, so a fixed pixel size.
const RING_WIDTH: f32 = 2.0;
/// The clear gap between a stop's edge and its ring, so the ring reads as
/// its own mark even where the stop's border has the ring's colour.
const RING_GAP: f32 = 1.0;
/// How far the ring reaches outside the stop: the gap, then the stroke.
const RING_OUT: f32 = RING_GAP + RING_WIDTH;
/// The border every tab stop draws (`border_1`, clear on a text button).
const STOP_BORDER: f32 = 1.0;
/// The stops' corner radius (`rounded_md`), in rems.
const STOP_RADIUS_REMS: f32 = 0.375;

/// The search chip's icon, before its `Find`.
pub(super) const SEARCH_ICON: IconName = IconName::Search;
/// The caret after a menu chip's label: the chip opens a list.
pub(super) const CARET_ICON: IconName = IconName::ChevronDown;
/// The tick before a picked row's label.
pub(super) const TICK_ICON: IconName = IconName::Check;
/// The bar's icons, one Lucide family with the gap and pill icons (tests).
#[cfg(test)]
pub(crate) const BAR_ICONS: [IconName; 3] = [SEARCH_ICON, CARET_ICON, TICK_ICON];

/// The colour of a button's label: the text colour, so what can be pressed
/// stands apart from the muted meta text (counts, the hint, the summary). A
/// `dimmed` chip keeps the muted colour, so its dimming shows.
pub(super) fn label_color(dimmed: bool, t: &Theme) -> Rgba {
    if dimmed {
        theme::muted(t)
    } else {
        theme::foreground(t)
    }
}

/// The fills of a control: under the pointer and while pressed. The press
/// is one step stronger than the hover ([`theme::pressed_background`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Fills {
    pub(crate) hover: Rgba,
    pub(crate) press: Rgba,
}

impl Fills {
    /// The fills of a control on the view's page.
    pub(super) fn page(t: &Theme) -> Self {
        Self::from_hover(theme::hover_background(t), t)
    }

    /// The fills that start from `hover`.
    fn from_hover(hover: Rgba, t: &Theme) -> Self {
        Self {
            hover,
            press: theme::pressed_background(hover, t),
        }
    }
}

/// The colours of a piece's crumbs strip and of the stops on it. They start
/// from the page ([`theme::facet_strip_background`]), so the strip is a faint
/// step off the view in every theme. Found by a search: the view takes them
/// once per frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Strip {
    pub(crate) bg: Rgba,
    pub(crate) fills: Fills,
}

impl Strip {
    /// The strip colours of `t`.
    pub(crate) fn of(t: &Theme) -> Self {
        Self {
            bg: theme::facet_strip_background(t),
            fills: Fills::from_hover(theme::facet_strip_hover(t), t),
        }
    }
}

/// The colour of a crumb's label: muted at rest, so the crumbs stay quieter
/// than the text under them. The text colour while it has the keyboard
/// (`focused`), as under the pointer.
pub(crate) fn crumb_color(focused: bool, t: &Theme) -> Rgba {
    if focused {
        theme::foreground(t)
    } else {
        theme::muted(t)
    }
}

/// A small bordered button, in the style of the sort chip, 24 px tall, its
/// label in [`label_color`]. A `dimmed` chip (a kind chip with no result in
/// view) has a fainter border and a muted label. It is still a button.
pub(super) fn chip(
    id: SharedString,
    label: impl Into<SharedString>,
    dimmed: bool,
    t: &Theme,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    chip_frame(id, label.clone(), dimmed, t).child(label)
}

/// A [`chip`] with `icon` before its `label`: the search chip.
pub(super) fn icon_label_chip(
    id: SharedString,
    icon: IconName,
    label: &'static str,
    t: &Theme,
) -> Stateful<Div> {
    chip_frame(id, label.into(), false, t)
        .gap_1()
        .child(Icon::new(icon).size_3())
        .child(label)
}

/// What the chips share: the frame named `name`, no label drawn yet. It
/// keeps the arrow pointer, as a native button does. A press fills it one
/// step past its hover.
fn chip_frame(id: SharedString, name: SharedString, dimmed: bool, t: &Theme) -> Stateful<Div> {
    let fills = Fills::page(t);
    let border = if dimmed {
        theme::divider_faint(t)
    } else {
        theme::divider(t)
    };
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(name)
        .flex()
        .flex_none()
        .h(button())
        .min_w(button())
        .items_center()
        .justify_center()
        .whitespace_nowrap()
        .px_1p5()
        .rounded_md()
        .border_1()
        .border_color(border)
        .text_size(rows::small_text())
        .text_color(label_color(dimmed, t))
        .hover(move |style| style.bg(fills.hover))
        .active(move |style| style.bg(fills.press))
}

/// A [`chip`] that shows an icon, 24 px square, named `label` for screen
/// readers and in a tooltip.
pub(super) fn icon_chip(
    id: SharedString,
    icon: IconName,
    label: &'static str,
    t: &Theme,
) -> Stateful<Div> {
    let fills = Fills::page(t);
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .flex()
        .flex_none()
        .size(button())
        .min_w(button())
        .items_center()
        .justify_center()
        .rounded_md()
        .border_1()
        .border_color(theme::divider(t))
        .text_color(theme::muted(t))
        .hover(move |style| style.bg(fills.hover))
        .active(move |style| style.bg(fills.press))
        .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
        .child(Icon::new(icon).size_3())
}

/// A button drawn as plain text in [`label_color`] with no border, 24 px
/// tall, on the page: a gap's `Show all`, `Refresh`.
pub(super) fn text_button(
    id: SharedString,
    label: impl Into<SharedString>,
    t: &Theme,
) -> Stateful<Div> {
    text_button_on(id, label, Fills::page(t), t)
}

/// A [`text_button`] on a surface with its own `fills`: a piece's `Open
/// note` on the crumbs strip.
pub(super) fn text_button_on(
    id: SharedString,
    label: impl Into<SharedString>,
    fills: Fills,
    t: &Theme,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    text_frame(id, label.clone(), fills, t)
        .flex_none()
        .min_w(button())
        .justify_center()
        .child(label)
}

/// How much faster an outer crumb shrinks than the innermost one: outer
/// crumbs give up their room first.
const OUTER_CRUMB_SHRINK: f32 = 1000.;

/// A crumb of a piece's breadcrumb: a [`text_button`] that shrinks when the
/// row is too narrow, its label cut with an ellipsis. Outer crumbs shrink
/// first; the innermost (`last`) only once they are down to a letter and
/// the ellipsis. A row too narrow even for that is cut at its right end.
/// Its label is in [`crumb_color`]: muted at rest, the text colour under
/// the pointer and while `focused`. It takes the strip's `fills`. A crumb
/// whose heading is `picked` shows it as a picked pill does: the selected
/// fill (kept under the pointer), a toggled state, and the name `Remove
/// <heading>`, since a press removes the pick.
pub(super) fn crumb(
    id: SharedString,
    label: impl Into<SharedString>,
    last: bool,
    (picked, focused): (bool, bool),
    fills: Fills,
    t: &Theme,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    let name = match picked {
        true => SharedString::from(format!("Remove {label}")),
        false => label.clone(),
    };
    let selected = theme::selected_background(t);
    let fills = match picked {
        true => Fills::from_hover(selected, t),
        false => fills,
    };
    // A shrunk crumb keeps a letter and its ellipsis: never an invisible
    // tab stop. The frame's hover gives the text colour back.
    let mut el = text_frame(id, name, fills, t)
        .aria_toggled(toggled(picked))
        .when(picked, |el| el.bg(selected))
        .text_color(crumb_color(focused, t))
        .min_w_8()
        .child(div().min_w_0().truncate().child(label));
    el.style().flex_shrink = Some(if last { 1. } else { OUTER_CRUMB_SHRINK });
    el
}

/// The focus ring of the tab stop `id`: a [`RING_WIDTH`] stroke in
/// [`theme::focus_ring`], [`RING_GAP`] outside the stop's own box. It is
/// placed absolutely, so it takes no room and moves nothing. The gap keeps
/// it apart from any border the stop draws (a picked pill's accent). Its
/// corners follow the stop's at the rem size `rem`. The stop is its parent;
/// containers that clip keep [`ring_room`] inside their edge.
pub(super) fn focus_ring(id: &str, rem: Pixels, t: &Theme) -> impl gpui_kit::IntoElement {
    // An absolute child is placed from inside its parent's border: past it.
    let out = px(-(RING_OUT + STOP_BORDER));
    div()
        .id(ring_id(id))
        .absolute()
        .top(out)
        .left(out)
        .right(out)
        .bottom(out)
        .border_2()
        .border_color(theme::focus_ring(t))
        .rounded(ring_radius(rem))
        .test_support()
}

/// The ring's corner radius at the rem size `rem`: the stop's own radius
/// pushed out by [`RING_OUT`], so the ring runs parallel to its corners.
pub(crate) fn ring_radius(rem: Pixels) -> Pixels {
    rem * STOP_RADIUS_REMS + px(RING_OUT)
}

/// How far a text button's label sits inside the button's edge at the rem
/// size `rem`: its clear border, then its padding ([`rows::text_pad`]).
pub(crate) fn label_inset(rem: Pixels) -> Pixels {
    px(STOP_BORDER) + rows::text_pad().to_pixels(rem)
}

/// The room a clipping container keeps inside its edge, so the focus ring of
/// a stop at that edge shows whole.
pub(super) fn ring_room() -> Pixels {
    px(RING_OUT)
}

/// What [`text_button`] and [`crumb`] share: the frame, no label drawn yet,
/// with the hover and press of `fills`. Under the pointer the label takes
/// the text colour, whatever colour it has at rest.
fn text_frame(id: SharedString, label: SharedString, fills: Fills, t: &Theme) -> Stateful<Div> {
    let hover = fills.hover;
    let text = label_color(false, t);
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .flex()
        .h(button())
        .items_center()
        .whitespace_nowrap()
        .px(rows::text_pad())
        .rounded_md()
        // A clear border, so a text button lines up with the bordered chips.
        .border_1()
        .border_color(gpui_kit::transparent_black())
        .text_size(rows::small_text())
        .text_color(text)
        .hover(move |style| style.bg(hover).text_color(text))
        .active(move |style| style.bg(fills.press).text_color(text))
}

/// The id of the focus ring around the tab stop `id`.
pub(crate) fn ring_id(id: &str) -> SharedString {
    format!("{id}:ring").into()
}

/// The toggled state a screen reader hears for a control that is `on`.
pub(super) fn toggled(on: bool) -> gpui_kit::accesskit::Toggled {
    match on {
        true => gpui_kit::accesskit::Toggled::True,
        false => gpui_kit::accesskit::Toggled::False,
    }
}
