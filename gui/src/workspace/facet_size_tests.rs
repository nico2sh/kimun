//! UI tests for how the **Facet view** sizes its pieces. A piece is as
//! tall as its wrapped text in the frame that draws it. No frame draws a
//! piece too short or too tall. No frame asks for a second frame to fix a
//! size. Each test reads the frame that the change drew, and draws no
//! frame of its own before it looks.

use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
use gpui_kit::{Entity, MouseButton, TestAppContext, VisualTestContext, px, size};

use super::facet::{FacetView, SHOW_MORE_ID, chunk_id, gap_down_id};
use super::facet_tests::{cell, chunk, click, next_frame, open, settle, tag, to_bottom};
use crate::measure_tests::time_frames;

/// What is wrong with the size of each piece in the last frame. It is
/// empty when every piece is as tall as its rows. It draws no frame.
pub(super) fn misfits(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|window, cx| {
        let rem = f32::from(window.rem_size());
        let chunks = view.read(cx).chunks().to_vec();
        let mut wrong = Vec::new();
        for (i, chunk) in chunks.iter().enumerate() {
            let editor = chunk.read(cx);
            let Some(painted) = editor.painted.clone() else {
                wrong.push(format!("piece {i} was not painted"));
                continue;
            };
            let rows = editor
                .editor
                .session()
                .display
                .layout
                .visual_line_count()
                .max(1);
            if usize::from(painted.rows) != rows {
                wrong.push(format!(
                    "piece {i}: {} rows painted of {rows}",
                    painted.rows
                ));
            }
            let Some(found) = window.try_find(chunk_id(i)) else {
                wrong.push(format!("piece {i} was not drawn"));
                continue;
            };
            // The rows, the pixel of slack and the editor's bottom inset.
            let end = f32::from(painted.origin.y)
                + rows as f32 * painted.cells.height()
                + 1.0
                + rem / 2.0;
            let bottom = f32::from(found.bounds().bottom());
            if (bottom - end).abs() > 0.6 {
                wrong.push(format!(
                    "piece {i}: bottom {bottom}, its {rows} rows end at {end}"
                ));
            }
        }
        wrong
    })
}

/// Assert that every piece in the last frame is as tall as its rows.
pub(super) fn assert_sized(view: &Entity<FacetView>, cx: &mut VisualTestContext, what: &str) {
    let wrong = misfits(view, cx);
    assert!(wrong.is_empty(), "{what}: {wrong:#?}");
}

/// The height of each piece in the last frame. It draws no frame.
fn heights(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> Vec<f32> {
    let count = view.read_with(cx, |view, _| view.chunks().len());
    cx.update(|window, _| {
        (0..count)
            .map(|i| f32::from(window.find(chunk_id(i)).bounds().size.height))
            .collect()
    })
}

/// Assert that the last frame needs no correcting frame. Every piece in
/// it is as tall as its rows. Then the next frames it asked for run (a
/// motion asks for its own), and they change the height of no piece.
pub(super) fn assert_no_correction(
    view: &Entity<FacetView>,
    cx: &mut VisualTestContext,
    what: &str,
) {
    assert_sized(view, cx, what);
    let before = heights(view, cx);
    next_frame(cx);
    assert_sized(view, cx, &format!("{what}, the next frame"));
    assert_eq!(
        heights(view, cx),
        before,
        "{what}: the next frame resized a piece"
    );
}

/// The visual rows of piece `i` now.
fn rows(view: &Entity<FacetView>, i: usize, cx: &mut VisualTestContext) -> usize {
    let chunk = chunk(view, i, cx);
    cx.update(|_, cx| {
        chunk
            .read(cx)
            .editor
            .session()
            .display
            .layout
            .visual_line_count()
    })
}

/// The top of piece `i` in the last frame. It draws no frame.
fn top(cx: &mut VisualTestContext, i: usize) -> f32 {
    cx.update(|window, _| f32::from(window.find(chunk_id(i)).bounds().top()))
}

fn words(n: usize, stem: &str) -> String {
    (0..n).map(|i| format!("{stem}{i} ")).collect()
}

/// Three tagged paragraphs that wrap, with an untagged one between each.
fn three_paragraphs() -> String {
    let long = |i: usize| words(40 + 10 * i, &format!("p{i}w"));
    format!(
        "#a {}\n\nother\n\n#a {}\n\nother\n\n#a {}\n",
        long(0),
        long(1),
        long(2)
    )
}

#[gpui_kit::test]
fn a_resize_sizes_every_piece_in_the_same_frame(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &three_paragraphs(), vec![tag("a")]);
    settle(cx);
    assert_eq!(view.read_with(cx, |view, _| view.chunks().len()), 3);
    for width in [260., 400., 333., 900., 300., 341.] {
        // The resize draws the frame at the new width.
        cx.simulate_resize(size(px(width), px(1200.)));
        assert_sized(&view, cx, &format!("the first frame at {width}"));
    }
    assert!(rows(&view, 2, cx) > 4, "the last piece wraps");
}

#[gpui_kit::test]
fn a_swapped_text_is_sized_by_its_own_wrap_in_its_first_frame(cx: &mut TestAppContext) {
    let first = format!("#a {}\n\nother\n", words(60, "word"));
    let (view, cx, _events, _sub) = open(cx, &first, vec![tag("a")]);
    cx.simulate_resize(size(px(260.), px(1200.)));
    settle(cx);
    assert!(rows(&view, 0, cx) > 4, "the old text wraps");
    // Fewer rows than the old wrap, then more rows than written.
    view.update(cx, |view, cx| {
        view.set_text("#a short\n\nother\n".to_string(), cx)
    });
    assert_sized(&view, cx, "a shorter text");
    assert_eq!(rows(&view, 0, cx), 1);
    let longer = format!("#a {}\n\nother\n", words(90, "more"));
    view.update(cx, |view, cx| view.set_text(longer, cx));
    assert_sized(&view, cx, "a longer text");
    assert!(rows(&view, 0, cx) > 6, "the new text wraps");
}

/// A large chunk is laid out unwrapped first and wrapped on a background
/// task. The frame that installs the wrap also sizes the piece for it.
#[gpui_kit::test]
fn a_large_chunk_grows_in_the_frame_its_background_layout_lands(cx: &mut TestAppContext) {
    let rt = super::start_tests::runtime();
    let _guard = rt.enter();
    let rest: String = (0..1200).map(|i| format!("l{i}\n")).collect();
    let text = format!("{}\n{rest}", words(60, "word"));
    let written = text.split('\n').count();
    let (view, cx, _events, _sub) = open(cx, &text, vec![]);
    cx.simulate_resize(size(px(260.), px(800.)));
    assert_sized(&view, cx, "the stub frame");
    let mut landed = false;
    for frame in 0..100 {
        rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(10)));
        // The frame pumps a finished wrap in its measure: this frame lands it.
        cx.update(|window, cx| window.render_frame(cx));
        assert_sized(&view, cx, &format!("frame {frame}"));
        // The wake of a landed wrap draws one more frame.
        cx.run_until_parked();
        assert_sized(&view, cx, &format!("frame {frame}, after the wake"));
        if rows(&view, 0, cx) > written {
            landed = true;
            break;
        }
    }
    assert!(landed, "the background layout wrapped the paragraph");
}

#[gpui_kit::test]
fn a_revealed_block_moves_the_column_once(cx: &mut TestAppContext) {
    let text = format!("#x p0\n\n{}\n\np2\n\n#x p3\n", words(70, "hidden"));
    let (view, cx, _events, _sub) = open(cx, &text, vec![tag("x")]);
    cx.simulate_resize(size(px(300.), px(1200.)));
    settle(cx);
    let before = top(cx, 1);
    click(cx, gap_down_id(1));
    assert_eq!(view.read_with(cx, |view, _| view.chunks().len()), 2);
    assert!(rows(&view, 0, cx) > 6, "the revealed block wraps");
    assert_sized(&view, cx, "the reveal's frame");
    let moved = top(cx, 1);
    assert!(
        moved > before + 50.0,
        "the column moved: {before} to {moved}"
    );
    settle(cx);
    assert_eq!(top(cx, 1), moved, "and it moved only once");
}

#[gpui_kit::test]
fn a_click_that_unfolds_a_link_resizes_its_piece_in_the_same_frame(cx: &mut TestAppContext) {
    let url = "https://example.com/a/very/long/path/that/goes/on/and/on/0123456789";
    let text = format!("intro #t\nsee [label]({url}) and some more words after it\n");
    let (view, cx, _events, _sub) = open(cx, &text, vec![]);
    cx.simulate_resize(size(px(300.), px(1200.)));
    settle(cx);
    let folded = rows(&view, 0, cx);
    // The press puts the caret on the link, so the link shows written out.
    // The press alone draws the frame that is checked. The release draws
    // another one.
    let at = cell(&view, 0, 5, 1, cx);
    cx.update(|window, cx| window.pointer_down(at, MouseButton::Left, cx));
    cx.run_until_parked();
    let unfolded = rows(&view, 0, cx);
    assert!(
        unfolded > folded,
        "the link unfolds: {folded} to {unfolded}"
    );
    assert_sized(&view, cx, "the press's frame");
    cx.update(|window, cx| window.pointer_up(at, MouseButton::Left, cx));
    cx.run_until_parked();
    assert_sized(&view, cx, "the release's frame");
}

#[gpui_kit::test]
fn no_next_frame_is_asked_for_sizing_after_any_change(cx: &mut TestAppContext) {
    let text = format!(
        "#x p0 {}\n\n{}\n\np2\n\n#x p3\n",
        words(30, "a"),
        words(70, "b")
    );
    let (view, cx, _events, _sub) = open(cx, &text, vec![]);
    settle(cx);
    cx.simulate_resize(size(px(300.), px(1200.)));
    assert_sized(&view, cx, "a resize");
    assert_eq!(next_frame(cx), 0, "a resize asks for no next frame");
    view.update(cx, |view, cx| view.toggle(tag("x"), cx));
    assert_sized(&view, cx, "a pick");
    assert_eq!(next_frame(cx), 0, "a pick asks for no next frame");
    click(cx, gap_down_id(1));
    assert_sized(&view, cx, "a reveal");
    assert_eq!(next_frame(cx), 0, "a reveal asks for no next frame");
    let reloaded = format!("#x p0 {}\n\nshort\n\n#x p3\n", words(50, "c"));
    view.update(cx, |view, cx| view.set_text(reloaded, cx));
    assert_sized(&view, cx, "a reload");
    assert_eq!(next_frame(cx), 0, "a reload asks for no next frame");
}

/// A facet view at the cap: 150 tagged paragraphs that wrap, of which the
/// view draws 100 pieces. For the app mode (`draw_all` off, spacers for far
/// pieces) and for `draw_all` on, five rounds each time 20 steady frames,
/// 20 scroll frames (a few viewports down, one frame per step) and 20
/// resize frames. Every resize crosses a cell boundary. Prints the mean of
/// each round of 20 and the median of the rounds. The spike measured a bare
/// column of 100 measured pieces at 4.5 ms steady and 7.0 ms per resize.
#[gpui_kit::test]
#[ignore = "a timing bench, run by hand"]
fn bench_a_facet_view_of_100_pieces(cx: &mut TestAppContext) {
    let (view, cx) = open_pieces(cx, 150);
    assert_eq!(view.read_with(cx, |view, _| view.chunks().len()), 100);
    bench_pieces(&view, cx);
}

/// [`bench_a_facet_view_of_100_pieces`] after `Show more` twice: 400
/// pieces of 450.
#[gpui_kit::test]
#[ignore = "a timing bench, run by hand"]
fn bench_a_facet_view_of_400_pieces(cx: &mut TestAppContext) {
    let (view, cx) = open_pieces(cx, 450);
    for shown in [200, 400] {
        to_bottom(&view, cx);
        click(cx, SHOW_MORE_ID);
        settle(cx);
        assert_eq!(view.read_with(cx, |view, _| view.chunks().len()), shown);
    }
    bench_pieces(&view, cx);
}

/// A view of `n` tagged paragraphs that wrap, picked, 600 by 600, settled.
fn open_pieces(cx: &mut TestAppContext, n: usize) -> (Entity<FacetView>, &mut VisualTestContext) {
    let text: String = (0..n)
        .map(|i| {
            format!(
                "#a piece {i} {}\n\nuntagged {i}\n\n",
                words(40, &format!("w{i}x"))
            )
        })
        .collect();
    let (view, cx, _events, _sub) = open(cx, &text, vec![tag("a")]);
    cx.simulate_resize(size(px(600.), px(600.)));
    settle(cx);
    (view, cx)
}

/// The rounds of the facet benches, in app mode and with `draw_all` on.
fn bench_pieces(view: &Entity<FacetView>, cx: &mut VisualTestContext) {
    use super::facet_window_view_tests::scroll_to;
    let view = view.clone();
    for all in [false, true] {
        view.update(cx, |view, _| view.set_draw_all(all));
        scroll_to(&view, 0., cx);
        let mut rounds: [Vec<f64>; 3] = Default::default();
        for round in 0..5 {
            let steady = time_frames(cx, 20, |cx, _| {
                cx.update(|window, cx| window.render_frame(cx));
            });
            let scroll = time_frames(cx, 20, |cx, i| {
                scroll_to(
                    &view,
                    if i < 10 {
                        211. * i as f32
                    } else {
                        211. * (20 - i) as f32
                    },
                    cx,
                );
            });
            let resize = time_frames(cx, 20, |cx, i| {
                cx.simulate_resize(size(px(600. + 9. * (i % 10) as f32), px(600.)));
            });
            if all {
                // `assert_sized` reads every piece: only drawn when all are.
                assert_sized(&view, cx, "the last resize frame");
            }
            eprintln!(
                "draw_all {all} round {round}: steady {steady:.2} ms, scroll {scroll:.2} ms, resize {resize:.2} ms"
            );
            for (r, t) in rounds.iter_mut().zip([steady, scroll, resize]) {
                r.push(t);
            }
        }
        let [steady, scroll, resize] = rounds.map(|mut r| {
            r.sort_by(f64::total_cmp);
            r[r.len() / 2]
        });
        eprintln!(
            "draw_all {all} MEDIAN: steady {steady:.2} ms, scroll {scroll:.2} ms, resize {resize:.2} ms"
        );
    }
}
