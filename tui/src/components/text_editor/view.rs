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

/// Describes how `view.update`'s Gate 1 modified the parse caches this
/// frame. Read by Gate 2 to decide what subset of `rendered_cache` and
/// `WordWrapLayout` needs to be rebuilt.
#[derive(Debug, Clone)]
enum TextChangeKind {
    /// No text change this frame (cursor-only update). Gate 2 may keep
    /// its caches and only refresh the cursor-row entry.
    None,
    /// Gate 1 took the incremental splice path; only rows in this
    /// range had their ParsedLine entries replaced. Gate 2 should
    /// rebuild rendered_cache only for these rows + the cursor rows.
    Incremental(std::ops::Range<usize>),
    /// Full rebuild (initial parse, line-count change, cap trip,
    /// structural-marker change, post-slice verification miss). Gate 2
    /// must rebuild rendered_cache for every row.
    Full,
}

enum RenderedCacheRebuild {
    Full,
    Rows(Vec<usize>),
    None,
}

#[derive(Clone)]
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
    /// Tracks how Gate 1 changed (or did not change) the parse caches.
    /// Gate 2 reads this to decide the scope of rendered_cache rebuild.
    last_text_change: TextChangeKind,
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

/// True when `line` looks syntactically like a fenced-code-block marker
/// per CommonMark: optional leading indent (≤3 spaces), then 3+ backticks
/// or 3+ tildes (no mixing).
fn looks_like_fence_marker(line: &str) -> bool {
    let trimmed = line.trim_start_matches(' ');
    // Allow up to 3 spaces of indent (CommonMark spec §4.5).
    let indent = line.len() - trimmed.len();
    if indent > 3 {
        return false;
    }
    (trimmed.starts_with("```")
        && trimmed.chars().take_while(|c| *c == '`').count() >= 3)
        || (trimmed.starts_with("~~~")
            && trimmed.chars().take_while(|c| *c == '~').count() >= 3)
}

/// True when `line` looks like a setext underline (line of only `=` or
/// only `-`, possibly with leading/trailing whitespace).
fn looks_like_setext_underline(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty()
        && (trimmed.chars().all(|c| c == '=') || trimmed.chars().all(|c| c == '-'))
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
            last_text_change: TextChangeKind::Full, // first update is a full rebuild
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
            self.last_text_change = match self.try_incremental_parse(lines, cursor) {
                Some((range, slice)) => {
                    self.parsed_buffer.splice(range.clone(), slice);
                    self.last_parse_was_incremental = true;
                    TextChangeKind::Incremental(range)
                }
                None => {
                    self.parsed_buffer = ParsedBuffer::parse(lines);
                    self.last_parse_was_incremental = false;
                    TextChangeKind::Full
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
        } else {
            self.last_text_change = TextChangeKind::None;
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
            let width_changed = rect.width != self.last_layout_width;
            let cursor_changed = cursor.0 != self.last_layout_cursor.0;
            // Determine the set of rows to rebuild in rendered_cache.
            let rebuild_strategy = if self.rendered_cache.len() != lines.len() {
                // Line count differs → full rebuild required.
                RenderedCacheRebuild::Full
            } else {
                match &self.last_text_change {
                    TextChangeKind::Full => RenderedCacheRebuild::Full,
                    TextChangeKind::Incremental(range) => {
                        let mut rows: Vec<usize> = range.clone().collect();
                        if cursor_changed {
                            rows.push(self.last_layout_cursor.0);
                            rows.push(cursor.0);
                        }
                        rows.sort();
                        rows.dedup();
                        RenderedCacheRebuild::Rows(rows)
                    }
                    TextChangeKind::None => {
                        if cursor_changed {
                            let mut rows = vec![self.last_layout_cursor.0, cursor.0];
                            rows.sort();
                            rows.dedup();
                            RenderedCacheRebuild::Rows(rows)
                        } else {
                            RenderedCacheRebuild::None
                        }
                    }
                }
            };

            // Width-only change: masks are width-independent; skip rendered_cache rebuild.
            let _ = width_changed; // acknowledged: width doesn't affect rendered_cache
            match rebuild_strategy {
                RenderedCacheRebuild::Full => {
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
                }
                RenderedCacheRebuild::Rows(rows) => {
                    for row in rows {
                        if row >= lines.len() {
                            continue; // defensive
                        }
                        let force_raw = self.is_in_code_block(row);
                        let cursor_col = if row == cursor.0 { Some(cursor.1) } else { None };
                        let new_entry = MarkdownSpanner::visible_positions_with(
                            &lines[row],
                            &self.parsed_buffer.lines[row],
                            cursor_col,
                            force_raw,
                        );
                        if let Some(entry) = self.rendered_cache.get_mut(row) {
                            *entry = new_entry;
                        }
                    }
                }
                RenderedCacheRebuild::None => {
                    // Width-only change or no change: masks are width-independent; nothing to rebuild.
                }
            }

            // Width-aware wrap path:
            // - Width change or line-count change: full recompute (wrap
            //   depends on width; visual_lines indexing depends on row count).
            // - TextChangeKind::Full: full recompute.
            // - TextChangeKind::Incremental(range): splice only the affected rows.
            // - TextChangeKind::None: skip wrap entirely (cursor-only update;
            //   wrap is content-independent of cursor position).
            let line_count_changed = self.layout.row_starts_len() != lines.len();
            if width_changed || line_count_changed {
                self.layout = WordWrapLayout::compute(lines, rect.width, &self.rendered_cache);
            } else {
                match &self.last_text_change {
                    TextChangeKind::Full => {
                        self.layout = WordWrapLayout::compute(lines, rect.width, &self.rendered_cache);
                    }
                    TextChangeKind::Incremental(range) => {
                        self.layout.splice_range(
                            lines,
                            rect.width,
                            &self.rendered_cache,
                            range.clone(),
                        );
                    }
                    TextChangeKind::None => {
                        // Cursor-only update. Wrap is content-independent of
                        // cursor position; existing layout is still correct.
                    }
                }
            }
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
        use super::parse_incremental::{LineConstructKind, compute_damage_range, widen_to_safe, WidenResult};

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

        // Structural-marker change guard: any edit that converts a fence
        // marker line into a non-marker (or vice versa) can shift the
        // fence's extent beyond the widening window. Same for setext
        // underlines. Conservative fallback to full parse for correctness.
        for row in damaged.clone() {
            let old_kind = self.parsed_buffer.kinds[row];
            let old_line = self.lines_snapshot[row].as_str();
            let new_line = lines[row].as_str();

            // Old was a fence marker — any edit here may change its role
            // (opener ↔ closer ↔ content), shifting the fence extent.
            if matches!(old_kind, LineConstructKind::FenceMarker) {
                return None;
            }
            // New content introduces or removes a fence-marker prefix.
            let old_fence = looks_like_fence_marker(old_line);
            let new_fence = looks_like_fence_marker(new_line);
            if old_fence != new_fence {
                return None;
            }
            // Old was a setext underline — same logic: removing or altering
            // the underline changes which line above it becomes a heading.
            if matches!(old_kind, LineConstructKind::SetextUnderline) {
                return None;
            }
            // New content looks like a setext underline but old did not
            // (or vice versa) — the heading classification propagates up.
            if looks_like_setext_underline(new_line) != looks_like_setext_underline(old_line) {
                return None;
            }
        }

        let widened = match widen_to_safe(&self.parsed_buffer.kinds, damaged.clone()) {
            WidenResult::Widened(r) => r,
            WidenResult::FullRebuild => return None,
        };

        let slice = ParsedBuffer::parse_range(lines, widened.clone());

        // Undamaged-row verification: if the slice-in-isolation classifies any
        // undamaged row differently from the initial buffer (in kind or element
        // count), the widened window lacked context from outside its bounds.
        // Fall back to a full parse for correctness.
        //
        // This check subsumes the earlier context-boundary guards and handles
        // all known cases where widen_to_safe's D5 extension produces a slice
        // start that is not a true parse-state reset point (e.g. IndentedCode
        // spanning rows into the window, Blockquote lazy continuation, and
        // loose-list continuations across blank lines).
        for row in widened.clone() {
            if damaged.contains(&row) {
                continue; // Damaged row: kind change is expected/irrelevant.
            }
            let idx = row - widened.start;
            if slice.kinds[idx] != self.parsed_buffer.kinds[row] {
                return None;
            }
            if slice.lines[idx].elements.len() != self.parsed_buffer.lines[row].elements.len() {
                return None;
            }
            if slice.lines[idx].content_vis != self.parsed_buffer.lines[row].content_vis {
                return None;
            }
        }

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

    /// Test accessor: the kinds vector of the current parsed buffer.
    /// Used by the proptest harness to assert incremental = full parse.
    pub fn parsed_buffer_kinds(&self) -> &[super::parse_incremental::LineConstructKind] {
        &self.parsed_buffer.kinds
    }

    /// Test accessor: the parsed lines of the current parsed buffer.
    pub fn parsed_buffer_lines(&self) -> &[super::markdown::ParsedLine] {
        &self.parsed_buffer.lines
    }

    /// Test accessor: the rendered-position bitmask cache.
    /// Used by tests to construct a fresh `WordWrapLayout` from the same
    /// masks the view is using, for equivalence checks.
    #[cfg(test)]
    pub(crate) fn rendered_cache_for_testing(&self) -> &[Vec<bool>] {
        &self.rendered_cache
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
    fn incremental_falls_back_when_fence_marker_modified() {
        // Regression: editing a row that is currently a FenceMarker can
        // change the fence's extent across the rest of the buffer.
        // Incremental parsing's window-bounded widening cannot capture
        // this, so we must fall back to a full parse.
        let mut v = MarkdownEditorView::new();
        let mut lines = vec![
            "```".to_string(),
            "".to_string(),
            "```".to_string(),
        ];
        // Fill out the buffer with blank lines so the cap doesn't trip first.
        for _ in 0..31 {
            lines.push(String::new());
        }
        v.update(&lines, (2, 0), rect(40), 1, None);

        // Edit the closing fence marker — append a char so it's no longer a closer.
        let mut new_lines = lines.clone();
        new_lines[2].push('0');
        v.update(&new_lines, (2, 4), rect(40), 2, None);

        assert!(
            !v.last_parse_was_incremental,
            "fence-marker edit must trigger full-rebuild fallback"
        );
        // And the resulting state must equal a fresh parse (which the
        // fallback path does anyway, but assert defensively).
        full_rebuild_equals_view_state(&v, &new_lines);
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

    #[test]
    fn g1_nested_list_three_indent_continuation() {
        // Deeply nested continuation: damaged range touches a 3-indent
        // continuation line. Widening must reach the outermost col-0
        // ListMarker — otherwise parse_range sees `      text` as
        // IndentedCode.
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "intro".to_string(),
            "".to_string(),
            "- level 0".to_string(),
            "  - level 1".to_string(),
            "    - level 2".to_string(),
            "      continuation at 6 indent".to_string(),
            "".to_string(),
            "after".to_string(),
        ];
        v.update(&lines, (5, 0), rect(40), 1, None);

        let mut new_lines = lines.clone();
        new_lines[5] = "      continuation at 6 indent EDITED".to_string();
        v.update(&new_lines, (5, 30), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn g3_hashtag_inside_fence_not_labeled_after_incremental_edit() {
        // `#tag` inside a fenced code block must NOT produce a Label element.
        // After an incremental edit fully inside the fence, the widened
        // slice includes both fence markers — the label-suppression scan
        // sees the fence and skips. This test verifies the round-trip.
        let mut v = MarkdownEditorView::new();
        let lines = vec![
            "intro".to_string(),
            "".to_string(),
            "```".to_string(),
            "let s = \"#tag\";".to_string(),
            "// another #tag".to_string(),
            "```".to_string(),
            "".to_string(),
            "outro".to_string(),
        ];
        v.update(&lines, (4, 0), rect(40), 1, None);

        use crate::components::text_editor::markdown::ElementKind;

        // Pre-condition: no Label elements in the fence interior.
        for row in 3..5 {
            let has_label = v.parsed_buffer.lines[row].elements.iter().any(|e| {
                matches!(e.kind, ElementKind::Label)
            });
            assert!(!has_label, "row {row} should have no Label inside the fence");
        }

        // Edit one of the in-fence lines.
        let mut new_lines = lines.clone();
        new_lines[4] = "// edited #tag here".to_string();
        v.update(&new_lines, (4, 19), rect(40), 2, None);

        // Post-condition: still no Label elements in the fence interior.
        for row in 3..5 {
            let has_label = v.parsed_buffer.lines[row].elements.iter().any(|e| {
                matches!(e.kind, ElementKind::Label)
            });
            assert!(!has_label, "row {row} should still have no Label after incremental edit");
        }
        full_rebuild_equals_view_state(&v, &new_lines);
    }

    #[test]
    fn g8a_typing_into_empty_buffer() {
        let mut v = MarkdownEditorView::new();
        let empty = vec!["".to_string()];
        v.update(&empty, (0, 0), rect(40), 1, None);

        let one = vec!["h".to_string()];
        v.update(&one, (0, 1), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &one);

        let two = vec!["he".to_string()];
        v.update(&two, (0, 2), rect(40), 3, None);
        full_rebuild_equals_view_state(&v, &two);

        let many = vec!["hello world".to_string()];
        v.update(&many, (0, 11), rect(40), 4, None);
        full_rebuild_equals_view_state(&v, &many);
    }

    #[test]
    fn g8b_delete_last_char_one_line_buffer() {
        let mut v = MarkdownEditorView::new();
        let one = vec!["h".to_string()];
        v.update(&one, (0, 1), rect(40), 1, None);

        let empty = vec!["".to_string()];
        v.update(&empty, (0, 0), rect(40), 2, None);
        full_rebuild_equals_view_state(&v, &empty);
    }

    #[test]
    fn incremental_text_change_produces_same_layout_as_full_recompute() {
        let mut v = MarkdownEditorView::new();
        let lines: Vec<String> = (0..200)
            .map(|i| format!("paragraph {i} with some text that may wrap depending on width"))
            .collect();
        v.update(&lines, (100, 0), rect(40), 1, None);
        let baseline_visual_lines = v.layout.visual_lines().to_vec();

        // Edit a paragraph mid-buffer (no line count change).
        let mut edited = lines.clone();
        edited[100].push_str(" extra text");
        v.update(&edited, (100, edited[100].len()), rect(40), 2, None);

        // After incremental wrap, layout must equal a fresh compute of the edited buffer.
        let fresh_layout = WordWrapLayout::compute(
            &edited,
            40,
            v.rendered_cache_for_testing(),
        );

        let actual = v.layout.visual_lines();
        let fresh = fresh_layout.visual_lines();
        assert_eq!(actual.len(), fresh.len(), "visual_lines count diverges");
        for (i, (a, f)) in actual.iter().zip(fresh.iter()).enumerate() {
            assert_eq!(a, f, "visual line {i} diverges");
        }

        // Sanity: a row outside the edit should have unchanged visual lines.
        let row_50_before = baseline_visual_lines
            .iter()
            .filter(|vl| vl.logical_row == 50)
            .count();
        let row_50_after = v
            .layout
            .visual_lines()
            .iter()
            .filter(|vl| vl.logical_row == 50)
            .count();
        assert_eq!(
            row_50_before, row_50_after,
            "row 50 visual_lines count should be unchanged"
        );

        assert!(v.last_parse_was_incremental, "expected incremental path");
    }

    #[test]
    fn incremental_text_change_does_not_rebuild_all_of_rendered_cache() {
        // Verify that after an incremental text edit, rendered_cache rows
        // outside the widened range are NOT re-derived from scratch. We
        // can't directly observe the rebuild, but we CAN verify the cache
        // contents stay correct (matching a full rebuild's output).
        let mut v = MarkdownEditorView::new();
        let lines: Vec<String> = (0..200).map(|i| format!("paragraph {i} with some text")).collect();
        v.update(&lines, (100, 0), rect(40), 1, None);

        // Snapshot rendered_cache before the edit.
        let before: Vec<Vec<bool>> = v.rendered_cache.iter()
            .enumerate()
            .filter(|(i, _)| *i < 50 || *i > 150)
            .map(|(_, v)| v.clone())
            .collect();

        // Edit a paragraph in the middle.
        let mut edited = lines.clone();
        edited[100].push('x');
        v.update(&edited, (100, edited[100].len()), rect(40), 2, None);

        // Rows far outside the damaged range must be byte-identical.
        let after: Vec<Vec<bool>> = v.rendered_cache.iter()
            .enumerate()
            .filter(|(i, _)| *i < 50 || *i > 150)
            .map(|(_, v)| v.clone())
            .collect();
        assert_eq!(before, after, "rendered_cache rows outside damaged range must be unchanged");

        // The incremental path must have been taken.
        assert!(v.last_parse_was_incremental);
    }
}
