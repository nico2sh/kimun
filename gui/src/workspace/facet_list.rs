//! The **Facet view**'s lists ([`super::facet`]): the kit's [`Popover`]
//! anchored to what opened it, floating over the view so nothing under it
//! moves. The list of every tag and link hangs from the `+N` chip, or from
//! the end of the filter line when every pill is inline (`/` opens it
//! then); the headings list hangs from the `Sections N` chip. Its filter
//! field comes first and takes the keyboard when it opens; Escape and a
//! press outside close it (the kit's dismissal), and a press outside still
//! does what it does there. The view owns the open state and passes it in
//! ([`Popover::open`]); the kit reports a dismissal through
//! `on_open_change`.

use gpui_kit::base::{Selectable, TestSupportExt as _};
use gpui_kit::component::input::InputState;
use gpui_kit::component::popover::Popover;
use gpui_kit::{
    Anchor, AnyElement, App, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, ParentElement as _, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, px,
};
use kimun_notes::settings::themes::Theme;

use super::facet::{ListKind, MORE_LIST_ID, MORE_QUERY_ID};
use super::popup_layer;
use super::query_field::query_field;
use crate::theme;

/// The list's width: room for a few pills a line.
const LIST_WIDTH: f32 = 320.;
/// The most the pills take before they scroll inside the list.
const PILLS_MAX_HEIGHT: f32 = 280.;

/// The popover of `kind`'s list.
fn popover_id(kind: ListKind) -> SharedString {
    match kind {
        ListKind::Facets => "facet-list:facets".into(),
        ListKind::Sections => "facet-list:sections".into(),
    }
}

/// The kit's popover asks its trigger to be [`Selectable`] (to mark it open);
/// a chip shows no open state, so this wraps any element as one.
struct Trigger(AnyElement);

impl Selectable for Trigger {
    fn selected(self, _: bool) -> Self {
        self
    }

    fn is_selected(&self) -> bool {
        false
    }
}

impl IntoElement for Trigger {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        self.0
    }
}

/// A group of the open list: a small muted label (`id`, text) over its
/// pills. The headings list is one group with no label.
pub(super) struct ListGroup {
    pub label: Option<(SharedString, &'static str)>,
    pub pills: Vec<AnyElement>,
}

/// The open list: its field (`input`, which takes the keyboard) and its
/// groups of pills, in the frame the app's other floating lists use.
pub(super) struct OpenList {
    pub input: Entity<InputState>,
    pub groups: Vec<ListGroup>,
}

/// `trigger` with `kind`'s list hung under it, opening toward `anchor`. A
/// press on the trigger opens or closes the list; `changed` hears every
/// open and close the kit makes (a press on the trigger, Escape, a press
/// outside). `list` is `Some` while the view holds the list open.
pub(super) fn list_popover(
    kind: ListKind,
    anchor: Anchor,
    trigger: AnyElement,
    list: Option<OpenList>,
    changed: impl Fn(&bool, &mut Window, &mut App) + 'static,
    t: &Theme,
    cx: &App,
) -> AnyElement {
    let popover = Popover::new(popover_id(kind))
        .appearance(false)
        .anchor(anchor)
        .trigger(Trigger(trigger))
        .open(list.is_some())
        .on_open_change(changed);
    match list {
        Some(list) => {
            let focus: FocusHandle = list.input.read(cx).focus_handle(cx);
            popover
                .track_focus(&focus)
                .child(frame(list, t))
                .into_any_element()
        }
        None => popover.into_any_element(),
    }
}

fn frame(list: OpenList, t: &Theme) -> impl IntoElement {
    let groups = list.groups.into_iter().map(|group| {
        let label = group.label.map(|(id, text)| {
            div()
                .id(id)
                .role(Role::Label)
                .aria_label(text)
                .text_xs()
                .text_color(theme::muted(t))
                .child(text)
                .test_support()
        });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .children(label)
            .child(div().flex().flex_wrap().gap_1().children(group.pills))
    });
    popup_layer::frame(
        div()
            .id(MORE_LIST_ID)
            .role(Role::Group)
            .aria_label("Facets")
            .gap_1()
            .pt_2()
            .pb_1(),
        px(LIST_WIDTH),
        t,
    )
    .child(query_field(&list.input, MORE_QUERY_ID, "Filter facets", t))
    .child(
        div()
            .id("facet-list-pills")
            .flex()
            .flex_col()
            .gap_2()
            .px_2()
            .pb_1()
            .max_h(px(PILLS_MAX_HEIGHT))
            .overflow_y_scroll()
            .children(groups),
    )
    .test_support()
}
