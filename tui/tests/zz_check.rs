//! Candidate regression test — an overlay must paint on the cells its text
//! actually occupies.
//!
//! The snapshot corpus deliberately excludes overlays, so the mapping from
//! logical columns to rendered cells has no cell-level cover. Every case here is
//! a row where a marker char `Z` is painted somewhere and an overlay is set on
//! `Z`'s logical column: the highlight must land on `Z`, whatever the sigils,
//! the blockquote bar, or a tab did to the column in between.

use std::num::NonZeroU64;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

use kimun_notes::components::text_editor::snapshot::EditorSnapshot;
use kimun_notes::components::text_editor::view::{MarkdownEditorView, Overlay, OverlayKind};
use kimun_notes::settings::themes::Theme;

/// Render `line` with an overlay over the logical column of its single `Z`, and
/// return `(cell of the painted Z, cells carrying the overlay's background)`.
fn painted_vs_highlighted(line: &str, vrow: u16) -> (Option<usize>, Vec<usize>) {
    let col = line.chars().position(|c| c == 'Z').expect("one Z");
    let lines: Vec<String> = vec![line.to_string(), String::new()];
    let revision = NonZeroU64::new(1).expect("nonzero");
    // Cursor parked off the row, so the row renders concealed, not revealed.
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
    let buf = terminal.backend().buffer();
    let base = buf.cell((0u16, 9u16)).expect("blank row").bg;
    let mut z = None;
    let mut hits = Vec::new();
    for x in 0..rect.width {
        let cell = buf.cell((x, vrow)).expect("in pane");
        if cell.symbol() == "Z" {
            z = Some(x as usize);
        }
        if cell.bg != base {
            hits.push(x as usize);
        }
    }
    (z, hits)
}

fn check(line: &str) {
    let (z, hits) = painted_vs_highlighted(line, 0);
    let z = z.expect("Z is painted");
    assert_eq!(hits, vec![z], "line {line:?}: Z paints at cell {z}");
}

#[test]
fn an_overlay_paints_on_the_cell_its_char_occupies() {
    check("a Z");
    check("a\tZ");
    check("> a Z");
    check("- a\tZ");
    check("## h\tZ");
    check("> a\tZ");
    check("> > a\tZ");
}
