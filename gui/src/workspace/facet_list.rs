//! The **Facet view**'s lists ([`super::facet`]): the kit's [`Popover`]
//! anchored to the chip that opened it, floating over the view so nothing
//! under it moves. A kind's list hangs from its menu chip. The list of every
//! kind hangs from the search chip, which `/` opens too. Its filter field
//! comes first and takes the keyboard when it opens. Under it the rows stand
//! one per line, as wide as the list, under a small label per kind in the
//! list of every kind. A filter that matches nothing leaves one muted line
//! that says so. Escape and a press outside close it (the kit's dismissal),
//! and a press outside still does what it does there. The view owns the
//! open state and passes it in ([`Popover::open`]). The kit reports a
//! dismissal through `on_open_change`. The chip that opened the list is
//! told the list is open, so it is drawn open and reports it.

use gpui_kit::base::{Selectable, TestSupportExt as _};
use gpui_kit::component::input::InputState;
use gpui_kit::component::popover::Popover;
use gpui_kit::{
    Anchor, AnyElement, App, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, ParentElement as _, Role, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use kimun_notes::settings::themes::Theme;

use super::facet::{LIST_EMPTY_ID, LIST_REST_ID, ListKind, MORE_LIST_ID, MORE_QUERY_ID};
use super::popup_layer;
use super::query_field::query_field;
use crate::theme;

/// The list's width: room for a long label and its count.
const LIST_WIDTH: f32 = 320.;
/// The most the rows take before they scroll inside the list.
const ROWS_MAX_HEIGHT: f32 = 280.;

/// The popover of `kind`'s list.
fn popover_id(kind: ListKind) -> SharedString {
    match kind {
        ListKind::Tags => "facet-list:tags".into(),
        ListKind::Links => "facet-list:links".into(),
        ListKind::Sections => "facet-list:sections".into(),
        ListKind::All => "facet-list:all".into(),
    }
}

/// The kit's popover asks its trigger to be [`Selectable`] (to mark it
/// open). The view draws the chip open itself, so this wraps the drawn chip
/// and only reports the open state the view gave it (`open`).
struct Trigger {
    el: AnyElement,
    open: bool,
}

impl Selectable for Trigger {
    fn selected(self, _: bool) -> Self {
        self
    }

    fn is_selected(&self) -> bool {
        self.open
    }
}

impl IntoElement for Trigger {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        self.el
    }
}

/// A group of the open list: a small muted label (`id`, text) over its
/// rows. A kind's list is one group with no label.
pub(super) struct ListGroup {
    pub label: Option<(SharedString, &'static str)>,
    pub rows: Vec<AnyElement>,
}

/// The open list: its field (`input`, which takes the keyboard), its name
/// and the name of its rows for screen readers (`Tags`, `Links`,
/// `Sections`, `Find`), the scroll of its rows, its groups of
/// rows, the muted line that says the filter matches nothing
/// (`empty`), and the muted last line that says how many matches it left
/// out (`rest`), in the frame the app's other floating lists use.
pub(super) struct OpenList {
    pub input: Entity<InputState>,
    pub name: &'static str,
    pub scroll: ScrollHandle,
    pub groups: Vec<ListGroup>,
    pub empty: Option<SharedString>,
    pub rest: Option<SharedString>,
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
        .trigger(Trigger {
            el: trigger,
            open: list.is_some(),
        })
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

/// A muted line of the list (`id`): not a button and not a row, so no tab
/// stop, and the highlight skips it.
fn note(id: &'static str, text: SharedString, t: &Theme) -> impl IntoElement {
    div()
        .id(id)
        .px_2()
        .pb_1()
        .role(Role::Status)
        .aria_label(text.clone())
        .text_xs()
        .text_color(theme::muted(t))
        .child(text)
        .test_support()
}

fn frame(list: OpenList, t: &Theme) -> impl IntoElement {
    let empty = list.empty.map(|text| note(LIST_EMPTY_ID, text, t));
    let rest = list.rest.map(|text| note(LIST_REST_ID, text, t));
    // The labels and rows are direct children of the scroll container, so
    // the view can scroll a row into view by its child index.
    let items = list.groups.into_iter().flat_map(|group| {
        let label = group.label.map(|(id, text)| {
            div()
                .id(id)
                .px_2()
                .pt_1()
                .role(Role::Label)
                .aria_label(text)
                .text_xs()
                .text_color(theme::muted(t))
                .child(text)
                .test_support()
                .into_any_element()
        });
        label.into_iter().chain(group.rows)
    });
    popup_layer::frame(
        div()
            .id(MORE_LIST_ID)
            .role(Role::Group)
            .aria_label(list.name)
            .gap_1()
            .pt_2()
            .pb_1(),
        px(LIST_WIDTH),
        t,
    )
    .child(query_field(&list.input, MORE_QUERY_ID, "Filter", t))
    .child(
        div()
            .id("facet-list-rows")
            .role(Role::ListBox)
            .aria_label(list.name)
            .track_scroll(&list.scroll)
            .flex()
            .flex_col()
            .px_1()
            .pb_1()
            .max_h(px(ROWS_MAX_HEIGHT))
            .overflow_y_scroll()
            .children(items),
    )
    .children(empty)
    // Under the scrolling rows, so it shows without a scroll.
    .children(rest)
    .test_support()
}
