//! Drawing helpers of the **Facet view** ([`super::facet`]): the chips,
//! pills and text buttons its pinned bar and pieces are drawn with.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Div, InteractiveElement as _, ParentElement as _, Rgba, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, div, rems,
};
use kimun_core::note::FacetKind;
use kimun_notes::logic::facet_bar::Pill;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::settings::themes::Theme;

use super::rail_icon;
use crate::theme;

/// A pill's padding on each side (`px_1p5`), in rems.
const PILL_PAD_REMS: f32 = 0.375;
/// The gap between a pill's parts (`gap_1`), in rems.
const PILL_GAP_REMS: f32 = 0.25;
/// A pill's icon (the outline icon, the remove mark), square, in rems
/// (`size_3`).
const PILL_ICON_REMS: f32 = 0.75;

/// A small bordered button, in the style of the sort chip, 24 px tall.
pub(super) fn chip(id: SharedString, label: impl Into<SharedString>, t: &Theme) -> Stateful<Div> {
    let hover = theme::hover_background(t);
    let label: SharedString = label.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .flex()
        .flex_none()
        .h_6()
        .min_w_6()
        .items_center()
        .justify_center()
        .whitespace_nowrap()
        .px_1p5()
        .rounded_md()
        .border_1()
        .border_color(theme::divider(t))
        .text_xs()
        .text_color(theme::muted(t))
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
        .size_6()
        .min_w_6()
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

/// A button drawn as muted text with no border, 24 px tall: a piece's
/// `Open source`, a gap's `Show all`.
pub(super) fn text_button(
    id: SharedString,
    label: impl Into<SharedString>,
    t: &Theme,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    text_frame(id, label.clone(), t)
        .flex_none()
        .min_w_6()
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

/// What [`text_button`] and [`crumb`] share: the frame, no label drawn yet.
fn text_frame(id: SharedString, label: SharedString, t: &Theme) -> Stateful<Div> {
    let hover = theme::hover_background(t);
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .flex()
        .h_6()
        .items_center()
        .whitespace_nowrap()
        .px_1()
        .rounded_md()
        // Room for the focus ring, so it moves nothing.
        .border_1()
        .border_color(gpui_kit::transparent_black())
        .text_xs()
        .text_color(theme::muted(t))
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

/// A facet's pill: the outline icon of a picked heading, its text
/// ([`pill_text`]) in its kind's colour, its count when more than one, and
/// a picked pill's remove mark; filled when picked. 24 px tall. The marks
/// are part of the pill: a press anywhere on it toggles it.
pub(super) fn pill(id: SharedString, p: &Pill, t: &Theme) -> Stateful<Div> {
    let hover = theme::hover_background(t);
    let color = match p.facet.kind {
        FacetKind::Tag => theme::tag(t),
        FacetKind::Heading => theme::bright(t),
        FacetKind::NoteLink => theme::accent(t),
    };
    let label = pill_text(p);
    // A press on a picked pill unpicks it: its name says so.
    let name = if p.picked {
        format!("Remove {label}")
    } else {
        label.clone()
    };
    let muted = theme::muted(t);
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
        .h_6()
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
        .when(p.count > 1, |pill| {
            pill.child(
                div()
                    .flex_none()
                    .text_color(muted)
                    .child(p.count.to_string()),
            )
        })
        .children(remove)
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
