//! Overlays: styled ranges of logical columns on one row (see CONTEXT.md
//! **Overlay**), and [`OverlaySet`], the per-run record of which kinds cover
//! it. Presentation (colours) lives with the painter, never here.

/// A styled range of logical columns on one row (see CONTEXT.md **Overlay**).
///
/// Every highlight the editor paints over a rendered line has this shape. They
/// arrive in logical coordinates so producers never reason about rendered
/// columns — markdown conceals sigils, so the two differ — and the mapping
/// happens once, in the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overlay {
    pub row: usize,
    /// Logical char column where the overlay starts.
    pub start: usize,
    /// Logical char column just past its end.
    pub end: usize,
    pub kind: OverlayKind,
}

impl Overlay {
    pub fn new(row: usize, start: usize, end: usize, kind: OverlayKind) -> Self {
        Self {
            row,
            start,
            end,
            kind,
        }
    }
}

/// What an [`Overlay`] means, and — by declaration order — how it stacks.
///
/// Later kinds paint over earlier ones. That order used to be implicit in
/// statement order across `view.rs`'s render loop and `mod.rs`'s cell post-pass,
/// which meant reasoning it out by hand for each new highlight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OverlayKind {
    /// A task checkbox: `- [ ]` / `- [x]`.
    TaskBox,
    /// The text of a completed task, struck through.
    TaskDone,
    /// A vault-search **needle** carried in from the query that opened the note.
    Needle,
    /// A **find pattern** match.
    Match,
    /// The editor's selection.
    Selection,
    /// The **current match** — where the next find-bar action lands.
    CurrentMatch,
    /// Text the **replace preview** is showing in place of a match.
    Preview,
    /// The previewed **current match**.
    PreviewCurrent,
    /// A **follow target** (wikilink, markdown link, image link, hashtag
    /// label) as drawn, emphasis inside its label included. No style of its
    /// own: a pointer host marks the cells clickable. Not derived for the
    /// element the caret is in, which is being edited as raw source.
    FollowTarget,
}

impl OverlayKind {
    /// Every kind, in stacking (declaration) order.
    pub const ALL: [OverlayKind; 9] = [
        OverlayKind::TaskBox,
        OverlayKind::TaskDone,
        OverlayKind::Needle,
        OverlayKind::Match,
        OverlayKind::Selection,
        OverlayKind::CurrentMatch,
        OverlayKind::Preview,
        OverlayKind::PreviewCurrent,
        OverlayKind::FollowTarget,
    ];

    /// The kind's position in declaration order. An exhaustive `match`, so
    /// adding a variant fails to compile here until [`Self::ALL`] and the
    /// assertions below are updated with it.
    const fn index(self) -> u8 {
        match self {
            OverlayKind::TaskBox => 0,
            OverlayKind::TaskDone => 1,
            OverlayKind::Needle => 2,
            OverlayKind::Match => 3,
            OverlayKind::Selection => 4,
            OverlayKind::CurrentMatch => 5,
            OverlayKind::Preview => 6,
            OverlayKind::PreviewCurrent => 7,
            OverlayKind::FollowTarget => 8,
        }
    }

    const fn bit(self) -> u16 {
        1 << self.index()
    }
}

// Guarantees: every `ALL` entry has `index()` and discriminant equal to its
// position, and the variant count fits `OverlaySet`'s `u16` bitset. Adding a
// variant fails the exhaustive `index()` match until it is handled. Omitting
// the LAST variant from `ALL` is not caught by the compiler.
const _: () = {
    assert!(OverlayKind::ALL.len() <= u16::BITS as usize);
    let mut i = 0;
    while i < OverlayKind::ALL.len() {
        assert!(OverlayKind::ALL[i].index() as usize == i);
        assert!(OverlayKind::ALL[i] as u8 as usize == i);
        i += 1;
    }
    assert!(
        OverlayKind::ALL[OverlayKind::ALL.len() - 1].index() as usize == OverlayKind::ALL.len() - 1
    );
};

/// A set of [`OverlayKind`]s, one bit each.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OverlaySet(u16);

impl OverlaySet {
    pub fn insert(&mut self, kind: OverlayKind) {
        self.0 |= kind.bit();
    }

    pub fn contains(self, kind: OverlayKind) -> bool {
        self.0 & kind.bit() != 0
    }

    /// The kinds in the set, in ascending [`OverlayKind`] (stacking) order.
    pub fn iter(self) -> impl Iterator<Item = OverlayKind> {
        OverlayKind::ALL
            .into_iter()
            .filter(move |k| self.contains(*k))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_set_iterates_in_kind_order_and_dedups() {
        let mut s = OverlaySet::default();
        s.insert(OverlayKind::Selection);
        s.insert(OverlayKind::TaskBox);
        s.insert(OverlayKind::Selection);
        assert_eq!(
            s.iter().collect::<Vec<_>>(),
            vec![OverlayKind::TaskBox, OverlayKind::Selection]
        );
    }

    #[test]
    fn overlay_kind_all_is_sorted_and_complete() {
        let mut sorted = OverlayKind::ALL;
        sorted.sort();
        assert_eq!(sorted, OverlayKind::ALL);
        let mut s = OverlaySet::default();
        for k in OverlayKind::ALL {
            assert!(!s.contains(k));
            s.insert(k);
            assert!(s.contains(k));
        }
        assert_eq!(s.iter().count(), 9);
    }
}
