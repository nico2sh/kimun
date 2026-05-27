use super::markdown::{MarkdownSpanner, ParsedBuffer};
use super::word_wrap::WordWrapLayout;
use crate::settings::themes::Theme;
use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use ratatui::text::{Line, Text};
use ratatui::widgets::Paragraph;
use std::ops::Range;
use std::sync::OnceLock;
use unicode_width::UnicodeWidthStr;

pub struct MarkdownEditorView {
    pub layout: WordWrapLayout,
    pub visual_scroll_offset: usize,
    pub lines_snapshot: Vec<String>,
    pub cursor_snapshot: (usize, usize),
    /// Line ranges of every fenced code block in the buffer. Text-keyed
    /// (rebuilt only when `text_revision` changes); `is_in_code_block`
    /// does a cheap point lookup against this list per row so all fenced
    /// blocks render `force_raw` regardless of where the cursor is.
    fence_ranges: Vec<Range<usize>>,
    /// Cursor's last on-screen position (col, row), or `None` when the
    /// cursor was scrolled off-screen or the view was unfocused at the
    /// time of the previous `render`. Used as the anchor for floating
    /// overlays like the autocomplete popup, which is drawn after the
    /// editor itself.
    pub last_cursor_screen: Option<(u16, u16)>,
    /// Per-line parse cache built in `update()`. Eliminates redundant pulldown-cmark
    /// invocations across `render()`, cursor placement, and click mapping.
    parsed_buffer: ParsedBuffer,
    /// Last `text_revision` seen — gates the lines clone and parse-cache rebuild.
    /// Cursor-only moves do not bump `text_revision`, so navigating with the
    /// arrow keys reuses the parse cache instead of re-running pulldown-cmark
    /// over the whole buffer.
    last_seen_generation: u64,
    /// `text_revision`/width/cursor at which the layout was last computed.
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
    /// Diagnostic: true when the most recent Gate 1 invocation used the
    /// incremental splice path, false when it took the full-parse fallback.
    /// Read by tests; not part of the production observable surface.
    pub last_parse_was_incremental: bool,
}

#[cfg(debug_assertions)]
fn verify_incremental_enabled() -> bool {
    static VERIFY: OnceLock<bool> = OnceLock::new();
    *VERIFY.get_or_init(|| {
        std::env::var("KIMUN_VIEW_VERIFY_INCREMENTAL")
            .map(|v| !v.is_empty() && v != "0")
            .unwrap_or(false)
    })
}

impl MarkdownEditorView {
    pub fn new() -> Self {
        Self {
            layout: WordWrapLayout::default(),
            visual_scroll_offset: 0,
            lines_snapshot: Vec::new(),
            cursor_snapshot: (0, 0),
            fence_ranges: Vec::new(),
            last_cursor_screen: None,
            parsed_buffer: ParsedBuffer { lines: Vec::new(), kinds: Vec::new() },
            last_seen_generation: u64::MAX, // force rebuild on first update
            last_layout_generation: u64::MAX,
            last_layout_width: 0,
            last_layout_cursor: (usize::MAX, usize::MAX),
            cursor_vrow: 0,
            rendered_cache: Vec::new(),
            selection: None,
            last_parse_was_incremental: false,
        }
    }

    pub fn update(
        &mut self,
        lines: &[String],
        cursor: (usize, usize),
        rect: Rect,
        generation: u64,
        selection: Option<((usize, usize), (usize, usize))>,
    ) {
        self.selection = selection;
        if rect.height == 0 {
            return;
        }

        // Gate 1: content changed — rebuild parse cache and snapshots.
        if generation != self.last_seen_generation {
            self.last_parse_was_incremental = match self.try_incremental_parse(lines, cursor) {
                Some((range, slice)) => {
                    self.parsed_buffer.splice(range, slice);
                    true
                }
                None => {
                    self.parsed_buffer = ParsedBuffer::parse(lines);
                    false
                }
            };
            #[cfg(debug_assertions)]
            if self.last_parse_was_incremental && verify_incremental_enabled() {
                let fresh = ParsedBuffer::parse(lines);
                assert_eq!(
                    self.parsed_buffer.kinds, fresh.kinds,
                    "incremental kinds diverge from full parse at generation={generation}"
                );
                assert_eq!(
                    self.parsed_buffer.lines.len(),
                    fresh.lines.len(),
                    "incremental lines.len() diverges from full parse at generation={generation}"
                );
                for (i, (got, exp)) in self.parsed_buffer.lines.iter().zip(fresh.lines.iter()).enumerate() {
                    got.debug_assert_eq_to(exp, i);
                }
            }
            self.fence_ranges = super::parse_incremental::fence_ranges_from_kinds(&self.parsed_buffer.kinds);
            self.lines_snapshot = lines.to_vec();
            self.last_seen_generation = generation;
        }

        self.cursor_snapshot = cursor;

        // Gate 2: layout rebuild.
        // Skip when content, width, and the *effective element expansion* are all unchanged.
        // Horizontal cursor movement within the same element (or plain text with no elements)
        // does not change any wrap boundary — no recompute needed.
        let new_expanded = self
            .parsed_buffer.lines
            .get(cursor.0)
            .and_then(|p| p.elem_at(cursor.1));
        let old_expanded = self
            .parsed_buffer.lines
            .get(self.last_layout_cursor.0)
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
                self.rendered_cache = lines
                    .iter()
                    .enumerate()
                    .map(|(i, l)| {
                        let force_raw = self.is_in_code_block(i);
                        let cursor_col = if i == cursor.0 { Some(cursor.1) } else { None };
                        MarkdownSpanner::visible_positions_with(
                            l,
                            &self.parsed_buffer.lines[i],
                            cursor_col,
                            force_raw,
                        )
                    })
                    .collect();
            } else if !width_changed {
                // Partial rebuild — only the two rows whose cursor_col argument changed.
                let old_row = self.last_layout_cursor.0;
                let new_row = cursor.0;
                for row in [old_row, new_row] {
                    if let Some(l) = lines.get(row)
                        && let Some(p) = self.parsed_buffer.lines.get(row)
                    {
                        let force_raw = self.is_in_code_block(row);
                        let cursor_col = if row == new_row { Some(cursor.1) } else { None };
                        if let Some(entry) = self.rendered_cache.get_mut(row) {
                            *entry = MarkdownSpanner::visible_positions_with(
                                l, p, cursor_col, force_raw,
                            );
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

    /// Attempt an incremental Gate-1 parse.
    ///
    /// Returns `Some((range, slice))` when the damage can be cheaply
    /// isolated and widened to safe boundaries; `None` when the caller
    /// should fall back to a fresh full-buffer `ParsedBuffer::parse`.
    fn try_incremental_parse(
        &self,
        lines: &[String],
        cursor: (usize, usize),
    ) -> Option<(std::ops::Range<usize>, ParsedBuffer)> {
        use super::parse_incremental::{compute_damage_range, widen_to_safe, WidenResult};

        if self.parsed_buffer.lines.is_empty() {
            return None; // First parse — no snapshot to diff against.
        }
        // Line count changes (insertions/deletions) require a full rebuild:
        // the widened range covers the same number of lines in the new buffer
        // as in the old kinds array, so a splice cannot reconcile the length
        // mismatch.
        if lines.len() != self.parsed_buffer.lines.len() {
            return None;
        }
        let damaged = compute_damage_range(&self.lines_snapshot, lines, cursor.0)?;
        let widened = match widen_to_safe(&self.parsed_buffer.kinds, damaged) {
            WidenResult::Widened(r) => r,
            WidenResult::FullRebuild => return None,
        };
        let slice = ParsedBuffer::parse_range(lines, widened.clone());
        Some((widened, slice))
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        if rect.height == 0 {
            return;
        }
        let lines = &self.lines_snapshot;
        let cursor = self.cursor_snapshot;
        let scroll = self.visual_scroll_offset;
        let height = rect.height as usize;
        let vlines = self.layout.visual_lines();

        let selection = self.selection;
        let parsed_lines = &self.parsed_buffer.lines;
        let fence_ranges = &self.fence_ranges;

        let visible: Vec<Line> = vlines
            .iter()
            .skip(scroll)
            .take(height)
            .map(|vl| {
                let cursor_col = if vl.logical_row == cursor.0 {
                    Some(cursor.1)
                } else {
                    None
                };
                let force_raw = fence_ranges.iter().any(|r| r.contains(&vl.logical_row));
                let logical_line = lines.get(vl.logical_row).map(|s| s.as_str()).unwrap_or("");
                let parsed = &parsed_lines[vl.logical_row];
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
                                logical_line,
                                parsed,
                                vl.start_col,
                                sel_sc,
                                vl.is_first_visual_line,
                                force_raw,
                            )
                        } else {
                            0
                        };
                        let end_rendered = if row == sel_er {
                            MarkdownSpanner::rendered_cursor_col_with(
                                logical_line,
                                parsed,
                                vl.start_col,
                                sel_ec,
                                vl.is_first_visual_line,
                                force_raw,
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
            Paragraph::new(Text::from(visible)).style(theme.base_style()),
            rect,
        );

        // Draw terminal cursor when focused
        self.last_cursor_screen = None;
        if focused {
            let cursor_vrow = self.cursor_vrow;
            if cursor_vrow >= scroll
                && cursor_vrow < scroll + height
                && let Some(vl) = self.layout.visual_lines().get(cursor_vrow)
                && let Some(parsed) = self.parsed_buffer.lines.get(cursor.0)
            {
                // Use `.get()` on both layout.visual_lines and parsed_cache so
                // a transiently-stale cursor (e.g. Nvim snapshot arriving
                // after a shrink) cannot panic the render path. `lines.get`
                // was already guarded; this brings the other two into sync.
                let logical_line = lines.get(cursor.0).map(|s| s.as_str()).unwrap_or("");
                let force_raw = self.is_in_code_block(cursor.0);
                let rendered_col = MarkdownSpanner::rendered_cursor_col_with(
                    logical_line,
                    parsed,
                    vl.start_col,
                    cursor.1,
                    vl.is_first_visual_line,
                    force_raw,
                );
                let cx = rect.x + rendered_col as u16;
                let cy = rect.y + (cursor_vrow - scroll) as u16;
                f.set_cursor_position(Position { x: cx, y: cy });
                self.last_cursor_screen = Some((cx, cy));
            }
        }
    }

    fn is_in_code_block(&self, row: usize) -> bool {
        // Every line inside any fenced block renders force-raw (no markdown
        // re-styling, distinct fg color). Previously this checked only the
        // fence the cursor was sitting in, so fenced blocks elsewhere in
        // the buffer looked like plain text until the cursor moved into
        // them.
        self.fence_ranges.iter().any(|r| r.contains(&row))
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
        let logical_line = self
            .lines_snapshot
            .get(vl.logical_row)
            .map(|s| s.as_str())
            .unwrap_or("");
        let force_raw = self.is_in_code_block(vl.logical_row);
        let logical_col = MarkdownSpanner::rendered_col_to_logical_with(
            logical_line,
            &self.parsed_buffer.lines[vl.logical_row],
            vl.start_col,
            vcol,
            vl.is_first_visual_line,
            force_raw,
        );
        let row = vl.logical_row.min(u16::MAX as usize) as u16;
        let col = logical_col.min(u16::MAX as usize) as u16;
        (row, col)
    }

}

impl Default for MarkdownEditorView {
    fn default() -> Self {
        Self::new()
    }
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
                byte_offset_for_display_width(&content[prefix_byte..], selected_width)
                    + prefix_byte;

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

    fn rect(h: u16) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: 40,
            height: h,
        }
    }

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
        let pb = ParsedBuffer::parse(&lines);
        let ranges = super::super::parse_incremental::fence_ranges_from_kinds(&pb.kinds);
        let block = ranges.iter().find(|r| r.contains(&2)).cloned();
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
        let pb = ParsedBuffer::parse(&lines);
        let ranges = super::super::parse_incremental::fence_ranges_from_kinds(&pb.kinds);
        assert!(ranges.iter().find(|r| r.contains(&0)).is_none());
    }

    #[test]
    fn render_does_not_panic_on_stale_cursor_past_line_count() {
        // Regression: render() previously did self.parsed_cache[cursor.0]
        // and self.layout.visual_lines()[cursor_vrow] directly. A stale
        // Nvim snapshot whose cursor row landed past the new line count
        // would panic the render thread. The guards now use `.get()`.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let theme = Theme::gruvbox_dark();
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        let mut v = MarkdownEditorView::new();
        // Populate with 2 lines and a valid cursor first so parsed_cache /
        // layout are non-empty.
        v.update(&["alpha".to_string(), "beta".to_string()], (0, 0), rect(8), 1, None);
        // Now feed a cursor row that exceeds the line count for this update
        // (simulates a stale snapshot arriving after a shrink). update() at
        // line 277 already uses `lines.get(cursor.0)` so it won't panic; the
        // real risk was the [] indexes inside render(). cursor_snapshot ends
        // up at (5, 0) which exceeds the parsed_cache len of 2 below.
        v.update(&["alpha".to_string(), "beta".to_string()], (5, 0), rect(8), 1, None);
        // Render with focus so the cursor branch runs.
        terminal
            .draw(|f| v.render(f, f.area(), &theme, true))
            .expect("render must not panic on stale cursor");
    }

    #[test]
    fn is_in_code_block_returns_true_for_any_fence_regardless_of_cursor() {
        // Regression: after commit cceef444, every fenced block renders
        // force-raw — not just the one the cursor sits in. Verify by
        // probing `is_in_code_block` for a row in a fence while the
        // cursor is positioned elsewhere.
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "intro".to_string(),
            "```".to_string(),
            "code".to_string(),
            "```".to_string(),
            "outro".to_string(),
        ];
        // Cursor on the prose line; fence interior must still report in-block.
        v.update(&lines, (4, 0), rect(10), 1, None);
        assert!(v.is_in_code_block(2), "fence interior is in-block");
        assert!(!v.is_in_code_block(0), "prose line is not in-block");
        assert!(!v.is_in_code_block(4), "trailing prose is not in-block");
    }

    #[test]
    fn parsed_cache_populated_after_update() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["hello".to_string(), "**bold**".to_string()];
        v.update(&lines, (0, 0), rect(10), 1, None);
        assert_eq!(v.parsed_buffer.lines.len(), 2);
    }

    #[test]
    fn layout_skipped_on_horizontal_cursor_move_in_plain_text() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["hello world".to_string()];
        v.update(&lines, (0, 0), rect(40), 1, None);
        let layout_gen_after_first = v.last_layout_generation;
        // Move cursor right — same row, no elements, same generation → layout must be skipped.
        v.update(&lines, (0, 5), rect(40), 1, None);
        assert_eq!(
            v.last_layout_cursor,
            (0, 0),
            "layout cursor unchanged = layout was skipped"
        );
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
        v.update(
            &lines,
            (0, 0),
            Rect {
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            },
            1,
            None,
        );
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

    #[test]
    fn typing_single_char_in_long_buffer_uses_incremental_path() {
        let mut v = MarkdownEditorView::new();
        let mut lines: Vec<String> = (0..1000).map(|i| format!("paragraph {i}")).collect();
        v.update(&lines, (500, 0), rect(40), 1, None);

        // Single-char insert at row 500.
        lines[500].push('x');
        let edited_len = lines[500].len();
        v.update(&lines, (500, edited_len), rect(40), 2, None);

        // The spliced result must equal a fresh full parse.
        let fresh = ParsedBuffer::parse(&lines);
        assert_eq!(v.parsed_buffer.lines.len(), fresh.lines.len());
        assert_eq!(v.parsed_buffer.kinds, fresh.kinds);
        // And the incremental path was actually taken.
        assert!(
            v.last_parse_was_incremental,
            "single-char paragraph edit should take incremental path"
        );
    }

    #[test]
    fn fence_toggle_triggers_full_rebuild_fallback() {
        let mut v = MarkdownEditorView::new();
        // Use 1000 lines so that an unclosed fence at row 500 widens to
        // end-of-buffer (~501 rows), exceeding both the absolute cap (256)
        // and the fractional cap (50% of 1001 = 500 rows). The `&&`
        // cap check fires and forces full rebuild.
        let mut lines: Vec<String> = (0..1000).map(|i| format!("paragraph {i}")).collect();
        v.update(&lines, (500, 0), rect(40), 1, None);

        // Open a fence mid-buffer — structurally invasive, line count changes.
        lines.insert(500, "```".to_string());
        v.update(&lines, (500, 3), rect(40), 2, None);

        let fresh = ParsedBuffer::parse(&lines);
        assert_eq!(v.parsed_buffer.kinds, fresh.kinds, "spliced kinds must equal fresh full parse");
        // The unclosed fence at row 500 widens to end-of-buffer (>256 lines
        // and >50% of 1001), so the cap trips and the fallback fires.
        assert!(
            !v.last_parse_was_incremental,
            "fence toggle (unclosed fence, 1000-line buffer) should fall back to full rebuild"
        );
    }

    fn full_rebuild_equals_view_state(v: &MarkdownEditorView, lines: &[String]) {
        let fresh = ParsedBuffer::parse(lines);
        assert_eq!(v.parsed_buffer.kinds, fresh.kinds, "kinds diverge");
        assert_eq!(v.parsed_buffer.lines.len(), fresh.lines.len(), "row count diverge");
        for (i, (got, exp)) in v.parsed_buffer.lines.iter().zip(fresh.lines.iter()).enumerate() {
            got.debug_assert_eq_to(exp, i);
        }
    }

    #[test]
    fn incremental_paste_large_block_falls_back() {
        let mut v = MarkdownEditorView::new();
        let mut lines: Vec<String> = (0..50).map(|i| format!("line {i}")).collect();
        v.update(&lines, (25, 0), rect(40), 1, None);

        // Insert 300 lines at row 25.
        let payload: Vec<String> = (0..300).map(|i| format!("pasted {i}")).collect();
        for (offset, p) in payload.into_iter().enumerate() {
            lines.insert(25 + offset, p);
        }
        v.update(&lines, (25, 0), rect(40), 2, None);
        assert!(!v.last_parse_was_incremental, "300-line paste must fall back");
        full_rebuild_equals_view_state(&v, &lines);
    }

    #[test]
    fn incremental_enter_at_line_end() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["alpha".to_string(), "beta".to_string()];
        v.update(&lines, (0, 5), rect(40), 1, None);

        // Press Enter at end of "alpha".
        let new_lines = vec!["alpha".to_string(), "".to_string(), "beta".to_string()];
        v.update(&new_lines, (1, 0), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn incremental_backspace_merging_lines() {
        let mut v = MarkdownEditorView::new();
        let lines = vec!["alpha".to_string(), "beta".to_string()];
        v.update(&lines, (1, 0), rect(40), 1, None);

        // Backspace at start of "beta" merges into "alphabeta".
        let new_lines = vec!["alphabeta".to_string()];
        v.update(&new_lines, (0, 5), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn incremental_inside_fence_widens_both_markers() {
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "intro".to_string(),
            "".to_string(),
            "```rust".to_string(),
            "let x = 1;".to_string(),
            "let y = 2;".to_string(),
            "```".to_string(),
            "".to_string(),
            "outro".to_string(),
        ];
        v.update(&lines, (3, 0), rect(40), 1, None);

        // Edit inside the fence (same-length, no line-count change).
        let mut new_lines = lines.clone();
        new_lines[3] = "let x = 999;".to_string();
        v.update(&new_lines, (3, 8), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn incremental_list_continuation_widens_to_outer_marker() {
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "- top".to_string(),
            "  body of top".to_string(),
            "  - nested".to_string(),
            "    body of nested".to_string(),
            "    body two".to_string(),
            "".to_string(),
            "outro".to_string(),
        ];
        v.update(&lines, (4, 0), rect(40), 1, None);

        // Edit the nested continuation line.
        let mut new_lines = lines.clone();
        new_lines[4] = "    body two changed".to_string();
        v.update(&new_lines, (4, 10), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn incremental_setext_underline_edit() {
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "heading text".to_string(),
            "====".to_string(),
            "".to_string(),
            "body".to_string(),
        ];
        v.update(&lines, (1, 0), rect(40), 1, None);

        // Edit the underline (same line count).
        let mut new_lines = lines.clone();
        new_lines[1] = "======".to_string();
        v.update(&new_lines, (1, 6), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn incremental_blockquote_paragraph_edit() {
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "intro".to_string(),
            "".to_string(),
            "> quoted line one".to_string(),
            "> quoted line two".to_string(),
            "> quoted line three".to_string(),
            "".to_string(),
            "outro".to_string(),
        ];
        v.update(&lines, (3, 0), rect(40), 1, None);

        let mut new_lines = lines.clone();
        new_lines[3] = "> quoted line TWO".to_string();
        v.update(&new_lines, (3, 17), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn incremental_html_block_edit() {
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "before".to_string(),
            "".to_string(),
            "<div>".to_string(),
            "body".to_string(),
            "</div>".to_string(),
            "".to_string(),
            "after".to_string(),
        ];
        v.update(&lines, (3, 0), rect(40), 1, None);

        let mut new_lines = lines.clone();
        new_lines[3] = "body changed".to_string();
        v.update(&new_lines, (3, 12), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &new_lines);
    }
}
