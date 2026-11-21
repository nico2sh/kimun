//! Tests for the facet view's height table and drawn range. The table is
//! plain data, so these tests need no window.

use std::num::NonZeroU64;

use gpui_kit::{Pixels, px};

use super::facet::window::{Heights, Metrics, Row, Shape, drawn_range, pinned};

const LINE: f32 = 20.0;
const DATELINE: f32 = 30.0;
const GAP_ROW: f32 = 18.0;
const CRUMBS: f32 = 22.0;
const UP_ROW: f32 = 26.0;
const INSET: f32 = 8.0;
const COLUMN_GAP: f32 = 12.0;
const MARGIN: f32 = 6.0;
/// How far a down gap row moves up toward the piece above it.
const PULL: f32 = 4.0;

fn stamp(n: u64) -> (NonZeroU64, (usize, usize)) {
    (NonZeroU64::new(n).unwrap(), (0, 0))
}

fn metrics() -> Metrics {
    Metrics {
        line: px(LINE),
        dateline: px(DATELINE),
        dateline_margin: px(MARGIN),
        gap_row: px(GAP_ROW),
        gap_pull: px(PULL),
        crumbs: px(CRUMBS),
        up_row: px(UP_ROW),
        inset: px(INSET),
        column_gap: px(COLUMN_GAP),
    }
}

/// A table of `n` plain pieces of `rows` rows each, at width 400.
fn table(n: usize, rows: usize) -> Heights {
    let mut h = Heights::default();
    h.fit(px(400.), px(14.), px(16.), 0, metrics());
    h.set_shapes(vec![Shape::default(); n]);
    for i in 0..n {
        h.set_rows(i, rows, i as u64, stamp(1));
    }
    h
}

/// The height of one plain piece of `rows` rows.
fn plain(rows: usize) -> f32 {
    CRUMBS + rows as f32 * LINE + INSET
}

fn f(p: Pixels) -> f32 {
    f32::from(p)
}

#[test]
fn the_range_covers_the_viewport_and_one_viewport_each_side() {
    // 40 pieces of 3 rows each, with gaps between them.
    let h = table(40, 3);
    assert_eq!(f(h.top_of(1)), plain(3) + COLUMN_GAP);
    let step = plain(3) + COLUMN_GAP;
    let (offset, viewport) = (2000., 600.);
    let range = drawn_range(px(offset), px(viewport), &h);
    for i in 0..40 {
        let (top, bottom) = (f(h.top_of(i)), f(h.top_of(i)) + plain(3));
        let touches = bottom > offset - viewport && top < offset + 2. * viewport;
        assert_eq!(range.contains(&i), touches, "piece {i}");
    }
    assert!(range.len() as f32 * step >= 3. * viewport);
}

#[test]
fn the_range_follows_the_scroll_offset_and_clamps_at_both_ends() {
    let h = table(40, 3);
    let top = drawn_range(px(0.), px(600.), &h);
    assert_eq!(top.start, 0);
    let low = drawn_range(px(-500.), px(600.), &h);
    assert_eq!(low, top, "a negative offset reads as the top");
    let max = f(h.total()) - 600.;
    let end = drawn_range(px(max), px(600.), &h);
    assert_eq!(end.end, 40);
    let past = drawn_range(px(max + 9000.), px(600.), &h);
    assert_eq!(past, end, "an offset past the end reads as the end");
    let mid = drawn_range(px(2000.), px(600.), &h);
    assert!(mid.start > top.start && mid.end < end.end);
    assert!(drawn_range(px(0.), px(600.), &Heights::default()).is_empty());
}

#[test]
fn the_table_top_of_matches_the_sum_of_heights_and_gaps() {
    let mut h = Heights::default();
    h.fit(px(400.), px(14.), px(16.), 0, metrics());
    h.set_shapes(vec![
        Shape {
            dateline: true,
            lead_row: false,
            ..Shape::default()
        },
        Shape {
            dateline: false,
            lead_row: true,
            ..Shape::default()
        },
        Shape::default(),
    ]);
    for (i, rows) in [2, 5, 1].into_iter().enumerate() {
        h.set_rows(i, rows, i as u64, stamp(1));
    }
    let a = DATELINE + plain(2);
    let b = GAP_ROW + plain(5);
    let c = plain(1);
    assert_eq!(f(h.top_of(0)), 0.);
    // The lead row of piece 1 moves up toward piece 0 by the pull.
    assert_eq!(f(h.top_of(1)), a + COLUMN_GAP - PULL);
    assert_eq!(f(h.top_of(2)), a + b + 2. * COLUMN_GAP - PULL);
    assert_eq!(f(h.total()), a + b + c + 2. * COLUMN_GAP - PULL);
    // A spacer starts after the column gap: it stands in for the pull too.
    assert_eq!(f(h.span(1..3)), b + c + COLUMN_GAP - PULL);
    assert_eq!(f(h.span(2..2)), 0.);
}

#[test]
fn a_width_change_or_a_font_size_change_clears_the_rows() {
    let mut h = table(3, 4);
    assert_eq!(h.rows(0, 0, stamp(1)), Some(4));
    assert!(
        !h.fit(px(400.), px(14.), px(16.), 0, metrics()),
        "same keys"
    );
    assert_eq!(h.rows(0, 0, stamp(1)), Some(4));
    assert!(
        h.fit(px(420.), px(14.), px(16.), 0, metrics()),
        "a new width"
    );
    assert_eq!(h.rows(0, 0, stamp(1)), None);
    h.set_rows(0, 4, 0, stamp(1));
    assert!(
        h.fit(px(420.), px(15.), px(16.), 0, metrics()),
        "a new font"
    );
    assert_eq!(h.rows(0, 0, stamp(1)), None);
    h.set_rows(0, 4, 0, stamp(1));
    assert!(h.fit(px(420.), px(15.), px(18.), 0, metrics()), "a new rem");
    assert_eq!(h.rows(0, 0, stamp(1)), None);
}

#[test]
fn a_changed_edit_stamp_replaces_only_that_piece() {
    let mut h = table(3, 4);
    assert_eq!(h.rows(1, 1, stamp(2)), None, "another stamp is a miss");
    assert_eq!(h.rows(0, 0, stamp(1)), Some(4));
    assert_eq!(h.rows(2, 2, stamp(1)), Some(4));
    h.set_rows(1, 7, 1, stamp(2));
    assert_eq!(h.rows(1, 1, stamp(2)), Some(7));
    assert_eq!(h.rows(0, 0, stamp(1)), Some(4));
    assert_eq!(f(h.span(0..1)), plain(4));
    assert_eq!(f(h.top_of(2)), plain(4) + plain(7) + 2. * COLUMN_GAP);
}

#[test]
fn a_gap_with_nothing_hidden_adds_no_row_height() {
    let mut h = table(2, 3);
    let flat = f(h.total());
    h.set_shapes(vec![
        Shape {
            dateline: false,
            lead_row: false,
            ..Shape::default()
        };
        2
    ]);
    assert_eq!(f(h.total()), flat, "rows survive a reshape");
    h.set_shapes(vec![
        Shape {
            dateline: false,
            lead_row: true,
            ..Shape::default()
        },
        Shape::default(),
    ]);
    assert_eq!(f(h.total()), flat + GAP_ROW);
}

#[test]
fn pins_hold_the_focused_and_the_dragged_piece_once_each() {
    assert!(pinned(None, None).is_empty());
    assert_eq!(pinned(Some(3), None), vec![3]);
    assert_eq!(pinned(None, Some(5)), vec![5]);
    assert_eq!(pinned(Some(3), Some(5)), vec![3, 5]);
    assert_eq!(pinned(Some(4), Some(4)), vec![4]);
}

#[test]
fn a_different_piece_at_the_same_index_does_not_reuse_the_rows() {
    let mut h = table(3, 4);
    assert_eq!(h.rows(1, 1, stamp(1)), Some(4));
    assert_eq!(h.rows(1, 99, stamp(1)), None, "another piece, same stamp");
    h.set_rows(1, 6, 99, stamp(1));
    assert_eq!(h.rows(1, 99, stamp(1)), Some(6));
    assert_eq!(h.rows(1, 1, stamp(1)), None, "the old piece is replaced");
}

#[test]
fn a_viewport_taller_than_the_column_gives_the_whole_range() {
    let h = table(5, 2);
    assert_eq!(drawn_range(px(0.), px(9000.), &h), 0..5);
    assert_eq!(drawn_range(px(300.), px(9000.), &h), 0..5);
}

#[test]
fn a_zero_viewport_draws_the_pieces_at_the_offset() {
    let h = table(10, 2);
    let at = f(h.top_of(4)) + 1.;
    let range = drawn_range(px(at), px(0.), &h);
    assert_eq!(range, 4..5);
}

#[test]
fn a_single_piece_is_always_drawn() {
    let h = table(1, 3);
    for offset in [-10., 0., 50., 5000.] {
        assert_eq!(drawn_range(px(offset), px(600.), &h), 0..1);
    }
}

#[test]
fn unmeasured_pieces_count_as_five_rows() {
    let mut h = Heights::default();
    h.fit(px(400.), px(14.), px(16.), 0, metrics());
    h.set_shapes(vec![Shape::default(); 2]);
    assert_eq!(f(h.height_of(0)), plain(5));
    assert_eq!(f(h.total()), 2. * plain(5) + COLUMN_GAP);
}

#[test]
fn the_range_of_a_long_table_is_cheap() {
    let h = table(5000, 3);
    let max = f(h.total()) - 600.;
    let start = std::time::Instant::now();
    for k in 0..1000 {
        let range = drawn_range(px(max * k as f32 / 1000.), px(600.), &h);
        assert!(!range.is_empty());
    }
    let took = start.elapsed();
    assert!(took.as_millis() < 500, "1000 ranges took {took:?}");
}

#[test]
fn a_later_dateline_keeps_its_margin_and_a_tail_gap_adds_a_row() {
    let mut h = table(3, 2);
    let dated = Shape {
        dateline: true,
        ..Shape::default()
    };
    let tail = Shape {
        tail_hidden: true,
        ..Shape::default()
    };
    h.set_shapes(vec![dated, tail, dated]);
    // The first row of the column has no margin; a later dateline has.
    assert_eq!(f(h.top_of(0)), 0.);
    let a = DATELINE + plain(2);
    // The tail row moves up toward its piece by the pull.
    let b = GAP_ROW - PULL + plain(2);
    assert_eq!(f(h.top_of(1)), a + COLUMN_GAP);
    assert_eq!(f(h.top_of(2)), a + b + 2. * COLUMN_GAP + MARGIN);
    // A spacer for the last piece stands in for its margin too.
    assert_eq!(f(h.span(2..3)), MARGIN + DATELINE + plain(2));
    assert_eq!(f(h.span(0..3)), f(h.total()));
}

#[test]
fn an_unchanged_row_count_keeps_the_table_fresh() {
    let mut h = table(3, 4);
    let total = h.total();
    h.set_rows(1, 4, 1, stamp(1));
    h.set_shapes(vec![Shape::default(); 3]);
    assert!(!h.is_stale(), "the same rows and shapes change nothing");
    assert_eq!(h.total(), total);
}

#[test]
fn a_new_face_clears_the_rows() {
    let mut h = table(3, 4);
    assert_eq!(h.rows(0, 0, stamp(1)), Some(4));
    assert!(
        h.fit(px(400.), px(14.), px(16.), 7, metrics()),
        "a new face"
    );
    assert_eq!(h.rows(0, 0, stamp(1)), None);
    assert!(
        !h.fit(px(400.), px(14.), px(16.), 7, metrics()),
        "same face"
    );
}

#[test]
fn the_first_known_width_keeps_the_rows_and_a_new_width_clears_them() {
    let mut h = Heights::default();
    h.set_metrics(px(14.), px(16.), 0, metrics());
    h.set_shapes(vec![Shape::default(); 3]);
    h.set_rows(0, 4, 0, stamp(1));
    assert!(!h.set_width(px(0.)), "0 is no measure");
    assert_eq!(h.rows(0, 0, stamp(1)), Some(4));
    assert!(!h.set_width(px(400.)), "the first width");
    assert_eq!(h.rows(0, 0, stamp(1)), Some(4));
    assert!(!h.set_width(px(400.004)), "the same width");
    assert!(!h.set_width(px(0.)), "0 again");
    assert_eq!(h.rows(0, 0, stamp(1)), Some(4));
    assert!(h.set_width(px(420.)), "a new width");
    assert_eq!(h.rows(0, 0, stamp(1)), None);
}

/// A gap between two pieces draws two rows: one above the crumbs and the
/// up row under them. A gap above a note's first piece draws the up row
/// only.
#[test]
fn a_split_gap_adds_both_rows_and_a_first_gap_adds_the_up_row_only() {
    let mut h = table(2, 3);
    let flat = f(h.total());
    let between = Shape {
        lead_row: true,
        up_row: true,
        ..Shape::default()
    };
    h.set_shapes(vec![Shape::default(), between]);
    assert_eq!(f(h.total()), flat + GAP_ROW + UP_ROW - PULL);
    let first = Shape {
        up_row: true,
        ..Shape::default()
    };
    h.set_shapes(vec![first, Shape::default()]);
    assert_eq!(f(h.total()), flat + UP_ROW);
}

/// `j` and `k` skip the row above the crumbs, never the up row: the up
/// row lies under the crumbs.
#[test]
fn the_step_top_skips_the_row_above_the_crumbs_only() {
    let mut h = table(3, 2);
    let between = Shape {
        lead_row: true,
        up_row: true,
        ..Shape::default()
    };
    let first = Shape {
        up_row: true,
        ..Shape::default()
    };
    h.set_shapes(vec![first, between, Shape::default()]);
    assert_eq!(f(h.step_top(0)), 0.);
    assert_eq!(f(h.step_top(1)), f(h.top_of(1)) + GAP_ROW);
    assert_eq!(f(h.step_top(2)), f(h.top_of(2)));
}

/// The up row lies under the crumbs: the reveal scrolls to it there.
#[test]
fn the_up_row_spans_from_the_crumbs_to_the_text() {
    let mut h = table(2, 2);
    let between = Shape {
        lead_row: true,
        up_row: true,
        ..Shape::default()
    };
    h.set_shapes(vec![Shape::default(), between]);
    let top = f(h.top_of(1));
    let (lead, lead_end) = h.row_span(Row::Lead(1));
    assert_eq!((f(lead), f(lead_end)), (top, top + GAP_ROW - COLUMN_GAP));
    let (crumbs, crumbs_end) = h.row_span(Row::Crumbs(1));
    assert_eq!(
        (f(crumbs), f(crumbs_end)),
        (top + GAP_ROW, top + GAP_ROW + CRUMBS)
    );
    let (up, up_end) = h.row_span(Row::Up(1));
    assert_eq!((f(up), f(up_end)), (f(crumbs_end), f(crumbs_end) + UP_ROW));
}
