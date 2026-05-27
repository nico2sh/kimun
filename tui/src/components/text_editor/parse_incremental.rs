#![allow(dead_code)]
//! Incremental-parse machinery: line-construct classification cache,
//! damage-diff against the previous buffer snapshot, safe-boundary
//! widening, and fence-range derivation. Pure functions only — no
//! `pulldown_cmark` calls (those live in `markdown.rs`).

use std::ops::Range;

/// Coarse classification of a buffer line for safe-boundary widening.
///
/// A line is a *safe boundary* when re-parsing a slice ending on that
/// line is equivalent to the corresponding slice of a full-buffer parse.
/// `Blank` and `Plain` are unconditional boundaries when their neighbour
/// is also `Blank`/`Plain` or end-of-buffer. Structural markers
/// (`FenceMarker`, `ListMarker`, etc.) are NEVER boundaries — widening
/// must reach the outer terminator of whatever construct they belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineConstructKind {
    Blank,
    Plain,
    FenceMarker,
    FenceContent,
    IndentedCode,
    ListMarker,
    ListContinuation,
    Blockquote(u8),
    SetextUnderline,
    HtmlBlock,
    Heading,
}

/// Result of widening a damaged range to safe construct boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WidenResult {
    /// Widened range; caller passes this to `ParsedBuffer::parse_range`.
    Widened(Range<usize>),
    /// Range cannot be cheaply widened (cap trip, unbounded construct).
    /// Caller falls back to `ParsedBuffer::parse(lines)`.
    FullRebuild,
}

/// Maximum fraction of buffer the widened range may cover before we
/// abandon incremental and fall back to a full parse. Half the buffer
/// is the empirical cross-over where parse+splice overhead exceeds a
/// fresh full parse on the same input.
pub(super) const MAX_INCREMENTAL_FRACTION: f32 = 0.5;

/// Absolute cap on the widened range. Independent of buffer size; keeps
/// large-fence edits bounded even on small buffers.
pub(super) const MAX_INCREMENTAL_LINES: usize = 256;

/// Cursor-row hint scan window for `compute_damage_range`. Empirically
/// covers single-character edits, IME composition of up to 3 graphemes,
/// and one Enter at line end. Multi-line pastes intentionally fall
/// through to the LCP/LCS slow path.
pub(super) const CURSOR_HINT_WINDOW: usize = 4;

/// Compute the row range that differs between `old` and `new`, with a
/// cursor-row hint to accelerate the common single-character-edit case.
///
/// **Contract:** `cursor_row` must be the row that was actually edited
/// (the editor's cursor position after the keystroke). The fast path
/// trusts this — if `cursor_row` does not identify the real edit point,
/// the function may under-report the damaged range for an edit shape
/// that single-keystroke editing cannot produce. Distant simultaneous
/// edits are out of scope; they can only happen via programmatic
/// buffer replacement, which goes through `set_text` and bumps
/// `text_revision` such that the LCP/LCS slow path is taken naturally
/// (the cursor row's content will match between old and new, so the
/// fast path declines and the slow path runs).
///
/// Returns `None` when the buffers are byte-identical (defensive
/// guard — callers should already have gated on `text_revision`).
///
/// Fast path: same line count, the row at `cursor_row` differs, and
/// no other line in `±CURSOR_HINT_WINDOW` differs. Returns
/// `Some(cursor_row..cursor_row + 1)`. O(`CURSOR_HINT_WINDOW`).
///
/// Slow path: longest common prefix (LCP) and longest common suffix
/// (LCS); damaged range is the middle slice. O(min(buffer_size,
/// damage_size)).
pub fn compute_damage_range(
    old: &[String],
    new: &[String],
    cursor_row: usize,
) -> Option<Range<usize>> {
    if old == new {
        return None;
    }

    // Fast path: same line count, cursor row differs, no other diff in window.
    if old.len() == new.len() && cursor_row < old.len() && old[cursor_row] != new[cursor_row] {
        let lo = cursor_row.saturating_sub(CURSOR_HINT_WINDOW);
        let hi = (cursor_row + CURSOR_HINT_WINDOW + 1).min(old.len());
        let other_diff_in_window = (lo..hi).any(|i| i != cursor_row && old[i] != new[i]);
        if !other_diff_in_window {
            return Some(cursor_row..cursor_row + 1);
        }
    }

    // Slow path: longest common prefix + suffix.
    let lcp = old
        .iter()
        .zip(new.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let lcs = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    // Guard against overlap when both buffers share a long common stretch.
    // Clamp lcs so the resulting range is non-empty and start <= end.
    let new_end = new.len().saturating_sub(lcs);
    let old_end = old.len().saturating_sub(lcs);
    let start = lcp.min(new_end).min(old_end);
    let end = new_end.max(start);
    Some(start..end)
}

/// Return true when `kind` is a self-contained, safe boundary line.
/// Blank lines and ordinary paragraph lines are safe; everything else
/// belongs to a multi-line construct that widening must include in
/// full.
fn is_safe_boundary(kind: LineConstructKind) -> bool {
    matches!(kind, LineConstructKind::Blank | LineConstructKind::Plain)
}

/// Walk upward from `damaged.start` (the first damaged row) until the
/// row just above is a safe boundary AND we are not still inside a
/// list. Returns the new start row (inclusive).
///
/// The list rule (G1): if any row we passed (or `damaged.start - 1`
/// itself) is `ListMarker` or `ListContinuation`, we are inside a list
/// — keep walking up until we find a `Plain`/`Blank` row that is NOT a
/// ListContinuation. This guarantees the parse_range slice includes
/// the outermost col-0 list marker, so the synthetic-list-parent trick
/// in `ParsedLine::parse` is not needed.
fn widen_up(kinds: &[LineConstructKind], damaged_start: usize) -> usize {
    if damaged_start == 0 {
        return 0;
    }
    let mut row = damaged_start;
    let mut in_list = false;
    while row > 0 {
        let candidate = row - 1;
        let k = kinds[candidate];
        // Track list state — once entered, we stay in_list until we
        // see a row that is NEITHER ListMarker NOR ListContinuation
        // (and is a safe boundary).
        if matches!(k, LineConstructKind::ListMarker | LineConstructKind::ListContinuation) {
            in_list = true;
        }
        if is_safe_boundary(k) && !in_list {
            return candidate;
        }
        if is_safe_boundary(k) && in_list {
            // Found a safe boundary but we were in a list — keep going
            // up past the list marker / continuation rows. Reset
            // in_list and continue.
            in_list = false;
            // Don't return yet — continue walking up.
        }
        row = candidate;
    }
    0
}

/// Walk downward from `damaged.end` (the first row past the damage)
/// until we land on a safe boundary or end of buffer. Returns the
/// exclusive end index.
fn widen_down(kinds: &[LineConstructKind], damaged_end: usize) -> usize {
    let mut row = damaged_end;
    while row < kinds.len() {
        if is_safe_boundary(kinds[row]) {
            return row + 1;
        }
        row += 1;
    }
    kinds.len()
}

/// Widen `damaged` outward to safe construct boundaries, applying
/// D5's +1 extra row and the D4 cap.
///
/// Returns `Widened(range)` when the widened range fits under the cap,
/// or `FullRebuild` when the cap is exceeded or the buffer is empty.
pub fn widen_to_safe(kinds: &[LineConstructKind], damaged: Range<usize>) -> WidenResult {
    if kinds.is_empty() {
        return WidenResult::FullRebuild;
    }

    let mut start = widen_up(kinds, damaged.start);
    let mut end = widen_down(kinds, damaged.end);

    // D5: widen one extra row on each side.
    start = start.saturating_sub(1);
    end = (end + 1).min(kinds.len());

    let widened_len = end - start;
    let cap_abs = MAX_INCREMENTAL_LINES;
    let cap_frac = ((kinds.len() as f32) * MAX_INCREMENTAL_FRACTION) as usize;
    if widened_len > cap_abs && widened_len > cap_frac {
        return WidenResult::FullRebuild;
    }

    WidenResult::Widened(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::text_editor::markdown::ParsedBuffer;

    fn kinds_of(lines: &[&str]) -> Vec<LineConstructKind> {
        let owned: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        ParsedBuffer::parse(&owned).kinds
    }

    #[test]
    fn plain_paragraph() {
        assert_eq!(kinds_of(&["hello world"]), vec![LineConstructKind::Plain]);
    }

    #[test]
    fn blank_line() {
        assert_eq!(kinds_of(&[""]), vec![LineConstructKind::Blank]);
    }

    #[test]
    fn atx_heading() {
        assert_eq!(kinds_of(&["# title"]), vec![LineConstructKind::Heading]);
    }

    #[test]
    fn setext_underline_above_is_plain() {
        let k = kinds_of(&["title", "====="]);
        assert_eq!(k, vec![LineConstructKind::Plain, LineConstructKind::SetextUnderline]);
    }

    #[test]
    fn fence_pair() {
        let k = kinds_of(&["```rust", "let x = 1;", "```"]);
        assert_eq!(
            k,
            vec![
                LineConstructKind::FenceMarker,
                LineConstructKind::FenceContent,
                LineConstructKind::FenceMarker,
            ]
        );
    }

    #[test]
    fn list_marker_and_continuation() {
        let k = kinds_of(&["- item", "  continuation"]);
        assert_eq!(
            k,
            vec![LineConstructKind::ListMarker, LineConstructKind::ListContinuation]
        );
    }

    #[test]
    fn blockquote_levels() {
        let k = kinds_of(&[">> two"]);
        assert_eq!(k, vec![LineConstructKind::Blockquote(2)]);
    }

    #[test]
    fn indented_code() {
        let k = kinds_of(&["", "    let x = 1;"]);
        assert_eq!(k[1], LineConstructKind::IndentedCode);
    }

    #[test]
    fn html_block() {
        let k = kinds_of(&["<div>", "body", "</div>"]);
        assert!(matches!(k[0], LineConstructKind::HtmlBlock));
    }

    fn lines(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn damage_single_char_insert_uses_cursor_hint() {
        let old = lines(&["hello", "world"]);
        let new = lines(&["hello", "worldx"]);
        assert_eq!(compute_damage_range(&old, &new, 1), Some(1..2));
    }

    #[test]
    fn damage_no_change_returns_none() {
        let old = lines(&["a", "b"]);
        assert_eq!(compute_damage_range(&old, &old, 0), None);
    }

    #[test]
    fn damage_enter_at_line_end_uses_lcp_lcs() {
        let old = lines(&["alpha", "beta"]);
        let new = lines(&["alpha", "be", "ta"]);
        let dmg = compute_damage_range(&old, &new, 1).unwrap();
        assert_eq!(dmg.start, 1);
        assert_eq!(dmg.end, new.len()); // damaged = [1..3)
    }

    #[test]
    fn damage_backspace_merging_lines() {
        let old = lines(&["alpha", "beta", "gamma"]);
        let new = lines(&["alphabeta", "gamma"]);
        let dmg = compute_damage_range(&old, &new, 0).unwrap();
        assert_eq!(dmg.start, 0);
    }

    #[test]
    fn damage_multi_diff_within_window_falls_through_to_slow_path() {
        // Two rows differ, both within CURSOR_HINT_WINDOW of the cursor.
        // Fast path's other-diff-in-window check trips → LCP/LCS slow path.
        let old = lines(&["a", "b", "c", "d", "e"]);
        let mut new = old.clone();
        new[1] = "B".to_string();
        new[2] = "C".to_string();
        // Cursor at row 1; the window covers rows 0..=4 (full buffer here).
        let dmg = compute_damage_range(&old, &new, 1).unwrap();
        // Slow path: LCP=1, LCS=2 → 1..3
        assert_eq!(dmg, 1..3);
    }

    fn kinds_str(s: &str) -> Vec<LineConstructKind> {
        // Compact spec: one char per line.
        // P=Plain, B=Blank, F=FenceMarker, C=FenceContent,
        // L=ListMarker, l=ListContinuation, Q=Blockquote(1),
        // S=SetextUnderline, H=Heading, I=IndentedCode, X=HtmlBlock.
        s.chars().map(|c| match c {
            'P' => LineConstructKind::Plain,
            'B' => LineConstructKind::Blank,
            'F' => LineConstructKind::FenceMarker,
            'C' => LineConstructKind::FenceContent,
            'L' => LineConstructKind::ListMarker,
            'l' => LineConstructKind::ListContinuation,
            'Q' => LineConstructKind::Blockquote(1),
            'S' => LineConstructKind::SetextUnderline,
            'H' => LineConstructKind::Heading,
            'I' => LineConstructKind::IndentedCode,
            'X' => LineConstructKind::HtmlBlock,
            _ => panic!("bad kind char {c}"),
        }).collect()
    }

    #[test]
    fn widen_plain_paragraph_to_blank_boundaries() {
        // P B P P P B P — damage row 3 → widen to blank rows 1 and 5
        // (plus the D5 +1 each side: 0 and 6 — but the buffer ends are
        // also boundaries; clamp).
        let k = kinds_str("PBPPPBP");
        match widen_to_safe(&k, 3..4) {
            WidenResult::Widened(r) => {
                // Must include the blank rows at 1 and 5 (or wider).
                assert!(r.start <= 1, "widen.start <= 1, got {}", r.start);
                assert!(r.end >= 6, "widen.end >= 6, got {}", r.end);
            }
            x => panic!("expected Widened, got {x:?}"),
        }
    }

    #[test]
    fn widen_fence_interior_includes_both_markers() {
        // P B F C C C F B P — damage row 4 (inside fence) → widen
        // to include both fence markers + one extra line on each side.
        let k = kinds_str("PBFCCCFBP");
        match widen_to_safe(&k, 4..5) {
            WidenResult::Widened(r) => {
                assert!(r.start <= 2, "must include opening fence marker at row 2, got start {}", r.start);
                assert!(r.end >= 7, "must include closing fence marker at row 6 (end >= 7), got end {}", r.end);
            }
            x => panic!("expected Widened, got {x:?}"),
        }
    }

    #[test]
    fn widen_list_continuation_reaches_outermost_marker() {
        // L l L l l l B P — damage at row 4 (nested continuation) → widen
        // up to outermost ListMarker at row 0.
        let k = kinds_str("LlLlllBP");
        match widen_to_safe(&k, 4..5) {
            WidenResult::Widened(r) => assert_eq!(r.start, 0, "must reach col-0 list marker"),
            x => panic!("expected Widened, got {x:?}"),
        }
    }

    #[test]
    fn widen_setext_underline_includes_text_line_above() {
        // P S P — damage at row 1 (underline) → widen to include row 0
        // (heading text line).
        let k = kinds_str("PSP");
        match widen_to_safe(&k, 1..2) {
            WidenResult::Widened(r) => assert_eq!(r.start, 0, "must include row above setext underline"),
            x => panic!("expected Widened, got {x:?}"),
        }
    }

    #[test]
    fn widen_html_block_includes_whole_block() {
        // P X X X B P — damage at row 2 (middle of HTML) → widen to
        // include all HtmlBlock rows.
        let k = kinds_str("PXXXBP");
        match widen_to_safe(&k, 2..3) {
            WidenResult::Widened(r) => {
                assert!(r.start <= 1, "must include first HtmlBlock row, got start {}", r.start);
                assert!(r.end >= 4, "must include last HtmlBlock row, got end {}", r.end);
            }
            x => panic!("expected Widened, got {x:?}"),
        }
    }

    #[test]
    fn widen_exceeds_cap_returns_full_rebuild() {
        // 300-line all-FenceContent buffer; the damage is one line;
        // widening tries to reach the fence ends but the buffer is
        // uniformly fence content, so widening goes to 0..300, which
        // exceeds MAX_INCREMENTAL_LINES (256).
        let k = vec![LineConstructKind::FenceContent; 300];
        assert_eq!(widen_to_safe(&k, 150..151), WidenResult::FullRebuild);
    }

    #[test]
    fn widen_at_buffer_start_clamps_to_zero() {
        let k = kinds_str("PPPPP");
        match widen_to_safe(&k, 0..1) {
            WidenResult::Widened(r) => assert_eq!(r.start, 0),
            x => panic!("expected Widened, got {x:?}"),
        }
    }

    #[test]
    fn widen_at_buffer_end_clamps_to_len() {
        let k = kinds_str("PPPPP");
        match widen_to_safe(&k, 4..5) {
            WidenResult::Widened(r) => assert_eq!(r.end, 5),
            x => panic!("expected Widened, got {x:?}"),
        }
    }

    #[test]
    fn widen_blockquote_includes_whole_block() {
        // P Q Q Q B P — damage in the middle of a blockquote → widen
        // to include the whole blockquote.
        let k = kinds_str("PQQQBP");
        match widen_to_safe(&k, 2..3) {
            WidenResult::Widened(r) => {
                assert!(r.start <= 1, "must include first Blockquote row, got start {}", r.start);
                assert!(r.end >= 4, "must include last Blockquote row, got end {}", r.end);
            }
            x => panic!("expected Widened, got {x:?}"),
        }
    }
}
