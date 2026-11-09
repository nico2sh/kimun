//! The Edit surface's autocomplete (`[[` wikilinks, `#` hashtags): the host
//! adapter the shared engine reads the editor through, where the popup sits,
//! and the popup itself.
//!
//! The rules (trigger detection, the exclusion zones, the debounced and
//! generation-guarded query, the accept edit) are
//! [`kimun_notes::logic::autocomplete`]'s, shared with the TUI. This file only
//! decides where the popup goes and what it looks like.
//!
//! **Where it sits.** The popup is a **cell-aligned** box in the editor's own
//! monospace grid: a whole number of cells wide and tall, so it reads as part
//! of the Edit surface. It opens below the caret cell, flips above when the
//! window has no room below, and slides left to stay inside the window. It is
//! drawn as a deferred layer of the [`EditorElement`](crate::editor_element),
//! in the frame that knows where the caret is, so it never trails the text by
//! a frame.

use std::num::NonZeroU64;

use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyElement, App, Bounds, Div, Entity, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, Pixels, Point, Role, ScrollEvent, SharedString, Size, Stateful,
    StatefulInteractiveElement, Styled, Window, div, point, px, size,
};
use kimun_notes::logic::autocomplete::{
    AutocompleteHost, AutocompleteState, PopupCommand, Suggestion,
};
use kimun_notes::logic::editor::Editor;
use kimun_notes::logic::editor::snapshot::EditorSnapshot;
use kimun_notes::settings::themes::Theme;

use crate::editor_view::EditorView;
use crate::grid::CellMetrics;
use crate::row_paint::cells_of;
use crate::theme;

/// The widest and narrowest the popup gets, in cells (the TUI's cap).
const MAX_COLS: u16 = 60;
const MIN_COLS: u16 = 16;
/// Cells of breathing room: inside each edge of a row, and between a row's
/// name and its secondary text.
const ROW_PAD_COLS: u16 = 1;
const GAP_COLS: u16 = 2;
/// The popup's border: a physical hairline, not layout.
const BORDER: f32 = 1.0;
/// Stays this far inside the window edge.
const WINDOW_MARGIN: f32 = 4.0;

/// The editor as the engine's host: its buffer, cursor and revision. The
/// popup is placed from the painted caret, so the anchor the engine asks for
/// is a placeholder (a `None` there would close the popup as off-screen).
pub(crate) struct EditorHost<'a>(pub(crate) &'a Editor);

impl AutocompleteHost for EditorHost<'_> {
    fn buffer_snapshot(&self) -> EditorSnapshot {
        self.0.view_snapshot()
    }

    fn cache_key(&self) -> Option<NonZeroU64> {
        Some(self.0.content_revision())
    }

    fn screen_anchor_for(&self, _byte_offset: usize) -> Option<(u16, u16)> {
        Some((0, 0))
    }
}

/// The popup's size in cells: as many rows as are visible, wide enough for
/// the longest row (name, a gap, secondary text) plus padding.
pub(crate) fn popup_cells(state: &AutocompleteState) -> (u16, u16) {
    let (start, end) = state.visible_window();
    let widest = state.items[start..end]
        .iter()
        .map(|item| {
            let secondary = item
                .secondary
                .as_deref()
                .map_or(0, |s| GAP_COLS + cells_of(s));
            cells_of(&item.display).saturating_add(secondary)
        })
        .max()
        .unwrap_or(0);
    let cols = widest
        .saturating_add(ROW_PAD_COLS * 2)
        .clamp(MIN_COLS, MAX_COLS);
    (cols, u16::try_from(end - start).unwrap_or(u16::MAX))
}

/// Where the popup's top-left goes: below the caret cell, else above it, else
/// pinned to the window's bottom; slid left to keep its right edge in. The
/// popup is never placed outside `window`, unless it is bigger than it.
pub(crate) fn place(
    caret: Bounds<Pixels>,
    popup: Size<Pixels>,
    window: Bounds<Pixels>,
) -> Point<Pixels> {
    let margin = px(WINDOW_MARGIN);
    let (left, right) = (window.left() + margin, window.right() - margin);
    let (top, bottom) = (window.top() + margin, window.bottom() - margin);
    let x = caret.left().min(right - popup.width).max(left);
    let below = caret.bottom();
    let above = caret.top() - popup.height;
    let y = if below + popup.height <= bottom {
        below
    } else if above >= top {
        above
    } else {
        (bottom - popup.height).max(top)
    };
    point(x, y)
}

/// The popup's pixel size for `cols` x `rows` cells: the grid plus the border.
pub(crate) fn popup_size(cells: &CellMetrics, cols: u16, rows: u16) -> Size<Pixels> {
    size(
        px(cells.width() * f32::from(cols) + BORDER * 2.0),
        px(cells.height() * f32::from(rows) + BORDER * 2.0),
    )
}

/// What a popup row draws: the typed prefix (matched case-insensitively
/// against the start of the name) set apart from the rest.
pub(crate) fn split_prefix<'a>(display: &'a str, query: &str) -> (&'a str, &'a str) {
    let n = query.chars().count();
    let end = display
        .char_indices()
        .nth(n)
        .map_or(display.len(), |(i, _)| i);
    let (head, tail) = display.split_at(end);
    if !query.is_empty() && head.to_lowercase() == query.to_lowercase() {
        (head, tail)
    } else {
        ("", display)
    }
}

/// How the popup is drawn: everything one frame needs, resolved in prepaint.
pub(crate) struct PopupStyle<'a> {
    pub(crate) cells: &'a CellMetrics,
    pub(crate) font_family: SharedString,
    pub(crate) font_size: Pixels,
    pub(crate) theme: &'a Theme,
}

/// One suggestion row's content, apart from how its surface lays it out.
pub(crate) struct PopupRow<'a> {
    pub(crate) item: &'a Suggestion,
    /// The typed query: its matching prefix is set apart.
    pub(crate) query: &'a str,
    /// The row's offset from the top of the visible window.
    pub(crate) offset: usize,
    pub(crate) highlighted: bool,
    /// Expose the row to assistive technology (a list-box option).
    pub(crate) accessible: bool,
}

/// A suggestion row, shared by every popup: the editor's cell grid and the
/// query fields' list differ only in `shape` (the row's size and padding) and
/// `shape_secondary` (the secondary text's), and in what a press and a hover
/// do. What a row shows (the accented typed prefix, the rest, the secondary
/// text, the highlight) is decided here once.
pub(crate) fn popup_row(
    row: PopupRow<'_>,
    t: &Theme,
    shape: impl FnOnce(Stateful<Div>) -> Stateful<Div>,
    shape_secondary: impl FnOnce(Div) -> Div,
    press: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    hover: impl Fn(&gpui_kit::MouseMoveEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let PopupRow {
        item,
        query,
        offset,
        highlighted,
        accessible,
    } = row;
    let (typed, rest) = split_prefix(&item.display, query);
    let base = div()
        // Keyed by what the row is, not where: the list reorders as the query
        // narrows.
        .id((SharedString::from(item.display.clone()), offset))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .text_color(theme::foreground(t));
    let base = if accessible {
        base.role(Role::ListBoxOption)
            .aria_label(SharedString::from(item.display.clone()))
            .aria_selected(highlighted)
    } else {
        base
    };
    let row = shape(base)
        .child(
            div()
                .flex()
                .flex_row()
                .whitespace_nowrap()
                .child(div().text_color(theme::accent(t)).child(typed.to_string()))
                .child(rest.to_string()),
        )
        .on_mouse_down(MouseButton::Left, press)
        .on_mouse_move(hover);
    let row = match &item.secondary {
        Some(secondary) => row.child(shape_secondary(
            div()
                .text_color(theme::secondary(t))
                .child(secondary.clone()),
        )),
        None => row,
    };
    let row = if highlighted {
        row.bg(theme::selected_background(t))
    } else {
        row
    };
    row.test_support()
}

/// What the wheel does over a popup: a step up for a scroll up, a step down
/// for a scroll down. Nothing for a delta with no vertical part (a sideways
/// swipe, a trackpad's trailing events).
pub(crate) fn wheel_command(y: Pixels) -> Option<PopupCommand> {
    if y > px(0.) {
        Some(PopupCommand::Up)
    } else if y < px(0.) {
        Some(PopupCommand::Down)
    } else {
        None
    }
}

/// The popup's element: a bordered, cell-sized box of rows. A press on a row
/// accepts it, a move over one highlights it, the wheel moves the highlight;
/// the box blocks the pointer from the text beneath it.
pub(crate) fn popup_element(
    view: &Entity<EditorView>,
    state: &AutocompleteState,
    style: &PopupStyle<'_>,
    window: &Window,
) -> AnyElement {
    let (cols, rows) = popup_cells(state);
    let PopupStyle {
        cells,
        font_family,
        font_size,
        theme: t,
    } = style;
    let (start, end) = state.visible_window();
    let pad = px(cells.width() * f32::from(ROW_PAD_COLS));
    let line_height = px(cells.height());

    let list = (start..end).map(|idx| {
        let offset = idx - start;
        popup_row(
            PopupRow {
                item: &state.items[idx],
                query: &state.query,
                offset,
                highlighted: idx == state.highlighted,
                accessible: false,
            },
            t,
            // A cell-aligned row: one grid line tall, padded in whole cells.
            |row| row.w_full().h(line_height).px(pad).whitespace_nowrap(),
            |secondary| secondary,
            window.listener_for(view, move |view, _: &MouseDownEvent, window, cx| {
                view.autocomplete_click(offset, window, cx);
                cx.stop_propagation();
            }),
            window.listener_for(view, move |view, _: &gpui_kit::MouseMoveEvent, _, cx| {
                view.autocomplete_hover(offset, cx);
            }),
        )
    });

    div()
        .id("autocomplete-popup")
        .test_support()
        .absolute()
        .occlude()
        .flex()
        .flex_col()
        .w(px(cells.width() * f32::from(cols) + BORDER * 2.0))
        .h(px(cells.height() * f32::from(rows) + BORDER * 2.0))
        .border(px(BORDER))
        .border_color(theme::focus_border(t))
        .bg(theme::panel_background(t))
        .font_family(font_family.clone())
        .text_size(*font_size)
        .overflow_hidden()
        .on_scroll(
            window.listener_for(view, move |view, event: &ScrollEvent, _, cx| {
                if let Some(command) = wheel_command(event.delta.pixel_delta(line_height).y) {
                    view.autocomplete_command(command, cx);
                }
                // Over the popup the wheel never reaches the editor.
                cx.stop_propagation();
            }),
        )
        .children(list)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_notes::logic::autocomplete::{Suggestion, TriggerKind};

    fn state(items: &[(&str, Option<&str>)]) -> AutocompleteState {
        let mut st = AutocompleteState::new(TriggerKind::Wikilink, (0, 0));
        st.set_items(
            items
                .iter()
                .map(|(d, s)| Suggestion {
                    display: (*d).to_string(),
                    secondary: s.map(str::to_string),
                })
                .collect(),
        );
        st
    }

    fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    #[test]
    fn the_wheel_steps_the_highlight_only_when_it_scrolls_vertically() {
        assert_eq!(wheel_command(px(3.)), Some(PopupCommand::Up));
        assert_eq!(wheel_command(px(-3.)), Some(PopupCommand::Down));
        assert_eq!(wheel_command(px(0.)), None);
    }

    #[test]
    fn the_popup_is_as_wide_as_its_widest_visible_row_within_the_caps() {
        let st = state(&[
            ("a", None),
            ("a long note name", Some("work/a-long-note-name")),
        ]);
        // 16 + 2 gap + 21 + 2 padding.
        assert_eq!(popup_cells(&st), (41, 2));
        // Short rows still get the minimum.
        assert_eq!(popup_cells(&state(&[("a", None)])), (MIN_COLS, 1));
        // And never past the cap.
        let long = "x".repeat(200);
        assert_eq!(popup_cells(&state(&[(long.as_str(), None)])), (MAX_COLS, 1));
    }

    #[test]
    fn the_popup_height_is_the_visible_rows_not_all_of_them() {
        let names: Vec<String> = (0..30).map(|i| format!("n{i}")).collect();
        let items: Vec<(&str, Option<&str>)> = names.iter().map(|n| (n.as_str(), None)).collect();
        let (_, rows) = popup_cells(&state(&items));
        assert_eq!(
            usize::from(rows),
            kimun_notes::logic::autocomplete::DEFAULT_MAX_VISIBLE_ROWS
        );
    }

    #[test]
    fn it_opens_below_the_caret_when_there_is_room() {
        let window = bounds(0., 0., 800., 600.);
        let caret = bounds(100., 100., 8., 20.);
        assert_eq!(
            place(caret, size(px(200.), px(100.)), window),
            point(px(100.), px(120.))
        );
    }

    #[test]
    fn it_flips_above_the_caret_when_below_does_not_fit() {
        let window = bounds(0., 0., 800., 600.);
        let caret = bounds(100., 560., 8., 20.);
        // 560 - 100: above the caret row.
        assert_eq!(
            place(caret, size(px(200.), px(100.)), window),
            point(px(100.), px(460.))
        );
    }

    #[test]
    fn it_slides_left_to_keep_its_right_edge_inside() {
        let window = bounds(0., 0., 800., 600.);
        let caret = bounds(700., 100., 8., 20.);
        let at = place(caret, size(px(300.), px(100.)), window);
        assert_eq!(at.x, px(800. - WINDOW_MARGIN - 300.));
    }

    #[test]
    fn with_no_room_either_side_it_pins_to_the_bottom_edge() {
        let window = bounds(0., 0., 800., 200.);
        let caret = bounds(10., 90., 8., 20.);
        let at = place(caret, size(px(100.), px(150.)), window);
        assert_eq!(at.y, px(200. - WINDOW_MARGIN - 150.));
    }

    #[test]
    fn the_typed_prefix_is_matched_case_insensitively_and_whole() {
        assert_eq!(split_prefix("Meeting", "mee"), ("Mee", "ting"));
        assert_eq!(split_prefix("Meeting", ""), ("", "Meeting"));
        // A name that does not start with the query (fuzzy hit): no split.
        assert_eq!(split_prefix("Standup", "mee"), ("", "Standup"));
        // A query longer than the name.
        assert_eq!(split_prefix("ab", "abc"), ("", "ab"));
        // Multibyte: split on chars, never mid-codepoint.
        assert_eq!(split_prefix("Éclair", "é"), ("É", "clair"));
    }
}
