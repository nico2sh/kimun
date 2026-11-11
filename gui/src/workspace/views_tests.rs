//! UI integration tests for the LINKS and OUTLINE drawer views (M7a G1): a
//! real `Workspace` on a real, indexed temp vault, driven through the
//! window's keys and pointer.
//!
//! Like the other vault tests, each test owns a current-thread tokio runtime
//! and drives it between GPUI's `run_until_parked` (`start_tests::drive`).
#![cfg(test)]

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, Focusable as _, SharedString, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::layout::{Panel, Side};
use kimun_notes::logic::message::AppMessage;
use kimun_notes::logic::nav::{DrawerView, LinksTab};
use kimun_notes::logic::views::{LinkEntry, OutlineEntry};
use tokio::runtime::Runtime;

use super::drawer_list::{DrawerList, DrawerListEvent, chevron_id};
use super::file_dialogs::FileOpKind;
use super::file_ops::FileOpRequest;
use super::links::{LINKS_ID, LinksView, NO_NOTE_TEXT, link_row_id};
use super::modal::ModalKind;
use super::outline::{OUTLINE_ID, OutlineView, heading_row_id};
use super::rail_cell_id;
use super::start_tests::{Started, drive, open_started, runtime, send, vault_start};
use super::start_tests::{copy_key, select_all_key};
use crate::startup::FirstNote;
use crate::status_row::BACKLINKS_ID;
use crate::test_vault::{TestVault, test_vault};

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

fn disk(rt: &Runtime, v: &TestVault, p: &str) -> Option<String> {
    rt.block_on(v.vault.get_note_text(&path(p))).ok()
}

/// A vault holding `notes`, started on `first`, driven until its text shows
/// and the index is built (LINKS reads it).
fn started_on(
    cx: &mut TestAppContext,
    rt: &Runtime,
    notes: &[(&str, &str)],
    first: &str,
) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(notes, &[]));
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path(first))),
        v.settings.clone(),
    );
    let want = notes.iter().find(|(p, _)| *p == first).unwrap().1;
    drive(rt, cx, |cx| {
        started.text(cx) == want && !started.indexing(cx)
    });
    (v, started)
}

/// The LINKS fixture: `alpha` links to `beta` (twice) and `delta`; `beta`
/// and `epsilon` link to `alpha`; `gamma` mentions it by name only.
const LINKED: &[(&str, &str)] = &[
    (
        "alpha.md",
        "# Alpha notes\nsee [[beta]], [[delta]] and [[beta]] again",
    ),
    ("beta.md", "# Beta\nback to [[alpha]]"),
    ("delta.md", "# Delta\nnothing here"),
    ("epsilon.md", "# Epsilon\n[[alpha]] too"),
    ("gamma.md", "# Gamma\nalpha is mentioned here"),
];

fn press(s: &Started, key: &str, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| window.press(key, cx))
        .unwrap();
    cx.run_until_parked();
}

fn type_in(s: &Started, text: &str, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| window.input(text, cx))
        .unwrap();
    cx.run_until_parked();
}

fn click(s: &Started, id: impl Into<SharedString>, cx: &mut TestAppContext) {
    let id = id.into();
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn links(s: &Started, cx: &TestAppContext) -> Entity<LinksView> {
    cx.read(|cx| s.workspace.read(cx).links.clone().expect("LINKS is built"))
}

fn links_list(s: &Started, cx: &TestAppContext) -> Entity<DrawerList<LinkEntry>> {
    let view = links(s, cx);
    cx.read(|cx| view.read(cx).list.clone())
}

fn outline(s: &Started, cx: &TestAppContext) -> Entity<OutlineView> {
    cx.read(|cx| {
        s.workspace
            .read(cx)
            .outline
            .clone()
            .expect("OUTLINE is built")
    })
}

fn outline_list(s: &Started, cx: &TestAppContext) -> Entity<DrawerList<OutlineEntry>> {
    let view = outline(s, cx);
    cx.read(|cx| view.read(cx).list.clone())
}

fn tab(s: &Started, cx: &TestAppContext) -> LinksTab {
    let view = links(s, cx);
    cx.read(|cx| view.read(cx).tab())
}

/// LINKS' rows once they have landed, as sorted note names.
fn link_rows(s: &Started, rt: &Runtime, cx: &mut TestAppContext) -> Vec<String> {
    let list = links_list(s, cx);
    drive(rt, cx, |cx| !cx.read(|cx| list.read(cx).is_loading()));
    let mut rows: Vec<String> = cx.read(|cx| {
        list.read(cx)
            .rows()
            .iter()
            .map(|r| r.path.get_clean_name())
            .collect()
    });
    rows.sort();
    rows
}

fn link_empty_text(s: &Started, cx: &TestAppContext) -> String {
    let list = links_list(s, cx);
    cx.read(|cx| list.read(cx).empty_text().to_string())
}

/// OUTLINE's rows: each heading indented two spaces per nesting step.
fn outline_rows(s: &Started, cx: &TestAppContext) -> Vec<String> {
    let list = outline_list(s, cx);
    cx.read(|cx| {
        list.read(cx)
            .rows()
            .iter()
            .map(|r| format!("{}{}", "  ".repeat(r.depth - 1), r.heading()))
            .collect()
    })
}

fn open_note(s: &Started, cx: &TestAppContext) -> Option<VaultPath> {
    cx.read(|cx| s.workspace.read(cx).notes.as_ref()?.note.clone())
}

fn on(s: &Started, p: &str, cx: &TestAppContext) -> bool {
    open_note(s, cx).is_some_and(|n| n.is_like(&path(p)))
}

fn shown(s: &Started, view: DrawerView, cx: &TestAppContext) -> bool {
    cx.read(|cx| s.workspace.read(cx).is_shown(view))
}

fn region(s: &Started, cx: &TestAppContext) -> Panel {
    cx.read(|cx| s.workspace.read(cx).focused_panel())
}

fn list_focused<R: super::drawer_list::DrawerRow>(
    s: &Started,
    list: &Entity<DrawerList<R>>,
    cx: &mut TestAppContext,
) -> bool {
    let list = list.clone();
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        list.read(cx).focus_handle(cx).is_focused(window)
    })
    .unwrap()
}

fn editor_focused(s: &Started, cx: &mut TestAppContext) -> bool {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.find("editor").focused() == Some(true)
    })
    .unwrap()
}

fn caret(s: &Started, cx: &TestAppContext) -> (usize, usize) {
    cx.read(|cx| s.workspace.read(cx).editor.read(cx).editor.cursor())
}

/// Run a leader sequence: the gateway, then `keys`.
fn leader(s: &Started, keys: &str, cx: &mut TestAppContext) {
    press(s, "ctrl-shift-b", cx);
    type_in(s, keys, cx);
}

fn settle(s: &Started, cx: &mut TestAppContext) {
    s.settle_dialog(cx);
}

// ── LINKS ────────────────────────────────────────────────────────────────

/// Backlinks, outgoing links (each target once, missing ones too) and
/// unlinked mentions (not the linkers, not the note itself), one tab at a
/// time; `b`, `o`, `u` and Left/Right switch tabs while the list has the
/// keyboard.
#[gpui_kit::test]
fn links_lists_each_tab_and_the_keys_switch_tabs(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, LINKED, "alpha.md");
    leader(&started, "lb", cx);
    assert!(shown(&started, DrawerView::Links, cx));
    assert!(list_focused(&started, &links_list(&started, cx), cx));
    assert_eq!(tab(&started, cx), LinksTab::Backlinks);
    assert_eq!(link_rows(&started, &rt, cx), ["beta", "epsilon"]);

    press(&started, "o", cx);
    assert_eq!(tab(&started, cx), LinksTab::Outgoing);
    assert_eq!(link_rows(&started, &rt, cx), ["beta", "delta"]);

    press(&started, "u", cx);
    assert_eq!(tab(&started, cx), LinksTab::Unlinked);
    assert_eq!(link_rows(&started, &rt, cx), ["gamma"]);

    press(&started, "left", cx);
    assert_eq!(tab(&started, cx), LinksTab::Outgoing);
    press(&started, "right", cx);
    press(&started, "right", cx);
    assert_eq!(tab(&started, cx), LinksTab::Backlinks, "wraps");
    press(&started, "b", cx);
    assert_eq!(link_rows(&started, &rt, cx), ["beta", "epsilon"]);
    assert_eq!(started.text(cx), LINKED[0].1, "nothing typed into the note");
}

/// A click on a tab of the segmented control switches to it and leaves the
/// keyboard on the list.
#[gpui_kit::test]
fn a_tab_click_switches_tabs(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, LINKED, "alpha.md");
    click(&started, rail_cell_id(DrawerView::Links), cx);
    assert!(shown(&started, DrawerView::Links, cx));
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .within(super::links::LINKS_TABS_ID)
            .click(gpui_kit::ElementId::from(2usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(tab(&started, cx), LinksTab::Unlinked);
    assert_eq!(link_rows(&started, &rt, cx), ["gamma"]);
    assert!(list_focused(&started, &links_list(&started, cx), cx));
}

/// Enter opens the selected row through the note session: the open note's
/// edits are saved first, and the editor takes the keyboard.
#[gpui_kit::test]
fn enter_opens_the_row_saving_the_open_note_first(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, LINKED, "alpha.md");
    type_in(&started, "X", cx);
    leader(&started, "lo", cx);
    assert_eq!(link_rows(&started, &rt, cx), ["beta", "delta"]);
    let list = links_list(&started, cx);
    let first = cx.read(|cx| list.read(cx).rows()[0].path.clone());
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| {
        open_note(&started, cx).as_ref() == Some(&first)
    });
    assert!(
        disk(&rt, &v, "alpha.md").is_some_and(|t| t.starts_with('X')),
        "saved before the switch"
    );
    assert!(editor_focused(&started, cx));
}

/// A click opens a row, as Enter does (C1).
#[gpui_kit::test]
fn a_click_opens_a_row(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, LINKED, "alpha.md");
    leader(&started, "lb", cx);
    link_rows(&started, &rt, cx);
    let id = link_row_id(&path("epsilon.md"));
    let list = links_list(&started, cx);
    let row = cx.read(|cx| {
        list.read(cx)
            .rows()
            .into_iter()
            .find(|r| r.path.is_like(&path("epsilon.md")))
            .map(|r| link_row_id(&r.path))
            .unwrap_or(id)
    });
    click(&started, row, cx);
    drive(&rt, cx, |cx| on(&started, "epsilon.md", cx));
}

/// The element id of the LINKS row for note `p`.
fn links_row(s: &Started, p: &str, cx: &TestAppContext) -> SharedString {
    let list = links_list(s, cx);
    cx.read(|cx| {
        list.read(cx)
            .rows()
            .into_iter()
            .find(|r| r.path.is_like(&path(p)))
            .map(|r| link_row_id(&r.path))
            .unwrap_or_else(|| panic!("no row {p}"))
    })
}

/// A double-click opens a row once: its second press is not a second
/// activation (the note opened, the list stays as it was).
#[gpui_kit::test]
fn a_double_click_opens_a_row_once(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, LINKED, "alpha.md");
    leader(&started, "lb", cx);
    link_rows(&started, &rt, cx);
    let list = links_list(&started, cx);
    let opened = Rc::new(Cell::new(0));
    let count = opened.clone();
    let _counting = cx.update(|cx| {
        cx.subscribe(&list, move |_, event: &DrawerListEvent<LinkEntry>, _| {
            if matches!(event, DrawerListEvent::Activate(_)) {
                count.set(count.get() + 1);
            }
        })
    });
    let row = links_row(&started, "epsilon.md", cx);
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.double_click(row, cx)
    })
    .unwrap();
    drive(&rt, cx, |cx| on(&started, "epsilon.md", cx));
    assert_eq!(opened.get(), 1, "one activation");
}

/// The expand chevron is off by default and LINKS never switches it on:
/// neither the hovered nor the selected row draws one, and a click still
/// opens the row.
#[gpui_kit::test]
fn links_rows_draw_no_chevron(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, LINKED, "alpha.md");
    leader(&started, "lb", cx);
    link_rows(&started, &rt, cx);
    let row = links_row(&started, "epsilon.md", cx);
    let chevrons = cx
        .update_window(started.handle, |_, window, cx| {
            window.render_frame(cx);
            window.hover(row.clone(), cx);
            window.render_frame(cx);
            window.find_all(chevron_id(&row)).len()
        })
        .unwrap();
    assert_eq!(chevrons, 0);
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(row, cx)
    })
    .unwrap();
    drive(&rt, cx, |cx| on(&started, "epsilon.md", cx));
}

/// A right click is the entry menu's: it opens nothing.
#[gpui_kit::test]
fn a_right_click_opens_no_row(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, LINKED, "alpha.md");
    leader(&started, "lb", cx);
    link_rows(&started, &rt, cx);
    let row = links_row(&started, "epsilon.md", cx);
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(row, cx);
    })
    .unwrap();
    cx.run_until_parked();
    press(&started, "escape", cx);
    drive(&rt, cx, |_| true);
    assert!(on(&started, "alpha.md", cx), "alpha stays open");
}

/// LINKS follows the open note: another note opened, the note renamed (it
/// lists for the new path), the note deleted ("No note open").
#[gpui_kit::test]
fn links_follows_a_switch_a_rename_and_a_delete(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, LINKED, "alpha.md");
    // In the right slot, so the FILES a delete shows does not hide it.
    started.workspace.update(cx, |w, cx| {
        w.move_to_side(DrawerView::Links, Side::Right);
        w.refresh_views(cx);
    });
    assert!(shown(&started, DrawerView::Links, cx));
    assert_eq!(link_rows(&started, &rt, cx), ["beta", "epsilon"]);

    send(&started, AppMessage::open(path("beta.md")), cx);
    drive(&rt, cx, |cx| on(&started, "beta.md", cx));
    assert_eq!(link_rows(&started, &rt, cx), ["alpha"]);

    let request = FileOpRequest::Rename {
        from: path("beta.md"),
        to: path("bravo.md"),
    };
    let workspace = started.workspace.clone();
    cx.update_window(started.handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.request_file_op(request, window, cx))
    })
    .unwrap();
    drive(&rt, cx, |cx| on(&started, "bravo.md", cx));
    let view = links(&started, cx);
    let listed = cx.read(|cx| view.read(cx).listed_note().cloned());
    assert!(listed.is_some_and(|n| n.is_like(&path("bravo.md"))));
    let list = links_list(&started, cx);
    drive(&rt, cx, |cx| !cx.read(|cx| list.read(cx).is_loading()));

    let request = FileOpRequest::Delete(path("bravo.md"));
    let workspace = started.workspace.clone();
    cx.update_window(started.handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.request_file_op(request, window, cx))
    })
    .unwrap();
    drive(&rt, cx, |cx| open_note(&started, cx).is_none());
    assert!(shown(&started, DrawerView::Links, cx));
    assert!(link_rows(&started, &rt, cx).is_empty());
    assert_eq!(link_empty_text(&started, cx), NO_NOTE_TEXT);
}

/// A LINKS row has the FILES entry menu: Rename… opens the rename dialog
/// for the row's note (not the open one).
#[gpui_kit::test]
fn a_links_row_has_the_entry_menu(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, LINKED, "alpha.md");
    leader(&started, "lb", cx);
    link_rows(&started, &rt, cx);
    let list = links_list(&started, cx);
    let row = link_row_id(&cx.read(|cx| {
        list.read(cx)
            .rows()
            .into_iter()
            .find(|r| r.path.is_like(&path("beta.md")))
            .unwrap()
            .path
    }));
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(row, cx);
    })
    .unwrap();
    cx.run_until_parked();
    press(&started, "down", cx);
    press(&started, "enter", cx);
    settle(&started, cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Rename));
    let target = cx.read(|cx| match started.workspace.read(cx).file_dialog.clone() {
        Some(super::file_ops::FileDialogView::Rename(view)) => {
            view.upgrade().unwrap().read(cx).target().clone()
        }
        _ => panic!("no rename dialog"),
    });
    assert!(target.is_like(&path("beta.md")));
    let _ = FileOpKind::Rename;
}

/// The Footer's backlink count shows LINKS on its Backlinks tab, with the
/// keyboard.
#[gpui_kit::test]
fn the_footer_backlinks_count_shows_links(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, LINKED, "alpha.md");
    leader(&started, "lu", cx);
    press(&started, "escape", cx);
    click(&started, rail_cell_id(DrawerView::Links), cx);
    assert!(!shown(&started, DrawerView::Links, cx));
    drive(&rt, cx, |cx| started.shows(BACKLINKS_ID, cx));
    click(&started, BACKLINKS_ID, cx);
    assert!(shown(&started, DrawerView::Links, cx));
    assert_eq!(tab(&started, cx), LinksTab::Backlinks);
    assert_eq!(region(&started, cx), Panel::Drawer(Side::Left));
    assert!(list_focused(&started, &links_list(&started, cx), cx));
    assert_eq!(link_rows(&started, &rt, cx), ["beta", "epsilon"]);
}

/// The leader's `l o` shows Outgoing; the palette's links rows run.
#[gpui_kit::test]
fn the_leader_and_the_palette_pick_the_tab(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, LINKED, "alpha.md");
    leader(&started, "lo", cx);
    assert_eq!(tab(&started, cx), LinksTab::Outgoing);
    assert_eq!(link_rows(&started, &rt, cx), ["beta", "delta"]);

    press(&started, "ctrl-shift-p", cx);
    type_in(&started, "links unlinked", cx);
    press(&started, "enter", cx);
    settle(&started, cx);
    assert_eq!(tab(&started, cx), LinksTab::Unlinked);
    assert_eq!(link_rows(&started, &rt, cx), ["gamma"]);
}

/// LINKS, OUTLINE, FIND, TAGS and CFG draw their views, not the
/// placeholder.
#[gpui_kit::test]
fn the_placeholder_is_gone_for_links_and_outline(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, LINKED, "alpha.md");
    click(&started, rail_cell_id(DrawerView::Links), cx);
    assert!(started.shows(LINKS_ID, cx));
    click(&started, rail_cell_id(DrawerView::Outline), cx);
    assert!(started.shows(OUTLINE_ID, cx));
    assert!(!started.shows(LINKS_ID, cx));
    let built = |view: DrawerView, cx: &TestAppContext| {
        cx.read(|cx| started.workspace.read(cx).view_content(view).is_some())
    };
    for view in [
        DrawerView::Links,
        DrawerView::Outline,
        DrawerView::Find,
        DrawerView::Tags,
        DrawerView::Config,
    ] {
        assert!(built(view, cx), "{view:?} is built");
    }
}

/// Hidden views do no work: opening notes, typing and saving read nothing
/// for LINKS or OUTLINE until they are shown.
#[gpui_kit::test]
fn hidden_views_do_no_work(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, LINKED, "alpha.md");
    type_in(&started, "# New\n", cx);
    press(&started, "ctrl-s", cx);
    drive(&rt, cx, |cx| {
        !cx.read(|cx| started.workspace.read(cx).editor.read(cx).editor.is_dirty())
    });
    send(&started, AppMessage::open(path("beta.md")), cx);
    drive(&rt, cx, |cx| on(&started, "beta.md", cx));
    let list = links_list(&started, cx);
    assert!(
        !cx.read(|cx| list.read(cx).is_started()),
        "LINKS loaded nothing"
    );
    let view = links(&started, cx);
    assert_eq!(cx.read(|cx| view.read(cx).listed_note().cloned()), None);
    let view = outline(&started, cx);
    assert_eq!(cx.read(|cx| view.read(cx).model().revision()), None);
    let list = outline_list(&started, cx);
    assert!(!cx.read(|cx| list.read(cx).is_started()));

    // Shown, they catch up at once.
    leader(&started, "ol", cx);
    assert_eq!(outline_rows(&started, cx), ["Beta"]);
}

// ── OUTLINE ──────────────────────────────────────────────────────────────

const OUTLINED: &str = "# Top\nintro\n## First\none\n#### Deep\nx\n## Second\ntwo\n# Other";

/// OUTLINE lists the headings nested by enclosing heading (not by markdown
/// level); Enter puts the caret on the heading's line and gives the editor
/// the keyboard.
#[gpui_kit::test]
fn outline_lists_nested_headings_and_enter_jumps(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("o.md", OUTLINED)], "o.md");
    leader(&started, "fh", cx);
    assert!(shown(&started, DrawerView::Outline, cx));
    assert!(list_focused(&started, &outline_list(&started, cx), cx));
    assert_eq!(
        outline_rows(&started, cx),
        ["Top", "  First", "    Deep", "  Second", "Other"]
    );
    press(&started, "down", cx);
    press(&started, "down", cx);
    press(&started, "enter", cx);
    assert_eq!(caret(&started, cx).0, 4, "on `#### Deep`");
    assert!(editor_focused(&started, cx));

    // End and Enter; then a click on a row jumps at once (C1).
    leader(&started, "ol", cx);
    press(&started, "end", cx);
    press(&started, "enter", cx);
    assert_eq!(caret(&started, cx).0, 8, "on `# Other`");
    leader(&started, "ol", cx);
    click(
        &started,
        heading_row_id(&outline_entry(&started, "Second", cx)),
        cx,
    );
    assert_eq!(caret(&started, cx).0, 6, "on `## Second`");
    assert!(
        editor_focused(&started, cx),
        "the click gave the editor the keyboard"
    );
}

fn outline_entry(
    s: &Started,
    heading: &str,
    cx: &TestAppContext,
) -> kimun_notes::logic::nav::HeadingTarget {
    let list = outline_list(s, cx);
    cx.read(|cx| {
        list.read(cx)
            .rows()
            .into_iter()
            .find(|r| r.heading() == heading)
            .map(|r| r.target)
            .unwrap_or_else(|| panic!("no heading {heading}"))
    })
}

/// A heading renamed in the buffer since it was listed: the jump says it is
/// gone, and the caret stays.
#[gpui_kit::test]
fn a_deleted_heading_flashes(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("o.md", "# Top\n## Gone\ntext")], "o.md");
    // In the right slot, so the editor can be typed in while it shows.
    started.workspace.update(cx, |w, cx| {
        w.move_to_side(DrawerView::Outline, Side::Right);
        w.refresh_views(cx);
    });
    assert_eq!(outline_rows(&started, cx), ["Top", "  Gone"]);
    // Rename `## Gone` to `## Gons` (the caret starts on line 1).
    press(&started, "down", cx);
    press(&started, "end", cx);
    press(&started, "backspace", cx);
    type_in(&started, "s", cx);
    assert_eq!(started.text(cx), "# Top\n## Gons\ntext");
    assert_eq!(
        outline_rows(&started, cx),
        ["Top", "  Gone"],
        "typing syncs nothing"
    );
    let target = outline_entry(&started, "Gone", cx);
    let before = caret(&started, cx);
    let workspace = started.workspace.clone();
    cx.update_window(started.handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.jump_to_heading(&target, window, cx))
    })
    .unwrap();
    assert_eq!(
        started.flash(cx).as_deref(),
        Some("heading \"Gone\" is no longer in the note")
    );
    assert_eq!(caret(&started, cx), before);
    assert_eq!(
        outline_rows(&started, cx),
        ["Top", "  Gons"],
        "synced first"
    );
}

/// OUTLINE follows the headings after a save while it is shown; typing
/// alone does not change it.
#[gpui_kit::test]
fn outline_updates_after_a_save(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("o.md", "# One\ntext")], "o.md");
    started.workspace.update(cx, |w, cx| {
        w.move_to_side(DrawerView::Outline, Side::Right);
        w.refresh_views(cx);
    });
    assert_eq!(outline_rows(&started, cx), ["One"]);
    assert_eq!(region(&started, cx), Panel::Editor);
    press(&started, "down", cx);
    press(&started, "end", cx);
    type_in(&started, "\n## Two", cx);
    assert_eq!(started.text(cx), "# One\ntext\n## Two");
    assert_eq!(outline_rows(&started, cx), ["One"]);
    press(&started, "ctrl-s", cx);
    drive(&rt, cx, |cx| {
        disk(&rt, &v, "o.md").is_some_and(|t| t.ends_with("## Two"))
            && outline_rows(&started, cx).len() == 2
    });
    assert_eq!(outline_rows(&started, cx), ["One", "  Two"]);
}

/// `YankRow` (primary+C) in LINKS copies the row's path, in OUTLINE the
/// heading's text.
#[gpui_kit::test]
fn the_copy_chord_copies_the_selected_row(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, LINKED, "alpha.md");
    leader(&started, "lb", cx);
    link_rows(&started, &rt, cx);
    let list = links_list(&started, cx);
    let selected = cx.read(|cx| list.read(cx).selected_row().map(|r| r.path.to_string()));
    press(&started, "ctrl-c", cx);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied, selected);
    assert_eq!(started.flash(cx).as_deref(), Some("path copied"));

    leader(&started, "ol", cx);
    press(&started, "ctrl-c", cx);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("Alpha notes"));
    assert_eq!(started.flash(cx).as_deref(), Some("heading copied"));
}

/// Escape in a list gives the keyboard back to the editor.
#[gpui_kit::test]
fn escape_returns_to_the_editor(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, LINKED, "alpha.md");
    leader(&started, "ol", cx);
    assert!(list_focused(&started, &outline_list(&started, cx), cx));
    press(&started, "escape", cx);
    assert!(editor_focused(&started, cx));
    cx.executor().advance_clock(Duration::from_millis(10));
}

/// In the FILES filter the copy chord is the field's own: it copies the
/// selected text, not a row.
#[gpui_kit::test]
fn the_copy_chord_in_the_files_filter_copies_its_text(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, LINKED, "alpha.md");
    press(&started, "ctrl-e", cx);
    type_in(&started, "gam", cx);
    press(&started, select_all_key(), cx);
    press(&started, copy_key(), cx);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("gam"));
    assert_eq!(started.flash(cx), None);
}
