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

    // Slow path: longest common prefix + suffix. O(buffer_len)
    // String equalities; each compare is a length check + at most one
    // SIMD memcmp on the first-differing byte. ~14µs on a 5000-line
    // buffer for a single-row backspace.
    //
    // A cursor-anchored bound was explored as perf #12 and rejected:
    //  - Capping the scan at `cursor_row + slack` saves nothing,
    //    because the scan naturally stops at the first-differing
    //    row, which IS `cursor_row` for keystroke-driven edits.
    //  - Starting the LCP scan at `cursor_row - slack` (trusting
    //    rows above to be unchanged) would skip the prefix scan but
    //    introduces silent miscompilation risk on edits whose actual
    //    diff is far from the cursor (paste, undo, programmatic
    //    edit) — the post-slice verify only checks rows WITHIN the
    //    widened range, so a misidentified damage range outside
    //    that range is not caught.
    //  - Maintaining per-row hashes alongside `lines_snapshot` would
    //    let us replace string compares with u64 compares, but
    //    requires plumbing damage hints from the editor's edit
    //    surface to view.update for incremental hash maintenance —
    //    bigger change than the 10µs win justifies.
    //
    // Until per-row hashes ship as part of a broader edit-surface
    // refactor, the full O(buffer) scan stays.
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

/// Walk upward from `damaged_start` (the first damaged row) until the
/// row just above is a safe boundary. Returns the new start row
/// (inclusive).
///
/// `ListMarker` and `ListContinuation` are non-safe, so the walk
/// passes through them automatically — landing on the safe row above
/// the outermost list (Blank, or Plain that is not a continuation),
/// which is the G1-required outermost-list-ancestor stopping point.
fn widen_up(kinds: &[LineConstructKind], damaged_start: usize) -> usize {
    let mut row = damaged_start;
    while row > 0 {
        let candidate = row - 1;
        if is_safe_boundary(kinds[candidate]) {
            return candidate;
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

/// Expand `damaged` to the nearest reset boundaries on each side.
/// A reset boundary is a row where pulldown-cmark's parser state is
/// provably reset (see `ParsedBuffer::reset_boundaries`), so the
/// returned range is provably equivalent to a fresh parse over the
/// same slice — no post-slice verification needed in release.
///
/// `boundaries` must be sorted and contain `0` and `lines_len` as
/// sentinels (every `ParsedBuffer::parse` ensures this). Returns
/// `FullRebuild` if the expanded range trips either cap (same
/// semantics as `widen_to_safe`).
///
/// This replaces the heuristic `widen_to_safe`-plus-structural-marker
/// guard tower. The latter is kept available as a behavioural
/// comparison source for one release cycle (per the openspec
/// migration plan) before being deleted.
pub fn expand_to_reset_boundary(
    boundaries: &[usize],
    lines_len: usize,
    damaged: Range<usize>,
) -> WidenResult {
    if lines_len == 0 {
        return WidenResult::FullRebuild;
    }
    debug_assert!(
        damaged.start <= lines_len && damaged.end <= lines_len,
        "expand_to_reset_boundary: damaged range {:?} out of bounds for lines_len = {}",
        damaged,
        lines_len,
    );

    // Greatest boundary <= damaged.start.
    let start = boundaries
        .iter()
        .rev()
        .find(|&&b| b <= damaged.start)
        .copied()
        .unwrap_or(0);
    // Least boundary >= damaged.end. Sentinel `lines_len` is always
    // present in a well-formed boundary set so the `unwrap_or` is
    // unreachable; kept as a defensive fallback to avoid an inverted
    // range if the invariant is ever violated.
    let end = boundaries
        .iter()
        .find(|&&b| b >= damaged.end)
        .copied()
        .unwrap_or(lines_len);

    let widened_len = end - start;
    let cap_abs = MAX_INCREMENTAL_LINES;
    // Same cap policy as widen_to_safe; see its docstring for the
    // rationale on flooring `cap_frac` at `cap_abs`.
    let cap_frac = (((lines_len as f32) * MAX_INCREMENTAL_FRACTION) as usize).max(cap_abs);
    if widened_len > cap_abs || widened_len > cap_frac {
        return WidenResult::FullRebuild;
    }
    WidenResult::Widened(start..end)
}

/// Widen `damaged` outward to safe construct boundaries, applying
/// D5's +1 extra row and the D4 cap.
///
/// Returns `Widened(range)` when the widened range fits under the cap,
/// or `FullRebuild` when the cap is exceeded or the buffer is empty.
///
/// Kept available for one release cycle as a behavioural comparison
/// source against `expand_to_reset_boundary` (see openspec change
/// `parse-reset-boundaries`). New call sites should use
/// `expand_to_reset_boundary` instead.
pub fn widen_to_safe(kinds: &[LineConstructKind], damaged: Range<usize>) -> WidenResult {
    if kinds.is_empty() {
        return WidenResult::FullRebuild;
    }
    debug_assert!(
        damaged.start <= kinds.len() && damaged.end <= kinds.len(),
        "widen_to_safe: damaged range {:?} out of bounds for kinds.len() = {}",
        damaged,
        kinds.len(),
    );

    let mut start = widen_up(kinds, damaged.start);
    let mut end = widen_down(kinds, damaged.end);

    // D5: widen one extra row on each side.
    start = start.saturating_sub(1);
    end = (end + 1).min(kinds.len());

    let widened_len = end - start;
    let cap_abs = MAX_INCREMENTAL_LINES;
    // Fractional cap encodes the empirical "fresh full parse beats
    // parse+splice" cross-over. It is only meaningful once full-parse
    // cost is non-trivial; floor it at `cap_abs` so a 50%-widening on
    // a tiny buffer (where both options are sub-millisecond) stays on
    // the incremental path. Above `2 * cap_abs` lines the fractional
    // cap dominates and catches large widenings the absolute cap
    // would otherwise miss — this is the regime the previous `&&`
    // operator left unguarded.
    let cap_frac = (((kinds.len() as f32) * MAX_INCREMENTAL_FRACTION) as usize).max(cap_abs);
    if widened_len > cap_abs || widened_len > cap_frac {
        return WidenResult::FullRebuild;
    }

    WidenResult::Widened(start..end)
}

/// Derive fence-range half-open intervals from the per-line construct
/// kinds. The view layer uses these to decide which logical rows
/// render `force_raw` (no markdown re-styling, code-block fg color).
///
/// Half-open: a fence spanning rows `start..=end_inclusive` (both markers
/// included) is returned as `start..end_inclusive + 1`. An unclosed
/// fence runs to the end of the buffer.
pub fn fence_ranges_from_kinds(kinds: &[LineConstructKind]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < kinds.len() {
        if kinds[i] == LineConstructKind::FenceMarker {
            let start = i;
            i += 1;
            while i < kinds.len() && kinds[i] == LineConstructKind::FenceContent {
                i += 1;
            }
            if i < kinds.len() && kinds[i] == LineConstructKind::FenceMarker {
                ranges.push(start..i + 1);
                i += 1;
            } else {
                // Unclosed fence — extends to end of buffer.
                ranges.push(start..kinds.len());
            }
        } else {
            i += 1;
        }
    }
    ranges
}

/// Line ranges of every code block (fenced AND indented) in the buffer,
/// in ascending order. Reuses [`fence_ranges_from_kinds`] for fenced blocks
/// (incl. unclosed-fence handling) and adds maximal `IndentedCode` runs.
/// Used by the view to paint the code-box background.
pub fn code_block_ranges_from_kinds(kinds: &[LineConstructKind]) -> Vec<Range<usize>> {
    let mut ranges = fence_ranges_from_kinds(kinds);
    let mut i = 0;
    while i < kinds.len() {
        if kinds[i] == LineConstructKind::IndentedCode {
            let start = i;
            while i < kinds.len() && kinds[i] == LineConstructKind::IndentedCode {
                i += 1;
            }
            ranges.push(start..i);
        } else {
            i += 1;
        }
    }
    // Fenced ranges are collected first then indented ones appended; sort so the
    // combined list is ascending. Fenced and indented spans never overlap.
    ranges.sort_by_key(|r| r.start);
    ranges
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
        assert_eq!(
            k,
            vec![LineConstructKind::Plain, LineConstructKind::SetextUnderline]
        );
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
            vec![
                LineConstructKind::ListMarker,
                LineConstructKind::ListContinuation
            ]
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

    #[test]
    fn inline_html_inside_paragraph_does_not_become_html_block() {
        // Regression: `Event::InlineHtml` previously painted the
        // paragraph row as HtmlBlock, defeating safe-boundary widening
        // for any paragraph containing inline HTML like `<br>` or
        // `<span>`.
        let k = kinds_of(&["hello <br> world"]);
        assert_eq!(
            k[0],
            LineConstructKind::Plain,
            "paragraph with inline HTML must stay Plain"
        );
        let k = kinds_of(&["see <span>x</span> end"]);
        assert_eq!(k[0], LineConstructKind::Plain);
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
        s.chars()
            .map(|c| match c {
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
            })
            .collect()
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
                assert!(
                    r.start <= 2,
                    "must include opening fence marker at row 2, got start {}",
                    r.start
                );
                assert!(
                    r.end >= 7,
                    "must include closing fence marker at row 6 (end >= 7), got end {}",
                    r.end
                );
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
            WidenResult::Widened(r) => {
                assert_eq!(r.start, 0, "must include row above setext underline")
            }
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
                assert!(
                    r.start <= 1,
                    "must include first HtmlBlock row, got start {}",
                    r.start
                );
                assert!(
                    r.end >= 4,
                    "must include last HtmlBlock row, got end {}",
                    r.end
                );
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
    fn widen_trips_when_fractional_cap_exceeds_absolute() {
        // Regression: cap-trip used `&&` instead of `||`, so on a buffer
        // big enough that `cap_frac > cap_abs` (kinds.len() > 512), a
        // widened range between the two thresholds slipped through.
        // 600-line buffer of FenceContent → cap_abs=256, cap_frac=300.
        // Widening covers the whole buffer (no safe boundaries), so
        // widened_len=600 must trip the fallback.
        let k = vec![LineConstructKind::FenceContent; 600];
        assert_eq!(widen_to_safe(&k, 300..301), WidenResult::FullRebuild);
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
    fn parse_records_boundaries_for_blank_separated_paragraphs() {
        // Realistic markdown layout: each paragraph followed by a
        // blank line. Pulldown ends each Paragraph; depth drops to
        // 0 at the following blank row. The boundary set should
        // contain every blank row.
        use super::super::markdown::ParsedBuffer;
        let mut lines: Vec<String> = Vec::with_capacity(8);
        for i in 0..4 {
            lines.push(format!("paragraph {i}"));
            lines.push(String::new());
        }
        let pb = ParsedBuffer::parse(&lines);
        // Expected: 0, then every Blank row (1, 3, 5, 7), then lines.len() (8).
        // The blank at row 7 == lines.len()-1 may or may not be
        // present depending on whether depth==0 was reached at that
        // row; check the interior at least.
        assert!(pb.reset_boundaries.contains(&0), "sentinel 0 missing");
        assert!(
            pb.reset_boundaries.contains(&lines.len()),
            "sentinel lines.len() missing"
        );
        assert!(
            pb.reset_boundaries.contains(&1),
            "blank after paragraph 0 should be a boundary, got {:?}",
            pb.reset_boundaries
        );
        assert!(
            pb.reset_boundaries.contains(&3),
            "blank after paragraph 1 should be a boundary, got {:?}",
            pb.reset_boundaries
        );
    }

    #[test]
    fn expand_to_reset_uses_nearest_sentinels() {
        // Only sentinels [0, 5] in the boundary set — every edit
        // expands to the full buffer.
        let boundaries = vec![0, 5];
        match expand_to_reset_boundary(&boundaries, 5, 2..3) {
            WidenResult::Widened(r) => assert_eq!(r, 0..5),
            x => panic!("expected Widened, got {x:?}"),
        }
    }

    #[test]
    fn expand_to_reset_snaps_to_interior_boundaries() {
        // Boundaries at rows 0, 3, 6, 10 (e.g. blank-separated
        // blocks). Damage at row 4 expands to 3..6.
        let boundaries = vec![0, 3, 6, 10];
        match expand_to_reset_boundary(&boundaries, 10, 4..5) {
            WidenResult::Widened(r) => assert_eq!(r, 3..6),
            x => panic!("expected Widened, got {x:?}"),
        }
    }

    #[test]
    fn expand_to_reset_damage_at_exact_boundary_is_zero_span() {
        // Damage range coincides with a boundary point. The function
        // returns the smallest enclosing boundary pair.
        let boundaries = vec![0, 3, 6, 10];
        // damaged.start == damaged.end == 6. Expands to 6..6 (empty).
        match expand_to_reset_boundary(&boundaries, 10, 6..6) {
            WidenResult::Widened(r) => assert_eq!(r, 6..6),
            x => panic!("expected Widened, got {x:?}"),
        }
    }

    #[test]
    fn expand_to_reset_empty_buffer_falls_back() {
        let boundaries = vec![0];
        assert_eq!(
            expand_to_reset_boundary(&boundaries, 0, 0..0),
            WidenResult::FullRebuild
        );
    }

    #[test]
    fn expand_to_reset_caps_trip_fallback() {
        // 600-row buffer, no interior boundaries. Damage at 300
        // expands to 0..600 which exceeds cap_abs (256) and cap_frac
        // (300, floored at cap_abs).
        let boundaries = vec![0, 600];
        assert_eq!(
            expand_to_reset_boundary(&boundaries, 600, 300..301),
            WidenResult::FullRebuild
        );
    }

    #[test]
    fn widen_blockquote_includes_whole_block() {
        // P Q Q Q B P — damage in the middle of a blockquote → widen
        // to include the whole blockquote.
        let k = kinds_str("PQQQBP");
        match widen_to_safe(&k, 2..3) {
            WidenResult::Widened(r) => {
                assert!(
                    r.start <= 1,
                    "must include first Blockquote row, got start {}",
                    r.start
                );
                assert!(
                    r.end >= 4,
                    "must include last Blockquote row, got end {}",
                    r.end
                );
            }
            x => panic!("expected Widened, got {x:?}"),
        }
    }

    #[test]
    fn widen_multi_list_does_not_over_pull_across_blank() {
        // Two independent lists separated by a blank line. Damage in
        // the second list must not pull the first list into the slice.
        let k = kinds_str("LlBLll");
        match widen_to_safe(&k, 4..5) {
            WidenResult::Widened(r) => {
                // The blank at row 2 is the separator. Widening must
                // stop there (or at the row above, after D5 +1).
                assert!(
                    r.start >= 1,
                    "widen.start must be >= 1 (D5 may pull past Blank by one row), got {}",
                    r.start
                );
                assert!(
                    r.start <= 2,
                    "widen.start must not pull in list A, got {}",
                    r.start
                );
            }
            x => panic!("expected Widened, got {x:?}"),
        }
    }

    #[test]
    fn fence_ranges_single_fence() {
        // P F C C F P — fence covers rows 1..5 (half-open: both markers + content).
        let k = kinds_str("PFCCFP");
        let r = fence_ranges_from_kinds(&k);
        assert_eq!(r, vec![1..5]);
    }

    #[test]
    fn fence_ranges_two_fences() {
        // F C F P F C F — two fences at 0..3 and 4..7.
        let k = kinds_str("FCFPFCF");
        let r = fence_ranges_from_kinds(&k);
        assert_eq!(r, vec![0..3, 4..7]);
    }

    #[test]
    fn fence_ranges_unclosed_extends_to_end() {
        // P F C C C — unclosed fence runs to end of buffer.
        let k = kinds_str("PFCCC");
        let r = fence_ranges_from_kinds(&k);
        assert_eq!(r, vec![1..5]);
    }

    #[test]
    fn fence_ranges_empty() {
        assert!(fence_ranges_from_kinds(&[]).is_empty());
    }

    #[test]
    fn code_block_ranges_covers_fenced_and_indented() {
        // Fenced block then a blank then an indented code block.
        let k = kinds_of(&[
            "```",          // FenceMarker
            "let x = 1;",   // FenceContent
            "```",          // FenceMarker
            "",             // Blank
            "    indented", // IndentedCode
            "    code",     // IndentedCode
        ]);
        let r = code_block_ranges_from_kinds(&k);
        assert_eq!(r, vec![0..3, 4..6]);
    }

    #[test]
    fn investigate_list_fence_indented_code_interaction() {
        // Initial: row 7 "    a" is after "- a" (row 1) with 5 blank lines in between.
        // After editing row 9 (blank → space inside fence), fresh parse changes row 7.
        let initial: Vec<String> = vec![
            "".to_string(),      // 0: Blank
            "- a".to_string(),   // 1: ListMarker
            "".to_string(),      // 2: Blank
            "".to_string(),      // 3: Blank
            "".to_string(),      // 4: Blank
            "".to_string(),      // 5: Blank
            "".to_string(),      // 6: Blank
            "    a".to_string(), // 7: ? - before fence
            "```".to_string(),   // 8: FenceMarker
            "".to_string(),      // 9: FenceContent -> edit to " "
            "".to_string(),      // 10: FenceContent
            "".to_string(),      // 11: FenceContent
            "".to_string(),      // 12: FenceContent
            "".to_string(),      // 13: FenceContent
            "".to_string(),      // 14: FenceContent
            "".to_string(),      // 15: FenceContent
            "".to_string(),      // 16: FenceContent
            "> a".to_string(),   // 17: FenceContent
            "".to_string(),      // 18: FenceContent
            ">  ".to_string(),   // 19: FenceContent
            "".to_string(),      // 20: FenceContent
            "".to_string(),      // 21: FenceContent
            "".to_string(),      // 22: FenceContent (last row → FenceMarker?)
        ];
        let initial_pb = ParsedBuffer::parse(&initial);
        eprintln!("initial kinds: {:?}", initial_pb.kinds);

        let mut edited = initial.clone();
        edited[9].push(' ');
        let edited_pb = ParsedBuffer::parse(&edited);
        eprintln!("edited  kinds: {:?}", edited_pb.kinds);

        // Compare just the first 10 rows to see where divergence starts
        for i in 0..23 {
            if initial_pb.kinds[i] != edited_pb.kinds[i] {
                eprintln!(
                    "Row {} differs: initial={:?}, edited={:?}",
                    i, initial_pb.kinds[i], edited_pb.kinds[i]
                );
            }
        }
    }
}
