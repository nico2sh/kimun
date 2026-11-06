//! The **query field above a drawer list**: what FIND, SEM, the Sources
//! view, TAGS and FILES share. One styled field ([`query_field`]) and the
//! keyboard hand-over between it and the list under it. What differs per
//! view (what Enter, Escape and a typed query do) stays with the view.
//!
//! The hand-over: Down or Enter in the field give the list the keyboard
//! ([`focus_list`]); Up on the list's first row, or with nothing selected
//! ([`at_top`]), gives it back ([`focus_input`], or [`up_returns_to_field`]
//! for a view with no other key to catch first).

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{
    App, Context, Div, Entity, Focusable as _, KeyDownEvent, ParentElement as _, Styled as _,
    Window, div,
};
use kimun_notes::logic::input::{Key, Mods};
use kimun_notes::settings::themes::Theme;

use super::drawer_list::{DrawerList, DrawerRow};
use crate::input;
use crate::theme;

/// The field in its drawer-view frame: `input` under element id `id`,
/// named `label` for assistive tech, in the theme's field colours.
pub(super) fn query_field(
    input: &Entity<InputState>,
    id: &'static str,
    label: &'static str,
    t: &Theme,
) -> Div {
    div().flex_none().px_2().pb_1().child(
        Input::new(input)
            .id(id)
            .aria_label(label)
            .bg(theme::background(t))
            .border_color(theme::divider(t))
            .text_color(theme::foreground(t)),
    )
}

/// Give the keyboard to `list`.
pub(super) fn focus_list<R: DrawerRow, V: 'static>(
    list: &Entity<DrawerList<R>>,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    let handle = list.read(cx).focus_handle(cx);
    window.focus(&handle, cx);
}

/// Give the keyboard to the query field `input`.
pub(super) fn focus_input(input: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    let handle = input.read(cx).focus_handle(cx);
    window.focus(&handle, cx);
}

/// Whether `list` has the keyboard.
pub(super) fn list_focused<R: DrawerRow>(
    list: &Entity<DrawerList<R>>,
    window: &Window,
    cx: &App,
) -> bool {
    list.read(cx).focus_handle(cx).is_focused(window)
}

/// Whether the selection is on `list`'s first row, or there is none: the
/// place an Up key leaves for the field.
pub(super) fn at_top<R: DrawerRow>(list: &Entity<DrawerList<R>>, cx: &App) -> bool {
    list.read(cx).selected().is_none_or(|pos| pos == 0)
}

/// The key policy of a view with nothing else to catch before the list:
/// a bare Up on the list's top row moves to `field`. True when taken.
pub(super) fn up_returns_to_field<R: DrawerRow>(
    list: &Entity<DrawerList<R>>,
    field: &Entity<InputState>,
    event: &KeyDownEvent,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    if !list_focused(list, window, cx) || !at_top(list, cx) {
        return false;
    }
    let Some(key) = input::key_down_input(event) else {
        return false;
    };
    if key.mods != Mods::NONE || key.key != Key::Up {
        return false;
    }
    focus_input(field, window, cx);
    true
}
