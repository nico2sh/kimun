//! Read-only views into the facet view, for tests.
//!
//! It owns the test accessors of [`FacetView`]. Only test builds compile it.
//! They let a test look at the view without a change to its fields.
//! It does not own any behavior of the view. A seam here must not change
//! state, so a test sees what a user would see.

use std::ops::Range;

use gpui_kit::component::input::InputState;
use gpui_kit::{App, Entity, FocusHandle, Pixels, SharedString};
use kimun_core::note::Facet;

use super::keys::Stop;
use super::{FacetView, ListKind};
use crate::editor_view::EditorView;

impl FacetView {
    /// The note text a single-note view shows; empty for a collection
    /// (tests).
    pub(crate) fn text(&self) -> &str {
        self.single().map_or("", |member| &member.text)
    }

    /// The chunks' editors, in order (tests).
    pub(crate) fn chunks(&self) -> &[Entity<EditorView>] {
        &self.chunks
    }

    /// The open list's query field (tests).
    pub(crate) fn more_input(&self) -> Option<Entity<InputState>> {
        self.more.as_ref().map(|more| more.input.clone())
    }

    /// The open list's results, in order (tests).
    pub(crate) fn list_results(&self, cx: &App) -> Vec<Facet> {
        self.list_rows(cx).into_iter().map(|p| p.facet).collect()
    }

    /// The open list's drawn rows, in order (tests).
    pub(crate) fn list_row_pills(&self, cx: &App) -> Vec<kimun_notes::logic::facet_bar::Pill> {
        self.list_rows(cx)
    }

    /// The ids of the tab stops the last frame drew, in order (tests).
    pub(crate) fn drawn_stop_ids(&self) -> Vec<SharedString> {
        self.stops.borrow().drawn().iter().map(Stop::id).collect()
    }

    /// The element drawn with the focus ring in the last frame (tests).
    pub(crate) fn ringed(&self) -> Option<SharedString> {
        self.stops.borrow().ringed.as_ref().map(Stop::id)
    }

    /// How many cleared pick sets Ctrl+Z can bring back (tests).
    pub(crate) fn undo_depth(&self) -> usize {
        self.undo.sets().len()
    }

    /// The cleared pick sets on the undo stack, oldest first (tests).
    pub(crate) fn undo_sets(&self) -> &[Vec<Facet>] {
        self.undo.sets()
    }

    /// The chip the last frame drew open, in the selected style (tests).
    pub(crate) fn chip_drawn_open(&self) -> Option<ListKind> {
        self.open_chip.get()
    }

    /// The chips the last frame drew dimmed (tests).
    pub(crate) fn chips_drawn_dimmed(&self) -> Vec<ListKind> {
        self.dimmed_chips.borrow().clone()
    }

    /// How many times the chunks were planned (tests).
    pub(crate) fn plans_run(&self) -> usize {
        self.plans_run
    }

    /// How many measuring frames the bar drew (tests).
    pub(crate) fn bar_measure_frames(&self) -> usize {
        self.bar_motion.measure_frames()
    }

    /// Whether `kind`'s list is the open one (tests).
    pub(crate) fn list_is_open(&self, kind: ListKind) -> bool {
        self.list_open(kind)
    }

    /// Draw every piece (`true`), as before the window existed, or only
    /// those near the viewport (tests).
    pub(crate) fn set_draw_all(&mut self, all: bool) {
        self.draw_all = all;
    }

    /// Draw at most `limit` pieces, as `Show more` would set it (tests).
    pub(crate) fn cap_pieces(&mut self, limit: usize, cx: &mut gpui_kit::Context<Self>) {
        self.limit = limit;
        self.plan(cx);
    }

    /// The pieces the last frame drew as a range, pins left out (tests).
    pub(crate) fn drawn_range(&self) -> Range<usize> {
        self.drawn.clone()
    }

    /// How many drawn pieces the last frame checked against the height
    /// table (tests).
    pub(crate) fn checked_items(&self) -> usize {
        self.checked.get()
    }

    /// Add `extra` to the crumbs row the height table counts, so the table
    /// is wrong (tests).
    pub(crate) fn forge_crumbs(&mut self, extra: Pixels) {
        self.forged = extra;
    }

    /// The rows the height table holds for piece `i` now, and the editor
    /// width it measured at (tests).
    pub(crate) fn table_rows(&self, i: usize, cx: &App) -> (Option<usize>, Pixels) {
        let heights = self.heights.borrow();
        let chunk = &self.chunks[i];
        let stamp = chunk.read(cx).edit_stamp();
        let rows = heights.rows(i, chunk.entity_id().as_u64(), stamp);
        (rows, heights.width())
    }

    /// How many piece editors were asked for the keyboard so far (tests).
    pub(crate) fn focus_scans(&self) -> usize {
        self.focus_scans.get()
    }

    /// The focus handle of piece `i`'s `Open note`, while it is drawn
    /// (tests).
    pub(crate) fn open_handle(&self, i: usize) -> Option<FocusHandle> {
        self.stops.borrow().handle(&Stop::Open(i))
    }

    /// Ask that piece `i`'s `Open note` takes the keyboard once drawn
    /// (tests).
    pub(crate) fn ask_focus_open(&mut self, i: usize) {
        self.focus_after = Some(Stop::Open(i));
    }
}
