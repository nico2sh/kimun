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
    StatefulInteractiveElement as _, Styled as _, div, px, rems,
};
use kimun_core::note::FacetKind;
use kimun_notes::logic::facet_bar::Pill;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::settings::themes::Theme;

use super::facet::rows::button;
use super::rail_icon;
use crate::theme;

/// A pill's padding on each side (`px_1p5`), in rems.
const PILL_PAD_REMS: f32 = 0.375;
/// The gap between a pill's parts (`gap_1`), in rems.
const PILL_GAP_REMS: f32 = 0.25;
/// A pill's icon (the outline icon, the remove mark), square, in rems
/// (`size_3`).
const PILL_ICON_REMS: f32 = 0.75;

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

/// The tick before a picked row's label.
const TICK: &str = "\u{2713}";

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

/// A small bordered button, in the style of the sort chip, 24 px tall, its
/// label in [`label_color`]. A `dimmed` chip (a kind chip with no result in
/// view) has a fainter border and a muted label. It is still a button.
pub(super) fn chip(
    id: SharedString,
    label: impl Into<SharedString>,
    dimmed: bool,
    t: &Theme,
) -> Stateful<Div> {
    let hover = theme::hover_background(t);
    let border = if dimmed {
        // The divider colour, halfway to the background.
        theme::mix_rgba(theme::divider(t), theme::background(t), 0.5)
    } else {
        theme::divider(t)
    };
    let label: SharedString = label.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
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
        .text_xs()
        .text_color(label_color(dimmed, t))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(label)
}

/// A [`chip`] that shows an icon, 24 px square, named `label` for screen
/// readers and in a tooltip.
pub(super) fn icon_chip(
    id: SharedString,
    icon: IconName,
    label: &'static str,
    t: &Theme,
) -> Stateful<Div> {
    let hover = theme::hover_background(t);
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
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
        .child(Icon::new(icon).size_3())
}

/// A button drawn as plain text in [`label_color`] with no border, 24 px
/// tall: a piece's `Open note`, a gap's `Show all`.
pub(super) fn text_button(
    id: SharedString,
    label: impl Into<SharedString>,
    t: &Theme,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    text_frame(id, label.clone(), t)
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
pub(super) fn crumb(
    id: SharedString,
    label: impl Into<SharedString>,
    last: bool,
    t: &Theme,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    // A shrunk crumb keeps a letter and its ellipsis: never an invisible
    // tab stop.
    let mut el = text_frame(id, label.clone(), t)
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

/// The room a clipping container keeps inside its edge, so the focus ring of
/// a stop at that edge shows whole.
pub(super) fn ring_room() -> Pixels {
    px(RING_OUT)
}

/// What [`text_button`] and [`crumb`] share: the frame, no label drawn yet.
fn text_frame(id: SharedString, label: SharedString, t: &Theme) -> Stateful<Div> {
    let hover = theme::hover_background(t);
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .flex()
        .h(button())
        .items_center()
        .whitespace_nowrap()
        .px_1()
        .rounded_md()
        // A clear border, so a text button lines up with the bordered chips.
        .border_1()
        .border_color(gpui_kit::transparent_black())
        .text_xs()
        .text_color(label_color(false, t))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
}

/// A facet's text on its pill: the kind in its own syntax, a tag `#name`,
/// a note link `[[name]]`, a heading its text.
pub(super) fn pill_text(p: &Pill) -> String {
    match p.facet.kind {
        FacetKind::Tag => format!("#{}", p.label),
        FacetKind::NoteLink => format!("[[{}]]", p.label),
        FacetKind::Heading => p.label.clone(),
    }
}

/// Whether `p` draws the outline icon in front of its text: a picked
/// heading.
fn has_outline(p: &Pill) -> bool {
    p.picked && p.facet.kind == FacetKind::Heading
}

/// The id of the outline icon on the pill `id`.
pub(super) fn outline_mark(id: &str) -> SharedString {
    format!("{id}:outline").into()
}

/// The id of the remove mark on the pill `id`.
pub(super) fn remove_mark(id: &str) -> SharedString {
    format!("{id}:remove").into()
}

/// The id of the focus ring around the tab stop `id`.
pub(crate) fn ring_id(id: &str) -> SharedString {
    format!("{id}:ring").into()
}

/// The id of the tick on the list row `id`.
pub(super) fn row_tick(id: &str) -> SharedString {
    format!("{id}:tick").into()
}

/// The id of the count on the pill `id`.
pub(super) fn pill_count(id: &str) -> SharedString {
    format!("{id}:count").into()
}

/// The id of the count on the list row `id`.
pub(super) fn row_count(id: &str) -> SharedString {
    format!("{id}:count").into()
}

/// Whether the list row of `p` is dimmed: it gives nothing in the current
/// view and is not picked. A dimmed row can still be picked.
pub(super) fn dimmed(p: &Pill) -> bool {
    p.in_view == 0 && !p.picked
}

/// A facet's pill: the outline icon of a picked heading, its text
/// ([`pill_text`]) in its kind's colour, its count in view
/// ([`Pill::in_view`], what its list row shows) when more than one, and
/// a picked pill's remove mark; filled when picked. 24 px tall. The marks
/// are part of the pill: a press anywhere on it toggles it.
pub(super) fn pill(id: SharedString, p: &Pill, t: &Theme) -> Stateful<Div> {
    let hover = theme::hover_background(t);
    let color = kind_color(p.facet.kind, t);
    let label = pill_text(p);
    // A press on a picked pill unpicks it: its name says so.
    let name = if p.picked {
        format!("Remove {label}")
    } else {
        label.clone()
    };
    let muted = theme::muted(t);
    let count_id = pill_count(&id);
    let outline =
        has_outline(p).then(|| mark(outline_mark(&id), rail_icon(DrawerView::Outline), muted));
    let remove = p
        .picked
        .then(|| mark(remove_mark(&id), IconName::Close, muted));
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(SharedString::from(name))
        .flex()
        .flex_none()
        // A long name is cut with an ellipsis: the marks stay in view.
        .max_w_full()
        .h(button())
        .items_center()
        .gap(rems(PILL_GAP_REMS))
        .whitespace_nowrap()
        .px(rems(PILL_PAD_REMS))
        .rounded_md()
        .border_1()
        .text_xs()
        .cursor_pointer()
        .map(|pill| {
            if p.picked {
                pill.border_color(theme::accent(t))
                    .bg(theme::selected_background(t))
            } else {
                pill.border_color(theme::divider(t))
                    .hover(move |style| style.bg(hover))
            }
        })
        .children(outline)
        .child(div().min_w_0().truncate().text_color(color).child(label))
        .when(p.in_view > 1, |pill| {
            let n = p.in_view.to_string();
            pill.child(
                div()
                    .id(count_id)
                    .flex_none()
                    .aria_label(SharedString::from(n.clone()))
                    .text_color(muted)
                    .child(n)
                    .test_support(),
            )
        })
        .children(remove)
}

/// The colour of a facet's text: its kind's.
fn kind_color(kind: FacetKind, t: &Theme) -> Rgba {
    match kind {
        FacetKind::Tag => theme::tag(t),
        FacetKind::Heading => theme::bright(t),
        FacetKind::NoteLink => theme::accent(t),
    }
}

/// The words that name a row's count for screen readers.
fn in_view_text(n: usize) -> String {
    match n {
        0 => "No pieces in view".into(),
        1 => "1 piece in view".into(),
        n => format!("{n} pieces in view"),
    }
}

/// A facet's row in an open list, as wide as the list and 24 px tall: a
/// slot that holds the tick of a picked facet, its text ([`pill_text`]) cut
/// with an ellipsis when long, and at the right end its count in view
/// ([`Pill::in_view`]). The text is in its kind's colour, muted when the
/// row is [`dimmed`]. A lit row (`lit`, the keyboard highlight) has the
/// background of the app's other lit list rows. A press anywhere on it
/// toggles the facet.
pub(super) fn row(id: SharedString, p: &Pill, lit: bool, t: &Theme) -> Stateful<Div> {
    let hover = theme::hover_background(t);
    let muted = theme::muted(t);
    let color = if dimmed(p) {
        muted
    } else {
        kind_color(p.facet.kind, t)
    };
    let label = pill_text(p);
    let tick = p.picked.then(|| {
        div()
            .id(row_tick(&id))
            .flex_none()
            .text_color(theme::accent(t))
            .child(TICK)
            .test_support()
    });
    let n = p.in_view.to_string();
    let count = div()
        .id(row_count(&id))
        .flex_none()
        .aria_label(SharedString::from(n.clone()))
        .text_color(muted)
        .child(n)
        .test_support();
    let toggled = if p.picked {
        gpui_kit::accesskit::Toggled::True
    } else {
        gpui_kit::accesskit::Toggled::False
    };
    div()
        .id(id)
        .role(Role::ListBoxOption)
        .aria_label(SharedString::from(label.clone()))
        .aria_description(SharedString::from(in_view_text(p.in_view)))
        .aria_selected(lit)
        .aria_toggled(toggled)
        .flex()
        .flex_none()
        .w_full()
        .h_6()
        .items_center()
        .gap(rems(PILL_GAP_REMS))
        .whitespace_nowrap()
        .px_2()
        .rounded_md()
        .text_xs()
        .cursor_pointer()
        .map(|row| {
            if lit {
                row.bg(theme::selected_background(t))
            } else {
                row.hover(move |style| style.bg(hover))
            }
        })
        // A slot of fixed width, so every label starts in one column.
        .child(
            div()
                .flex()
                .flex_none()
                .w(rems(PILL_ICON_REMS))
                .justify_center()
                .children(tick),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(color)
                .child(label),
        )
        .child(count)
}

/// A small icon inside a pill, in a box of its own fixed size.
fn mark(id: SharedString, icon: IconName, color: Rgba) -> impl gpui_kit::IntoElement {
    div()
        .id(id)
        .flex()
        .flex_none()
        .size(rems(PILL_ICON_REMS))
        .items_center()
        .justify_center()
        .text_color(color)
        .child(Icon::new(icon).size_3())
        .test_support()
}
