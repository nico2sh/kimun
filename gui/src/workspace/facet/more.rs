//! The open list of the facet view.
//!
//! It owns the open list state, and opening, closing and sorting the list.
//! It does not draw the list. `line.rs` draws it.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    App, AppContext as _, Context, Entity, Focusable as _, ScrollHandle, Subscription, Window, px,
};
use kimun_core::note::{Facet, FacetKind};

use super::keys::Stop;
use super::{FacetView, ListKind, line};
use crate::workspace::query_field::focus_input;

/// The open list: its kind, its fuzzy query field, and the row Enter
/// toggles (lit, an index into the list's drawn rows in order, picked rows
/// too; a new query lights the first).
pub(super) struct More {
    pub(super) kind: ListKind,
    pub(super) input: Entity<InputState>,
    /// The order of the rows, set when the list opens and when its filter
    /// changes. A pick or unpick leaves it alone: rows do not move under
    /// the pointer or the highlight while the list is open.
    pub(super) order: line::Order,
    pub(super) highlight: usize,
    /// The scroll of the list's rows.
    pub(super) scroll: ScrollHandle,
    /// The highlight moved since the last frame: that frame scrolls the
    /// lit row into view.
    pub(super) reveal: std::cell::Cell<bool>,
    /// The last move of the highlight was up. Up onto the first row of a
    /// group scrolls to the group's label, so it shows above the row.
    pub(super) up: bool,
    _events: Subscription,
}

impl FacetView {
    /// Whether `kind`'s list holds any facet, picked or not, in its scope
    /// (its kind, or any kind for [`ListKind::All`]). An open list stays
    /// open while this holds.
    pub(super) fn list_holds(&self, kind: ListKind) -> bool {
        let entries = &self.members.merged.entries;
        let of = |k: FacetKind| entries.iter().any(|e| e.facet.kind == k);
        match kind {
            ListKind::Tags => of(FacetKind::Tag),
            ListKind::Links => of(FacetKind::NoteLink),
            ListKind::Sections => of(FacetKind::Heading),
            ListKind::All => !entries.is_empty(),
        }
    }

    /// Whether `kind`'s list has anything to pick: an unpicked facet in
    /// its scope (its kind, or any kind for [`ListKind::All`]). Ticked rows
    /// do not count: a list opens only while this holds.
    pub(super) fn list_drawable(&self, kind: ListKind) -> bool {
        let any = self
            .bar
            .menus(&self.members.merged, &|f| self.members.in_view_of(f))
            .any;
        match kind {
            ListKind::Tags => any.tags,
            ListKind::Links => any.links,
            ListKind::Sections => any.sections,
            ListKind::All => any.tags || any.links || any.sections,
        }
    }

    pub(super) fn list_open(&self, kind: ListKind) -> bool {
        self.more.as_ref().is_some_and(|more| more.kind == kind)
    }

    /// Open `kind`'s list, its filter field empty and holding the keyboard.
    /// It replaces a list already open.
    pub(crate) fn open_list(
        &mut self,
        kind: ListKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Nothing left to pick in its scope.
        if !self.list_drawable(kind) {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        let events = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                // A new query sorts the rows again and lights its first
                // row, at the top.
                this.sort_list(cx);
                if let Some(more) = &mut this.more {
                    more.highlight = 0;
                    more.scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
                }
                cx.notify();
            }
        });
        focus_input(&input, window, cx);
        let order = line::order_of(&self.scope(kind, ""));
        self.more = Some(More {
            kind,
            input,
            order,
            highlight: 0,
            scroll: ScrollHandle::new(),
            reveal: std::cell::Cell::new(false),
            up: false,
            _events: events,
        });
        cx.notify();
    }

    /// Close the open list. The keyboard goes back to the chip that opens
    /// it, or to the view when that chip is no longer drawn.
    pub(crate) fn close_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(more) = self.more.take() else {
            return;
        };
        let handle = self.stops.borrow().handle(&Stop::Menu(more.kind));
        window.focus(&handle.unwrap_or_else(|| self.focus.clone()), cx);
        cx.notify();
    }

    /// The kit opened or closed `kind`'s popover (a press on its chip,
    /// Escape, a press outside). A close of a list no longer open (another
    /// chip's list replaced it) changes nothing.
    pub(super) fn list_changed(
        &mut self,
        kind: ListKind,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_open = self.more.as_ref().is_some_and(|more| more.kind == kind);
        if open && !is_open {
            self.open_list(kind, window, cx);
        } else if !open && is_open {
            self.close_list(window, cx);
        }
    }

    /// Sort the open list's rows again for its current filter: picked rows
    /// first, then the others as [`mod@super::line`] orders them.
    pub(super) fn sort_list(&mut self, cx: &App) {
        let Some(more) = &self.more else {
            return;
        };
        let query = more.input.read(cx).value().to_string();
        let order = line::order_of(&self.scope(more.kind, &query));
        if let Some(more) = &mut self.more {
            more.order = order;
        }
    }

    /// A press or Enter on a row of the open list: pick its facet, or unpick
    /// it. The list stays open, its filter kept and holding the keyboard, for
    /// the next pick. The row keeps its place and shows its tick there. The
    /// highlight goes to it, which for Enter is where it already was, and
    /// the list does not move. A row the cap now leaves out lights the last
    /// row. The list stays open even when every row is ticked.
    pub(in crate::workspace) fn toggle_row(
        &mut self,
        facet: Facet,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle(facet.clone(), cx);
        let rows = self.list_rows(cx);
        let at = rows.iter().position(|p| p.facet == facet);
        let last = rows.len().saturating_sub(1);
        if let Some(more) = &mut self.more {
            more.highlight = at.unwrap_or(more.highlight.min(last));
            more.reveal.set(true);
            more.up = false;
            focus_input(&more.input, window, cx);
        }
        cx.notify();
    }

    /// Close the open list when new text leaves it no facet in its scope.
    /// The next frame then gives the keyboard to the view.
    pub(super) fn close_list_if_gone(&mut self, cx: &mut Context<Self>) {
        // New text can leave the open list no facet at all in its scope
        // (its kind, or every kind for the search list): it closes, and the
        // next frame hands the keyboard from its field to the view. A pick
        // never closes it: the picked rows stay, ticked.
        let gone = self
            .more
            .as_ref()
            .is_some_and(|more| !self.list_holds(more.kind));
        if gone && let Some(more) = self.more.take() {
            self.stranded = Some(more.input.read(cx).focus_handle(cx));
        }
    }
}
