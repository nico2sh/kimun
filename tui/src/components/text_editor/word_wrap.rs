#[derive(Debug, Clone, PartialEq)]
pub struct VisualLine {
    pub logical_row: usize,
    /// Character offset (Unicode scalar) where this visual line begins in the original line.
    pub start_col: usize,
    /// Character offset (exclusive) where this visual line ends.
    pub end_col: usize,
    pub content: String,
    pub is_first_visual_line: bool,
}

pub struct WordWrapLayout {
    visual_lines: Vec<VisualLine>,
}

impl WordWrapLayout {
    pub fn compute(lines: &[String], width: u16) -> Self {
        let width = width as usize;
        let mut visual_lines = Vec::new();

        if lines.is_empty() {
            return Self::default();
        }

        for (row, line) in lines.iter().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            if chars.is_empty() || width == 0 {
                visual_lines.push(VisualLine {
                    logical_row: row,
                    start_col: 0,
                    end_col: 0,
                    content: String::new(),
                    is_first_visual_line: true,
                });
                continue;
            }

            let total = chars.len();
            let mut start = 0;
            let mut is_first = true;

            while start < total {
                let remaining = total - start;
                if remaining <= width {
                    visual_lines.push(VisualLine {
                        logical_row: row,
                        start_col: start,
                        end_col: total,
                        content: chars[start..total].iter().collect(),
                        is_first_visual_line: is_first,
                    });
                    break;
                }
                // Find break point: if char AT end is whitespace, break there;
                // otherwise scan backward for last whitespace in [start..end].
                let end = start + width;
                let (content_end, next_start) = if chars[end].is_whitespace() {
                    (end, end + 1) // break before space, skip it
                } else {
                    match chars[start..end]
                        .iter()
                        .enumerate()
                        .rev()
                        .find(|(_, c)| c.is_whitespace())
                    {
                        Some((i, _)) => (start + i, start + i + 1), // break before space, skip it
                        None => (end, end), // hard break, no space found
                    }
                };

                visual_lines.push(VisualLine {
                    logical_row: row,
                    start_col: start,
                    end_col: content_end,
                    content: chars[start..content_end].iter().collect(),
                    is_first_visual_line: is_first,
                });
                start = next_start;
                is_first = false;
            }
        }

        Self { visual_lines }
    }

    pub fn total_visual_lines(&self) -> usize {
        self.visual_lines.len()
    }

    pub fn visual_lines(&self) -> &[VisualLine] {
        &self.visual_lines
    }

    /// Convert logical (row, col) to (visual_row, visual_col).
    pub fn logical_to_visual(&self, row: usize, col: usize) -> (usize, usize) {
        // Clamp row to the last logical row present in the layout.
        let last_logical = self.visual_lines.last().map(|vl| vl.logical_row).unwrap_or(0);
        let row = row.min(last_logical);
        // Find the last visual line for `row` whose start_col <= col.
        let vrow = self.visual_lines
            .iter()
            .enumerate()
            .filter(|(_, vl)| vl.logical_row == row && vl.start_col <= col)
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
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
                content: String::new(),
                is_first_visual_line: true,
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ls(s: &str) -> Vec<String> {
        s.lines().map(str::to_owned).collect()
    }

    #[test]
    fn empty_input_produces_one_visual_line() {
        let layout = WordWrapLayout::compute(&[], 40);
        assert_eq!(layout.total_visual_lines(), 1);
        assert_eq!(layout.visual_lines()[0].logical_row, 0);
        assert!(layout.visual_lines()[0].is_first_visual_line);
    }

    #[test]
    fn empty_string_produces_one_visual_line() {
        let layout = WordWrapLayout::compute(&[String::new()], 40);
        assert_eq!(layout.total_visual_lines(), 1);
        assert_eq!(layout.visual_lines()[0].content, "");
        assert!(layout.visual_lines()[0].is_first_visual_line);
    }

    #[test]
    fn short_line_fits_on_one_visual_line() {
        let layout = WordWrapLayout::compute(&ls("hello world"), 40);
        assert_eq!(layout.total_visual_lines(), 1);
        assert_eq!(layout.visual_lines()[0].content, "hello world");
        assert!(layout.visual_lines()[0].is_first_visual_line);
    }

    #[test]
    fn long_line_wraps_at_whitespace() {
        // "hello world foo" width=11 → "hello world" (11) fits; " foo" wraps
        let layout = WordWrapLayout::compute(&ls("hello world foo"), 11);
        assert_eq!(layout.total_visual_lines(), 2);
        assert_eq!(layout.visual_lines()[0].content, "hello world");
        assert_eq!(layout.visual_lines()[1].content, "foo");
        assert!(layout.visual_lines()[0].is_first_visual_line);
        assert!(!layout.visual_lines()[1].is_first_visual_line);
    }

    #[test]
    fn long_word_hard_breaks_at_width() {
        let layout = WordWrapLayout::compute(&["abcdefgh".to_string()], 4);
        assert_eq!(layout.total_visual_lines(), 2);
        assert_eq!(layout.visual_lines()[0].content, "abcd");
        assert_eq!(layout.visual_lines()[1].content, "efgh");
    }

    #[test]
    fn two_logical_lines_have_correct_logical_rows() {
        let layout = WordWrapLayout::compute(&ls("abc\nxyz"), 10);
        assert_eq!(layout.total_visual_lines(), 2);
        assert_eq!(layout.visual_lines()[0].logical_row, 0);
        assert_eq!(layout.visual_lines()[1].logical_row, 1);
    }

    #[test]
    fn unicode_chars_counted_not_bytes() {
        // "あいう" is 3 chars, 9 bytes. width=2 → hard break at 2 chars.
        let layout = WordWrapLayout::compute(&["あいう".to_string()], 2);
        assert_eq!(layout.total_visual_lines(), 2);
        assert_eq!(layout.visual_lines()[0].content, "あい");
        assert_eq!(layout.visual_lines()[1].content, "う");
    }

    #[test]
    fn logical_to_visual_start_of_line() {
        let layout = WordWrapLayout::compute(&ls("hello world"), 40);
        assert_eq!(layout.logical_to_visual(0, 0), (0, 0));
    }

    #[test]
    fn logical_to_visual_wrapped_cursor() {
        // "hello world foo" width=11 → vline0 ends at col 11, vline1 starts at col 12
        let layout = WordWrapLayout::compute(&ls("hello world foo"), 11);
        let (vrow, vcol) = layout.logical_to_visual(0, 12);
        assert_eq!(vrow, 1);
        assert_eq!(vcol, 0); // "foo" starts at col 12 in logical line
    }

    #[test]
    fn visual_to_logical_first_line() {
        let layout = WordWrapLayout::compute(&ls("hello"), 40);
        assert_eq!(layout.visual_to_logical(0, 3), (0, 3));
    }

    #[test]
    fn visual_to_logical_accounts_for_start_col() {
        // vline1.start_col = 12 (after "hello world ")
        let layout = WordWrapLayout::compute(&ls("hello world foo"), 11);
        let (row, col) = layout.visual_to_logical(1, 0);
        assert_eq!(row, 0);
        assert_eq!(col, 12);
    }

    #[test]
    fn coordinate_roundtrip_vrow_zero() {
        let layout = WordWrapLayout::compute(&ls("hello world foo"), 11);
        let (row, col) = layout.visual_to_logical(0, 3);
        let (vrow2, vcol2) = layout.logical_to_visual(row, col);
        assert_eq!((vrow2, vcol2), (0, 3));
    }
}
