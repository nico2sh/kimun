//! Where rows break, and which cell a position is drawn in.
//!
//! A layout is the visual lines a [`Text`] wraps into at a width, plus the
//! mapping between a [`Position`] and a screen cell. It is derived, and it is
//! derived from three things: the text, the width, and the caller's per-row
//! [`RowHints`].
//!
//! A layout does not outlive a resize, which is why it is separate from the
//! buffer, and it does not hold the text, which is why the queries that need to
//! measure characters take the text again.
//!
//! # Why hints
//!
//! A syntax layer that conceals characters — markdown hiding the `#` of a
//! heading — changes how wide a row draws without changing what it contains. A
//! layout that measured the row's text would break lines in the wrong places. So
//! the caller says, per row, which clusters are drawn and how far the row is
//! inset, and this module never learns what a heading is.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::position::{Column, Position, Revision};
use crate::text::Text;
use crate::width::Metrics;

/// What a syntax layer tells the layout about one logical row.
#[derive(Debug, Clone, Copy, Default)]
pub struct RowHints<'a> {
    /// Per Unicode scalar of the row: `false` where the renderer draws nothing.
    /// A shorter slice than the row means the rest is visible, and an empty one
    /// means all of it is — so a caller with no syntax layer passes nothing.
    ///
    /// Stated as *visible* rather than hidden because that is what a syntax layer
    /// computes: it walks a row deciding what to draw. Inverting it here would cost
    /// an allocation per row per frame to say the same thing.
    pub visible: &'a [bool],
    /// Cells of gutter the renderer draws before the row's text, on the first
    /// visual line and every continuation of it.
    pub inset: usize,
}

/// One drawn line: a slice of a logical row that fits the width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualLine {
    pub logical_row: usize,
    /// Scalar offsets within the logical row.
    pub chars: Range<usize>,
    /// Byte offsets within the logical row.
    pub bytes: Range<usize>,
    /// Whether this is the row's first visual line, as against a continuation.
    pub first: bool,
}

/// A screen cell, relative to the top-left of the laid-out text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// Index into [`Layout::visual_lines`].
    pub row: usize,
    /// Cells from the left edge, gutter included.
    pub column: usize,
}

/// Where a text's rows break at a given width.
#[derive(Debug, Clone)]
pub struct Layout {
    lines: Vec<VisualLine>,
    /// Logical row → index of its first visual line. Turns a lookup into a walk
    /// over one row's wrap count rather than over every visual line.
    row_starts: Vec<usize>,
    width: usize,
    metrics: Metrics,
    /// Which text this describes.
    ///
    /// A [`VisualLine`] holds byte ranges into the text it was laid out from, and
    /// reading one against a newer text slices out of bounds. Callers used to
    /// guess at staleness by comparing row counts, which an edit within a single
    /// row does not change — so shrinking a row and pressing an arrow before the
    /// next frame panicked. The text already carries an identity; recording it is
    /// what makes the question answerable rather than approximable.
    revision: Revision,
}

impl Layout {
    /// Whether this layout still describes `text`.
    ///
    /// The only safe precondition for anything that reads a [`VisualLine`]'s byte
    /// range against a text — `cell_of`, `position_at_cell`, and any caller
    /// slicing a row itself. A row count is not a substitute: an edit inside one
    /// row leaves it unchanged while every byte range after the edit moves.
    pub fn describes(&self, text: &Text) -> bool {
        self.revision == text.revision()
    }

    /// Lay `text` out as one unwrapped visual line per row — no grapheme
    /// segmentation, no width measurement, no break search.
    ///
    /// For a caller that needs *some* layout describing `text` right now and
    /// cannot afford `compute`'s cost this instant (a large buffer, off the
    /// keystroke that triggered a full rebuild). `describes` is true the
    /// moment this returns, so nothing downstream has to know the wrap is
    /// wrong — only that a genuinely long row will not soft-wrap until a
    /// real `compute` replaces this one. `row_count` still matches `text`,
    /// which is the invariant every other reader depends on.
    pub fn unwrapped(text: &Text) -> Self {
        let mut lines = Vec::with_capacity(text.line_count());
        let mut row_starts = Vec::with_capacity(text.line_count());
        for row in 0..text.line_count() {
            row_starts.push(lines.len());
            let Some(source) = text.line(row) else {
                continue;
            };
            lines.push(VisualLine {
                logical_row: row,
                chars: 0..source.chars().count(),
                bytes: 0..source.len(),
                first: true,
            });
        }
        Self {
            lines,
            row_starts,
            width: 0,
            metrics: Metrics::default(),
            revision: text.revision(),
        }
    }

    /// Lay `text` out at `width` cells.
    pub fn compute(text: &Text, width: usize, metrics: Metrics, hints: &[RowHints<'_>]) -> Self {
        let mut layout = Self {
            lines: Vec::new(),
            row_starts: Vec::with_capacity(text.line_count()),
            width,
            metrics,
            revision: text.revision(),
        };
        let mut scratch = Vec::new();
        for row in 0..text.line_count() {
            layout.row_starts.push(layout.lines.len());
            wrap_row(
                text,
                row,
                width,
                metrics,
                hint_for(hints, row),
                &mut scratch,
                &mut layout.lines,
            );
        }
        layout
    }

    /// Re-wrap `rows` in place, leaving the rest alone.
    ///
    /// For a caller holding a [`Change`](crate::Change), whose `rows` is exactly
    /// this argument. Rows outside the range must be unchanged in content and in
    /// hints; rows inside it may have become any number of visual lines.
    pub fn relayout_rows(
        &mut self,
        text: &Text,
        hints: &[RowHints<'_>],
        rows: Range<usize>,
        line_delta: isize,
    ) {
        // Whatever else this does, afterwards the layout describes `text`.
        self.revision = text.revision();
        // `rows` is in the *new* text's numbering, because that is what a `Change`
        // reports. The layout is still in the old text's, so the region being
        // replaced has to be named twice: once to find what to throw away, once to
        // say what replaces it.
        let rows = rows.start.min(text.line_count())..rows.end.min(text.line_count());
        let old_rows = {
            let end = (rows.end as isize - line_delta).max(rows.start as isize) as usize;
            rows.start.min(self.row_starts.len())..end.min(self.row_starts.len())
        };
        if rows.is_empty() && old_rows.is_empty() {
            return;
        }

        let old_start = self
            .row_starts
            .get(old_rows.start)
            .copied()
            .unwrap_or(self.lines.len());
        let old_end = self
            .row_starts
            .get(old_rows.end)
            .copied()
            .unwrap_or(self.lines.len());

        let mut replacement = Vec::new();
        let mut starts = Vec::with_capacity(rows.len());
        let mut scratch = Vec::new();
        for row in rows.clone() {
            starts.push(old_start + replacement.len());
            wrap_row(
                text,
                row,
                self.width,
                self.metrics,
                hint_for(hints, row),
                &mut scratch,
                &mut replacement,
            );
        }

        let added = replacement.len();
        self.lines.splice(old_start..old_end, replacement);

        // Every visual line after the replaced region belongs to a row that has
        // moved. Renumbering them is what keeps a visual line pointing at the row
        // it draws — without it, a row inserted above leaves every line below
        // slicing the wrong row's text, which reads as corruption rather than as a
        // stale layout.
        if line_delta != 0 {
            for line in &mut self.lines[old_start + added..] {
                line.logical_row = (line.logical_row as isize + line_delta) as usize;
            }
        }

        self.row_starts.splice(old_rows, starts);
        let shift = added as isize - (old_end - old_start) as isize;
        if shift != 0 {
            let tail = rows.end.min(self.row_starts.len());
            for start in &mut self.row_starts[tail..] {
                *start = (*start as isize + shift) as usize;
            }
        }
        debug_assert_eq!(
            self.row_starts.len(),
            text.line_count(),
            "relayout left the layout describing a different number of rows"
        );
    }

    pub fn visual_lines(&self) -> &[VisualLine] {
        &self.lines
    }

    /// How many visual lines the text occupies. Never zero.
    pub fn visual_line_count(&self) -> usize {
        self.lines.len()
    }

    /// How many logical rows this layout was built for. A caller comparing this
    /// with the text's row count is asking whether the layout is stale.
    pub fn row_count(&self) -> usize {
        self.row_starts.len()
    }

    pub fn width(&self) -> usize {
        self.width
    }

    /// Which visual line `position` is drawn on.
    pub fn visual_row_of(&self, position: Position) -> usize {
        let row = position.row().min(self.row_starts.len().saturating_sub(1));
        let first = self.row_starts.get(row).copied().unwrap_or(0);
        let column = position.column().get();
        self.lines[first..]
            .iter()
            .take_while(|line| line.logical_row == row)
            .enumerate()
            .filter(|(_, line)| line.chars.start <= column)
            .map(|(offset, _)| first + offset)
            .last()
            .unwrap_or(first)
    }

    /// Which cell `position` is drawn in.
    ///
    /// Takes the text and the hints because the layout stores where rows break,
    /// not what they contain, and a cell is a measurement of content.
    pub fn cell_of(&self, text: &Text, hints: &[RowHints<'_>], position: Position) -> Cell {
        // Returns a cell rather than an option, so it cannot refuse a stale text
        // the way `position_at_cell` does — the caller has to have checked. This
        // is what says so, and what catches a caller that has not.
        debug_assert!(
            self.describes(text),
            "cell_of read against a text this layout does not describe"
        );
        let row = self.visual_row_of(position);
        let line = &self.lines[row];
        let hint = hint_for(hints, line.logical_row);
        let Some(source) = text.line(line.logical_row) else {
            return Cell {
                row,
                column: hint.inset,
            };
        };
        let mut column = hint.inset;
        let mut chars = line.chars.start;
        for cluster in source[line.bytes.clone()].graphemes(true) {
            if chars >= position.column().get() {
                break;
            }
            if visible(&hint, chars) {
                column += self.metrics.width_at(cluster, column - hint.inset);
            }
            chars += cluster.chars().count();
        }
        Cell { row, column }
    }

    /// The position drawn at `cell`, or `None` if there is no such visual line.
    ///
    /// A column past the end of a visual line lands at its end, and a column
    /// inside a wide cluster lands on that cluster: a click between the halves of
    /// a CJK character means the character.
    pub fn position_at_cell(
        &self,
        text: &Text,
        hints: &[RowHints<'_>],
        cell: Cell,
    ) -> Option<Position> {
        // A visual line's byte range addresses the text this was laid out from.
        // Read against a newer one it slices out of bounds, so a stale layout is
        // refused here rather than trusted — see [`Self::describes`].
        if !self.describes(text) {
            return None;
        }
        let line = self.lines.get(cell.row)?;
        let hint = hint_for(hints, line.logical_row);
        let source = text.line(line.logical_row)?;
        let mut column = hint.inset;
        let mut chars = line.chars.start;
        // No short circuit for a cell inside the inset. Returning the row's
        // first char here would skip the loop that walks past the row's leading
        // undrawn clusters, and a syntax layer that hides a marker *and* insets
        // the row for it — a blockquote drawing a bar in place of `> ` — would
        // land a click on the hidden marker rather than on the first drawn
        // character. The loop already answers this: undrawn clusters measure
        // zero, so a cell in the gutter falls into the first drawn cluster's
        // span and resolves to it.
        for cluster in source[line.bytes.clone()].graphemes(true) {
            let width = if visible(&hint, chars) {
                self.metrics.width_at(cluster, column - hint.inset)
            } else {
                0
            };
            if width > 0 && cell.column < column + width {
                return text.position(line.logical_row, Column::new(chars));
            }
            column += width;
            chars += cluster.chars().count();
        }
        text.position(line.logical_row, Column::new(line.chars.end))
    }
}

/// The visible part of a scrolled layout.
///
/// Kept apart from [`Layout`] on purpose: a layout is thrown away and rebuilt
/// when the pane is resized, and where the reader had scrolled to is not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Viewport {
    top: usize,
    height: usize,
}

impl Viewport {
    pub fn new(height: usize) -> Self {
        Self { top: 0, height }
    }

    pub fn top(&self) -> usize {
        self.top
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn set_height(&mut self, height: usize) {
        self.height = height;
    }

    /// The visual lines on screen.
    pub fn rows(&self, layout: &Layout) -> Range<usize> {
        let top = self.top.min(layout.visual_line_count().saturating_sub(1));
        top..(top + self.height).min(layout.visual_line_count())
    }

    /// Scroll the least amount that brings `cursor` on screen. Returns whether it
    /// moved.
    pub fn follow(&mut self, layout: &Layout, cursor: Position) -> bool {
        if self.height == 0 {
            return false;
        }
        let row = layout.visual_row_of(cursor);
        let was = self.top;
        if row < self.top {
            self.top = row;
        } else if row >= self.top + self.height {
            self.top = row + 1 - self.height;
        }
        self.top != was
    }

    /// Scroll by `delta` visual lines, without moving the cursor. Returns whether
    /// it moved.
    pub fn scroll_by(&mut self, layout: &Layout, delta: isize) -> bool {
        let was = self.top;
        let last = layout.visual_line_count().saturating_sub(1);
        self.top = if delta >= 0 {
            (self.top + delta.unsigned_abs()).min(last)
        } else {
            self.top.saturating_sub(delta.unsigned_abs())
        };
        self.top != was
    }
}

// -- wrapping ----------------------------------------------------------------

/// One cluster of a row, as wrapping sees it.
struct Cluster {
    chars: usize,
    bytes: usize,
    /// Byte length, so the cluster's text can be re-sliced to measure it.
    len: usize,
    /// A single whitespace scalar, and so a place a break may land. A cluster of
    /// several scalars is never whitespace.
    breakable: bool,
}

fn hint_for<'a>(hints: &'a [RowHints<'a>], row: usize) -> RowHints<'a> {
    hints.get(row).copied().unwrap_or_default()
}

fn visible(hint: &RowHints<'_>, chars: usize) -> bool {
    hint.visible.get(chars).copied().unwrap_or(true)
}

/// Wrap one logical row, appending at least one visual line.
fn wrap_row(
    text: &Text,
    row: usize,
    width: usize,
    metrics: Metrics,
    hint: RowHints<'_>,
    scratch: &mut Vec<Cluster>,
    out: &mut Vec<VisualLine>,
) {
    // The gutter eats into the width available for text. `.max(1)` keeps forward
    // progress when the gutter is as wide as the pane; a genuinely zero-width pane
    // is left at zero so it falls into the degenerate case below.
    let width = if hint.inset == 0 {
        width
    } else {
        width.saturating_sub(hint.inset).max(1)
    };

    let Some(source) = text.line(row) else {
        return;
    };

    scratch.clear();
    let mut chars = 0;
    for (bytes, cluster) in source.grapheme_indices(true) {
        let len = cluster.chars().count();
        scratch.push(Cluster {
            chars,
            bytes,
            len: cluster.len(),
            breakable: len == 1 && cluster.chars().next().is_some_and(char::is_whitespace),
        });
        chars += len;
    }
    let total_chars = chars;
    let total_bytes = source.len();

    if scratch.is_empty() || width == 0 {
        out.push(VisualLine {
            logical_row: row,
            chars: 0..0,
            bytes: 0..0,
            first: true,
        });
        return;
    }

    let cell_width = |index: usize, column: usize| -> usize {
        let cluster = &scratch[index];
        if visible(&hint, cluster.chars) {
            let at = cluster.bytes;
            metrics.width_at(&source[at..at + cluster.len], column)
        } else {
            0
        }
    };
    let char_at =
        |index: usize| -> usize { scratch.get(index).map(|c| c.chars).unwrap_or(total_chars) };
    let byte_at =
        |index: usize| -> usize { scratch.get(index).map(|c| c.bytes).unwrap_or(total_bytes) };

    let total = scratch.len();
    let mut start = 0;
    let mut first = true;

    while start < total {
        // Where the row stops fitting. The column resets per visual line, so a tab
        // on a continuation row measures from that row's own left edge — which is
        // what the renderer draws.
        let fit_end = {
            let mut column = 0;
            let mut index = start;
            while index < total {
                let cells = cell_width(index, column);
                if column + cells > width {
                    break;
                }
                column += cells;
                index += 1;
            }
            // A single cluster wider than the pane must still advance, or the loop
            // never ends.
            if index == start { start + 1 } else { index }
        };

        if fit_end >= total {
            out.push(VisualLine {
                logical_row: row,
                chars: char_at(start)..total_chars,
                bytes: byte_at(start)..total_bytes,
                first,
            });
            break;
        }

        // Prefer breaking at the last whitespace that fits; otherwise break mid
        // word, always on a cluster boundary.
        let (content_end, next_start) = if scratch[fit_end].breakable {
            (fit_end, fit_end + 1)
        } else {
            match scratch[start..fit_end]
                .iter()
                .enumerate()
                .rev()
                .find(|(_, cluster)| cluster.breakable)
            {
                Some((offset, _)) => (start + offset, start + offset + 1),
                None => (fit_end, fit_end),
            }
        };

        out.push(VisualLine {
            logical_row: row,
            chars: char_at(start)..char_at(content_end),
            bytes: byte_at(start)..byte_at(content_end),
            first,
        });
        start = next_start;
        first = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Text {
        Text::from(s)
    }

    fn plain(text: &Text, width: usize) -> Layout {
        Layout::compute(text, width, Metrics::default(), &[])
    }

    /// The drawn content of each visual line.
    fn drawn(text: &Text, layout: &Layout) -> Vec<String> {
        layout
            .visual_lines()
            .iter()
            .map(|line| {
                let row = text.line(line.logical_row).expect("row exists");
                row[line.bytes.clone()].to_string()
            })
            .collect()
    }

    fn at(text: &Text, row: usize, col: usize) -> Position {
        text.position(row, Column::new(col)).expect("addressable")
    }

    // -- wrapping -----------------------------------------------------------

    #[test]
    fn a_row_that_fits_is_one_visual_line() {
        let t = text("short");
        assert_eq!(drawn(&t, &plain(&t, 10)), ["short"]);
    }

    #[test]
    fn wrapping_prefers_a_space() {
        let t = text("aaaa bbbb");
        assert_eq!(drawn(&t, &plain(&t, 6)), ["aaaa", "bbbb"]);
    }

    #[test]
    fn a_word_longer_than_the_width_breaks_mid_word() {
        let t = text("aaaaaaaa");
        assert_eq!(drawn(&t, &plain(&t, 3)), ["aaa", "aaa", "aa"]);
    }

    #[test]
    fn an_empty_row_is_still_a_visual_line() {
        let t = text("a\n\nb");
        let layout = plain(&t, 10);
        assert_eq!(layout.visual_line_count(), 3);
        assert_eq!(drawn(&t, &layout), ["a", "", "b"]);
    }

    #[test]
    fn a_zero_width_pane_still_produces_one_line_per_row() {
        let t = text("a\nb");
        let layout = plain(&t, 0);
        assert_eq!(layout.visual_line_count(), 2);
    }

    #[test]
    fn a_cluster_wider_than_the_pane_still_advances() {
        // A width-2 glyph in a width-1 pane cannot fit, and must not loop.
        let t = text("\u{3042}\u{3042}");
        let layout = plain(&t, 1);
        assert_eq!(layout.visual_line_count(), 2);
    }

    #[test]
    fn a_cluster_is_never_split_across_visual_lines() {
        // A break landing inside a cluster would hand the renderer half a glyph,
        // and the halves would reclusterl differently from the whole — so every
        // column derived from either row would be wrong from that point on.
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let t = text(&format!("ab{family}cd"));
        for width in 1..10 {
            let layout = plain(&t, width);
            let row = t.line(0).expect("one row");
            for line in layout.visual_lines() {
                assert!(
                    row.is_char_boundary(line.bytes.start) && row.is_char_boundary(line.bytes.end),
                    "width {width}: {:?} splits a character",
                    line.bytes
                );
                let intact: Vec<usize> = row.grapheme_indices(true).map(|(at, _)| at).collect();
                assert!(
                    intact.contains(&line.bytes.start) || line.bytes.start == row.len(),
                    "width {width}: {:?} starts inside a cluster",
                    line.bytes
                );
                assert!(
                    intact.contains(&line.bytes.end) || line.bytes.end == row.len(),
                    "width {width}: {:?} ends inside a cluster",
                    line.bytes
                );
            }
        }
    }

    #[test]
    fn a_gutter_eats_into_the_width() {
        let t = text("aaaa bbbb");
        let hints = [RowHints {
            visible: &[],
            inset: 2,
        }];
        let layout = Layout::compute(&t, 9, Metrics::default(), &hints);
        assert_eq!(
            drawn(&t, &layout),
            ["aaaa", "bbbb"],
            "nine cells less a two-cell gutter does not fit nine characters"
        );
        assert_eq!(
            plain(&t, 9).visual_line_count(),
            1,
            "and without the gutter it does"
        );
    }

    #[test]
    fn a_gutter_as_wide_as_the_pane_still_makes_progress() {
        let t = text("aaaa");
        let hints = [RowHints {
            visible: &[],
            inset: 4,
        }];
        let layout = Layout::compute(&t, 4, Metrics::default(), &hints);
        assert_eq!(
            layout.visual_line_count(),
            4,
            "one cell per line, not a loop"
        );
    }

    #[test]
    fn undrawn_clusters_take_no_width() {
        // "## " concealed, as a heading's sigils are: the row draws as "heading"
        // and so fits a pane that its raw text would not.
        let t = text("## heading");
        let visible = vec![
            false, false, false, true, true, true, true, true, true, true,
        ];
        let hints = [RowHints {
            visible: &visible,
            inset: 0,
        }];
        let layout = Layout::compute(&t, 7, Metrics::default(), &hints);
        assert_eq!(layout.visual_line_count(), 1);
        assert_eq!(
            plain(&t, 7).visual_line_count(),
            2,
            "measuring the sigils would wrap it"
        );
    }

    #[test]
    fn a_tab_is_measured_to_its_stop_when_wrapping() {
        // Four cells of tab plus four of text is eight; a seven-cell pane wraps.
        let t = text("\tabcd");
        assert_eq!(plain(&t, 8).visual_line_count(), 1);
        assert_eq!(plain(&t, 7).visual_line_count(), 2);
    }

    #[test]
    fn a_tab_is_itself_a_place_to_break() {
        // A tab is whitespace, so when it does not fit it becomes the break rather
        // than being pushed to the next line.
        let t = text("ab cd\tef");
        assert_eq!(drawn(&t, &plain(&t, 5)), ["ab cd", "ef"]);
    }

    #[test]
    fn a_tab_measures_from_the_start_of_its_own_visual_line() {
        // On the continuation line the tab sits at column 2, so it advances 2 cells
        // to the next stop and "c" no longer fits. Measured from the logical row's
        // column 7 it would advance only 1, and "c" would fit — so this is the
        // assertion that pins which of the two models is in use.
        let t = text("aaaa bb\tc");
        assert_eq!(drawn(&t, &plain(&t, 4)), ["aaaa", "bb", "c"]);
    }

    // -- lookups ------------------------------------------------------------

    #[test]
    fn a_position_knows_which_visual_line_draws_it() {
        let t = text("aaaa bbbb cccc");
        let layout = plain(&t, 5);
        assert_eq!(drawn(&t, &layout), ["aaaa", "bbbb", "cccc"]);
        assert_eq!(layout.visual_row_of(at(&t, 0, 0)), 0);
        assert_eq!(layout.visual_row_of(at(&t, 0, 5)), 1);
        assert_eq!(layout.visual_row_of(at(&t, 0, 12)), 2);
    }

    #[test]
    fn visual_rows_are_found_across_logical_rows() {
        let t = text("aaaa bbbb\nsecond");
        let layout = plain(&t, 5);
        assert_eq!(drawn(&t, &layout), ["aaaa", "bbbb", "secon", "d"]);
        assert_eq!(layout.visual_row_of(at(&t, 1, 0)), 2);
        assert_eq!(layout.visual_row_of(at(&t, 1, 5)), 3);
    }

    #[test]
    fn a_cell_accounts_for_the_gutter() {
        let t = text("abc");
        let hints = [RowHints {
            visible: &[],
            inset: 2,
        }];
        let layout = Layout::compute(&t, 10, Metrics::default(), &hints);
        assert_eq!(
            layout.cell_of(&t, &hints, at(&t, 0, 1)),
            Cell { row: 0, column: 3 }
        );
    }

    #[test]
    fn a_cell_skips_hidden_clusters() {
        let t = text("## heading");
        let visible = vec![false, false, false];
        let hints = [RowHints {
            visible: &visible,
            inset: 0,
        }];
        let layout = Layout::compute(&t, 40, Metrics::default(), &hints);
        assert_eq!(
            layout.cell_of(&t, &hints, at(&t, 0, 3)),
            Cell { row: 0, column: 0 },
            "the first drawn character is in the first cell"
        );
    }

    #[test]
    fn a_cell_counts_a_wide_cluster_as_two() {
        let t = text("\u{3042}b");
        let layout = plain(&t, 40);
        assert_eq!(layout.cell_of(&t, &[], at(&t, 0, 1)).column, 2);
    }

    #[test]
    fn a_click_inside_a_wide_cluster_means_that_cluster() {
        let t = text("\u{3042}b");
        let layout = plain(&t, 40);
        for column in [0, 1] {
            let landed = layout
                .position_at_cell(&t, &[], Cell { row: 0, column })
                .expect("inside the line");
            assert_eq!(landed.column().get(), 0, "column {column}");
        }
        let landed = layout
            .position_at_cell(&t, &[], Cell { row: 0, column: 2 })
            .expect("inside the line");
        assert_eq!(landed.column().get(), 1);
    }

    #[test]
    fn a_click_past_the_end_of_a_visual_line_lands_at_its_end() {
        let t = text("aaaa bbbb");
        let layout = plain(&t, 5);
        let landed = layout
            .position_at_cell(&t, &[], Cell { row: 0, column: 99 })
            .expect("inside the line");
        assert_eq!(landed.column().get(), 4, "the end of the first visual line");
    }

    #[test]
    fn a_click_in_the_gutter_lands_at_the_start_of_the_text() {
        let t = text("abc");
        let hints = [RowHints {
            visible: &[],
            inset: 3,
        }];
        let layout = Layout::compute(&t, 10, Metrics::default(), &hints);
        let landed = layout
            .position_at_cell(&t, &hints, Cell { row: 0, column: 1 })
            .expect("inside the line");
        assert_eq!(landed.column().get(), 0);
    }

    #[test]
    fn a_click_below_the_text_finds_nothing() {
        let t = text("abc");
        let layout = plain(&t, 10);
        assert!(
            layout
                .position_at_cell(&t, &[], Cell { row: 9, column: 0 })
                .is_none()
        );
    }

    #[test]
    fn cells_and_positions_round_trip() {
        let t = text("aaaa bbbb cccc");
        let layout = plain(&t, 5);
        for column in 0..14 {
            let position = at(&t, 0, column);
            let cell = layout.cell_of(&t, &[], position);
            let back = layout
                .position_at_cell(&t, &[], cell)
                .expect("its own cell is inside the line");
            assert_eq!(back, position, "column {column}");
        }
    }

    // -- unwrapped ------------------------------------------------------------

    #[test]
    fn unwrapped_matches_row_count_and_describes_text() {
        let t = text("short\na longer row that would wrap at a narrow width\nlast");
        let layout = Layout::unwrapped(&t);
        assert_eq!(layout.row_count(), t.line_count());
        assert_eq!(layout.visual_line_count(), t.line_count());
        assert!(
            layout.describes(&t),
            "unwrapped must describe the text it was built from"
        );
        assert_eq!(
            drawn(&t, &layout),
            [
                "short",
                "a longer row that would wrap at a narrow width",
                "last"
            ],
            "one unwrapped visual line per row"
        );
    }

    #[test]
    fn unwrapped_handles_an_empty_text() {
        let t = text("");
        let layout = Layout::unwrapped(&t);
        assert_eq!(layout.row_count(), t.line_count());
        assert!(layout.describes(&t));
    }

    // -- relayout -----------------------------------------------------------

    #[test]
    fn relayout_rewraps_only_what_changed() {
        let mut buffer = crate::EditBuffer::new(text("aaaa bbbb\nkeep\ntail"));
        let mut layout = plain(buffer.text(), 5);
        assert_eq!(layout.visual_line_count(), 4);

        let end = buffer.text().position(0, Column::new(9)).unwrap();
        let mut txn = buffer.begin();
        txn.delete(buffer_span(&txn, 0, 4, 0, 9));
        let change = txn.commit().expect("changed");
        let _ = end;

        layout.relayout_rows(buffer.text(), &[], change.rows(), change.line_delta());
        assert_eq!(drawn(buffer.text(), &layout), ["aaaa", "keep", "tail"]);
        assert_eq!(layout.row_count(), buffer.text().line_count());
    }

    #[test]
    fn relayout_follows_added_rows() {
        let mut buffer = crate::EditBuffer::new(text("one\ntwo"));
        let mut layout = plain(buffer.text(), 10);
        let at_end = buffer.text().position(0, Column::new(3)).unwrap();
        let mut txn = buffer.begin();
        txn.insert(at_end, "\nmiddle");
        let change = txn.commit().expect("changed");

        layout.relayout_rows(buffer.text(), &[], change.rows(), change.line_delta());
        assert_eq!(drawn(buffer.text(), &layout), ["one", "middle", "two"]);
        assert_eq!(layout.row_count(), 3);
    }

    #[test]
    fn relayout_follows_removed_rows() {
        let mut buffer = crate::EditBuffer::new(text("one\ntwo\nthree\nfour"));
        let mut layout = plain(buffer.text(), 10);
        let mut txn = buffer.begin();
        txn.delete(buffer_span(&txn, 0, 3, 2, 5));
        let change = txn.commit().expect("changed");

        layout.relayout_rows(buffer.text(), &[], change.rows(), change.line_delta());
        assert_eq!(drawn(buffer.text(), &layout), ["one", "four"]);
        assert_eq!(layout.row_count(), 2);
    }

    #[test]
    fn relayout_matches_a_full_recompute() {
        // The cheap path and the honest path must agree, or an incremental
        // relayout is a way to be quietly wrong for the rest of the session.
        for (initial, row, col, inserted) in [
            ("aaaa bbbb\nkeep", 0, 4, " cccc"),
            ("one\ntwo\nthree", 1, 3, "\nsplit"),
            ("one\ntwo", 0, 0, "prefix "),
            ("wrapped line that is long\nnext", 0, 8, "\n"),
        ] {
            let mut buffer = crate::EditBuffer::new(text(initial));
            let mut layout = plain(buffer.text(), 6);
            let position = buffer.text().position(row, Column::new(col)).unwrap();
            let mut txn = buffer.begin();
            txn.insert(position, inserted);
            let change = txn.commit().expect("changed");

            layout.relayout_rows(buffer.text(), &[], change.rows(), change.line_delta());
            let fresh = plain(buffer.text(), 6);
            assert_eq!(
                layout.visual_lines(),
                fresh.visual_lines(),
                "relayout disagreed for {initial:?} + {inserted:?}"
            );
        }
    }

    fn buffer_span(
        txn: &crate::Txn<'_>,
        r1: usize,
        c1: usize,
        r2: usize,
        c2: usize,
    ) -> crate::Span {
        let text = txn.text();
        let a = text.position(r1, Column::new(c1)).expect("addressable");
        let b = text.position(r2, Column::new(c2)).expect("addressable");
        text.span(a, b).expect("same text")
    }

    // -- viewport -----------------------------------------------------------

    #[test]
    fn a_viewport_shows_its_height_of_lines() {
        let t = text("a\nb\nc\nd\ne");
        let layout = plain(&t, 10);
        let view = Viewport::new(3);
        assert_eq!(view.rows(&layout), 0..3);
    }

    #[test]
    fn a_viewport_clamps_to_what_there_is() {
        let t = text("a\nb");
        let layout = plain(&t, 10);
        let view = Viewport::new(10);
        assert_eq!(view.rows(&layout), 0..2);
    }

    #[test]
    fn following_the_cursor_scrolls_the_least_it_can() {
        let t = text("a\nb\nc\nd\ne");
        let layout = plain(&t, 10);
        let mut view = Viewport::new(3);
        assert!(view.follow(&layout, at(&t, 4, 0)));
        assert_eq!(view.top(), 2, "just enough to show the last row");
        assert!(!view.follow(&layout, at(&t, 3, 0)), "already on screen");
        assert!(view.follow(&layout, at(&t, 0, 0)));
        assert_eq!(view.top(), 0);
    }

    #[test]
    fn following_the_cursor_counts_visual_lines_not_rows() {
        let t = text("aaaa bbbb cccc\nlast");
        let layout = plain(&t, 5);
        assert_eq!(layout.visual_line_count(), 4);
        let mut view = Viewport::new(2);
        view.follow(&layout, at(&t, 0, 12));
        assert_eq!(view.top(), 1, "the third visual line of the first row");
    }

    #[test]
    fn scrolling_does_not_run_past_the_end() {
        let t = text("a\nb\nc");
        let layout = plain(&t, 10);
        let mut view = Viewport::new(2);
        view.scroll_by(&layout, 99);
        assert_eq!(view.top(), 2);
        view.scroll_by(&layout, -99);
        assert_eq!(view.top(), 0);
        assert!(!view.scroll_by(&layout, -1), "already at the top");
    }

    #[test]
    fn a_viewport_of_no_height_follows_nothing() {
        let t = text("a\nb");
        let layout = plain(&t, 10);
        let mut view = Viewport::new(0);
        assert!(!view.follow(&layout, at(&t, 1, 0)));
    }

    // -- properties ---------------------------------------------------------

    mod properties {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(200))]

            /// The incremental relayout agrees with a full recompute, for any edit
            /// at any place and any width.
            ///
            /// This is the property the incremental path lives or dies by. A
            /// relayout that is merely *close* is a way to be quietly wrong for the
            /// rest of a session, and the failure shows up as text drawn from the
            /// wrong row rather than as anything that looks like a layout bug.
            #[test]
            fn relayout_agrees_with_a_full_recompute(
                initial in "[a-z \n]{0,40}",
                inserted in "[a-z \n]{0,8}",
                byte in 0usize..48,
                width in 1usize..8,
            ) {
                let mut buffer = crate::EditBuffer::new(Text::from(initial.as_str()));
                let Some(position) = buffer.text().position_at_byte(byte.min(buffer.text().len_bytes()))
                else {
                    return Ok(());
                };
                let mut layout = Layout::compute(buffer.text(), width, Metrics::default(), &[]);

                let mut txn = buffer.begin();
                txn.insert(position, &inserted);
                let Some(change) = txn.commit() else {
                    return Ok(());
                };

                layout.relayout_rows(buffer.text(), &[], change.rows(), change.line_delta());
                let fresh = Layout::compute(buffer.text(), width, Metrics::default(), &[]);
                prop_assert_eq!(
                    layout.visual_lines(),
                    fresh.visual_lines(),
                    "{:?} + {:?} at byte {} width {}", initial, inserted, byte, width
                );
                prop_assert_eq!(layout.row_count(), fresh.row_count());
            }

            /// Same, for deletions.
            #[test]
            fn relayout_agrees_after_a_deletion(
                initial in "[a-z \n]{1,40}",
                from in 0usize..48,
                len in 0usize..12,
                width in 1usize..8,
            ) {
                let mut buffer = crate::EditBuffer::new(Text::from(initial.as_str()));
                let end = buffer.text().len_bytes();
                let Some(start) = buffer.text().position_at_byte(from.min(end)) else {
                    return Ok(());
                };
                let Some(stop) = buffer.text().position_at_byte((from + len).min(end)) else {
                    return Ok(());
                };
                let span = buffer.text().span(start, stop).expect("same text");
                let mut layout = Layout::compute(buffer.text(), width, Metrics::default(), &[]);

                let mut txn = buffer.begin();
                txn.delete(span);
                let Some(change) = txn.commit() else {
                    return Ok(());
                };

                layout.relayout_rows(buffer.text(), &[], change.rows(), change.line_delta());
                let fresh = Layout::compute(buffer.text(), width, Metrics::default(), &[]);
                prop_assert_eq!(
                    layout.visual_lines(),
                    fresh.visual_lines(),
                    "{:?} minus {}..{} at width {}", initial, from, from + len, width
                );
            }

            /// Every visual line covers a real slice of the row it names, and the
            /// lines of one row cover the row in order without gaps.
            #[test]
            fn visual_lines_tile_their_rows(
                initial in ".{0,40}",
                width in 1usize..8,
            ) {
                let t = Text::from(initial.as_str());
                let layout = Layout::compute(&t, width, Metrics::default(), &[]);
                prop_assert_eq!(layout.row_count(), t.line_count());
                let mut seen_rows = 0;
                let mut expected_row = 0;
                let mut cursor = 0;
                for line in layout.visual_lines() {
                    if line.first {
                        prop_assert_eq!(line.logical_row, expected_row, "rows must be in order");
                        expected_row += 1;
                        seen_rows += 1;
                        cursor = 0;
                    }
                    let row = t.line(line.logical_row).expect("a named row exists");
                    prop_assert!(line.bytes.end <= row.len(), "slice past the row");
                    prop_assert!(line.chars.start >= cursor, "a line went backwards");
                    prop_assert!(row.is_char_boundary(line.bytes.start));
                    prop_assert!(row.is_char_boundary(line.bytes.end));
                    // Both ends land between grapheme clusters, so a visual line's
                    // slice reclusters exactly as the whole row does.
                    let breaks: Vec<usize> = row
                        .grapheme_indices(true)
                        .map(|(at, _)| at)
                        .chain(std::iter::once(row.len()))
                        .collect();
                    prop_assert!(breaks.contains(&line.bytes.start), "start splits a cluster");
                    prop_assert!(breaks.contains(&line.bytes.end), "end splits a cluster");
                    cursor = line.chars.end;
                }
                prop_assert_eq!(seen_rows, t.line_count(), "every row is drawn");
            }
        }
    }
}
