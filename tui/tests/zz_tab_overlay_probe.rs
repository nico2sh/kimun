use std::num::NonZeroU64;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

use kimun_notes::components::text_editor::snapshot::EditorSnapshot;
use kimun_notes::components::text_editor::view::{MarkdownEditorView, Overlay, OverlayKind};
use kimun_notes::settings::themes::Theme;

fn probe_row(text: &str, ov: Overlay, width: u16, y: u16) {
    let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let revision = NonZeroU64::new(1).expect("nz");
    let snapshot = EditorSnapshot::borrowed(&lines, (lines.len() - 1, 0), revision);
    let rect = Rect {
        x: 0,
        y: 0,
        width,
        height: 10,
    };
    let theme = Theme::default();
    let mut view = MarkdownEditorView::new();
    view.update(&snapshot, rect);
    view.set_overlays(vec![ov]);
    let mut terminal = Terminal::new(TestBackend::new(rect.width, rect.height)).expect("backend");
    terminal
        .draw(|frame| view.render(frame, rect, &theme, false, None))
        .expect("draw");
    let buf = terminal.backend().buffer();
    let row: String = (0..width)
        .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
        .collect();
    let bgs: Vec<String> = (0..width)
        .map(|x| format!("{}:{:?}", x, buf.cell((x, y)).unwrap().bg))
        .collect();
    println!("text={text:?} y={y}\n  row=|{row}|\n  {}", bgs.join(" "));
}

#[test]
fn tab_overlay_probe() {
    // "> a\tb": overlay on logical cols 4..5 == the 'b'.
    probe(
        "> a\tb\n",
        Overlay::new(0, 4, 5, OverlayKind::CurrentMatch),
        20,
    );
    // control, no tab: "> a b", overlay on 4..5 == 'b'
    probe("> a b\n", Overlay::new(0, 4, 5, OverlayKind::CurrentMatch), 20);
    // non-blockquote tab row: "a\tb", overlay on 2..3 == 'b'
    probe("a\tb\n", Overlay::new(0, 2, 3, OverlayKind::CurrentMatch), 20);
}

#[test]
fn tab_overlay_probe2() {
    // nested blockquote, gutter 3, sigil "> > " = 4 chars
    probe("> > a\tb\n", Overlay::new(0, 6, 7, OverlayKind::Selection), 20);
    // wrapped blockquote: tab on the CONTINUATION visual line
    probe(
        "> aaaa bbbb cc\tdd\n",
        Overlay::new(0, 15, 17, OverlayKind::Selection),
        12,
    );
    // heading sigil + tab (sigil is drawn, not replaced by a gutter)
    probe("## h\tb\n", Overlay::new(0, 5, 6, OverlayKind::Selection), 20);
    // list sigil + tab
    probe("- a\tb\n", Overlay::new(0, 4, 5, OverlayKind::Selection), 20);
}

fn probe(text: &str, ov: Overlay, width: u16) { probe_row(text, ov, width, 0) }

#[test]
fn tab_overlay_probe3() {
    // wrapped blockquote, tab on the continuation row; dump row 1
    probe_row("> aaaa bbbb cc\tdd\n", Overlay::new(0, 15, 17, OverlayKind::Selection), 12, 1);
    probe_row("> aaaa bbbb cc\tdd\n", Overlay::new(0, 12, 14, OverlayKind::Selection), 12, 1);
}
