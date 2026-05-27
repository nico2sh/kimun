//! Property test: for any small random buffer + single-char edit,
//! when try_incremental_parse takes the incremental splice path the
//! spliced parsed_buffer must equal a fresh ParsedBuffer::parse.

use kimun_notes::components::text_editor::markdown::ParsedBuffer;
use kimun_notes::components::text_editor::view::MarkdownEditorView;
use proptest::prelude::*;
use ratatui::layout::Rect;

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
        view.update(&initial, (target_row, 0), test_rect(), 1, None);

        // Gen 2: apply the single-char edit.
        let col_after = edited[target_row].chars().count();
        view.update(&edited, (target_row, col_after), test_rect(), 2, None);

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
}
