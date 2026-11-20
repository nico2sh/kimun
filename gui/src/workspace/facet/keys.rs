//! The **Facet view**'s keyboard ([`super`]): its tab stops and its keys.
//!
//! **Tab stops.** Every chip, pill and text button the view draws is a tab
//! stop on a focus handle the view keeps ([`Stops`]), so the kit's root walks
//! them with Tab in the order they are painted: the bar's filter line (the
//! pills drawn, `+N`, `Sections N`), its summary line (`Clear`), then the
//! column top to bottom. The stop with the keyboard is drawn with the focus
//! ring (`theme::focus_border`), and Enter or Space on it does what a click
//! does ([`FacetView::activate`]). A stop that goes away with the keyboard
//! (`Clear` once the picks are cleared) hands it to the view.
//!
//! **Keys.** Taken before anything inside the view sees them (a piece's
//! read-only editor would take letters), and never passed on, so neither the
//! group's hidden editor nor the workspace's keymap gets them: `/` opens the
//! list of every unpicked tag and link, the one `+N` opens (the Sections
//! list when no tag or link is left unpicked, nothing when the note has no
//! heading either), `j`/`k` bring
//! the next/previous piece to the top, `r` asks a collection to run its
//! query again ([`FacetEvent::Refresh`]), Escape clears the picks. An open list
//! closes on Escape first: the kit's popover takes it before any key
//! listener. With the keyboard inside a piece (after a press there), Escape
//! first hands it to the view and only then clears, and Tab and Shift+Tab,
//! which the piece's editor leaves alone, walk on to the next stop. While
//! the list's filter field has the keyboard the view leaves plain keys to
//! it; Up and Down move the list's highlight over its pills in order (tags,
//! then links) and Enter toggles the lit facet.
//!
//! **The primary modifier.** Pressing or releasing it (Ctrl, or Cmd on
//! macOS) while the keyboard is anywhere in the view draws the view again,
//! with no pointer move: a piece's links and tags show the pointing hand only
//! while it is held.

use std::collections::HashMap;

use gpui_kit::component::input::{Enter, MoveDown, MoveUp};
use gpui_kit::{
    App, Context, Div, FocusHandle, Focusable as _, InteractiveElement, KeyDownEvent,
    ModifiersChangedEvent, Stateful, Styled, Window, point, px,
};
use kimun_core::note::{Facet, Reveal};
use kimun_notes::logic::input::{Key, Mods};
use kimun_notes::settings::themes::Theme;

use super::{FacetEvent, FacetView, ListKind};
use crate::input;
use crate::keymap::Platform;
use crate::theme;

/// A tab stop of the view: what it is, so a key can do what its click does.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Stop {
    /// The `+N` chip: opens the list of every tag and link.
    More,
    /// The `Sections N` chip: opens the headings list.
    Sections,
    Clear,
    /// The empty message's `Remove last pick`.
    RemoveLast,
    /// A collection's empty message's `Refresh`.
    Refresh,
    Pill(Facet),
    /// Crumb `.1` of chunk `.0`'s breadcrumb.
    Crumb(usize, usize),
    /// Chunk `.0`'s `Open source`.
    Open(usize),
    GapDown(usize),
    GapUp(usize),
    ShowAll(usize),
    ShowMore,
}

/// Element ids, for tests.
#[cfg(test)]
impl Stop {
    /// The stop's element id.
    pub(crate) fn id(&self) -> gpui_kit::SharedString {
        use super::*;
        match self {
            Stop::More => MORE_ID.into(),
            Stop::Sections => SECTIONS_ID.into(),
            Stop::Clear => CLEAR_ID.into(),
            Stop::RemoveLast => REMOVE_LAST_ID.into(),
            Stop::Refresh => REFRESH_ID.into(),
            Stop::Pill(facet) => pill_id(facet),
            Stop::Crumb(chunk, index) => crumb_id(*chunk, *index),
            Stop::Open(i) => open_id(*i),
            Stop::GapDown(g) => gap_down_id(*g),
            Stop::GapUp(g) => gap_up_id(*g),
            Stop::ShowAll(g) => show_all_id(*g),
            Stop::ShowMore => SHOW_MORE_ID.into(),
        }
    }
}

/// The view's tab stops: one focus handle per stop, kept while the stop is
/// drawn, so the keyboard stays on it across frames.
#[derive(Default)]
pub(super) struct Stops {
    handles: HashMap<Stop, FocusHandle>,
    /// The stops this frame drew, in order.
    drawn: Vec<Stop>,
    /// The stop that had the keyboard when this frame began.
    focused: Option<Stop>,
    /// The stop this frame drew with the focus ring (tests).
    pub(super) ringed: Option<Stop>,
}

impl Stops {
    /// The stop with the keyboard, if one has it.
    pub(super) fn focused(&self, window: &Window) -> Option<Stop> {
        self.handles
            .iter()
            .find(|(_, handle)| handle.is_focused(window))
            .map(|(stop, _)| stop.clone())
    }

    /// `stop`'s handle while it was drawn in the last frame.
    pub(super) fn handle(&self, stop: &Stop) -> Option<FocusHandle> {
        self.handles.get(stop).cloned()
    }

    /// A frame begins: note the stop with the keyboard, draw none yet.
    pub(super) fn begin(&mut self, window: &Window) {
        self.focused = self.focused(window);
        self.drawn.clear();
        self.ringed = None;
    }

    /// The frame is drawn: let go of the stops it did not draw. True when
    /// the stop with the keyboard was one of them; the caller gives the
    /// keyboard to the view.
    pub(super) fn end(&mut self) -> bool {
        let drawn = &self.drawn;
        self.handles.retain(|stop, _| drawn.contains(stop));
        self.focused
            .take()
            .is_some_and(|stop| !drawn.contains(&stop))
    }

    /// `el` as `stop`: a tab stop, ringed while it has the keyboard.
    pub(super) fn track<E: InteractiveElement + Styled>(
        &mut self,
        stop: Stop,
        el: E,
        t: &Theme,
        cx: &mut App,
    ) -> E {
        let handle = self
            .handles
            .entry(stop.clone())
            .or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone();
        let ring = self.focused.as_ref() == Some(&stop);
        if ring {
            self.ringed = Some(stop.clone());
        }
        self.drawn.push(stop);
        let el = el.track_focus(&handle);
        if ring {
            el.border_color(theme::focus_border(t))
        } else {
            el
        }
    }
}

/// The scroll offset that brings the next (`down`) or previous piece to
/// the top, from the pieces' tops in the column (`tops`, in order), the
/// column's offset now (`current`, ≥ 0) and how far it scrolls (`max`).
/// `None` when no piece lies that way. Above the first piece (the column's
/// inset) the first piece is the one shown: down goes to the second.
pub(super) fn piece_offset(tops: &[f32], current: f32, max: f32, down: bool) -> Option<f32> {
    let target = if down {
        let current = current.max(tops.first().copied().unwrap_or(0.0));
        tops.iter().copied().find(|&y| y > current + 0.5)
    } else {
        tops.iter().copied().rev().find(|&y| y < current - 0.5)
    }?;
    Some(target.clamp(0.0, max.max(0.0)))
}

/// The view's keys on its root element. See the module docs.
pub(super) fn keys(el: Stateful<Div>, cx: &mut Context<FacetView>) -> Stateful<Div> {
    el.capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
        if this.key_down(event, window, cx) {
            cx.stop_propagation();
            cx.notify();
        }
    }))
    .on_modifiers_changed(
        cx.listener(|this, event: &ModifiersChangedEvent, window, _| {
            let held = input::mods(&event.modifiers) == Platform::current().primary();
            if this.primary_held != held {
                this.primary_held = held;
                window.refresh();
            }
        }),
    )
    .capture_action(cx.listener(|this, _: &MoveUp, window, cx| {
        if this.field_has_keyboard(window, cx) {
            this.step_highlight(false, cx);
            cx.stop_propagation();
        }
    }))
    .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
        if this.field_has_keyboard(window, cx) {
            this.step_highlight(true, cx);
            cx.stop_propagation();
        }
    }))
    .capture_action(cx.listener(|this, _: &Enter, window, cx| {
        if this.field_has_keyboard(window, cx) {
            this.pick_highlighted(window, cx);
            cx.stop_propagation();
        }
    }))
}

impl FacetView {
    /// `el` as `stop`, a tab stop of the view (drawing helpers take `&self`).
    pub(super) fn track<E: InteractiveElement + Styled>(
        &self,
        stop: Stop,
        el: E,
        t: &Theme,
        cx: &mut App,
    ) -> E {
        self.stops.borrow_mut().track(stop, el, t, cx)
    }

    /// Whether the open list's filter field has the keyboard.
    pub(super) fn field_has_keyboard(&self, window: &Window, cx: &App) -> bool {
        self.more.as_ref().is_some_and(|more| {
            more.input
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx)
        })
    }

    /// Whether the keyboard is inside a piece (its read-only editor).
    fn piece_has_keyboard(&self, window: &Window, cx: &App) -> bool {
        self.chunks
            .iter()
            .any(|chunk| chunk.read(cx).focus_handle(cx).contains_focused(window, cx))
    }

    /// A key anywhere in the view. True when taken.
    fn key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(key) = input::key_down_input(event) else {
            return false;
        };
        if self.field_has_keyboard(window, cx) || key.mods.ctrl || key.mods.alt || key.mods.sup {
            return false;
        }
        let in_piece = self.piece_has_keyboard(window, cx);
        match key.key {
            // Shift is how some layouts type a slash.
            // No unpicked tag or link (none in the note, or every one
            // picked): the Sections list is all there is to filter, if the
            // note has headings (`open_list` opens nothing otherwise).
            Key::Char('/') => {
                let kind = if self.list_drawable(ListKind::Facets) {
                    ListKind::Facets
                } else {
                    ListKind::Sections
                };
                self.open_list(kind, window, cx)
            }
            Key::Char('j') => self.step_piece(true, cx),
            Key::Char('k') => self.step_piece(false, cx),
            // A collection runs its query again; a single note has no `r`.
            Key::Char('r') if key.mods == Mods::NONE => return self.refresh_key(cx),
            // A piece's read-only editor binds Tab to nothing: walk on.
            Key::Tab if in_piece && !key.mods.shift => window.focus_next(cx),
            Key::Tab | Key::BackTab if in_piece => window.focus_prev(cx),
            Key::Esc if self.more.is_some() => self.close_list(window, cx),
            // Out of a piece first (text may be selected there), then clear.
            Key::Esc if in_piece => window.focus(&self.focus, cx),
            Key::Esc => self.clear(cx),
            Key::Enter | Key::Char(' ') if key.mods == Mods::NONE => {
                let focused = self.stops.borrow().focused(window);
                if let Some(stop) = focused {
                    self.activate(stop, window, cx);
                }
            }
            _ => return false,
        }
        true
    }

    /// What a click on `stop` does.
    pub(super) fn activate(&mut self, stop: Stop, window: &mut Window, cx: &mut Context<Self>) {
        match stop {
            Stop::More | Stop::Sections => {
                let kind = match stop {
                    Stop::Sections => ListKind::Sections,
                    _ => ListKind::Facets,
                };
                if self.list_open(kind) {
                    self.close_list(window, cx);
                } else {
                    self.open_list(kind, window, cx);
                }
            }
            Stop::Clear => self.clear(cx),
            Stop::RemoveLast => self.remove_last_pick(cx),
            Stop::Refresh => {
                self.refresh_key(cx);
            }
            Stop::Pill(facet) => self.toggle(facet, cx),
            Stop::Crumb(chunk, index) => {
                if let Some(facet) = self.members.crumb_facet(chunk, index) {
                    self.toggle(facet, cx);
                }
            }
            Stop::Open(i) => {
                if let Some((note, line)) = self.chunk_source(i) {
                    cx.emit(FacetEvent::OpenSource { note, line });
                }
            }
            Stop::GapDown(g) => self.reveal(g, Reveal::Below, cx),
            Stop::GapUp(g) => self.reveal(g, Reveal::Above, cx),
            Stop::ShowAll(g) => self.reveal_all(g, cx),
            Stop::ShowMore => self.show_more(cx),
        }
    }

    /// Bring the next (`down`) or previous piece to the top of the column.
    fn step_piece(&mut self, down: bool, cx: &mut Context<Self>) {
        let handle = &self.scroll;
        let top = handle.bounds().top();
        let tops: Vec<f32> = self
            .piece_items
            .iter()
            .filter_map(|&ix| handle.bounds_for_item(ix))
            .map(|bounds| f32::from(bounds.top() - top))
            .collect();
        let offset = handle.offset();
        let current = -f32::from(offset.y);
        let max = f32::from(handle.max_offset().y);
        if let Some(y) = piece_offset(&tops, current, max, down) {
            handle.set_offset(point(offset.x, px(-y)));
            cx.notify();
        }
    }

    /// Move the open list's highlight one result down or up.
    fn step_highlight(&mut self, down: bool, cx: &mut Context<Self>) {
        let len = self.list_pills(cx).len();
        if let Some(more) = &mut self.more {
            let last = len.saturating_sub(1);
            more.highlight = if down {
                (more.highlight + 1).min(last)
            } else {
                more.highlight.min(last).saturating_sub(1)
            };
            cx.notify();
        }
    }

    /// Enter in the list: toggle the lit facet; the list stays open.
    fn pick_highlighted(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pills = self.list_pills(cx);
        let Some(more) = &self.more else {
            return;
        };
        let lit = more.highlight.min(pills.len().saturating_sub(1));
        let Some(facet) = pills.into_iter().nth(lit).map(|pill| pill.facet) else {
            return;
        };
        self.pick_more(facet.clone(), window, cx);
        // A toggle reorders the list: a picked tag or link leaves it (its
        // pill leads the line), a picked heading moves to the front of the
        // Sections list. The highlight follows a facet still listed, so the
        // next Enter toggles it back; else it keeps its place, on the facet
        // that moved up into it (the last when the list got shorter).
        let pills = self.list_pills(cx);
        let at = pills.iter().position(|p| p.facet == facet);
        if let Some(more) = &mut self.more {
            more.highlight = at.unwrap_or(more.highlight.min(pills.len().saturating_sub(1)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::piece_offset;

    const TOPS: [f32; 3] = [12.0, 200.0, 400.0];

    #[test]
    fn down_goes_to_the_first_piece_below_the_top() {
        assert_eq!(piece_offset(&TOPS, 0.0, 1000.0, true), Some(200.0));
        assert_eq!(piece_offset(&TOPS, 12.0, 1000.0, true), Some(200.0));
        assert_eq!(piece_offset(&TOPS, 400.0, 1000.0, true), None);
    }

    #[test]
    fn up_goes_to_the_last_piece_above_the_top() {
        assert_eq!(piece_offset(&TOPS, 200.0, 1000.0, false), Some(12.0));
        assert_eq!(piece_offset(&TOPS, 150.0, 1000.0, false), Some(12.0));
        assert_eq!(piece_offset(&TOPS, 12.0, 1000.0, false), None);
    }

    #[test]
    fn the_offset_stops_where_the_column_ends() {
        assert_eq!(piece_offset(&TOPS, 0.0, 150.0, true), Some(150.0));
        assert_eq!(piece_offset(&TOPS, 200.0, 300.0, true), Some(300.0));
    }
}
