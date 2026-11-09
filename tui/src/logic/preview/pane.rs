//! The Preview pane's state: how much of the selected note shows
//! ([`ExpandState`]), which row it belongs to, and the content scroll
//! ([`ContentScroll`], anchored vs user-owned). Lifted out of the TUI's
//! `PreviewPane` so the scroll/anchor logic — the subtle part — is testable on
//! its own and shared by every UI; the TUI composes one of these with its
//! header hit region and its render.

use std::ops::Range;

use kimun_core::nfs::VaultPath;

/// How much of the selected note the preview shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExpandState {
    /// List only, no preview.
    Collapsed,
    /// Half-height preview below the list; sticks across selection moves.
    Context,
    /// Preview takes the whole panel; the list is hidden.
    Full,
}

/// The Preview pane's one "what makes a line hot" seam. FIND highlights query
/// needles scattered anywhere in a line; Ask Sources highlights one contiguous
/// byte range (the retrieved section). Every other part of the render — chrome,
/// wrapping, indent, scroll anchoring — is a single body shared by both; this
/// enum is the only place that body branches on which kind of highlight is
/// active.
pub enum Highlight<'a> {
    /// FIND: a (wrapped) line is hot if it contains any of these needles,
    /// case-insensitively; each match renders bold within the line.
    Needles(&'a [String]),
    /// Ask Sources: a source line is hot if its byte range intersects this
    /// one; the whole line renders bold when it does. `None` when no section
    /// resolved (nothing is hot) — distinct from an empty needle list, which
    /// is expressed as `Needles(&[])`.
    Range(Option<&'a Range<usize>>),
}

/// Scroll state for the expanded content views (Full mode and the half-height
/// Context preview). The offset is either *anchored* — the Context render
/// recomputes it from the first needle match each frame — or user-owned after a
/// scroll. Every transition (take-over, re-anchor, clamp) lives here, so paths
/// that should re-anchor have one decision point and the offset is never out of
/// range between events.
#[derive(Clone, Copy)]
pub struct ContentScroll {
    /// True while the render owns the offset (anchor on the first needle
    /// match). The first tick that actually moves the view flips it;
    /// re-anchoring events set it back.
    anchored: bool,
    /// The rendered scroll offset (first visible content line).
    offset: usize,
    /// Maximum offset, recorded by render from content/viewport size.
    max: usize,
}

impl Default for ContentScroll {
    fn default() -> Self {
        Self::new()
    }
}

impl ContentScroll {
    pub fn new() -> Self {
        Self {
            anchored: true,
            offset: 0,
            max: 0,
        }
    }

    /// Back to the top, offset handed back to the auto-anchor.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Re-arm the auto-anchor without touching the offset (the next anchored
    /// render overwrites it).
    pub fn re_anchor(&mut self) {
        self.anchored = true;
    }

    /// One wheel/key tick up, clamped at the top. Only a tick that moves the
    /// view takes the offset over from the anchor — a saturated no-op must
    /// not silently disarm it.
    pub fn scroll_up(&mut self) {
        if self.offset > 0 {
            self.offset -= 1;
            self.anchored = false;
        }
    }

    /// One wheel/key tick down, clamped at `max` at mutation time so the
    /// offset is never out of range. Same no-op rule as [`scroll_up`].
    ///
    /// [`scroll_up`]: Self::scroll_up
    pub fn scroll_down(&mut self) {
        if self.offset < self.max {
            self.offset += 1;
            self.anchored = false;
        }
    }

    /// Render-time sync: record the current max offset and clamp — a resize
    /// can shrink the content below the held offset.
    pub fn set_max(&mut self, max: usize) {
        self.max = max;
        self.offset = self.offset.min(max);
    }

    /// Render-time anchor: while anchored, place the offset (clamped). A
    /// user-owned offset is left alone.
    pub fn anchor_to(&mut self, offset: usize) {
        if self.anchored {
            self.offset = offset.min(self.max);
        }
    }

    /// Anchor the view on the line holding the first needle match: if the
    /// content from the link to the end fits the viewport, scroll back to fill
    /// it; otherwise show two lines of context above the link. No-op unless
    /// anchored. `set_max` must run first (this clamps against `max`).
    pub fn anchor_to_link(&mut self, link_pos: usize, total: usize, viewport: usize) {
        let lines_after_link = total.saturating_sub(link_pos);
        let target = if lines_after_link <= viewport {
            self.max
        } else {
            link_pos.saturating_sub(2)
        };
        self.anchor_to(target);
    }

    /// True while the render owns the offset.
    pub fn is_anchored(&self) -> bool {
        self.anchored
    }

    /// The first visible content line.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// The largest offset, as of the last [`set_max`](Self::set_max).
    pub fn max(&self) -> usize {
        self.max
    }
}

/// The Preview pane's state beneath/over a result list: the expand state, the
/// row it belongs to, and the content scroll.
pub struct PreviewModel {
    expand: ExpandState,
    /// The path the expand state belongs to, so a selection change re-anchors.
    expand_path: Option<VaultPath>,
    scroll: ContentScroll,
}

impl Default for PreviewModel {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewModel {
    pub fn new() -> Self {
        Self {
            expand: ExpandState::Collapsed,
            expand_path: None,
            scroll: ContentScroll::new(),
        }
    }

    pub fn expand(&self) -> ExpandState {
        self.expand
    }

    pub fn is_collapsed(&self) -> bool {
        self.expand == ExpandState::Collapsed
    }

    pub fn is_context(&self) -> bool {
        self.expand == ExpandState::Context
    }

    pub fn is_full(&self) -> bool {
        self.expand == ExpandState::Full
    }

    /// Collapse to the list and re-arm the auto-anchor (programmatic resets:
    /// query change, sort, note change).
    pub fn reset(&mut self) {
        self.expand = ExpandState::Collapsed;
        self.expand_path = None;
        self.scroll.reset();
    }

    /// Re-arm the auto-anchor without changing the expand state (a query edit
    /// moves the matches, so a user scroll position is stale).
    pub fn re_anchor(&mut self) {
        self.scroll.re_anchor();
    }

    /// Re-point the preview at `selected`, keeping the expand state and
    /// re-anchoring the scroll from the top — for a *directed* reveal (Ask's
    /// `open_reader`, or a same-note section change) that must land revealed on
    /// the requested source rather than collapse the way a plain selection move
    /// ([`sync`](Self::sync)) would. No-op without a selection.
    pub fn repoint(&mut self, selected: Option<VaultPath>) {
        if selected.is_none() {
            return;
        }
        self.expand_path = selected;
        self.scroll.reset();
    }

    pub fn scroll_up(&mut self) {
        self.scroll.scroll_up();
    }

    pub fn scroll_down(&mut self) {
        self.scroll.scroll_down();
    }

    /// Re-anchor the expand state on the currently-selected row. The Context
    /// (half-height) preview sticks across selection moves: it stays open and
    /// re-anchors on the new row. Full collapses, and a vanished selection
    /// always collapses. Returns `true` when the state changed, so the caller
    /// drops its stale regions.
    pub fn sync(&mut self, selected: Option<VaultPath>) -> bool {
        if selected == self.expand_path {
            return false;
        }
        if self.expand != ExpandState::Context || selected.is_none() {
            self.expand = ExpandState::Collapsed;
        }
        self.expand_path = selected;
        self.scroll.reset();
        true
    }

    /// Cycle the selected row's preview: Collapsed → Context → Full →
    /// Collapsed. No-op without a selection.
    pub fn toggle(&mut self, selected: Option<VaultPath>) {
        if selected.is_none() {
            return;
        }
        self.expand_path = selected;
        match self.expand {
            ExpandState::Collapsed => {
                self.expand = ExpandState::Context;
                self.scroll.re_anchor();
            }
            ExpandState::Context => {
                self.scroll.reset();
                self.expand = ExpandState::Full;
            }
            ExpandState::Full => {
                self.scroll.reset();
                self.expand = ExpandState::Collapsed;
            }
        }
    }

    /// Step the reveal cycle backward: Full → Context → Collapsed, stopping at
    /// Collapsed (the vim-natural `h` mirror of [`Self::toggle`]). No-op without a
    /// selection, or already Collapsed.
    pub fn collapse_step(&mut self, selected: Option<VaultPath>) {
        if selected.is_none() {
            return;
        }
        self.expand_path = selected;
        match self.expand {
            ExpandState::Full => {
                // Back to the half-height preview, re-anchored on the row.
                self.scroll.reset();
                self.expand = ExpandState::Context;
            }
            ExpandState::Context => {
                self.scroll.reset();
                self.expand = ExpandState::Collapsed;
            }
            ExpandState::Collapsed => {}
        }
    }

    /// Whether the render should look for the first hot line: only while the
    /// scroll is anchored (a user-owned scroll never reads it).
    pub fn wants_hit(&self) -> bool {
        self.scroll.anchored
    }

    /// Render-time placement: the content laid out to `total` lines in a
    /// `viewport` of lines, `hit` = the first hot line (when
    /// [`wants_hit`](Self::wants_hit) asked for it). Records the max offset,
    /// anchors on the hit while anchored, and returns the offset to draw from.
    pub fn place(&mut self, total: usize, viewport: usize, hit: Option<usize>) -> usize {
        self.scroll.set_max(total.saturating_sub(viewport));
        self.scroll
            .anchor_to_link(hit.unwrap_or(0), total, viewport);
        self.scroll.offset
    }

    /// The content scroll (offset, max, anchored).
    pub fn scroll(&self) -> &ContentScroll {
        &self.scroll
    }

    /// Simulate a user-owned scroll without a viewport-sized content set.
    #[cfg(test)]
    pub(crate) fn force_user_scrolled(&mut self) {
        self.scroll.anchored = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> VaultPath {
        VaultPath::note_path_from(name)
    }

    #[test]
    fn toggle_cycles_collapsed_context_full() {
        let mut p = PreviewModel::new();
        let sel = || Some(path("a"));
        assert!(p.is_collapsed());
        p.toggle(sel());
        assert!(p.is_context());
        p.toggle(sel());
        assert!(p.is_full());
        p.toggle(sel());
        assert!(p.is_collapsed());
    }

    #[test]
    fn toggle_without_selection_is_noop() {
        let mut p = PreviewModel::new();
        p.toggle(None);
        assert!(p.is_collapsed());
    }

    #[test]
    fn sync_keeps_context_across_selection_change() {
        let mut p = PreviewModel::new();
        p.toggle(Some(path("a"))); // -> Context, anchored on "a"
        assert!(p.is_context());
        // Moving to another row: Context sticks, re-anchored on the new row.
        let changed = p.sync(Some(path("b")));
        assert!(changed, "selection change must clear the stale region");
        assert!(p.is_context());
    }

    #[test]
    fn sync_collapses_full_on_selection_change() {
        let mut p = PreviewModel::new();
        p.toggle(Some(path("a")));
        p.toggle(Some(path("a"))); // -> Full
        assert!(p.is_full());
        p.sync(Some(path("b")));
        assert!(p.is_collapsed(), "Full does not stick across rows");
    }

    #[test]
    fn sync_collapses_when_selection_vanishes() {
        let mut p = PreviewModel::new();
        p.toggle(Some(path("a"))); // Context
        p.sync(None);
        assert!(p.is_collapsed());
    }

    #[test]
    fn sync_same_selection_is_noop() {
        let mut p = PreviewModel::new();
        p.toggle(Some(path("a")));
        assert!(!p.sync(Some(path("a"))), "no change, no region clear");
    }

    #[test]
    fn repoint_keeps_full_and_rearms_the_anchor() {
        let mut p = PreviewModel::new();
        p.toggle(Some(path("a"))); // Context
        p.toggle(Some(path("a"))); // Full
        p.scroll_down();
        p.force_user_scrolled();
        assert!(p.is_full() && !p.scroll().is_anchored());
        // A directed re-point at another source stays Full and re-anchors.
        p.repoint(Some(path("b")));
        assert!(p.is_full(), "repoint keeps the expand state (unlike sync)");
        assert!(
            p.scroll().is_anchored(),
            "repoint re-arms the scroll anchor"
        );
        assert_eq!(p.scroll().offset(), 0);
    }

    #[test]
    fn repoint_without_selection_is_noop() {
        let mut p = PreviewModel::new();
        p.toggle(Some(path("a")));
        p.repoint(None);
        assert!(p.is_context(), "no-selection repoint must not change state");
    }

    #[test]
    fn reset_collapses_and_rearms() {
        let mut p = PreviewModel::new();
        p.toggle(Some(path("a")));
        p.scroll_down();
        p.reset();
        assert!(p.is_collapsed());
        assert!(p.scroll.anchored && p.scroll.offset == 0);
    }

    #[test]
    fn scroll_clamps_and_takes_over_from_anchor() {
        let mut s = ContentScroll::new();
        s.set_max(3);
        assert!(s.anchored);
        s.scroll_up(); // already at top → no-op, stays anchored
        assert!(s.anchored && s.offset == 0);
        s.scroll_down();
        assert!(!s.anchored, "a real move disarms the anchor");
        assert_eq!(s.offset, 1);
        s.scroll_down();
        s.scroll_down();
        s.scroll_down(); // clamped at max
        assert_eq!(s.offset, 3);
    }

    #[test]
    fn anchor_to_link_fills_viewport_when_tail_fits() {
        let mut s = ContentScroll::new();
        // 10 lines, viewport 5 → max offset 5. Link near the end: tail fits, so
        // scroll back to max to fill the viewport.
        s.set_max(5);
        s.anchor_to_link(8, 10, 5);
        assert_eq!(s.offset, 5);
    }

    #[test]
    fn anchor_to_link_shows_two_lines_of_context_above() {
        let mut s = ContentScroll::new();
        // Link deep in long content, tail does NOT fit → show link_pos - 2.
        s.set_max(100);
        s.anchor_to_link(40, 200, 10);
        assert_eq!(s.offset, 38);
    }

    #[test]
    fn anchor_to_link_is_noop_once_user_scrolled() {
        let mut s = ContentScroll::new();
        s.set_max(100);
        s.scroll_down(); // user owns the offset now (offset 1, not anchored)
        s.anchor_to_link(40, 200, 10);
        assert_eq!(s.offset, 1, "user-owned offset is not re-anchored");
    }

    #[test]
    fn collapse_step_steps_back_and_stops_at_collapsed() {
        let mut p = PreviewModel::new();
        let sel = || Some(path("a"));
        p.toggle(sel()); // Collapsed -> Context
        p.toggle(sel()); // Context -> Full
        assert!(p.is_full());
        p.collapse_step(sel()); // Full -> Context
        assert!(p.is_context());
        p.collapse_step(sel()); // Context -> Collapsed
        assert!(p.is_collapsed());
        p.collapse_step(sel()); // Collapsed stays Collapsed (h no-op at bottom)
        assert!(p.is_collapsed());
    }

    #[test]
    fn collapse_step_without_selection_is_noop() {
        let mut p = PreviewModel::new();
        p.toggle(Some(path("a")));
        p.collapse_step(None);
        assert!(
            p.is_context(),
            "no-selection collapse_step must not change state"
        );
    }
}
