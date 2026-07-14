//! The editor's one revision clock. `current` advances only when the buffer
//! text actually changes — bumped synchronously on the textarea backend,
//! adopted from the built frame snapshot on the nvim backend — and every
//! comparison against it (dirty tracking, search-needle staleness) lives
//! here. One domain, one `+1` site, no parallel counters: the old
//! `edit_generation` (write-only) and the render-time double mirror were
//! deleted when this type concentrated the bookkeeping.
//!
//! `NonZeroU64` makes 0 unrepresentable so "no revision" is
//! `Option<NonZeroU64>::None` rather than a magic value; `bump` substitutes
//! 1 on the astronomical wrap-around.

use std::num::NonZeroU64;

/// Revision bookkeeping for one editor buffer: the current content
/// revision plus the two snapshots compared against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Revisions {
    /// The buffer's content revision. Stable across cursor moves and idle
    /// frames; changes iff the text changed.
    current: NonZeroU64,
    /// The revision the on-disk content matches. `None` when a save is
    /// known to have diverged from the buffer.
    saved: Option<NonZeroU64>,
    /// The revision the search-match needles were armed against. Needles
    /// die on the first edit (`needles_stale`).
    needles: Option<NonZeroU64>,
}

impl Revisions {
    /// A fresh buffer starts at revision 1, clean, with no needles armed.
    pub fn new() -> Self {
        let one = NonZeroU64::new(1).unwrap();
        Self {
            current: one,
            saved: Some(one),
            needles: None,
        }
    }

    pub fn current(&self) -> NonZeroU64 {
        self.current
    }

    /// Advance the content revision (the buffer text changed). Skips zero
    /// on wrap-around so `current` stays `NonZeroU64`.
    pub fn bump(&mut self) {
        let next = self.current.get().wrapping_add(1);
        self.current = NonZeroU64::new(next).unwrap_or(NonZeroU64::new(1).unwrap());
    }

    /// Map the nvim backend's `content_gen` counter (0-based, bumped by
    /// the reverse-refresh task on real diffs) into the revision domain.
    /// The one gen→revision convention site: `snapshot_from_backend` calls
    /// this, and [`adopt`](Self::adopt) takes the result — revision math
    /// never lives outside this type.
    pub fn rev_from_gen(content_gen: u64) -> NonZeroU64 {
        NonZeroU64::new(content_gen.saturating_add(1)).unwrap_or(NonZeroU64::new(1).unwrap())
    }

    /// Adopt an externally-produced revision — the per-frame nvim mirror,
    /// where the frame snapshot carries [`rev_from_gen`](Self::rev_from_gen)
    /// of the backend's `content_gen`. An unconditional assignment: the
    /// textarea path adopts the value it supplied to the snapshot (a
    /// self-assign), and the nvim path trusts the snapshot as the single
    /// producer. It can therefore move the clock backward if handed a
    /// stale value — the sole production caller adopts the snapshot built
    /// in the same frame, which cannot be stale.
    pub fn adopt(&mut self, rev: NonZeroU64) {
        self.current = rev;
    }

    /// Whether the buffer diverges from its last saved snapshot.
    pub fn is_dirty(&self) -> bool {
        self.saved != Some(self.current)
    }

    /// Mark `rev` saved iff it is still the current revision — a stale
    /// async save completion must not clobber a newer clean state. Returns
    /// whether the mark applied, so the caller can run backend side
    /// effects (nvim `mark_clean`) only on the applied path.
    pub fn mark_saved_at(&mut self, rev: NonZeroU64) -> bool {
        if rev != self.current {
            return false;
        }
        self.saved = Some(rev);
        true
    }

    /// Mark the current revision saved (synchronous save path — the caller
    /// held `&mut` across the whole save, so no newer state can exist).
    pub fn mark_saved_current(&mut self) {
        self.saved = Some(self.current);
    }

    /// A save diverged from the buffer: forget the saved snapshot so
    /// `is_dirty` reads true until the next successful save.
    pub fn mark_diverged(&mut self) {
        self.saved = None;
    }

    /// Arm the search-needle emphasis against the current revision.
    pub fn arm_needles(&mut self) {
        self.needles = Some(self.current);
    }

    /// Whether armed needles have been outlived by an edit.
    pub fn needles_stale(&self) -> bool {
        self.needles.is_some_and(|r| r != self.current)
    }

    pub fn disarm_needles(&mut self) {
        self.needles = None;
    }
}

impl Default for Revisions {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nz(v: u64) -> NonZeroU64 {
        NonZeroU64::new(v).unwrap()
    }

    #[test]
    fn fresh_buffer_is_clean_at_revision_one() {
        let r = Revisions::new();
        assert_eq!(r.current(), nz(1));
        assert!(!r.is_dirty());
        assert!(!r.needles_stale());
    }

    #[test]
    fn bump_dirties_and_mark_saved_current_cleans() {
        let mut r = Revisions::new();
        r.bump();
        assert_eq!(r.current(), nz(2));
        assert!(r.is_dirty());
        r.mark_saved_current();
        assert!(!r.is_dirty());
    }

    #[test]
    fn stale_save_completion_does_not_apply() {
        let mut r = Revisions::new();
        r.bump(); // rev 2, save issued here
        let issued = r.current();
        r.bump(); // rev 3, user typed during the save
        assert!(!r.mark_saved_at(issued), "stale completion must not apply");
        assert!(r.is_dirty(), "buffer stays dirty after a stale completion");
    }

    #[test]
    fn current_save_completion_applies_and_cleans() {
        let mut r = Revisions::new();
        r.bump();
        let issued = r.current();
        assert!(r.mark_saved_at(issued));
        assert!(!r.is_dirty());
    }

    #[test]
    fn diverged_save_stays_dirty_until_next_save() {
        let mut r = Revisions::new();
        r.mark_diverged();
        assert!(r.is_dirty());
        r.mark_saved_current();
        assert!(!r.is_dirty());
    }

    #[test]
    fn adopt_moves_the_clock_and_preserves_dirty_semantics() {
        let mut r = Revisions::new();
        r.mark_saved_current(); // clean at 1
        r.adopt(nz(7)); // nvim refresh observed content changes
        assert_eq!(r.current(), nz(7));
        assert!(r.is_dirty(), "saved snapshot (1) no longer matches");
        r.adopt(nz(7)); // same-value adopt is a no-op
        assert_eq!(r.current(), nz(7));
    }

    #[test]
    fn bump_skips_zero_on_wraparound() {
        let mut r = Revisions::new();
        r.adopt(nz(u64::MAX));
        r.bump();
        assert_eq!(r.current(), nz(1), "wrap-around must substitute 1, never 0");
    }

    #[test]
    fn rev_from_gen_maps_the_backend_counter_into_the_nonzero_domain() {
        // gen 0 (fresh backend, nothing observed yet) maps to revision 1 —
        // the same value a fresh buffer starts at, so nothing looks edited.
        assert_eq!(Revisions::rev_from_gen(0), nz(1));
        assert_eq!(Revisions::rev_from_gen(6), nz(7));
        // Saturation at the top keeps the result nonzero instead of
        // wrapping to 0.
        assert_eq!(Revisions::rev_from_gen(u64::MAX), nz(u64::MAX));
    }

    #[test]
    fn needles_die_on_first_edit() {
        let mut r = Revisions::new();
        r.arm_needles();
        assert!(!r.needles_stale(), "just armed — not stale");
        r.bump();
        assert!(r.needles_stale(), "an edit outlives the needles");
        r.disarm_needles();
        assert!(!r.needles_stale(), "disarmed needles are never stale");
    }
}
