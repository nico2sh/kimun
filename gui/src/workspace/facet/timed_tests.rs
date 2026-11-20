//! Tests for the timed notes of the facet view.
//!
//! They check the timer helper on its own, then the two notes it ends:
//! the `Undo` offer after a clear and the report of a refresh.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext};
use kimun_core::nfs::VaultPath;

use super::timed::Timed;
use super::{CLEAR_ID, CLEARED_ID, COLLECTION_LINE_ID, FacetView, MemberInit, UNDO_ID};
use crate::workspace::facet_tests::{click, label, open, open_with, pick_from_menu, shows, tag};

const NOTE: &str = "# Head\n\nsome #a\n\nmore #b\n";

/// Move the clock on by `secs` seconds and run what is due.
fn wait(cx: &mut VisualTestContext, secs: u64) {
    cx.executor().advance_clock(Duration::from_secs(secs));
    cx.run_until_parked();
}

fn undo_timer(view: &gpui_kit::Entity<FacetView>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |view, _| view.undo.timer_active())
}

#[gpui_kit::test]
fn a_fired_timer_is_not_active_without_a_cancel(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![]);
    let mut timed = Timed::default();
    // The callback does not call `cancel`.
    view.update(cx, |_, cx| {
        timed.start(Duration::from_secs(2), cx, |_, _| {})
    });
    wait(cx, 1);
    assert!(timed.is_active(), "it runs before its time");
    wait(cx, 1);
    assert!(!timed.is_active(), "it ended when it fired");
    // A new start runs again.
    view.update(cx, |_, cx| {
        timed.start(Duration::from_secs(2), cx, |_, _| {})
    });
    assert!(timed.is_active());
}

#[gpui_kit::test]
fn a_second_start_replaces_the_first_timer(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![]);
    let first = Rc::new(Cell::new(false));
    let second = Rc::new(Cell::new(false));
    let mut timed = Timed::default();
    let (a, b) = (first.clone(), second.clone());
    view.update(cx, |_, cx| {
        timed.start(Duration::from_secs(2), cx, move |_, _| a.set(true));
        timed.start(Duration::from_secs(5), cx, move |_, _| b.set(true));
    });
    cx.run_until_parked();
    // The first timer went with its callback.
    assert_eq!(Rc::strong_count(&first), 1);
    assert!(timed.is_active());
    wait(cx, 3);
    assert!(!first.get(), "the first timer never fires");
    assert!(!second.get());
    wait(cx, 2);
    assert!(second.get(), "the second timer fires at its own time");
    timed.cancel();
    assert!(!timed.is_active());
}

#[gpui_kit::test]
fn dropping_the_view_cancels_the_timer(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a")]);
    // A second view, held only here.
    let view = cx.new(|cx| FacetView::new(VaultPath::new("/notes/m.md"), NOTE.into(), vec![], cx));
    let fired = Rc::new(Cell::new(false));
    let probe = fired.clone();
    view.update(cx, |view, cx| {
        view.undo.offer(cx, move |_, _| probe.set(true));
    });
    assert_eq!(Rc::strong_count(&fired), 2, "the timer holds its callback");
    let weak = view.downgrade();
    drop(view);
    cx.run_until_parked();
    assert!(weak.upgrade().is_none(), "the view is gone");
    // Released state drops at the next flush.
    cx.update(|_, _| {});
    cx.run_until_parked();
    assert_eq!(Rc::strong_count(&fired), 1, "the timer went with the view");
    wait(cx, 9);
    assert!(!fired.get());
}

#[gpui_kit::test]
fn the_undo_offer_ends_after_eight_seconds(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a")]);
    click(cx, CLEAR_ID);
    assert!(shows(cx, UNDO_ID));
    assert!(undo_timer(&view, cx));
    wait(cx, 7);
    assert!(shows(cx, CLEARED_ID), "still there after 7 seconds");
    assert!(undo_timer(&view, cx));
    wait(cx, 1);
    assert!(!shows(cx, UNDO_ID));
    assert!(!shows(cx, CLEARED_ID));
    assert!(!undo_timer(&view, cx), "the ended offer keeps no timer");
}

#[gpui_kit::test]
fn the_refresh_report_ends_after_six_seconds(cx: &mut TestAppContext) {
    let note = |path: &str| MemberInit {
        path: VaultPath::new(path),
        title: "A".into(),
        text: "#a\n".into(),
    };
    let first = vec![note("/a.md")];
    let (view, cx, _events, _sub) = open_with(cx, move |cx| {
        FacetView::new_collection_at("#a".into(), first, 1, vec![], ten, cx)
    });
    let refreshed = vec![note("/a.md"), note("/b.md")];
    view.update(cx, |view, cx| view.replace_members(refreshed, 2, cx));
    let report = Some("Refreshed: 1 note added".to_string());
    assert_eq!(label(cx, COLLECTION_LINE_ID), report);
    assert!(view.read_with(cx, |view, _| view.refresh_report_timer()));
    wait(cx, 5);
    assert_eq!(label(cx, COLLECTION_LINE_ID), report, "still there");
    wait(cx, 1);
    assert_ne!(label(cx, COLLECTION_LINE_ID), report);
    assert!(!view.read_with(cx, |view, _| view.refresh_report_timer()));
}

#[gpui_kit::test]
fn a_pick_ends_the_undo_offer_and_cancels_its_timer(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![tag("a")]);
    click(cx, CLEAR_ID);
    assert!(undo_timer(&view, cx));
    pick_from_menu(&view, cx, &tag("b"));
    assert!(!shows(cx, UNDO_ID));
    assert!(!undo_timer(&view, cx), "the pick dropped the timer");
}

/// The test clock: the time of every load.
fn ten() -> chrono::NaiveTime {
    chrono::NaiveTime::from_hms_opt(10, 0, 0).unwrap()
}
