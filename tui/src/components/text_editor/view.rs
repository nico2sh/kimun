use std::ops::Range;
use ratatui::Frame;
use ratatui::layout::Rect;
use crate::settings::themes::Theme;
use super::word_wrap::WordWrapLayout;

pub struct MarkdownEditorView {
    pub layout: WordWrapLayout,
    pub visual_scroll_offset: usize,
    pub lines_snapshot: Vec<String>,
    pub cursor_snapshot: (usize, usize),
    pub cursor_code_block: Option<Range<usize>>,
}

impl MarkdownEditorView {
    pub fn new() -> Self {
        Self {
            layout: WordWrapLayout::default(),
            visual_scroll_offset: 0,
            lines_snapshot: Vec::new(),
            cursor_snapshot: (0, 0),
            cursor_code_block: None,
        }
    }

    pub fn update(&mut self, lines: &[String], cursor: (usize, usize), rect: Rect) {
        if rect.height == 0 { return; }
        self.lines_snapshot = lines.to_vec();
        self.cursor_snapshot = cursor;
        self.cursor_code_block = Self::find_code_block(lines, cursor.0);
        self.layout = WordWrapLayout::compute(lines, rect.width);

        let cursor_vrow = self.layout.logical_to_visual(cursor.0, cursor.1).0;
        let height = rect.height as usize;
        if cursor_vrow < self.visual_scroll_offset {
            self.visual_scroll_offset = cursor_vrow;
        } else if cursor_vrow >= self.visual_scroll_offset + height {
            self.visual_scroll_offset = cursor_vrow - height + 1;
        }
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        todo!()
    }

    /// Convert mouse visual position (relative to rect, scroll-adjusted) to
    /// logical cursor position. Returns (u16, u16) for CursorMove::Jump.
    pub fn visual_to_logical_u16(&self, vrow: usize, vcol: usize) -> (u16, u16) {
        let (row, col) = self.layout.visual_to_logical(vrow, vcol);
        (row.min(u16::MAX as usize) as u16, col.min(u16::MAX as usize) as u16)
    }

    fn find_code_block(lines: &[String], cursor_row: usize) -> Option<Range<usize>> {
        let mut open: Option<usize> = None;
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim();
            if t.starts_with("```") {
                match open {
                    None => open = Some(i),
                    Some(start) => {
                        let range = start..i + 1;
                        if range.contains(&cursor_row) {
                            return Some(range);
                        }
                        open = None;
                    }
                }
            }
        }
        None
    }
}

impl Default for MarkdownEditorView {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    fn rect(h: u16) -> Rect { Rect { x: 0, y: 0, width: 40, height: h } }

    #[test]
    fn new_has_zero_scroll() {
        assert_eq!(MarkdownEditorView::new().visual_scroll_offset, 0);
    }

    #[test]
    fn zero_height_rect_does_not_panic() {
        let mut v = MarkdownEditorView::new();
        v.update(&["hello".to_string()], (0, 0), rect(0));
        // Should return early without panic
    }

    #[test]
    fn scroll_follows_cursor_down() {
        let mut v = MarkdownEditorView::new();
        // 5 single-word lines, each fits on one visual line, height=3
        let lines: Vec<String> = (0..5).map(|i| format!("line{}", i)).collect();
        v.update(&lines, (4, 0), rect(3)); // cursor on row 4
        // cursor_vrow = 4, scroll must be at least 4 - 3 + 1 = 2
        assert!(v.visual_scroll_offset >= 2);
    }

    #[test]
    fn scroll_follows_cursor_up() {
        let mut v = MarkdownEditorView::new();
        let lines: Vec<String> = (0..5).map(|i| format!("line{}", i)).collect();
        // First move cursor to bottom to push scroll down
        v.update(&lines, (4, 0), rect(3));
        // Now move cursor back to top
        v.update(&lines, (0, 0), rect(3));
        assert_eq!(v.visual_scroll_offset, 0);
    }

    #[test]
    fn visual_to_logical_u16_accounts_for_scroll() {
        let mut v = MarkdownEditorView::new();
        let lines: Vec<String> = (0..10).map(|i| format!("line{}", i)).collect();
        v.update(&lines, (5, 0), rect(3));
        let scroll = v.visual_scroll_offset;
        // Visual row 0 on screen = logical row `scroll`
        let (row, _col) = v.visual_to_logical_u16(scroll, 0);
        assert_eq!(row as usize, scroll);
    }

    #[test]
    fn code_block_detection_cursor_inside() {
        let lines = vec![
            "text".to_string(),
            "```rust".to_string(),
            "let x = 1;".to_string(),
            "```".to_string(),
            "more".to_string(),
        ];
        let block = MarkdownEditorView::find_code_block(&lines, 2);
        assert!(block.is_some());
        let r = block.unwrap();
        assert_eq!(r.start, 1);
        assert_eq!(r.end, 4); // exclusive end = line after closing fence
    }

    #[test]
    fn code_block_detection_cursor_outside() {
        let lines = vec![
            "text".to_string(),
            "```".to_string(),
            "code".to_string(),
            "```".to_string(),
        ];
        assert!(MarkdownEditorView::find_code_block(&lines, 0).is_none());
    }
}
