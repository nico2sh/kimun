//! The text-sized editor of a facet piece, in isolation: a column of
//! read-only editors that measure their own height, in a test window.
//! These tests prove that a measure lays a piece out at one width per
//! frame, and that the measured height matches what prepaint draws.

use gpui_kit::base::TestSupportExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, TestAppContext, VisualTestContext, Window,
    div, px, size,
};

use crate::editor_font;
use crate::editor_view::{self, EditorView, measure_probe as probe};
use crate::keymap::{self, Keymap, Platform};

/// A column of pieces laid out like the facet view's column. A piece that
/// is not text-sized sits in a holder of fixed height, sized from the rows
/// of its last layout, as the facet view does today.
pub(crate) struct Column {
    pub(crate) pieces: Vec<Entity<EditorView>>,
}

pub(crate) fn piece_id(i: usize) -> SharedString {
    SharedString::from(format!("piece-{i}"))
}

impl Render for Column {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let line_height = f32::from(editor_view::line_height(editor_font::font_size(cx)));
        let rem = f32::from(window.rem_size());
        let pieces = self.pieces.iter().enumerate().map(|(i, piece)| {
            let holder = div().flex_none().child(piece.clone());
            let holder = if !piece.read(cx).sized_to_text {
                let rows = visual_rows(piece, cx).max(1);
                holder.h(px(rows as f32 * line_height + rem + 1.0))
            } else {
                holder
            };
            div()
                .id(piece_id(i))
                .test_support()
                .flex()
                .flex_col()
                .flex_none()
                .overflow_hidden()
                .child(holder)
        });
        div().size_full().flex().flex_col().child(
            div()
                .id("column")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .gap_3()
                .px_4()
                .py_3()
                .overflow_y_scroll()
                .children(pieces),
        )
    }
}

/// The visual rows of `piece`'s current layout.
pub(crate) fn visual_rows(piece: &Entity<EditorView>, cx: &gpui_kit::App) -> usize {
    piece
        .read(cx)
        .editor
        .session()
        .display
        .layout
        .visual_line_count()
}

/// A read-only editor over `text`, text-sized when `sized`.
pub(crate) fn piece(text: &str, sized: bool, cx: &mut Context<EditorView>) -> EditorView {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    // Nothing reads the channel here: a send to it is dropped.
    drop(rx);
    let mut view = EditorView::new_read_only_on(text, tx, cx);
    view.sized_to_text = sized;
    view
}

/// A window `width` wide holding one piece per text, drawn once. Each
/// piece is text-sized when `sized`.
pub(crate) fn open_column<'a>(
    cx: &'a mut TestAppContext,
    texts: &[String],
    sized: bool,
    width: f32,
) -> (Vec<Entity<EditorView>>, &'a mut VisualTestContext) {
    let pieces: Vec<(String, bool)> = texts.iter().map(|t| (t.clone(), sized)).collect();
    open_mixed(cx, &pieces, width)
}

/// A window at the test default size holding one text-sized piece per
/// text, and what the measures saw in its first frame.
fn open_logged<'a>(
    cx: &'a mut TestAppContext,
    texts: &[String],
) -> (
    Vec<Entity<EditorView>>,
    &'a mut VisualTestContext,
    std::collections::HashMap<gpui_kit::EntityId, probe::Measures>,
) {
    init(cx);
    let texts = texts.to_vec();
    probe::take();
    let (column, cx) = cx.add_window_view(move |_, cx| Column {
        pieces: texts
            .iter()
            .map(|text| cx.new(|cx| piece(text, true, cx)))
            .collect(),
    });
    let log = probe::take();
    let pieces = cx.update(|_, cx| column.read(cx).pieces.clone());
    (pieces, cx, log)
}

pub(crate) fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
    });
}

/// A window `width` wide holding one piece per text, each text-sized as
/// given.
pub(crate) fn open_mixed<'a>(
    cx: &'a mut TestAppContext,
    pieces: &[(String, bool)],
    width: f32,
) -> (Vec<Entity<EditorView>>, &'a mut VisualTestContext) {
    init(cx);
    add_column(cx, pieces, width)
}

/// [`open_mixed`] for an app already set up.
fn add_column<'a>(
    cx: &'a mut TestAppContext,
    pieces: &[(String, bool)],
    width: f32,
) -> (Vec<Entity<EditorView>>, &'a mut VisualTestContext) {
    let pieces = pieces.to_vec();
    let (column, cx) = cx.add_window_view(move |_, cx| Column {
        pieces: pieces
            .iter()
            .map(|(text, sized)| cx.new(|cx| piece(text, *sized, cx)))
            .collect(),
    });
    cx.simulate_resize(size(px(width), px(600.)));
    let pieces = cx.update(|_, cx| column.read(cx).pieces.clone());
    (pieces, cx)
}

/// A paragraph that wraps at any width below a few thousand pixels.
pub(crate) fn paragraph(i: usize) -> String {
    let words: String = (0..40).map(|w| format!("word{i}x{w} ")).collect();
    format!("# Piece {i}\n{words}\nlast line")
}

/// One frame, and what the measures saw in it.
fn frame(
    cx: &mut VisualTestContext,
) -> std::collections::HashMap<gpui_kit::EntityId, probe::Measures> {
    probe::take();
    cx.update(|window, cx| window.render_frame(cx));
    probe::take()
}

/// Every piece measured at exactly one width, with at most `relayouts`
/// layouts built. Logs the widths of the first piece.
fn assert_one_width(
    log: &std::collections::HashMap<gpui_kit::EntityId, probe::Measures>,
    pieces: &[Entity<EditorView>],
    relayouts: usize,
    what: &str,
) {
    for (i, piece) in pieces.iter().enumerate() {
        let m = log
            .get(&piece.entity_id())
            .unwrap_or_else(|| panic!("{what}: piece {i} was not measured"));
        if i == 0 {
            eprintln!("{what}: piece 0 {m:?}");
        }
        assert_eq!(m.widths.len(), 1, "{what}: piece {i} widths {:?}", m.widths);
        assert!(
            m.relayouts <= relayouts,
            "{what}: piece {i} built {} layouts",
            m.relayouts
        );
    }
}

#[gpui_kit::test]
fn a_piece_is_measured_at_one_width_per_frame(cx: &mut TestAppContext) {
    let texts: Vec<String> = (0..100).map(paragraph).collect();
    let (pieces, cx, first) = open_logged(cx, &texts);
    assert_one_width(&first, &pieces, 1, "first frame");
    assert!(
        pieces.iter().all(|p| first[&p.entity_id()].relayouts == 1),
        "a new piece lays out once in its first frame"
    );
    let steady = frame(cx);
    assert_one_width(&steady, &pieces, 0, "same width");
    for width in [500., 437., 900.] {
        probe::take();
        cx.simulate_resize(size(px(width), px(600.)));
        let resized = probe::take();
        assert!(!resized.is_empty(), "a resize draws a frame");
        assert_one_width(&resized, &pieces, 1, &format!("resize to {width}"));
        let after = frame(cx);
        assert_one_width(&after, &pieces, 0, &format!("after the resize to {width}"));
    }
}

/// The rows a text-sized piece and a fixed-height piece of the same width
/// lay `text` out in, after one frame. The text-sized piece reports its
/// rows in that frame.
fn rows_both_ways(cx: &mut TestAppContext, text: &str, width: f32) -> (usize, usize) {
    let both = [(text.to_string(), true), (text.to_string(), false)];
    let (pieces, cx) = open_mixed(cx, &both, width);
    cx.update(|window, cx| window.render_frame(cx));
    cx.update(|_, cx| {
        let measured = visual_rows(&pieces[0], cx);
        assert_eq!(pieces[0].read(cx).measured_rows, Some(measured.max(1)));
        (measured, visual_rows(&pieces[1], cx))
    })
}

#[gpui_kit::test]
fn a_text_sized_editor_reports_its_wrapped_rows_for_a_width(cx: &mut TestAppContext) {
    let line: String = (0..60).map(|i| format!("word{i} ")).collect();
    let (measured, drawn) = rows_both_ways(cx, &line, 400.);
    assert!(measured > 3, "the line wraps: {measured} rows");
    assert_eq!(measured, drawn, "the measure wraps as prepaint does");
}

#[gpui_kit::test]
fn concealed_links_wrap_the_same_way_measured(cx: &mut TestAppContext) {
    let links: String = (0..12)
        .map(|i| format!("see [label{i}](https://example.com/a/very/long/path/{i}) and "))
        .collect();
    let text = format!("intro\n{links}\n[[a wiki link|shown]] and [[another note]] end");
    let (measured, drawn) = rows_both_ways(cx, &text, 420.);
    assert_eq!(measured, drawn, "concealed links wrap alike");
    // Written out, the links line alone would take 15 rows or more.
    assert!(measured <= 8, "the links are concealed: {measured} rows");
}

#[gpui_kit::test]
fn a_folded_front_matter_note_wraps_the_same_way_measured(cx: &mut TestAppContext) {
    let body: String = (0..50).map(|i| format!("body{i} ")).collect();
    let text = format!("---\ntitle: a note\ntags: [one, two]\n---\n{body}");
    let (measured, drawn) = rows_both_ways(cx, &text, 380.);
    assert_eq!(measured, drawn, "the fold hides the same rows");
    // The body takes 9 rows. Unfolded, the front matter adds 4 more.
    assert!(measured < 13, "the front matter is folded: {measured} rows");
}

/// The piece's height in the frame just drawn equals its painted rows
/// times the line height, plus the inset and the slack, and its painted
/// rows equal its layout's rows.
pub(crate) fn assert_as_tall_as_rows(
    cx: &mut VisualTestContext,
    piece: &Entity<EditorView>,
    what: &str,
) {
    cx.update(|window, cx| {
        let painted = piece.read(cx).painted.clone().expect("a frame was painted");
        let rows = visual_rows(piece, cx).max(1);
        assert_eq!(usize::from(painted.rows), rows, "{what}: painted rows");
        let line_height = painted.cells.height();
        let rem = f32::from(window.rem_size());
        let height = f32::from(window.find(piece_id(0)).bounds().size.height);
        let expected = rows as f32 * line_height + 1.0 + rem;
        assert!(
            (height - expected).abs() < 0.01,
            "{what}: height {height} for {rows} rows (expected {expected})"
        );
    });
}

#[gpui_kit::test]
fn a_text_sized_editor_is_as_tall_as_its_rows(cx: &mut TestAppContext) {
    let line: String = (0..60).map(|i| format!("word{i} ")).collect();
    let (pieces, cx) = open_column(cx, &[line], true, 300.);
    // The open drew a first frame at the default size, and the resize one
    // more. Each frame at a new width must be right on its own.
    assert_as_tall_as_rows(cx, &pieces[0], "first frame");
    for width in 300..=340 {
        cx.simulate_resize(size(px(width as f32), px(600.)));
        cx.update(|window, cx| window.render_frame(cx));
        assert_as_tall_as_rows(cx, &pieces[0], &format!("width {width}"));
    }
}

#[gpui_kit::test]
fn a_text_sized_editor_is_right_at_a_fractional_scale(cx: &mut TestAppContext) {
    let line: String = (0..60).map(|i| format!("word{i} ")).collect();
    let (pieces, cx) = open_column(cx, &[line], true, 300.);
    // Odd widths at a scale of 1.5 put edges on half device pixels.
    cx.simulate_scale_factor_change(1.5);
    for width in 300..=320 {
        cx.simulate_resize(size(px(width as f32 + 0.5), px(600.)));
        cx.update(|window, cx| window.render_frame(cx));
        cx.update(|_, cx| {
            let view = pieces[0].read(cx);
            let painted = view.painted.clone().expect("painted");
            assert_eq!(
                usize::from(painted.rows),
                visual_rows(&pieces[0], cx).max(1)
            );
        });
    }
}

#[gpui_kit::test]
fn the_note_editor_still_fills_its_pane(cx: &mut TestAppContext) {
    init(cx);
    let (view, cx) =
        cx.add_window_view(|_, cx| EditorView::new(Some("one line".into()), false, cx));
    cx.simulate_resize(size(px(640.), px(480.)));
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(!view.read(cx).sized_to_text);
        let bounds = window.find("editor").bounds();
        assert_eq!(bounds.size, size(px(640.), px(480.)), "the pane is filled");
        let painted = view.read(cx).painted.clone().expect("painted");
        let rem = f32::from(window.rem_size());
        // The grid fills the pane inside its inset, not the text's one row.
        let (cols, rows) = painted.cells.fit(640. - 2. * rem, 480. - rem);
        assert_eq!(painted.rows, rows, "rows fit the pane");
        assert!(cols > 50, "{cols} columns");
    });
}

#[gpui_kit::test]
fn a_large_piece_grows_in_the_frame_its_background_layout_lands(cx: &mut TestAppContext) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let _guard = rt.enter();
    let long: String = (0..60).map(|i| format!("word{i} ")).collect();
    let rest: String = (0..1200).map(|i| format!("l{i}\n")).collect();
    let text = format!("{long}\n{rest}");
    let written = text.split('\n').count();
    let (pieces, cx) = open_column(cx, &[text], true, 260.);
    assert_as_tall_as_rows(cx, &pieces[0], "stub frame");
    let stub = cx.update(|_, cx| visual_rows(&pieces[0], cx));
    assert_eq!(stub, written, "the first layout is the unwrapped stub");
    let mut landed = false;
    for frame in 0..100 {
        rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(10)));
        cx.run_until_parked();
        cx.update(|window, cx| window.render_frame(cx));
        assert_as_tall_as_rows(cx, &pieces[0], &format!("frame {frame}"));
        if cx.update(|_, cx| visual_rows(&pieces[0], cx)) > written {
            landed = true;
            break;
        }
    }
    assert!(landed, "the background layout wrapped the paragraph");
}

/// The mean time of `frames` frames drawn by `draw`, in milliseconds.
pub(crate) fn time_frames(
    cx: &mut VisualTestContext,
    frames: usize,
    mut draw: impl FnMut(&mut VisualTestContext, usize),
) -> f64 {
    let start = std::time::Instant::now();
    for i in 0..frames {
        draw(cx, i);
    }
    start.elapsed().as_secs_f64() * 1000.0 / frames as f64
}

/// The steady and the resize frame times of a column of 100 pieces.
fn bench_column(cx: &mut VisualTestContext, extra_frame: bool) -> (f64, f64) {
    let steady = time_frames(cx, 20, |cx, _| {
        cx.update(|window, cx| window.render_frame(cx));
    });
    // Every frame crosses a cell boundary. The fixed path today draws one
    // more frame after each resize to correct the rows.
    let resize = time_frames(cx, 20, |cx, i| {
        cx.simulate_resize(size(px(600. + 9. * (i % 10) as f32), px(600.)));
        if extra_frame {
            cx.update(|window, cx| window.render_frame(cx));
        }
    });
    (steady, resize)
}

#[gpui_kit::test]
#[ignore = "a timing bench, run by hand"]
fn bench_100_measured_pieces_against_fixed_holders(cx: &mut TestAppContext) {
    let measured: Vec<(String, bool)> = (0..100).map(|i| (paragraph(i), true)).collect();
    let fixed: Vec<(String, bool)> = (0..100).map(|i| (paragraph(i), false)).collect();
    init(cx);
    // Each path twice, in turns, so neither gains from running second.
    for round in 0..2 {
        let (_, fixed_cx) = add_column(cx, &fixed, 600.);
        let (f_steady, f_resize) = bench_column(fixed_cx, true);
        let (_, measured_cx) = add_column(cx, &measured, 600.);
        let (m_steady, m_resize) = bench_column(measured_cx, false);
        eprintln!(
            "round {round} fixed:    steady {f_steady:.2} ms, resize {f_resize:.2} ms (+1 frame)"
        );
        eprintln!("round {round} measured: steady {m_steady:.2} ms, resize {m_resize:.2} ms");
    }
}
