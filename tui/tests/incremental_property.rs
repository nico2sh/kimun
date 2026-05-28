//! Property test: for any small random buffer + single-char edit,
//! when try_incremental_parse takes the incremental splice path the
//! spliced parsed_buffer must equal a fresh ParsedBuffer::parse.

use kimun_notes::components::text_editor::markdown::ParsedBuffer;
use kimun_notes::components::text_editor::parse_incremental::{
    expand_to_reset_boundary, WidenResult,
};
use kimun_notes::components::text_editor::snapshot::EditorSnapshot;
use kimun_notes::components::text_editor::view::MarkdownEditorView;
use proptest::prelude::*;
use ratatui::layout::Rect;
use std::num::NonZeroU64;

fn snap_for<'a>(
    lines: &'a [String],
    cursor: (usize, usize),
    generation: u64,
) -> EditorSnapshot<'a> {
    let rev = NonZeroU64::new(generation.max(1)).unwrap();
    let clamped = if lines.is_empty() {
        (0, 0)
    } else {
        (cursor.0.min(lines.len() - 1), cursor.1)
    };
    EditorSnapshot::borrowed(lines, clamped, rev)
}

fn line_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z ]{0,30}".prop_map(|s| s),
        Just("".to_string()),
        "(- |\\* |\\+ )[a-z ]{1,20}".prop_map(|s| s),
        "(#|##|###) [a-z ]{1,15}".prop_map(|s| s),
        Just("```".to_string()),
        "> [a-z ]{1,20}".prop_map(|s| s),
    ]
}

fn buffer_strategy() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(line_strategy(), 1..=50)
}

fn test_rect() -> Rect {
    Rect { x: 0, y: 0, width: 80, height: 40 }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1000),
        .. ProptestConfig::default()
    })]

    #[test]
    fn incremental_matches_full_for_random_single_char_edit(
        initial in buffer_strategy(),
        row in 0usize..50,
        ch in any::<char>().prop_filter("ascii printable", |c| c.is_ascii() && !c.is_control()),
    ) {
        if initial.is_empty() {
            return Ok(());
        }
        let target_row = row % initial.len();
        let mut edited = initial.clone();
        edited[target_row].push(ch);

        // Drive try_incremental_parse through the real MarkdownEditorView
        // so the fallback guards in try_incremental_parse are all applied.
        let mut view = MarkdownEditorView::new();

        // Gen 1: populate with the initial buffer.
        view.update(&snap_for(&initial, (target_row, 0), 1), test_rect(), None);

        // Gen 2: apply the single-char edit.
        let col_after = edited[target_row].chars().count();
        view.update(&snap_for(&edited, (target_row, col_after), 2), test_rect(), None);

        // Only assert equality when the incremental path was actually taken.
        // When try_incremental_parse fell back (last_parse_was_incremental=false)
        // the view already contains a fresh full parse — no assertion needed.
        if !view.last_parse_was_incremental {
            return Ok(());
        }

        let fresh = ParsedBuffer::parse(&edited);
        prop_assert_eq!(view.parsed_buffer_kinds(), &fresh.kinds[..]);
        prop_assert_eq!(view.parsed_buffer_lines().len(), fresh.lines.len(),
            "spliced lines.len diverges from fresh");
        for (i, (g, e)) in view.parsed_buffer_lines().iter().zip(fresh.lines.iter()).enumerate() {
            prop_assert_eq!(&g.content_vis, &e.content_vis,
                "row {} content_vis diverges", i);
            prop_assert_eq!(g.elements.len(), e.elements.len(),
                "row {} elements.len diverges", i);
        }
    }

    /// Property: for any random buffer + single-char edit, the slice
    /// of a fresh full parse covering `expand_to_reset_boundary`'s
    /// range must match `parse_range` over the same row range. This
    /// is the reset-boundary invariant — if it ever fails, splicing
    /// the slice into the parent buffer would diverge from a fresh
    /// full parse.
    #[test]
    fn expand_to_reset_range_is_provably_equivalent_to_fresh_parse(
        initial in buffer_strategy(),
        row in 0usize..50,
        ch in any::<char>().prop_filter("ascii printable", |c| c.is_ascii() && !c.is_control()),
    ) {
        if initial.is_empty() {
            return Ok(());
        }
        let target_row = row % initial.len();
        let mut edited = initial.clone();
        edited[target_row].push(ch);
        // Same line count required for the splice path.
        if edited.len() != initial.len() {
            return Ok(());
        }

        // Build the post-edit boundaries by parsing `edited` fresh —
        // this is what splice would produce after applying the
        // incremental update.
        let fresh = ParsedBuffer::parse(&edited);
        let damaged = target_row..(target_row + 1);
        let widened = match expand_to_reset_boundary(
            &fresh.reset_boundaries,
            edited.len(),
            damaged,
        ) {
            WidenResult::Widened(r) => r,
            WidenResult::FullRebuild => return Ok(()),
        };

        // Slice parse vs fresh parse over the widened range MUST
        // produce identical `kinds` and per-line content_vis. If they
        // diverge, the boundary set is wrong (a reset boundary that
        // isn't actually a parser-state reset).
        let slice = ParsedBuffer::parse_range(&edited, widened.clone());
        for (offset, (slice_kind, fresh_kind)) in slice
            .kinds
            .iter()
            .zip(fresh.kinds[widened.clone()].iter())
            .enumerate()
        {
            prop_assert_eq!(
                slice_kind, fresh_kind,
                "kinds diverge at slice row {} (full row {}) for widened {:?}",
                offset, widened.start + offset, widened
            );
        }
        for (offset, (slice_line, fresh_line)) in slice
            .lines
            .iter()
            .zip(fresh.lines[widened.clone()].iter())
            .enumerate()
        {
            prop_assert_eq!(
                &slice_line.content_vis,
                &fresh_line.content_vis,
                "content_vis diverges at slice row {} (full row {})",
                offset, widened.start + offset
            );
            prop_assert_eq!(
                slice_line.elements.len(),
                fresh_line.elements.len(),
                "elements.len diverges at slice row {} (full row {})",
                offset, widened.start + offset
            );
        }
    }
}
