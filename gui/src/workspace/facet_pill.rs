//! The pills and list rows of the **Facet view** ([`super::facet`]): a
//! picked facet's pill in the bar, and a facet's row in an open list. Both
//! show the facet's text in its kind's colour and its count in view. Split
//! from [`super::facet_ui`], which keeps the chips, the buttons and the
//! focus ring.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Div, InteractiveElement as _, ParentElement as _, Rgba, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, div, rems,
};
use kimun_core::note::FacetKind;
use kimun_notes::logic::facet_bar::Pill;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::settings::themes::Theme;

use super::facet::rows::{self, button};
use super::facet_ui::{TICK_ICON, toggled};
use super::rail_icon;
use crate::theme;

/// A pill's padding on each side (`px_1p5`), in rems.
const PILL_PAD_REMS: f32 = 0.375;
/// The gap between a pill's parts (`gap_1`), in rems.
const PILL_GAP_REMS: f32 = 0.25;
/// A pill's icon (the outline icon, the remove mark), square, in rems
/// (`size_3`).
const PILL_ICON_REMS: f32 = 0.75;

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
        .text_size(rows::small_text())
        .map(|pill| {
            if p.picked {
                let selected = theme::selected_background(t);
                let press = theme::pressed_background(selected, t);
                pill.border_color(theme::accent(t))
                    .bg(selected)
                    .active(move |style| style.bg(press))
            } else {
                let press = theme::pressed_background(hover, t);
                pill.border_color(theme::divider(t))
                    .hover(move |style| style.bg(hover))
                    .active(move |style| style.bg(press))
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
            .flex()
            .text_color(theme::accent(t))
            .child(Icon::new(TICK_ICON).size_3())
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
    div()
        .id(id)
        .role(Role::ListBoxOption)
        .aria_label(SharedString::from(label.clone()))
        .aria_description(SharedString::from(in_view_text(p.in_view)))
        .aria_selected(lit)
        .aria_toggled(toggled(p.picked))
        .flex()
        .flex_none()
        .w_full()
        .h_6()
        .items_center()
        .gap(rems(PILL_GAP_REMS))
        .whitespace_nowrap()
        .px_2()
        .rounded_md()
        .text_size(rows::small_text())
        .map(|row| {
            if lit {
                let selected = theme::selected_background(t);
                let press = theme::pressed_background(selected, t);
                row.bg(selected).active(move |style| style.bg(press))
            } else {
                let press = theme::pressed_background(hover, t);
                row.hover(move |style| style.bg(hover))
                    .active(move |style| style.bg(press))
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
