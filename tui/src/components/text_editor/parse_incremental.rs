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
}
