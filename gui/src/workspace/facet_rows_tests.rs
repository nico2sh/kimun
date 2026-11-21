//! Tests that the height table's fixed row heights are the heights of the
//! rows the view draws, at several rem sizes. The table and the drawing
//! read the same numbers ([`super::facet::rows`]).

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Entity, Pixels, TestAppContext, VisualTestContext, px, size};
use kimun_core::nfs::VaultPath;

use super::facet::rows;
use super::facet::{CLEAR_ID, COLUMN_ID, FacetView, MemberInit, chunk_id, dateline_id, pill_id};
use super::facet_tests::{open_with, settle, tag};
use super::facet_window_view_tests::{clock, draw, member_text};

/// A collection of 3 notes, all drawn, at `rem` pixels per rem.
fn three(cx: &mut TestAppContext, rem: f32) -> (Entity<FacetView>, &mut VisualTestContext) {
    let members: Vec<MemberInit> = (0..3)
        .map(|m| MemberInit {
            path: VaultPath::new(format!("/notes/n{m}.md")),
            title: format!("Note {m}"),
            text: member_text(m),
        })
        .collect();
    let (view, cx, _events, _sub) = open_with(cx, move |cx| {
        FacetView::new_collection_at("#q".into(), members, 3, vec![tag("a")], clock, cx)
    });
    cx.simulate_resize(size(px(700.), px(4000.)));
    cx.update(|window, _| window.set_rem_size(px(rem)));
    settle(cx);
    draw(cx);
    (view, cx)
}

fn top(cx: &mut VisualTestContext, id: gpui_kit::SharedString) -> f32 {
    cx.update(|window, _| f32::from(window.find(id).bounds().top()))
}

fn bottom(cx: &mut VisualTestContext, id: gpui_kit::SharedString) -> f32 {
    cx.update(|window, _| f32::from(window.find(id).bounds().bottom()))
}

fn close(what: &str, drawn: f32, table: Pixels) {
    let table = f32::from(table);
    assert!(
        (drawn - table).abs() <= 0.5,
        "{what}: drawn {drawn}, table {table}"
    );
}

/// Each fixed height of the table, read off the drawn rows of a
/// collection: the dateline, the gap between items, a gap row, a tail
/// row with the margin above the next dateline, and every piece's crumbs
/// and inset around its rows.
fn assert_rows_match(view: &Entity<FacetView>, cx: &mut VisualTestContext, rem: f32) {
    let m = view.read_with(cx, |v, _| v.table_metrics());
    assert!(
        view.read_with(cx, |v, _| v.checked_items()) > 0,
        "{rem}: checked"
    );
    // Piece 0 has no gap row: its note's dateline, then the piece.
    let gap = top(cx, chunk_id(0)) - bottom(cx, dateline_id(0));
    close(&format!("{rem}: column gap"), gap, m.column_gap);
    let dateline = top(cx, chunk_id(0)) - top(cx, dateline_id(0));
    close(&format!("{rem}: dateline"), dateline, m.dateline);
    // A gap row stands between pieces 0 and 1.
    let row = top(cx, chunk_id(1)) - bottom(cx, chunk_id(0)) - gap;
    close(&format!("{rem}: gap row"), row, m.gap_row);
    // Piece 2 ends note 0 with a tail row; note 1's dateline has its margin.
    let after = top(cx, dateline_id(1)) - bottom(cx, chunk_id(2));
    close(
        &format!("{rem}: tail and margin"),
        after,
        m.gap_row + m.column_gap + m.dateline_margin,
    );
    // Each piece: crumbs, rows, inset.
    let n = view.read_with(cx, |v, _| v.chunks().len());
    for i in 0..n {
        let rows = view.read_with(cx, |v, cx| {
            v.chunks()[i]
                .read(cx)
                .editor
                .session()
                .display
                .layout
                .visual_line_count()
                .max(1)
        });
        let height = bottom(cx, chunk_id(i)) - top(cx, chunk_id(i));
        close(
            &format!("{rem}: piece {i}"),
            height,
            m.crumbs + m.inset + m.line * rows as f32,
        );
    }
}

#[gpui_kit::test]
fn the_table_rows_are_the_drawn_rows_at_rem_12(cx: &mut TestAppContext) {
    let (view, cx) = three(cx, 12.);
    assert_rows_match(&view, cx, 12.);
}

#[gpui_kit::test]
fn the_table_rows_are_the_drawn_rows_at_rem_16(cx: &mut TestAppContext) {
    let (view, cx) = three(cx, 16.);
    assert_rows_match(&view, cx, 16.);
}

#[gpui_kit::test]
fn the_table_rows_are_the_drawn_rows_at_rem_20(cx: &mut TestAppContext) {
    let (view, cx) = three(cx, 20.);
    assert_rows_match(&view, cx, 20.);
}

/// One shared number moves the drawn rows and the table together: taller
/// buttons make the gap rows and the crumbs rows taller in both.
#[gpui_kit::test]
fn a_shared_row_number_moves_the_drawn_row_and_the_table_together(cx: &mut TestAppContext) {
    let (view, cx) = three(cx, 16.);
    let before = view.read_with(cx, |v, _| v.table_metrics());
    let piece = bottom(cx, chunk_id(0)) - top(cx, chunk_id(0));
    rows::set_override(rows::BUTTON, Some(2.));
    settle(cx);
    draw(cx);
    let after = view.read_with(cx, |v, _| v.table_metrics());
    let grown = bottom(cx, chunk_id(0)) - top(cx, chunk_id(0)) - piece;
    rows::set_override(rows::BUTTON, None);
    // Half a rem more: 8 px at 16 px a rem.
    close("drawn crumbs", grown, px(8.));
    close(
        "table crumbs",
        f32::from(after.crumbs - before.crumbs),
        px(8.),
    );
    close(
        "table gap row",
        f32::from(after.gap_row - before.gap_row),
        px(8.),
    );
    assert_rows_match(&view, cx, 16.);
}

fn height(cx: &mut VisualTestContext, id: gpui_kit::SharedString) -> f32 {
    bottom(cx, id.clone()) - top(cx, id)
}

/// The bar's pills and chips (`Clear` here) read the one button size: moved, both are
/// still as tall as each other, and both moved by the same amount.
#[gpui_kit::test]
fn a_pill_and_a_chip_follow_the_one_button_size(cx: &mut TestAppContext) {
    let (_view, cx) = three(cx, 16.);
    let (pill, chip) = (pill_id(&tag("a")), gpui_kit::SharedString::from(CLEAR_ID));
    let (pill0, chip0) = (height(cx, pill.clone()), height(cx, chip.clone()));
    assert_eq!(pill0, chip0, "one height at rest");
    rows::set_override(rows::BUTTON, Some(2.));
    settle(cx);
    let (pill1, chip1) = (height(cx, pill), height(cx, chip));
    rows::set_override(rows::BUTTON, None);
    assert_eq!(pill1, chip1, "one height when moved");
    assert_eq!(pill1 - pill0, 8., "half a rem more at 16 px a rem");
    assert_eq!(chip1 - chip0, pill1 - pill0, "both moved together");
}

/// The column's padding and gap move the drawn column and the table
/// together.
#[gpui_kit::test]
fn the_column_pad_and_gap_move_the_drawn_column_and_the_table_together(cx: &mut TestAppContext) {
    let (view, cx) = three(cx, 16.);
    rows::set_override(rows::COLUMN_PAD, Some(1.5));
    rows::set_override(rows::COLUMN_GAP, Some(1.25));
    settle(cx);
    draw(cx);
    let pad = top(cx, dateline_id(0)) - top(cx, COLUMN_ID.into());
    let m = view.read_with(cx, |v, _| v.table_metrics());
    rows::set_override(rows::COLUMN_PAD, None);
    rows::set_override(rows::COLUMN_GAP, None);
    assert_eq!(pad, 24., "the drawn padding");
    close("the table gap", 20., m.column_gap);
    // Every row still matches: the drawn gap is the table's gap.
    assert_rows_match(&view, cx, 16.);
}
