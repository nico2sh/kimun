//! Drawing helpers of the **Facet view** ([`super::facet`]): the chips,
//! pills, text buttons and row icons its pinned bar and pieces are drawn
//! with.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Div, InteractiveElement as _, IntoElement, ParentElement as _, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, div,
};
use kimun_core::note::FacetKind;
use kimun_notes::logic::facet_bar::Pill;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::settings::themes::Theme;

use super::facet::row_leader_id;
use super::rail_icon;
use crate::theme;

/// The icon that leads `kind`'s row: the rail's icon for the drawer view
/// of that kind, named for screen readers and in a tooltip.
pub(super) fn leader(kind: FacetKind, t: &Theme) -> impl IntoElement {
    let (view, name) = match kind {
        FacetKind::Tag => (DrawerView::Tags, "Tags"),
        FacetKind::NoteLink => (DrawerView::Links, "Links"),
        FacetKind::Heading => (DrawerView::Outline, "Sections"),
    };
    div()
        .id(row_leader_id(kind))
        .role(Role::Image)
        .aria_label(name)
        .flex()
        .flex_none()
        .size_6()
        .items_center()
        .justify_center()
        .text_color(theme::muted(t))
        .tooltip(move |window, cx| Tooltip::new(name).build(window, cx))
        .child(Icon::new(rail_icon(view)).size_4())
        .test_support()
}

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
pub(super) const OUTER_CRUMB_SHRINK: f32 = 1000.;

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

/// A facet's pill: its label in its kind's colour, its count when more
/// than one, filled when picked.
pub(super) fn pill(id: SharedString, p: &Pill, t: &Theme) -> Stateful<Div> {
    let hover = theme::hover_background(t);
    let (label, color) = match p.facet.kind {
        FacetKind::Tag => (format!("#{}", p.label), theme::tag(t)),
        FacetKind::Heading => (p.label.clone(), theme::bright(t)),
        FacetKind::NoteLink => (p.label.clone(), theme::accent(t)),
    };
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(SharedString::from(label.clone()))
        .flex()
        .flex_none()
        .h_6()
        .items_center()
        .gap_1()
        .whitespace_nowrap()
        .px_1p5()
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
        .child(div().text_color(color).child(label))
        .when(p.count > 1, |pill| {
            pill.child(div().text_color(theme::muted(t)).child(p.count.to_string()))
        })
}
