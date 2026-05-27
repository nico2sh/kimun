#[derive(Debug, Clone, PartialEq)]
pub struct VisualLine {
    pub logical_row: usize,
    /// Character offset (Unicode scalar) where this visual line begins in the original line.
    pub start_col: usize,
    /// Character offset (exclusive) where this visual line ends.
    pub end_col: usize,
    /// Byte offset in the original logical line where this visual line begins.
    pub start_byte: usize,
    /// Byte offset (exclusive) in the original logical line where this visual line ends.
    pub end_byte: usize,
    pub is_first_visual_line: bool,
}

impl VisualLine {
    /// Borrow the content slice from the original logical line string.
    /// This avoids storing a redundant `String` copy on each `VisualLine`.
    pub fn content<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start_byte..self.end_byte]
    }
}

/// Wrap a single logical row at the given width. Returns the visual
/// lines produced for this row (one or more entries; at least one).
///
/// `rendered_row` is the per-char rendered-mask for this row (empty
/// slice if absent — falls back to raw char widths).
fn wrap_one_row(
    logical_row: usize,
    line: &str,
    width: usize,
    rendered_row: &[bool],
) -> Vec<VisualLine> {
    let ci: Vec<(usize, char)> = line.char_indices().collect();
    if ci.is_empty() || width == 0 {
        return vec![VisualLine {
            logical_row,
            start_col: 0,
            end_col: 0,
            start_byte: 0,
            end_byte: 0,
            is_first_visual_line: true,
        }];
    }

    let is_rendered = |pos: usize| -> bool {
        if pos < rendered_row.len() { rendered_row[pos] } else { true }
    };
    let byte_at = |pos: usize| -> usize {
        if pos < ci.len() { ci[pos].0 } else { line.len() }
    };

    let total = ci.len();
    let mut out = Vec::new();
    let mut start = 0;
    let mut is_first = true;

    while start < total {
        // Find the first position where rendered count from `start` exceeds `width`.
        let fit_end = {
            let mut rcount = 0usize;
            let mut pos = start;
            while pos < total {
                let r = is_rendered(pos) as usize;
                if rcount + r > width {
                    break;
                }
                rcount += r;
                pos += 1;
            }
            pos
        };

        if fit_end == total {
            out.push(VisualLine {
                logical_row,
                start_col: start,
                end_col: total,
                start_byte: byte_at(start),
                end_byte: line.len(),
                is_first_visual_line: is_first,
            });
            break;
        }

        // Find break point: prefer last whitespace in [start..fit_end].
        let (content_end, next_start) = if fit_end < total && ci[fit_end].1.is_whitespace() {
            (fit_end, fit_end + 1)
        } else {
            match ci[start..fit_end]
                .iter()
                .enumerate()
                .rev()
                .find(|(_, (_, c))| c.is_whitespace())
            {
                Some((i, _)) => (start + i, start + i + 1),
                None => (fit_end, fit_end), // hard break
            }
        };

        out.push(VisualLine {
            logical_row,
            start_col: start,
            end_col: content_end,
            start_byte: byte_at(start),
            end_byte: byte_at(content_end),
            is_first_visual_line: is_first,
        });
        start = next_start;
        is_first = false;
    }

    out
}

#[derive(Clone)]
pub struct WordWrapLayout {
    visual_lines: Vec<VisualLine>,
    /// Maps logical row index → index of its first `VisualLine` in `visual_lines`.
    /// Enables O(wrap-count) lookup in `logical_to_visual` instead of O(total visual lines).
    row_starts: Vec<usize>,
}

impl WordWrapLayout {
    /// Compute word-wrap layout.
    /// `rendered`: per-line bitmask of which char positions are actually rendered (visible).
    /// Pass `&[]` to use raw char widths (e.g. in tests that don't involve markdown).
    pub fn compute(lines: &[String], width: u16, rendered: &[Vec<bool>]) -> Self {
        let width = width as usize;
        let mut visual_lines = Vec::new();
        let mut row_starts = Vec::with_capacity(lines.len());

        if lines.is_empty() {
            return Self::default();
        }

        for (row, line) in lines.iter().enumerate() {
            row_starts.push(visual_lines.len());
            let rendered_row = rendered.get(row).map(|v| v.as_slice()).unwrap_or(&[]);
            visual_lines.extend(wrap_one_row(row, line, width, rendered_row));
        }

        Self {
            visual_lines,
            row_starts,
        }
    }

    /// Re-wrap only the rows in `row_range`, splicing the result into
    /// `visual_lines` and updating `row_starts` accordingly.
    ///
    /// **Contract:** caller must pass the SAME `lines` and `width` as the
    /// most recent `compute` call (or previous `splice_range`); only rows
    /// in `row_range` are assumed to have changed. Other rows' content,
    /// width, and rendered masks must be byte-identical.
    ///
    /// `row_range` is half-open in logical-row space. Empty ranges are
    /// a no-op.
    pub fn splice_range(
        &mut self,
        lines: &[String],
        width: u16,
        rendered: &[Vec<bool>],
        row_range: std::ops::Range<usize>,
    ) {
        if row_range.is_empty() {
            return;
        }
        let width = width as usize;
        debug_assert!(
            row_range.end <= lines.len(),
            "splice_range: row_range.end {} > lines.len() {}",
            row_range.end,
            lines.len(),
        );
        debug_assert!(
            row_range.start <= self.row_starts.len(),
            "splice_range: row_range.start {} > row_starts.len() {}",
            row_range.start,
            self.row_starts.len(),
        );

        // Compute the old visual-line index span for this row range.
        let old_vstart = self.row_starts[row_range.start];
        let old_vend = if row_range.end < self.row_starts.len() {
            self.row_starts[row_range.end]
        } else {
            self.visual_lines.len()
        };

        // Wrap the new contents of the affected rows. Also record per-row
        // starting indices inside the new slice so we can rebuild
        // row_starts[row_range] without searching.
        let mut new_slice: Vec<VisualLine> = Vec::new();
        let mut new_row_starts_for_range: Vec<usize> = Vec::with_capacity(row_range.len());
        for row in row_range.clone() {
            new_row_starts_for_range.push(new_slice.len());
            let rendered_row = rendered.get(row).map(|v| v.as_slice()).unwrap_or(&[]);
            new_slice.extend(wrap_one_row(row, &lines[row], width, rendered_row));
        }

        // Splice visual_lines.
        let new_vcount = new_slice.len();
        self.visual_lines.splice(old_vstart..old_vend, new_slice);

        // Shift row_starts for the range and for rows after the range.
        let old_vcount = old_vend - old_vstart;
        let delta_i = new_vcount as isize - old_vcount as isize;

        // Update row_starts within the spliced range (absolute indices).
        for (i, local_start) in new_row_starts_for_range.into_iter().enumerate() {
            self.row_starts[row_range.start + i] = old_vstart + local_start;
        }

        // Shift row_starts for rows AFTER the spliced range.
        if delta_i != 0 {
            for rs in &mut self.row_starts[row_range.end..] {
                *rs = ((*rs as isize) + delta_i) as usize;
            }
        }
    }

    pub fn total_visual_lines(&self) -> usize {
        self.visual_lines.len()
    }

    /// Returns the number of logical rows tracked by this layout.
    /// Used by `view.update` to detect line-count changes without exposing
    /// `row_starts` directly.
    pub fn row_starts_len(&self) -> usize {
        self.row_starts.len()
    }

    pub fn visual_lines(&self) -> &[VisualLine] {
        &self.visual_lines
    }

    /// Convert logical (row, col) to (visual_row, visual_col).
    pub fn logical_to_visual(&self, row: usize, col: usize) -> (usize, usize) {
        let row = row.min(self.row_starts.len().saturating_sub(1));
        let first = self.row_starts.get(row).copied().unwrap_or(0);
        let vrow = self.visual_lines[first..]
            .iter()
            .enumerate()
            .take_while(|(_, vl)| vl.logical_row == row)
            .filter(|(_, vl)| vl.start_col <= col)
            .last()
            .map(|(i, _)| first + i)
            .unwrap_or(first);
        let vl = &self.visual_lines[vrow];
        (vrow, col.saturating_sub(vl.start_col))
    }

    /// Convert visual (vrow, vcol) to logical (row, col).
    pub fn visual_to_logical(&self, vrow: usize, vcol: usize) -> (usize, usize) {
        let vrow = vrow.min(self.visual_lines.len().saturating_sub(1));
        let vl = &self.visual_lines[vrow];
        let col = (vl.start_col + vcol).min(vl.end_col);
        (vl.logical_row, col)
    }
}

impl Default for WordWrapLayout {
    fn default() -> Self {
        Self {
            visual_lines: vec![VisualLine {
                logical_row: 0,
                start_col: 0,
                end_col: 0,
                start_byte: 0,
                end_byte: 0,
                is_first_visual_line: true,
            }],
            row_starts: vec![0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ls(s: &str) -> Vec<String> {
        s.lines().map(str::to_owned).collect()
    }

    // Helper: get content string for a visual line from its source line.
    fn content_of<'a>(vl: &VisualLine, source: &'a str) -> &'a str {
        vl.content(source)
    }

    #[test]
    fn empty_input_produces_one_visual_line() {
        let layout = WordWrapLayout::compute(&[], 40, &[]);
        assert_eq!(layout.total_visual_lines(), 1);
        assert_eq!(layout.visual_lines()[0].logical_row, 0);
        assert!(layout.visual_lines()[0].is_first_visual_line);
    }

    #[test]
    fn empty_string_produces_one_visual_line() {
        let src = String::new();
        let layout = WordWrapLayout::compute(std::slice::from_ref(&src), 40, &[]);
        assert_eq!(layout.total_visual_lines(), 1);
        assert_eq!(content_of(&layout.visual_lines()[0], &src), "");
        assert!(layout.visual_lines()[0].is_first_visual_line);
    }

    #[test]
    fn short_line_fits_on_one_visual_line() {
        let lines = ls("hello world");
        let layout = WordWrapLayout::compute(&lines, 40, &[]);
        assert_eq!(layout.total_visual_lines(), 1);
        assert_eq!(
            content_of(&layout.visual_lines()[0], &lines[0]),
            "hello world"
        );
        assert!(layout.visual_lines()[0].is_first_visual_line);
    }

    #[test]
    fn long_line_wraps_at_whitespace() {
        // "hello world foo" width=11 → "hello world" (11) fits; " foo" wraps
        let lines = ls("hello world foo");
        let layout = WordWrapLayout::compute(&lines, 11, &[]);
        assert_eq!(layout.total_visual_lines(), 2);
        assert_eq!(
            content_of(&layout.visual_lines()[0], &lines[0]),
            "hello world"
        );
        assert_eq!(content_of(&layout.visual_lines()[1], &lines[0]), "foo");
        assert!(layout.visual_lines()[0].is_first_visual_line);
        assert!(!layout.visual_lines()[1].is_first_visual_line);
    }

    #[test]
    fn long_word_hard_breaks_at_width() {
        let lines = vec!["abcdefgh".to_string()];
        let layout = WordWrapLayout::compute(&lines, 4, &[]);
        assert_eq!(layout.total_visual_lines(), 2);
        assert_eq!(content_of(&layout.visual_lines()[0], &lines[0]), "abcd");
        assert_eq!(content_of(&layout.visual_lines()[1], &lines[0]), "efgh");
    }

    #[test]
    fn two_logical_lines_have_correct_logical_rows() {
        let layout = WordWrapLayout::compute(&ls("abc\nxyz"), 10, &[]);
        assert_eq!(layout.total_visual_lines(), 2);
        assert_eq!(layout.visual_lines()[0].logical_row, 0);
        assert_eq!(layout.visual_lines()[1].logical_row, 1);
    }

    #[test]
    fn unicode_chars_counted_not_bytes() {
        // "あいう" is 3 chars, 9 bytes. width=2 → hard break at 2 chars.
        let lines = vec!["あいう".to_string()];
        let layout = WordWrapLayout::compute(&lines, 2, &[]);
        assert_eq!(layout.total_visual_lines(), 2);
        assert_eq!(content_of(&layout.visual_lines()[0], &lines[0]), "あい");
        assert_eq!(content_of(&layout.visual_lines()[1], &lines[0]), "う");
    }

    #[test]
    fn logical_to_visual_start_of_line() {
        let layout = WordWrapLayout::compute(&ls("hello world"), 40, &[]);
        assert_eq!(layout.logical_to_visual(0, 0), (0, 0));
    }

    #[test]
    fn logical_to_visual_wrapped_cursor() {
        let layout = WordWrapLayout::compute(&ls("hello world foo"), 11, &[]);
        let (vrow, vcol) = layout.logical_to_visual(0, 12);
        assert_eq!(vrow, 1);
        assert_eq!(vcol, 0);
    }

    #[test]
    fn visual_to_logical_first_line() {
        let layout = WordWrapLayout::compute(&ls("hello"), 40, &[]);
        assert_eq!(layout.visual_to_logical(0, 3), (0, 3));
    }

    #[test]
    fn visual_to_logical_accounts_for_start_col() {
        let layout = WordWrapLayout::compute(&ls("hello world foo"), 11, &[]);
        let (row, col) = layout.visual_to_logical(1, 0);
        assert_eq!(row, 0);
        assert_eq!(col, 12);
    }

    #[test]
    fn row_starts_index_multi_line_multi_wrap() {
        let lines = vec![
            "abc".to_string(),
            "hello world foo".to_string(),
            "xyz".to_string(),
        ];
        let layout = WordWrapLayout::compute(&lines, 11, &[]);
        assert_eq!(layout.row_starts, vec![0, 1, 3]);
        assert_eq!(layout.logical_to_visual(2, 0), (3, 0));
    }

    #[test]
    fn coordinate_roundtrip_vrow_zero() {
        let layout = WordWrapLayout::compute(&ls("hello world foo"), 11, &[]);
        let (row, col) = layout.visual_to_logical(0, 3);
        let (vrow2, vcol2) = layout.logical_to_visual(row, col);
        assert_eq!((vrow2, vcol2), (0, 3));
    }

    #[test]
    fn byte_offsets_correct_for_unicode() {
        // "あいう": あ=3 bytes, い=3 bytes, う=3 bytes
        // char 0 → byte 0, char 1 → byte 3, char 2 → byte 6
        let lines = vec!["あいう".to_string()];
        let layout = WordWrapLayout::compute(&lines, 2, &[]);
        let vl0 = &layout.visual_lines()[0];
        let vl1 = &layout.visual_lines()[1];
        assert_eq!((vl0.start_byte, vl0.end_byte), (0, 6)); // "あい"
        assert_eq!((vl1.start_byte, vl1.end_byte), (6, 9)); // "う"
    }

    #[test]
    fn splice_range_full_buffer_equals_compute() {
        let lines = ls("hello world\nfoo bar baz\nlast line");
        let mut layout = WordWrapLayout::compute(&lines, 40, &[]);
        layout.splice_range(&lines, 40, &[], 0..lines.len());
        let fresh = WordWrapLayout::compute(&lines, 40, &[]);
        assert_eq!(layout.visual_lines(), fresh.visual_lines());
        assert_eq!(layout.row_starts, fresh.row_starts);
    }

    #[test]
    fn splice_range_middle_row_only() {
        // Edit row 1 — splice should only re-wrap row 1.
        let lines_before = ls("alpha beta\nfoo bar\ngamma delta");
        let layout_before = WordWrapLayout::compute(&lines_before, 40, &[]);

        let lines_after = ls("alpha beta\nFOO BAR\ngamma delta");
        let mut layout = layout_before.clone();
        layout.splice_range(&lines_after, 40, &[], 1..2);

        let fresh = WordWrapLayout::compute(&lines_after, 40, &[]);
        assert_eq!(layout.visual_lines(), fresh.visual_lines());
        assert_eq!(layout.row_starts, fresh.row_starts);
    }

    #[test]
    fn splice_range_handles_wrap_count_change() {
        // Row 0: "short" (1 visual line) → "a very long line that will wrap" (2 visual lines at width 10).
        let lines_before = ls("short\ntail");
        let mut layout = WordWrapLayout::compute(&lines_before, 10, &[]);
        let lines_after = ls("a very long line that will wrap\ntail");
        layout.splice_range(&lines_after, 10, &[], 0..1);

        let fresh = WordWrapLayout::compute(&lines_after, 10, &[]);
        assert_eq!(layout.visual_lines(), fresh.visual_lines());
        assert_eq!(layout.row_starts, fresh.row_starts);
    }

    #[test]
    fn splice_range_at_buffer_start() {
        let lines = ls("first line\nsecond line\nthird line");
        let mut layout = WordWrapLayout::compute(&lines, 40, &[]);
        let edited = ls("first EDITED line\nsecond line\nthird line");
        layout.splice_range(&edited, 40, &[], 0..1);

        let fresh = WordWrapLayout::compute(&edited, 40, &[]);
        assert_eq!(layout.visual_lines(), fresh.visual_lines());
        assert_eq!(layout.row_starts, fresh.row_starts);
    }

    #[test]
    fn splice_range_at_buffer_end() {
        let lines = ls("first\nsecond\nthird");
        let mut layout = WordWrapLayout::compute(&lines, 40, &[]);
        let edited = ls("first\nsecond\nthird EDITED");
        layout.splice_range(&edited, 40, &[], 2..3);

        let fresh = WordWrapLayout::compute(&edited, 40, &[]);
        assert_eq!(layout.visual_lines(), fresh.visual_lines());
        assert_eq!(layout.row_starts, fresh.row_starts);
    }
}
