//! The **Facet view**'s keyboard ([`super`]): its tab stops and its keys.
//!
//! **Tab stops.** Every chip, pill and text button the view draws is a tab
//! stop on a focus handle the view keeps ([`Stops`]), so the kit's root walks
//! them with Tab in the order they are painted: the bar's kind chips, its
//! search chip, its picked pills, `Clear` (or `Undo` just after a clear),
//! then the column top to bottom. The stop with the keyboard is drawn with
//! the focus ring ([`focus_ring`], a 2 px stroke 1 px outside the stop), and
//! Enter or Space on it does what a click does ([`FacetView::activate`]).
//! Enter or Space on `Clear` hands the keyboard to `Undo`. A picked pill
//! that unpicks itself while it has the keyboard hands it to the next pick,
//! or to the one before when it was the last. Any other stop that goes away
//! with the keyboard (`Undo` once used, the only pick) hands it to the view.
//! A stop of the column that takes the keyboard scrolls into view with its
//! ring ([`mod@super::reveal`]), so Tab walks on past the drawn pieces.
//! Tab from the bar enters the column at piece 0 and Shift+Tab from
//! `Show more` at the last piece, wherever the column is scrolled. A
//! scroll never takes the keyboard from a stop: its piece stays drawn.
//!
//! **Keys.** Taken before anything inside the view sees them (a piece's
//! read-only editor would take letters), and never passed on, so neither the
//! group's hidden editor nor the workspace's keymap gets them: `/` opens the
//! list of every facet, the one the search chip opens (nothing when
//! every facet is picked), `j`/`k` bring the next/previous piece to the top,
//! `r` asks a collection to run its query again ([`FacetEvent::Refresh`]),
//! Escape clears the picks. An open list closes on Escape first: the kit's
//! popover takes it before any key listener. With the keyboard inside a
//! piece (after a press there), Escape first hands it to the view and only
//! then clears, and Tab and Shift+Tab, which the piece's editor leaves
//! alone, walk on to the next stop. While the list's filter field has the
//! keyboard the view leaves plain keys to it. Up and Down move the list's
//! highlight one row at a time over its drawn rows in order, ticked rows
//! included (tags, then links, then headings in the search list), and the
//! list scrolls the lit row into view. Enter toggles the lit row: it picks
//! an unpicked facet and unpicks a ticked one. The list stays open, the row
//! keeps its place and the highlight stays on it. A muted line (the empty
//! filter text, the count of rows left out) is never lit.
//!
//! **Undo.** Ctrl+Z (Cmd+Z on macOS) brings back the picks the last clear
//! removed (`Clear`, Escape), from anywhere in the view, a piece included.
//! While the list's filter field has the keyboard, Ctrl+Z is the field's
//! own text undo. The `Undo` button the summary row shows after a clear does
//! the same.

use std::collections::HashMap;

use gpui_kit::component::input::{Enter, MoveDown, MoveUp};
use gpui_kit::{
    App, Context, Div, FocusHandle, Focusable as _, InteractiveElement, KeyBinding, KeyDownEvent,
    ParentElement, Stateful, Styled, Window, actions, point, px,
};
use kimun_core::note::{Facet, Reveal};
use kimun_notes::logic::input::{Key, Mods};
use kimun_notes::settings::themes::Theme;

use super::body::to_px;
use super::rows::column_pad;
use super::{FacetEvent, FacetView, ListKind};
use crate::input;
use crate::keymap;
use crate::workspace::facet_ui::focus_ring;

actions!(kimun_facet, [FacetTab, FacetTabPrev]);

/// The key context of the view's root.
const KEY_CONTEXT: &str = "KimunFacet";

/// Bind Tab and Shift+Tab in the view, over the kit root's traversal, so
/// the view can enter its column in plan order. Call once, after
/// `gpui_kit::init`.
pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", FacetTab, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-tab", FacetTabPrev, Some(KEY_CONTEXT)),
    ]);
}

/// A tab stop of the view: what it is, so a key can do what its click does.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Stop {
    /// A menu chip: opens its kind's list, or the list of every kind (the
    /// search chip).
    Menu(ListKind),
    Clear,
    /// The summary row's `Undo`, after a clear.
    Undo,
    /// The empty message's `Remove last pick`.
    RemoveLast,
    /// A collection's empty message's `Refresh`.
    Refresh,
    Pill(Facet),
    /// Crumb `.1` of chunk `.0`'s breadcrumb.
    Crumb(usize, usize),
    /// Chunk `.0`'s `Open note`.
    Open(usize),
    GapDown(usize),
    GapUp(usize),
    ShowAll(usize),
    ShowMore,
}

/// Element ids.
impl Stop {
    /// The stop's element id.
    pub(crate) fn id(&self) -> gpui_kit::SharedString {
        use super::*;
        match self {
            Stop::Menu(kind) => menu_id(*kind),
            Stop::Clear => CLEAR_ID.into(),
            Stop::Undo => UNDO_ID.into(),
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
    /// The stop that had the keyboard when the frame before began.
    before: Option<Stop>,
    /// The stop this frame drew with the focus ring (tests).
    pub(super) ringed: Option<Stop>,
    /// The window's rem size this frame: the ring's corners follow it.
    rem: gpui_kit::Pixels,
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

    /// The stops the last frame drew, in the order they were tracked.
    pub(super) fn drawn(&self) -> &[Stop] {
        &self.drawn
    }

    /// The stop that took the keyboard since the last frame began, if any.
    pub(super) fn arrived(&self) -> Option<&Stop> {
        self.focused
            .as_ref()
            .filter(|stop| self.before.as_ref() != Some(*stop))
    }

    /// Whether `stop` had the keyboard when this frame began.
    pub(super) fn holds(&self, stop: &Stop) -> bool {
        self.focused.as_ref() == Some(stop)
    }

    /// The stop that took the keyboard since the last frame began, while
    /// this frame draws it. A stop that went away (`Clear` after a click on
    /// it) is not one.
    pub(super) fn arrived_drawn(&self) -> Option<&Stop> {
        self.arrived().filter(|stop| self.drawn.contains(*stop))
    }

    /// A frame begins: note the stop with the keyboard, draw none yet.
    pub(super) fn begin(&mut self, window: &Window) {
        self.focused = self.focused(window);
        self.rem = window.rem_size();
        self.drawn.clear();
        self.ringed = None;
    }

    /// The frame is drawn: let go of the stops it did not draw. True when
    /// the stop with the keyboard was one of them; the caller gives the
    /// keyboard to the view.
    pub(super) fn end(&mut self) -> bool {
        let drawn = &self.drawn;
        self.handles.retain(|stop, _| drawn.contains(stop));
        self.before = self.focused.take();
        self.before
            .as_ref()
            .is_some_and(|stop| !drawn.contains(stop))
    }

    /// `el` as `stop`: a tab stop, with the focus ring
    /// ([`focus_ring`]) while it has the keyboard.
    pub(super) fn track<E: InteractiveElement + Styled + ParentElement>(
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
        let id = stop.id();
        self.drawn.push(stop);
        let el = el.track_focus(&handle);
        if ring {
            el.child(focus_ring(&id, self.rem, t))
        } else {
            el
        }
    }
}

/// The view's keys on its root element. See the module docs.
pub(super) fn keys(el: Stateful<Div>, cx: &mut Context<FacetView>) -> Stateful<Div> {
    // Tab into the column from the bar or from `Show more` goes in plan
    // order; any other Tab goes on to the kit root's traversal.
    let tab = |back: bool| {
        move |this: &mut FacetView, window: &mut Window, cx: &mut Context<FacetView>| {
            if !this.enter_column(back, window, cx) {
                cx.propagate();
            }
        }
    };
    let (next, prev) = (tab(false), tab(true));
    el.key_context(KEY_CONTEXT)
        .on_action(cx.listener(move |this, _: &FacetTab, window, cx| next(this, window, cx)))
        .on_action(cx.listener(move |this, _: &FacetTabPrev, window, cx| prev(this, window, cx)))
        .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            if this.key_down(event, window, cx) {
                cx.stop_propagation();
                cx.notify();
            }
        }))
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
    pub(super) fn track<E: InteractiveElement + Styled + ParentElement>(
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

    /// The piece whose read-only editor holds the keyboard. Nothing is
    /// scanned while the keyboard is outside the view.
    pub(super) fn focused_piece(&self, window: &Window, cx: &App) -> Option<usize> {
        if !self.focus.contains_focused(window, cx) {
            return None;
        }
        self.chunks.iter().position(|chunk| {
            #[cfg(test)]
            self.focus_scans.set(self.focus_scans.get() + 1);
            chunk.read(cx).focus_handle(cx).contains_focused(window, cx)
        })
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
        if self.field_has_keyboard(window, cx) {
            return false;
        }
        // Ctrl+Z (Cmd+Z on macOS) brings back the last cleared picks, in a
        // piece too: a piece has no text undo.
        if key.key == Key::Char('z') && key.mods == keymap::primary(cx) {
            return self.undo_clear(cx);
        }
        if key.mods.ctrl || key.mods.alt || key.mods.sup {
            return false;
        }
        let in_piece = self.focused_piece(window, cx).is_some();
        match key.key {
            // Shift is how some layouts type a slash. With every facet
            // picked, `open_list` opens nothing.
            Key::Char('/') => self.open_list(ListKind::All, window, cx),
            Key::Char('j') => self.step_piece(true, window, cx),
            Key::Char('k') => self.step_piece(false, window, cx),
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
            Stop::Menu(kind) => {
                if self.list_open(kind) {
                    self.close_list(window, cx);
                } else {
                    self.open_list(kind, window, cx);
                }
            }
            Stop::Clear => {
                // With the keyboard on `Clear`, it moves to `Undo`.
                if self.stops.borrow().focused(window) == Some(Stop::Clear) {
                    self.focus_after = Some(Stop::Undo);
                }
                self.clear(cx);
            }
            Stop::Undo => {
                self.undo_clear(cx);
            }
            Stop::RemoveLast => self.remove_last_pick(cx),
            Stop::Refresh => {
                self.refresh_key(cx);
            }
            Stop::Pill(facet) => self.unpick_pill(facet, window, cx),
            Stop::Crumb(chunk, index) => {
                if let Some(facet) = self.members.crumb_facet(chunk, index) {
                    self.toggle(facet, cx);
                }
            }
            Stop::Open(i) => {
                if let Some((note, line)) = self.chunk_source(i) {
                    cx.emit(FacetEvent::OpenNote { note, line });
                }
            }
            Stop::GapDown(g) => self.reveal(g, Reveal::Below, cx),
            Stop::GapUp(g) => self.reveal(g, Reveal::Above, cx),
            Stop::ShowAll(g) => self.reveal_all(g, cx),
            Stop::ShowMore => self.show_more(cx),
        }
    }

    /// A picked pill's click: unpick its facet. When the pill has the
    /// keyboard, the next pick takes it (the one before, for the last), so
    /// no ring is left on a pill that is gone.
    fn unpick_pill(&mut self, facet: Facet, window: &mut Window, cx: &mut Context<Self>) {
        let stop = Stop::Pill(facet.clone());
        let next = (self.stops.borrow().focused(window).as_ref() == Some(&stop))
            .then(|| {
                let picks = self.bar.picked(&self.members.merged);
                let at = picks.iter().position(|p| p.facet == facet)?;
                let next = picks
                    .get(at + 1)
                    .or(at.checked_sub(1).and_then(|i| picks.get(i)));
                next.map(|p| Stop::Pill(p.facet.clone()))
            })
            .flatten()
            .and_then(|next| self.stops.borrow().handle(&next));
        self.toggle(facet, cx);
        if let Some(handle) = next {
            window.focus(&handle, cx);
        }
    }

    /// Bring the next (`down`) or previous piece to the top of the column.
    /// The pieces' tops come from the height table, so a piece that is
    /// not drawn is reached too.
    fn step_piece(&mut self, down: bool, window: &Window, cx: &mut Context<Self>) {
        // The table starts under the column's `py_3`.
        let pad = to_px(column_pad(), window.rem_size());
        let tops: Vec<f32> = {
            let heights = self.heights.borrow();
            (0..self.members.len())
                .map(|i| f32::from(pad + heights.step_top(i)))
                .collect()
        };
        let handle = &self.scroll;
        let offset = handle.offset();
        let current = -f32::from(offset.y);
        let max = f32::from(handle.max_offset().y);
        if let Some(y) = super::reveal::piece_offset(&tops, current, max, down) {
            handle.set_offset(point(offset.x, px(-y)));
            cx.notify();
        }
    }

    /// Move the open list's highlight one row down or up, and scroll the
    /// lit row into view (with its group's label, going up onto a group's
    /// first row).
    fn step_highlight(&mut self, down: bool, cx: &mut Context<Self>) {
        let len = self.list_rows(cx).len();
        if let Some(more) = &mut self.more {
            let last = len.saturating_sub(1);
            more.highlight = if down {
                (more.highlight + 1).min(last)
            } else {
                more.highlight.min(last).saturating_sub(1)
            };
            more.reveal.set(true);
            more.up = !down;
            cx.notify();
        }
    }

    /// Enter in the list: toggle the lit row; the list stays open and the
    /// highlight stays on the row ([`FacetView::toggle_row`]).
    fn pick_highlighted(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.list_rows(cx);
        let Some(more) = &self.more else {
            return;
        };
        let lit = more.highlight.min(rows.len().saturating_sub(1));
        if let Some(facet) = rows.into_iter().nth(lit).map(|row| row.facet) {
            self.toggle_row(facet, window, cx);
        }
    }
}
