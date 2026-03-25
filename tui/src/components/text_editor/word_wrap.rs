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
            // Build a single (byte_offset, char) index for the line — one allocation per
            // logical line that provides both char-indexed access and byte offsets for
            // slicing, eliminating the separate `Vec<char>` and per-VisualLine String copies.
            let ci: Vec<(usize, char)> = line.char_indices().collect();
            if ci.is_empty() || width == 0 {
                row_starts.push(visual_lines.len());
                visual_lines.push(VisualLine {
                    logical_row: row,
                    start_col: 0,
                    end_col: 0,
                    start_byte: 0,
                    end_byte: 0,
                    is_first_visual_line: true,
                });
                continue;
            }

            let flags: &[bool] = rendered.get(row).map(|v| v.as_slice()).unwrap_or(&[]);
            let is_rendered = |pos: usize| -> bool {
                if pos < flags.len() { flags[pos] } else { true }
            };

            // Helper: byte offset of char at position `pos` (or line.len() if pos == total).
            let byte_at = |pos: usize| -> usize {
                if pos < ci.len() { ci[pos].0 } else { line.len() }
            };

            let total = ci.len();
            let mut start = 0;
            let mut is_first = true;

            while start < total {
                if is_first {
                    row_starts.push(visual_lines.len());
                }

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
                    visual_lines.push(VisualLine {
                        logical_row: row,
                        start_col: start,
                        end_col: total,
                        start_byte: byte_at(start),
                        end_byte: line.len(),
                        is_first_visual_line: is_first,
                    });
                    break;
                }

                // Find break point: prefer last whitespace in [start..fit_end].
                let (content_end, next_start) =
                    if fit_end < total && ci[fit_end].1.is_whitespace() {
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

                visual_lines.push(VisualLine {
                    logical_row: row,
                    start_col: start,
                    end_col: content_end,
                    start_byte: byte_at(start),
                    end_byte: byte_at(content_end),
                    is_first_visual_line: is_first,
                });
                start = next_start;
                is_first = false;
            }
        }

        Self { visual_lines, row_starts }
    }

    pub fn total_visual_lines(&self) -> usize {
        self.visual_lines.len()
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
        let layout = WordWrapLayout::compute(&[src.clone()], 40, &[]);
        assert_eq!(layout.total_visual_lines(), 1);
        assert_eq!(content_of(&layout.visual_lines()[0], &src), "");
        assert!(layout.visual_lines()[0].is_first_visual_line);
    }

    #[test]
    fn short_line_fits_on_one_visual_line() {
        let lines = ls("hello world");
        let layout = WordWrapLayout::compute(&lines, 40, &[]);
        assert_eq!(layout.total_visual_lines(), 1);
        assert_eq!(content_of(&layout.visual_lines()[0], &lines[0]), "hello world");
        assert!(layout.visual_lines()[0].is_first_visual_line);
    }

    #[test]
    fn long_line_wraps_at_whitespace() {
        // "hello world foo" width=11 → "hello world" (11) fits; " foo" wraps
        let lines = ls("hello world foo");
        let layout = WordWrapLayout::compute(&lines, 11, &[]);
        assert_eq!(layout.total_visual_lines(), 2);
        assert_eq!(content_of(&layout.visual_lines()[0], &lines[0]), "hello world");
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
}
