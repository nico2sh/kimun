//! An overlay must paint on the cells its text actually occupies.
//!
//! `editor_render_snapshots.rs` deliberately excludes overlays, and its corpus
//! holds no tab characters, so the logical-column → rendered-cell mapping had no
//! cell-level cover in either direction. The defect this pins lived in exactly
//! that intersection: a **blockquote bar** row whose text contains a tab, where
//! the mapper credited the hidden `> ` as the cells the bar draws. That credit is
//! exact for every cluster whose width is column-independent — a tab is not one,
//! so it measured to a nearer stop and every overlay at or after it landed short
//! by the bar's width.
//!
//! Each case paints a marker `Z` and sets an overlay on `Z`'s logical column.
//! The highlight must land on `Z`, whatever the **sigils**, the bar, or a tab did
//! to the columns in between.

use std::num::NonZeroU64;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

use kimun_notes::components::text_editor::snapshot::EditorSnapshot;
use kimun_notes::components::text_editor::view::{MarkdownEditorView, Overlay, OverlayKind};
use kimun_notes::settings::themes::Theme;

/// Where `Z` is painted — `(visual row, cell)` — and the cells carrying an
/// overlay background *on that row*. Scans the whole pane rather than the first
/// row, so a case that wraps is measured on the row it actually lands on.
fn z_cell_and_highlight(line: &str) -> (Option<(usize, usize)>, Vec<usize>) {
    let col = line.chars().position(|c| c == 'Z').expect("the marker");
    let lines = vec![line.to_string(), String::new()];
    let revision = NonZeroU64::new(1).expect("one is not zero");
    // Cursor parked off the row, so it renders styled rather than revealed.
    let snapshot = EditorSnapshot::borrowed(&lines, (1, 0), revision);
    let rect = Rect {
        x: 0,
        y: 0,
        width: 24,
        height: 10,
    };
    let theme = Theme::default();
    let mut view = MarkdownEditorView::new();
    view.update(&snapshot, rect);
    view.set_overlays(vec![Overlay::new(
        0,
        col,
        col + 1,
        OverlayKind::CurrentMatch,
    )]);

    let mut terminal = Terminal::new(TestBackend::new(rect.width, rect.height)).expect("backend");
    terminal
        .draw(|frame| view.render(frame, rect, &theme, false, None))
        .expect("draw");

    let buffer = terminal.backend().buffer();
    // A row past the end of the text carries the editor's base style.
    let base = buffer.cell((0u16, 9u16)).expect("blank row").bg;
    let mut z = None;
    for y in 0..rect.height {
        for x in 0..rect.width {
            if buffer.cell((x, y)).expect("inside the pane").symbol() == "Z" {
                z = Some((y as usize, x as usize));
            }
        }
    }
    let Some((row, _)) = z else {
        return (None, Vec::new());
    };
    let highlighted = (0..rect.width)
        .filter(|&x| buffer.cell((x, row as u16)).expect("inside").bg != base)
        .map(|x| x as usize)
        .collect();
    (z, highlighted)
}

fn assert_overlay_lands_on_z(line: &str) {
    let (z, highlighted) = z_cell_and_highlight(line);
    let (row, cell) = z.unwrap_or_else(|| panic!("{line:?}: the marker was never painted"));
    assert_eq!(
        highlighted,
        vec![cell],
        "{line:?}: the marker paints at visual row {row}, cell {cell}, \
         so the overlay belongs there"
    );
}

/// Asserts the case genuinely wraps, so a test meant to cover a continuation
/// row cannot quietly measure the first one.
fn assert_overlay_lands_on_z_on_a_continuation_row(line: &str) {
    let (z, _) = z_cell_and_highlight(line);
    let (row, _) = z.unwrap_or_else(|| panic!("{line:?}: the marker was never painted"));
    assert!(
        row > 0,
        "{line:?}: expected to wrap, but the marker is on row 0"
    );
    assert_overlay_lands_on_z(line);
}

#[test]
fn an_overlay_paints_on_the_cell_its_char_occupies() {
    for line in ["a Z", "> a Z", "- a Z", "## h Z"] {
        assert_overlay_lands_on_z(line);
    }
}

#[test]
fn a_tab_does_not_shift_an_overlay_off_its_char() {
    for line in ["a\tZ", "- a\tZ", "## h\tZ"] {
        assert_overlay_lands_on_z(line);
    }
}

/// The regression: bar plus tab, on the first visual line.
#[test]
fn a_blockquote_bar_and_a_tab_do_not_shift_an_overlay() {
    assert_overlay_lands_on_z("> a\tZ");
    assert_overlay_lands_on_z("> > a\tZ");
}

/// A continuation row takes the other branch — the bar is added as an offset and
/// no sigils are credited, since they are not on this row at all.
#[test]
fn a_tab_on_a_wrapped_blockquote_row_keeps_its_overlay() {
    assert_overlay_lands_on_z_on_a_continuation_row("> aaaa bbbb cccc dddd eeee ff\tZ");
}
