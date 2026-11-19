//! When a note's frontmatter block shows open and when it folds to one row.
//!
//! Pure state, no layout and no text: the display feeds it the cursor, the
//! pointer and the find bar, and asks whether the block is open. Four things
//! open it — the pointer over it, the cursor inside it, the find bar's current
//! match inside it, or a pin the reader set with a click. Otherwise it folds.

use std::ops::Range;

/// What keeps a frontmatter block open, besides the cursor's own place.
#[derive(Debug, Clone, Default)]
pub struct FrontmatterReveal {
    pinned: bool,
    pointer_over: bool,
    find_row: Option<usize>,
    /// Whether the reader has done anything since the note loaded. A note
    /// opens with its cursor on line 1, which is the block's own fence, so
    /// that spot alone must not open the block.
    armed: bool,
    /// Cursor and content revision at the first frame after a load.
    baseline: Option<((usize, usize), u64)>,
}

impl FrontmatterReveal {
    /// A different note replaced the buffer: forget the pin and the first-move
    /// state. The pointer stays where it is.
    pub fn note_loaded(&mut self) {
        *self = Self {
            pointer_over: self.pointer_over,
            ..Self::default()
        };
    }

    /// Record this frame's cursor and content revision. The first call after
    /// a load takes them as the baseline, and the first change from it arms.
    pub fn observe(&mut self, cursor: (usize, usize), revision: u64) {
        match self.baseline {
            None => self.baseline = Some((cursor, revision)),
            Some((at, rev)) if !self.armed && (at != cursor || rev != revision) => {
                self.armed = true;
            }
            Some(_) => {}
        }
    }

    pub fn set_pointer_over(&mut self, over: bool) {
        self.pointer_over = over;
    }

    /// The row of the find bar's current match, or `None` when there is none.
    pub fn set_find_row(&mut self, row: Option<usize>) {
        self.find_row = row;
    }

    pub fn pin(&mut self, pinned: bool) {
        self.pinned = pinned;
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned
    }

    /// Whether a block covering `lines` shows open with the cursor at `cursor`.
    pub fn is_open(&self, lines: &Range<usize>, cursor: (usize, usize)) -> bool {
        self.pinned
            || self.pointer_over
            || self.find_row.is_some_and(|row| lines.contains(&row))
            || self.cursor_inside(lines, cursor)
    }

    fn cursor_inside(&self, lines: &Range<usize>, cursor: (usize, usize)) -> bool {
        lines.contains(&cursor.0) && (self.armed || cursor != (0, 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: Range<usize> = 0..4;

    /// A reveal that has seen one frame of a freshly loaded note.
    fn loaded() -> FrontmatterReveal {
        let mut reveal = FrontmatterReveal::default();
        reveal.observe((0, 0), 1);
        reveal
    }

    #[test]
    fn a_block_is_folded_when_nothing_opens_it() {
        assert!(!loaded().is_open(&BLOCK, (6, 0)));
    }

    #[test]
    fn the_pointer_over_the_block_opens_it() {
        let mut reveal = loaded();
        reveal.set_pointer_over(true);
        assert!(reveal.is_open(&BLOCK, (6, 0)));
        reveal.set_pointer_over(false);
        assert!(!reveal.is_open(&BLOCK, (6, 0)));
    }

    #[test]
    fn the_cursor_inside_the_block_opens_it() {
        let reveal = loaded();
        for row in 1..4 {
            assert!(reveal.is_open(&BLOCK, (row, 0)), "row {row}");
        }
        assert!(!reveal.is_open(&BLOCK, (4, 0)), "the row below is outside");
    }

    #[test]
    fn the_cursor_on_line_one_of_a_fresh_note_does_not_open_it() {
        assert!(!loaded().is_open(&BLOCK, (0, 0)));
    }

    #[test]
    fn a_cursor_elsewhere_in_the_block_opens_it_even_on_a_fresh_note() {
        // A note restored with its cursor inside the block must not hide it.
        assert!(loaded().is_open(&BLOCK, (2, 3)));
        assert!(loaded().is_open(&BLOCK, (0, 1)));
    }

    #[test]
    fn a_move_arms_line_one_to_open_the_block() {
        let mut reveal = loaded();
        reveal.observe((6, 0), 1);
        assert!(reveal.is_open(&BLOCK, (0, 0)));
    }

    #[test]
    fn an_edit_arms_it_too() {
        let mut reveal = loaded();
        reveal.observe((0, 0), 2);
        assert!(reveal.is_open(&BLOCK, (0, 0)));
    }

    #[test]
    fn the_first_frame_after_a_load_is_the_baseline_not_a_move() {
        let mut reveal = FrontmatterReveal::default();
        reveal.observe((0, 0), 7);
        reveal.observe((0, 0), 7);
        assert!(!reveal.is_open(&BLOCK, (0, 0)));
    }

    #[test]
    fn loading_another_note_disarms_and_unpins() {
        let mut reveal = loaded();
        reveal.observe((6, 0), 1);
        reveal.pin(true);
        reveal.note_loaded();
        reveal.observe((0, 0), 1);
        assert!(!reveal.is_open(&BLOCK, (0, 0)));
        assert!(!reveal.is_pinned());
    }

    #[test]
    fn loading_another_note_keeps_the_pointer() {
        let mut reveal = loaded();
        reveal.set_pointer_over(true);
        reveal.note_loaded();
        assert!(reveal.is_open(&BLOCK, (6, 0)));
    }

    #[test]
    fn a_find_match_inside_the_block_opens_it() {
        let mut reveal = loaded();
        reveal.set_find_row(Some(2));
        assert!(reveal.is_open(&BLOCK, (6, 0)));
        reveal.set_find_row(Some(9));
        assert!(!reveal.is_open(&BLOCK, (6, 0)));
        reveal.set_find_row(None);
        assert!(!reveal.is_open(&BLOCK, (6, 0)));
    }

    #[test]
    fn a_pin_holds_the_block_open_until_it_is_released() {
        let mut reveal = loaded();
        reveal.pin(true);
        assert!(reveal.is_open(&BLOCK, (6, 0)));
        reveal.pin(false);
        assert!(!reveal.is_open(&BLOCK, (6, 0)));
    }
}
