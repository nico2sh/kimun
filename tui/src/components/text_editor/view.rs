use std::ops::Range;
use unicode_width::UnicodeWidthStr;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Text};
use ratatui::widgets::Paragraph;
use ratatui::layout::Position;
use crate::settings::themes::Theme;
use super::word_wrap::WordWrapLayout;
use super::markdown::{MarkdownSpanner, ParsedLine};

pub struct MarkdownEditorView {
    pub layout: WordWrapLayout,
    pub visual_scroll_offset: usize,
    pub lines_snapshot: Vec<String>,
    pub cursor_snapshot: (usize, usize),
    pub cursor_code_block: Option<Range<usize>>,
    /// Per-line parse cache built in `update()`. Eliminates redundant pulldown-cmark
    /// invocations across `render()`, cursor placement, and click mapping.
    parsed_cache: Vec<ParsedLine>,
    /// Last `edit_generation` seen — gates the lines clone and parse-cache rebuild.
    last_seen_generation: u64,
    /// Generation/width/cursor at which the layout was last computed.
    /// Used to skip `WordWrapLayout::compute()` when nothing affecting wrap has changed:
    /// horizontal cursor movement within the same element (or plain text) is free.
    last_layout_generation: u64,
    last_layout_width: u16,
    last_layout_cursor: (usize, usize),
    /// Visual row of the cursor, cached after layout so `render()` doesn't call
    /// `logical_to_visual` a second time.
    cursor_vrow: usize,
    /// Per-line rendered-position bitmask, cached between layout recomputes.
    /// Only the two cursor rows (old and new) are rebuilt when just the cursor row changes;
    /// all rows are rebuilt when content or width changes.
    rendered_cache: Vec<Vec<bool>>,
    /// Current selection range in logical (row, byte-col) coordinates.
    /// `None` when no selection is active.
    selection: Option<((usize, usize), (usize, usize))>,
}

impl MarkdownEditorView {
    pub fn new() -> Self {
        Self {
            layout: WordWrapLayout::default(),
            visual_scroll_offset: 0,
            lines_snapshot: Vec::new(),
            cursor_snapshot: (0, 0),
            cursor_code_block: None,
            parsed_cache: Vec::new(),
            last_seen_generation: u64::MAX, // force rebuild on first update
            last_layout_generation: u64::MAX,
            last_layout_width: 0,
            last_layout_cursor: (usize::MAX, usize::MAX),
            cursor_vrow: 0,
            rendered_cache: Vec::new(),
            selection: None,
        }
    }

    pub fn update(&mut self, lines: &[String], cursor: (usize, usize), rect: Rect, generation: u64, selection: Option<((usize, usize), (usize, usize))>) {
        self.selection = selection;
        if rect.height == 0 { return; }

        // Gate 1: content changed — rebuild parse cache and snapshots.
        if generation != self.last_seen_generation {
            self.lines_snapshot = lines.to_vec();
            self.cursor_code_block = Self::find_code_block(lines, cursor.0);
            self.parsed_cache = lines.iter().map(|l| ParsedLine::parse(l)).collect();
            self.last_seen_generation = generation;
        }

        self.cursor_snapshot = cursor;

        // Gate 2: layout rebuild.
        // Skip when content, width, and the *effective element expansion* are all unchanged.
        // Horizontal cursor movement within the same element (or plain text with no elements)
        // does not change any wrap boundary — no recompute needed.
        let new_expanded = self.parsed_cache.get(cursor.0).and_then(|p| p.elem_at(cursor.1));
        let old_expanded = self.parsed_cache.get(self.last_layout_cursor.0)
            .and_then(|p| p.elem_at(self.last_layout_cursor.1));
        let need_layout = generation != self.last_layout_generation
            || rect.width != self.last_layout_width
            || cursor.0 != self.last_layout_cursor.0
            || new_expanded != old_expanded;

        if need_layout {
            // Rebuild rendered-position masks. Full rebuild when content changed;
            // partial rebuild (only the two cursor rows) when only the cursor row moved.
            let content_changed = generation != self.last_layout_generation;
            let width_changed = rect.width != self.last_layout_width;
            if content_changed || self.rendered_cache.len() != lines.len() {
                // Full rebuild — content or line count changed.
                self.rendered_cache = lines.iter()
                    .enumerate()
                    .map(|(i, l)| {
                        let force_raw = self.is_in_code_block(i);
                        let cursor_col = if i == cursor.0 { Some(cursor.1) } else { None };
                        MarkdownSpanner::visible_positions_with(l, &self.parsed_cache[i], cursor_col, force_raw)
                    })
                    .collect();
            } else if !width_changed {
                // Partial rebuild — only the two rows whose cursor_col argument changed.
                let old_row = self.last_layout_cursor.0;
                let new_row = cursor.0;
                for row in [old_row, new_row] {
                    if let Some(l) = lines.get(row) {
                        if let Some(p) = self.parsed_cache.get(row) {
                            let force_raw = self.is_in_code_block(row);
                            let cursor_col = if row == new_row { Some(cursor.1) } else { None };
                            self.rendered_cache[row] =
                                MarkdownSpanner::visible_positions_with(l, p, cursor_col, force_raw);
                        }
                    }
                }
            }
            // Width-only change: masks are width-independent; reuse rendered_cache as-is.
            self.layout = WordWrapLayout::compute(lines, rect.width, &self.rendered_cache);
            self.last_layout_generation = generation;
            self.last_layout_width = rect.width;
            self.last_layout_cursor = cursor;
        }

        // Cache cursor_vrow for render() — avoids a second logical_to_visual call.
        self.cursor_vrow = self.layout.logical_to_visual(cursor.0, cursor.1).0;
        let height = rect.height as usize;
        if self.cursor_vrow < self.visual_scroll_offset {
            self.visual_scroll_offset = self.cursor_vrow;
        } else if self.cursor_vrow >= self.visual_scroll_offset + height {
            self.visual_scroll_offset = self.cursor_vrow - height + 1;
        }
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        if rect.height == 0 { return; }
        let lines = &self.lines_snapshot;
        let cursor = self.cursor_snapshot;
        let scroll = self.visual_scroll_offset;
        let height = rect.height as usize;
        let vlines = self.layout.visual_lines();

        let selection = self.selection;
        let parsed_cache = &self.parsed_cache;
        let cursor_code_block = &self.cursor_code_block;

        let visible: Vec<Line> = vlines
            .iter()
            .skip(scroll)
            .take(height)
            .map(|vl| {
                let cursor_col = if vl.logical_row == cursor.0 { Some(cursor.1) } else { None };
                let force_raw = cursor_code_block.as_ref().map_or(false, |r| r.contains(&vl.logical_row));
                let logical_line = lines.get(vl.logical_row).map(|s| s.as_str()).unwrap_or("");
                let parsed = &parsed_cache[vl.logical_row];
                let content = vl.content(logical_line);
                let spans = MarkdownSpanner::render_with(
                    content,
                    logical_line,
                    parsed,
                    vl.start_col,
                    cursor_col,
                    vl.is_first_visual_line,
                    force_raw,
                    rect.width,
                    theme,
                );

                // Apply selection highlight if this visual line is within the selection.
                let spans = if let Some(((sel_sr, sel_sc), (sel_er, sel_ec))) = selection {
                    let row = vl.logical_row;
                    if row >= sel_sr && row <= sel_er {
                        let start_rendered = if row == sel_sr {
                            MarkdownSpanner::rendered_cursor_col_with(
                                logical_line, parsed, vl.start_col, sel_sc,
                                vl.is_first_visual_line, force_raw,
                            )
                        } else {
                            0
                        };
                        let end_rendered = if row == sel_er {
                            MarkdownSpanner::rendered_cursor_col_with(
                                logical_line, parsed, vl.start_col, sel_ec,
                                vl.is_first_visual_line, force_raw,
                            )
                        } else {
                            // Entire line is selected; use a sentinel larger than any line width.
                            u16::MAX as usize
                        };
                        apply_selection_highlight(spans, start_rendered..end_rendered, theme)
                    } else {
                        spans
                    }
                } else {
                    spans
                };

                Line::from(spans)
            })
            .collect();

        f.render_widget(
            Paragraph::new(Text::from(visible))
                .style(theme.base_style()),
            rect,
        );

        // Draw terminal cursor when focused
        if focused {
            let cursor_vrow = self.cursor_vrow;
            if cursor_vrow >= scroll && cursor_vrow < scroll + height {
                let vl = &self.layout.visual_lines()[cursor_vrow];
                let logical_line = lines.get(cursor.0).map(|s| s.as_str()).unwrap_or("");
                let force_raw = self.is_in_code_block(cursor.0);
                let rendered_col = MarkdownSpanner::rendered_cursor_col_with(
                    logical_line, &self.parsed_cache[cursor.0], vl.start_col, cursor.1,
                    vl.is_first_visual_line, force_raw,
                );
                f.set_cursor_position(Position {
                    x: rect.x + rendered_col as u16,
                    y: rect.y + (cursor_vrow - scroll) as u16,
                });
            }
        }
    }

    fn is_in_code_block(&self, row: usize) -> bool {
        self.cursor_code_block.as_ref().map_or(false, |r| r.contains(&row))
    }

    /// Markdown-aware mouse click: maps a rendered screen column to the correct logical
    /// column, accounting for hidden markdown sigils (links, bold markers, etc.).
    pub fn click_to_logical_u16(&self, vrow: usize, vcol: usize) -> (u16, u16) {
        let vlines = self.layout.visual_lines();
        if vlines.is_empty() {
            return (0, 0);
        }
        let vrow = vrow.min(vlines.len() - 1);
        let vl = &vlines[vrow];
        let logical_line = self.lines_snapshot.get(vl.logical_row).map(|s| s.as_str()).unwrap_or("");
        let force_raw = self.is_in_code_block(vl.logical_row);
        let logical_col = MarkdownSpanner::rendered_col_to_logical_with(
            logical_line, &self.parsed_cache[vl.logical_row], vl.start_col, vcol,
            vl.is_first_visual_line, force_raw,
        );
        let row = vl.logical_row.min(u16::MAX as usize) as u16;
        let col = logical_col.min(u16::MAX as usize) as u16;
        (row, col)
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
        // Unclosed fence: per CommonMark, extends to end of document.
        if let Some(start) = open {
            let range = start..lines.len();
            if range.contains(&cursor_row) {
                return Some(range);
            }
        }
        None
    }
}

impl Default for MarkdownEditorView {
    fn default() -> Self { Self::new() }
}

/// Returns the byte offset into `s` after consuming exactly `target_width` display columns.
/// If `target_width` exceeds the string's display width, returns `s.len()`.
fn byte_offset_for_display_width(s: &str, target_width: usize) -> usize {
    let mut consumed = 0usize;
    for (byte_pos, ch) in s.char_indices() {
        if consumed >= target_width {
            return byte_pos;
        }
        consumed += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    s.len()
}

/// Re-style spans to apply `bg_selected` over the given rendered-column range.
///
/// `sel_cols` is a range of rendered (screen) column offsets within the visual line.
/// Spans that overlap the range are split at the boundaries; the overlapping portion
/// receives `.bg(theme.bg_selected)`. Non-overlapping portions keep their original style.
fn apply_selection_highlight<'a>(
    spans: Vec<ratatui::text::Span<'a>>,
    sel_cols: std::ops::Range<usize>,
    theme: &Theme,
) -> Vec<ratatui::text::Span<'a>> {
    if sel_cols.is_empty() {
        return spans;
    }

    let highlight_bg = theme.bg_selected.to_ratatui();
    let mut result = Vec::new();
    let mut col = 0usize;

    for span in spans {
        let content: &str = &span.content;
        let span_width = content.width();
        let span_end = col + span_width;

        let overlap_start = sel_cols.start.max(col);
        let overlap_end = sel_cols.end.min(span_end);

        if overlap_start >= overlap_end {
            // No overlap — emit as-is.
            result.push(span);
        } else {
            // Walk grapheme clusters by display width to find byte boundaries.
            let prefix_width = overlap_start - col;
            let selected_width = overlap_end - overlap_start;

            let prefix_byte = byte_offset_for_display_width(content, prefix_width);
            let selected_byte_end =
                byte_offset_for_display_width(&content[prefix_byte..], selected_width) + prefix_byte;

            // Prefix (before selection)
            if prefix_byte > 0 {
                result.push(ratatui::text::Span::styled(
                    content[..prefix_byte].to_string(),
                    span.style,
                ));
            }
            // Selected portion
            result.push(ratatui::text::Span::styled(
                content[prefix_byte..selected_byte_end].to_string(),
                span.style.bg(highlight_bg),
            ));
            // Suffix (after selection)
            if selected_byte_end < content.len() {
                result.push(ratatui::text::Span::styled(
                    content[selected_byte_end..].to_string(),
                    span.style,
                ));
            }
        }

        col = span_end;
    }

    result
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
        v.update(&["hello".to_string()], (0, 0), rect(0), 1, None);
    }

    #[test]
    fn scroll_follows_cursor_down() {
        let mut v = MarkdownEditorView::new();
        let lines: Vec<String> = (0..5).map(|i| format!("line{}", i)).collect();
        v.update(&lines, (4, 0), rect(3), 1, None);
        assert!(v.visual_scroll_offset >= 2);
    }

    #[test]
    fn scroll_follows_cursor_up() {
        let mut v = MarkdownEditorView::new();
        let lines: Vec<String> = (0..5).map(|i| format!("line{}", i)).collect();
        v.update(&lines, (4, 0), rect(3), 1, None);
        v.update(&lines, (0, 0), rect(3), 1, None); // same generation — scroll still adjusts
        assert_eq!(v.visual_scroll_offset, 0);
    }

    #[test]
    fn visual_to_logical_u16_accounts_for_scroll() {
        let mut v = MarkdownEditorView::new();
        let lines: Vec<String> = (0..10).map(|i| format!("line{}", i)).collect();
        v.update(&lines, (5, 0), rect(3), 1, None);
        let scroll = v.visual_scroll_offset;
        let (row, _col) = v.click_to_logical_u16(scroll, 0);
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
        assert_eq!(r.end, 4);
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

    #[test]
    fn parsed_cache_populated_after_update() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["hello".to_string(), "**bold**".to_string()];
        v.update(&lines, (0, 0), rect(10), 1, None);
        assert_eq!(v.parsed_cache.len(), 2);
    }

    #[test]
    fn layout_skipped_on_horizontal_cursor_move_in_plain_text() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["hello world".to_string()];
        v.update(&lines, (0, 0), rect(40), 1, None);
        let layout_gen_after_first = v.last_layout_generation;
        // Move cursor right — same row, no elements, same generation → layout must be skipped.
        v.update(&lines, (0, 5), rect(40), 1, None);
        assert_eq!(v.last_layout_cursor, (0, 0), "layout cursor unchanged = layout was skipped");
        assert_eq!(v.last_layout_generation, layout_gen_after_first);
    }

    #[test]
    fn layout_recomputed_on_row_change() {
        let mut v = MarkdownEditorView::new();
        let lines: Vec<String> = (0..3).map(|i| format!("line{}", i)).collect();
        v.update(&lines, (0, 0), rect(40), 1, None);
        v.update(&lines, (1, 0), rect(40), 1, None); // cursor moves to row 1
        assert_eq!(v.last_layout_cursor.0, 1, "layout recomputed on row change");
    }

    #[test]
    fn layout_recomputed_on_width_change() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["hello world foo bar".to_string()];
        v.update(&lines, (0, 0), rect(40), 1, None);
        v.update(&lines, (0, 0), Rect { x: 0, y: 0, width: 10, height: 10 }, 1, None);
        assert_eq!(v.last_layout_width, 10);
    }

    #[test]
    fn same_generation_skips_snapshot_rebuild() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["original".to_string()];
        v.update(&lines, (0, 0), rect(10), 1, None);
        // Update with different content but same generation — snapshot must NOT change.
        let lines2 = vec!["changed".to_string()];
        v.update(&lines2, (0, 0), rect(10), 1, None);
        assert_eq!(v.lines_snapshot, vec!["original".to_string()]);
    }

    #[test]
    fn new_generation_triggers_snapshot_rebuild() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["original".to_string()];
        v.update(&lines, (0, 0), rect(10), 1, None);
        let lines2 = vec!["changed".to_string()];
        v.update(&lines2, (0, 0), rect(10), 2, None);
        assert_eq!(v.lines_snapshot, vec!["changed".to_string()]);
    }

    #[test]
    fn update_stores_selection() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["hello world".to_string()];
        v.update(&lines, (0, 0), rect(40), 1, Some(((0, 0), (0, 5))));
        assert_eq!(v.selection, Some(((0, 0), (0, 5))));
    }

    #[test]
    fn update_clears_selection_when_none() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["hello world".to_string()];
        v.update(&lines, (0, 0), rect(40), 1, Some(((0, 0), (0, 5))));
        v.update(&lines, (0, 0), rect(40), 1, None);
        assert_eq!(v.selection, None);
    }
}
