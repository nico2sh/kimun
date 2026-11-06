//! UI integration tests for finding and making notes (M6 G4): the note
//! picker (the finder and the link chooser), the create dialog, the quick
//! note, today's journal and leader `n n`, on a real `Workspace` over a
//! real, indexed temp vault, driven through the window's keys; every
//! outcome is checked on disk.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`).
#![cfg(test)]

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, Focusable as _, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::files::FileListEntry;
use kimun_notes::logic::layout::{Panel, Side};
use kimun_notes::logic::message::AppMessage;
use tokio::runtime::Runtime;

use super::modal::ModalKind;
use super::picker::{NotePicker, PICKER_ERROR_ID, picker_row_id};
use super::start_tests::{Started, drive, open_started, runtime, send, vault_start};
use crate::startup::FirstNote;
use crate::test_vault::{TestVault, test_vault};

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

fn disk(rt: &Runtime, v: &TestVault, p: &str) -> Option<String> {
    rt.block_on(v.vault.get_note_text(&path(p))).ok()
}

fn exists(rt: &Runtime, v: &TestVault, p: &str) -> bool {
    rt.block_on(v.vault.exists(&path(p)))
}

/// Every note the index knows.
fn all_notes(rt: &Runtime, v: &TestVault) -> Vec<VaultPath> {
    rt.block_on(v.vault.get_all_notes())
        .unwrap()
        .into_iter()
        .map(|(entry, _)| entry.path)
        .collect()
}

/// A vault holding `notes`, started on `first`, driven until its text shows
/// and the index is built (the finder and follow by name read it).
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

fn picker(s: &Started, cx: &TestAppContext) -> Entity<NotePicker> {
    cx.read(|cx| {
        s.workspace
            .read(cx)
            .picker
            .as_ref()
            .and_then(|p| p.upgrade())
            .expect("a picker is open")
    })
}

/// The picker's rows once the current query's have landed: each note's
/// path, `Create: <path>` for the create row.
fn picker_rows(s: &Started, rt: &Runtime, cx: &mut TestAppContext) -> Vec<String> {
    let view = picker(s, cx);
    drive(rt, cx, |cx| !cx.read(|cx| view.read(cx).is_loading()));
    cx.read(|cx| {
        view.read(cx)
            .rows()
            .iter()
            .map(|row| match row {
                FileListEntry::CreateNote { path, .. } => format!("Create: {path}"),
                other => other.path().to_string(),
            })
            .collect()
    })
}

fn picker_selected(s: &Started, cx: &TestAppContext) -> Option<VaultPath> {
    let view = picker(s, cx);
    cx.read(|cx| view.read(cx).selected().map(|r| r.path().clone()))
}

fn open_note(s: &Started, cx: &TestAppContext) -> Option<VaultPath> {
    cx.read(|cx| s.workspace.read(cx).notes.as_ref()?.note.clone())
}

/// Whether the open note is `p`.
fn on(s: &Started, p: &str, cx: &TestAppContext) -> bool {
    open_note(s, cx).is_some_and(|n| n.is_like(&path(p)))
}

fn focused(s: &Started, id: &'static str, cx: &mut TestAppContext) -> bool {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).focused() == Some(true)
    })
    .unwrap()
}

fn files_filter_focused(s: &Started, cx: &mut TestAppContext) -> bool {
    let files = cx.read(|cx| s.workspace.read(cx).files.clone().unwrap());
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        files.read(cx).focus_handle(cx).is_focused(window)
    })
    .unwrap()
}

/// The element id of the picker row for note `p`.
fn row_of(s: &Started, p: &str, cx: &TestAppContext) -> String {
    let view = picker(s, cx);
    cx.read(|cx| {
        view.read(cx)
            .rows()
            .iter()
            .find(|row| {
                !matches!(row, FileListEntry::CreateNote { .. }) && row.path().is_like(&path(p))
            })
            .map(|row| picker_row_id(row).to_string())
            .unwrap_or_else(|| panic!("no row {p}"))
    })
}

fn click_on(s: &Started, id: String, how: Click, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        let id = gpui_kit::SharedString::from(id);
        match how {
            Click::Left => window.click(id, cx),
            Click::Double => window.double_click(id, cx),
            Click::Right => window.right_click(id, cx),
        }
    })
    .unwrap();
    cx.run_until_parked();
}

#[derive(Clone, Copy)]
enum Click {
    Left,
    Double,
    Right,
}

fn region(s: &Started, cx: &TestAppContext) -> Panel {
    cx.read(|cx| s.workspace.read(cx).layout.focused())
}

fn dirty(s: &Started, cx: &TestAppContext) -> bool {
    cx.read(|cx| s.workspace.read(cx).editor.read(cx).editor.is_dirty())
}

fn switching(s: &Started, cx: &TestAppContext) -> bool {
    cx.read(|cx| {
        s.workspace
            .read(cx)
            .notes
            .as_ref()
            .is_some_and(|h| h.is_switching() || h.is_open_wanted())
    })
}

fn note_error(s: &Started, cx: &TestAppContext) -> Option<String> {
    cx.read(|cx| s.workspace.read(cx).note_error(cx))
}

// ── the finder ───────────────────────────────────────────────────────────

/// Ctrl+O lists every note with its query focused; typing narrows them
/// fuzzily under the `Create:` row; choosing one opens it, the dirty open
/// note saved first, and the editor takes the keyboard.
#[gpui_kit::test]
fn the_finder_filters_and_opens_saving_the_open_note_first(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(
        cx,
        &rt,
        &[
            ("a.md", "alpha"),
            ("work/bravo.md", "Bravo"),
            ("charlie.md", "Charlie"),
        ],
        "a.md",
    );
    type_in(&started, "X", cx);
    press(&started, "ctrl-o", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Finder));
    assert!(focused(&started, super::picker::PICKER_INPUT_ID, cx));
    let mut rows = picker_rows(&started, &rt, cx);
    rows.sort();
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert!(rows.iter().all(|r| !r.starts_with("Create:")), "{rows:?}");

    type_in(&started, "brv", cx);
    let rows = picker_rows(&started, &rt, cx);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(
        rows[0].starts_with("Create: ") && rows[0].ends_with("brv.md"),
        "{rows:?}"
    );
    assert!(path(&rows[1]).is_like(&path("work/bravo.md")), "{rows:?}");
    press(&started, "down", cx);
    assert!(picker_selected(&started, cx).is_some_and(|p| p.is_like(&path("work/bravo.md"))));
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| started.text(cx) == "Bravo");
    assert!(on(&started, "work/bravo.md", cx));
    assert_eq!(
        disk(&rt, &v, "a.md").as_deref(),
        Some("Xalpha"),
        "saved first"
    );
    assert!(!exists(&rt, &v, "brv.md"), "nothing created");
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(focused(&started, "editor", cx));
}

/// `Create:` makes the note through core in the open note's folder and
/// opens it; FILES lists it.
#[gpui_kit::test]
fn the_create_row_makes_the_note_and_opens_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("d/a.md", "alpha")], "d/a.md");
    press(&started, "ctrl-o", cx);
    type_in(&started, "fresh", cx);
    let rows = picker_rows(&started, &rt, cx);
    let create = rows[0]
        .strip_prefix("Create: ")
        .expect("the create row leads");
    assert!(path(create).is_like(&path("d/fresh.md")), "{rows:?}");
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| on(&started, "d/fresh.md", cx));
    assert_eq!(disk(&rt, &v, "d/fresh.md").as_deref(), Some(""));
    assert_eq!(disk(&rt, &v, "d/a.md").as_deref(), Some("alpha"));
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(focused(&started, "editor", cx));
    drive(&rt, cx, |cx| {
        cx.read(|cx| {
            let files = started.workspace.read(cx).files.clone().unwrap();
            files.read(cx).model().core().is_some_and(|core| {
                core.rows()
                    .iter()
                    .any(|r| r.path().is_like(&path("d/fresh.md")))
            })
        })
    });
}

/// `Create:` for a name that is taken opens the note that is there; it is
/// never overwritten.
#[gpui_kit::test]
fn the_create_row_never_overwrites(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha"), ("b.md", "beta")], "a.md");
    press(&started, "ctrl-o", cx);
    type_in(&started, "b", cx);
    let rows = picker_rows(&started, &rt, cx);
    let create = rows[0]
        .strip_prefix("Create: ")
        .expect("the create row leads");
    assert!(path(create).is_like(&path("b.md")), "{rows:?}");
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| started.text(cx) == "beta");
    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta"));
}

/// A create core refuses stays in the picker, which stays open; the next
/// keystroke clears it.
#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_create_stays_in_the_picker(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("d/a.md", "alpha")], "d/a.md");
    let dir = v.workspace.as_path().join("d");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    press(&started, "ctrl-o", cx);
    type_in(&started, "fresh", cx);
    picker_rows(&started, &rt, cx);
    press(&started, "enter", cx);
    let view = picker(&started, cx);
    drive(&rt, cx, |cx| cx.read(|cx| view.read(cx).error().is_some()));
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Finder));
    assert!(started.shows(PICKER_ERROR_ID, cx));
    assert!(!exists(&rt, &v, "d/fresh.md"));
    assert!(on(&started, "d/a.md", cx));
    type_in(&started, "r", cx);
    assert_eq!(
        cx.read(|cx| view.read(cx).error().map(str::to_string)),
        None
    );
}

/// Escape closes the finder and gives the keyboard back to its opener: the
/// editor, or the FILES filter.
#[gpui_kit::test]
fn escape_gives_the_keyboard_back_to_the_opener(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    press(&started, "ctrl-o", cx);
    type_in(&started, "zz", cx);
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(focused(&started, "editor", cx));
    assert_eq!(started.text(cx), "alpha", "nothing typed into the note");

    press(&started, "ctrl-e", cx);
    assert!(files_filter_focused(&started, cx));
    press(&started, "ctrl-o", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Finder));
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(files_filter_focused(&started, cx));
    assert_eq!(region(&started, cx), Panel::Drawer(Side::Left));
}

/// One modal at a time: over the finder the palette and quick note chords
/// do nothing; the finder's own chord closes it.
#[gpui_kit::test]
fn one_modal_at_a_time_and_the_chord_toggles(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    press(&started, "ctrl-o", cx);
    for chord in ["ctrl-shift-p", "ctrl-shift-w", "f2", "ctrl-j"] {
        press(&started, chord, cx);
        assert_eq!(started.shown_modal(cx), Some(ModalKind::Finder), "{chord}");
    }
    press(&started, "ctrl-o", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(focused(&started, "editor", cx));

    press(&started, "ctrl-shift-w", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::QuickNote));
    press(&started, "ctrl-o", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::QuickNote));
    press(&started, "ctrl-shift-w", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
}

/// The leader's `f f` and the palette's `+find · files` open the finder.
#[gpui_kit::test]
fn the_leader_and_the_palette_open_the_finder(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    press(&started, "ctrl-shift-b", cx);
    type_in(&started, "ff", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Finder));
    press(&started, "escape", cx);
    started.settle_dialog(cx);

    press(&started, "ctrl-shift-p", cx);
    type_in(&started, "find files", cx);
    press(&started, "enter", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Finder));
}

// ── the pointer (C1) ─────────────────────────────────────────────────────

/// A click on a finder row opens it at once, as Enter does; a right click
/// opens nothing.
#[gpui_kit::test]
fn a_click_on_a_finder_row_opens_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha"), ("work/bravo.md", "Bravo")],
        "a.md",
    );
    press(&started, "ctrl-o", cx);
    picker_rows(&started, &rt, cx);
    let row = row_of(&started, "work/bravo.md", cx);
    click_on(&started, row.clone(), Click::Right, cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Finder));
    assert!(on(&started, "a.md", cx), "a right click opens nothing");
    click_on(&started, row, Click::Left, cx);
    drive(&rt, cx, |cx| started.text(cx) == "Bravo");
    assert!(on(&started, "work/bravo.md", cx));
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
}

/// A double-click on a finder row opens it once: the picker closed on the
/// first press, the second lands on the window under it.
#[gpui_kit::test]
fn a_double_click_on_a_finder_row_opens_it_once(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha"), ("work/bravo.md", "Bravo")],
        "a.md",
    );
    press(&started, "ctrl-o", cx);
    picker_rows(&started, &rt, cx);
    let row = row_of(&started, "work/bravo.md", cx);
    click_on(&started, row, Click::Double, cx);
    drive(&rt, cx, |cx| started.text(cx) == "Bravo");
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(on(&started, "work/bravo.md", cx));
}

/// While a `Create:` runs no row can be chosen: a click is inert, as Enter
/// is (the picker waits).
#[gpui_kit::test]
fn a_busy_finder_ignores_a_click(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha"), ("work/bravo.md", "Bravo")],
        "a.md",
    );
    press(&started, "ctrl-o", cx);
    picker_rows(&started, &rt, cx);
    let view = picker(&started, cx);
    view.update(cx, |picker, cx| picker.set_busy(true, None, cx));
    let row = row_of(&started, "work/bravo.md", cx);
    click_on(&started, row, Click::Left, cx);
    drive(&rt, cx, |_| true);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Finder));
    assert!(on(&started, "a.md", cx), "nothing opened");
}

/// A click on a link chooser row opens that note.
#[gpui_kit::test]
fn a_click_on_a_link_chooser_row_opens_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[
            ("a.md", "[[dup]]"),
            ("dup.md", "Dup one"),
            ("sub/dup.md", "Dup two"),
        ],
        "a.md",
    );
    press(&started, "ctrl-enter", cx);
    drive(&rt, cx, |cx| {
        started.shown_modal(cx) == Some(ModalKind::LinkChooser)
    });
    picker_rows(&started, &rt, cx);
    let row = row_of(&started, "sub/dup.md", cx);
    click_on(&started, row, Click::Left, cx);
    drive(&rt, cx, |cx| started.text(cx) == "Dup two");
    assert!(on(&started, "sub/dup.md", cx));
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
}

// ── following a link ─────────────────────────────────────────────────────

/// A name several notes share: the chooser lists every one of them (none
/// opens on its own); typing narrows them; the chosen one opens.
#[gpui_kit::test]
fn an_ambiguous_link_lists_every_match_and_opens_the_chosen(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[
            ("a.md", "[[dup]]"),
            ("dup.md", "Dup one"),
            ("sub/dup.md", "Dup two"),
            ("x/dup.md", "Dup three"),
        ],
        "a.md",
    );
    press(&started, "ctrl-enter", cx);
    drive(&rt, cx, |cx| {
        started.shown_modal(cx) == Some(ModalKind::LinkChooser)
    });
    let mut rows = picker_rows(&started, &rt, cx);
    rows.sort();
    assert_eq!(rows.len(), 3, "{rows:?}");
    for (row, want) in rows.iter().zip(["dup.md", "sub/dup.md", "x/dup.md"]) {
        assert!(path(row).is_like(&path(want)), "{rows:?}");
    }
    assert!(on(&started, "a.md", cx), "nothing opened yet");

    type_in(&started, "sub", cx);
    let rows = picker_rows(&started, &rt, cx);
    assert_eq!(rows.len(), 1, "{rows:?}");
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| started.text(cx) == "Dup two");
    assert!(on(&started, "sub/dup.md", cx));
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
}

/// A wikilink with a folder part names notes in any folder whose path ends
/// with it: the chooser lists `/work/dup` and `/a/work/dup`, never the root
/// `dup` nor the `notwork/dup` whose folder only ends with the same letters.
#[gpui_kit::test]
fn a_suffix_link_lists_only_the_notes_it_ends(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[
            ("journal/a.md", "[[work/dup]]"),
            ("dup.md", "Root"),
            ("work/dup.md", "Work"),
            ("a/work/dup.md", "Deep"),
            ("notwork/dup.md", "Not"),
        ],
        "journal/a.md",
    );
    press(&started, "ctrl-enter", cx);
    drive(&rt, cx, |cx| {
        started.shown_modal(cx) == Some(ModalKind::LinkChooser)
    });
    let mut rows = picker_rows(&started, &rt, cx);
    rows.sort();
    assert_eq!(rows.len(), 2, "{rows:?}");
    for (row, want) in rows.iter().zip(["a/work/dup.md", "work/dup.md"]) {
        assert!(path(row).is_like(&path(want)), "{rows:?}");
    }
}

/// A markdown link is a path, relative to the note's folder: one to
/// `work/dup.md` from `journal/` never finds the vault's `/work/dup.md`; it
/// offers to create `journal/work/dup.md`.
#[gpui_kit::test]
fn a_markdown_path_never_matches_another_folder(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(
        cx,
        &rt,
        &[
            ("journal/a.md", "[x](work/dup.md)"),
            ("work/dup.md", "Work"),
        ],
        "journal/a.md",
    );
    press(&started, "ctrl-enter", cx);
    drive(&rt, cx, |cx| {
        started.shown_modal(cx) == Some(ModalKind::CreateNote)
    });
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| on(&started, "journal/work/dup.md", cx));
    assert!(exists(&rt, &v, "journal/work/dup.md"));
    assert_eq!(disk(&rt, &v, "work/dup.md").as_deref(), Some("Work"));
}

/// A link to no note offers to create it; `Create` (Enter) makes it through
/// core and opens it.
#[gpui_kit::test]
fn a_missing_link_is_created_and_opened(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "[[nobody]]")], "a.md");
    press(&started, "ctrl-enter", cx);
    drive(&rt, cx, |cx| {
        started.shown_modal(cx) == Some(ModalKind::CreateNote)
    });
    assert!(!exists(&rt, &v, "nobody.md"), "nothing made before Create");
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| on(&started, "nobody.md", cx));
    assert_eq!(disk(&rt, &v, "nobody.md").as_deref(), Some(""));
    assert_eq!(started.text(cx), "");
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(focused(&started, "editor", cx));
}

/// Escape on the create dialog makes nothing and leaves the note as it was.
#[gpui_kit::test]
fn a_declined_create_makes_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "[[nobody]]")], "a.md");
    press(&started, "ctrl-enter", cx);
    drive(&rt, cx, |cx| {
        started.shown_modal(cx) == Some(ModalKind::CreateNote)
    });
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(!exists(&rt, &v, "nobody.md"));
    assert!(on(&started, "a.md", cx));
    assert!(focused(&started, "editor", cx));
}

/// An open of a note that is not there offers the create dialog; Create
/// makes it and opens it.
#[gpui_kit::test]
fn opening_a_missing_note_offers_to_create_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    send(&started, AppMessage::open(path("later/new.md")), cx);
    drive(&rt, cx, |cx| {
        started.shown_modal(cx) == Some(ModalKind::CreateNote)
    });
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| on(&started, "later/new.md", cx));
    assert!(exists(&rt, &v, "later/new.md"));
}

// ── journal ──────────────────────────────────────────────────────────────

/// Ctrl+J makes today's entry and opens it; again, it opens the same entry,
/// never made anew.
#[gpui_kit::test]
fn the_journal_is_made_once(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    press(&started, "ctrl-j", cx);
    drive(&rt, cx, |cx| {
        !on(&started, "a.md", cx) && open_note(&started, cx).is_some()
    });
    let entry = open_note(&started, cx).unwrap();
    assert!(focused(&started, "editor", cx));
    let made = disk(&rt, &v, &entry.to_string()).expect("the entry is on disk");
    assert_eq!(started.text(cx), made);
    let count = all_notes(&rt, &v).len();
    // An edit, saved: the second press must not replace it.
    press(&started, "ctrl-end", cx);
    type_in(&started, "X", cx);
    press(&started, "ctrl-s", cx);
    drive(&rt, cx, |cx| !dirty(&started, cx));

    press(&started, "ctrl-e", cx);
    press(&started, "ctrl-j", cx);
    drive(&rt, cx, |cx| focused(&started, "editor", cx));
    // The entry's `OpenPath` re-opens it: let that switch run out.
    let mut rounds = 0;
    drive(&rt, cx, |_| {
        rounds += 1;
        rounds > 20
    });
    drive(&rt, cx, |cx| !switching(&started, cx));
    assert_eq!(open_note(&started, cx), Some(entry.clone()));
    assert!(disk(&rt, &v, &entry.to_string()).unwrap().ends_with('X'));
    assert_eq!(all_notes(&rt, &v).len(), count, "no second entry");
}

// ── quick note ───────────────────────────────────────────────────────────

/// Ctrl+Shift+W: Enter on an empty line closes and makes nothing; Enter
/// saves into the inbox and closes (the note stays where it was);
/// Shift+Enter saves and opens the new note.
#[gpui_kit::test]
fn the_quick_note_saves_into_the_inbox(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    let before = all_notes(&rt, &v);

    press(&started, "ctrl-shift-w", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::QuickNote));
    assert!(focused(&started, super::create::QUICK_NOTE_INPUT_ID, cx));
    press(&started, "enter", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(focused(&started, "editor", cx));
    assert_eq!(all_notes(&rt, &v), before, "a blank line makes nothing");

    press(&started, "ctrl-shift-w", cx);
    type_in(&started, "first thought", cx);
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| started.shown_modal(cx).is_none());
    let made: Vec<VaultPath> = all_notes(&rt, &v)
        .into_iter()
        .filter(|p| !before.contains(p))
        .collect();
    assert_eq!(made.len(), 1, "{made:?}");
    assert!(
        disk(&rt, &v, &made[0].to_string())
            .unwrap()
            .contains("first thought")
    );
    assert!(on(&started, "a.md", cx), "Enter does not open it");
    assert_eq!(started.text(cx), "alpha");

    press(&started, "ctrl-shift-w", cx);
    type_in(&started, "second thought", cx);
    press(&started, "shift-enter", cx);
    drive(&rt, cx, |cx| started.text(cx).contains("second thought"));
    let opened = open_note(&started, cx).unwrap();
    assert!(!opened.is_like(&made[0]) && !opened.is_like(&path("a.md")));
    assert_eq!(
        opened.get_parent_path().0.get_slices(),
        made[0].get_parent_path().0.get_slices(),
        "both in the inbox"
    );
    started.settle_dialog(cx);
    assert!(focused(&started, "editor", cx));
    assert_eq!(note_error(&started, cx), None);
}

/// A file operation landing while the quick note is saving closes the
/// dialog (and forgets its status). The save still reports when it lands —
/// a failure as an error flash, since the dialog is gone — and leaves alone
/// the modal opened since.
#[cfg(unix)]
#[gpui_kit::test]
fn a_quick_note_still_reports_after_a_file_op_closed_it(cx: &mut TestAppContext) {
    use kimun_notes::logic::message::FileOp;
    use std::os::unix::fs::PermissionsExt;
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    // An inbox the quick note cannot write into.
    let inbox = v.vault.inbox_path().clone();
    rt.block_on(
        v.vault
            .create_note(&inbox.append(&VaultPath::note_path_from("old")), "old"),
    )
    .unwrap();
    let dir = v.vault.path_to_pathbuf(&inbox);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();

    press(&started, "ctrl-shift-w", cx);
    type_in(&started, "a thought", cx);
    press(&started, "enter", cx);
    assert!(cx.read(|cx| started.workspace.read(cx).note_task.is_some()));
    // A file operation lands before the save does: every modal closes.
    send(
        &started,
        AppMessage::FileOp(FileOp::Deleted(path("gone.md"))),
        cx,
    );
    cx.run_until_parked();
    assert_eq!(started.shown_modal(cx), None);
    press(&started, "ctrl-o", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Finder));

    let last = |cx: &TestAppContext| {
        cx.read(|cx| {
            started
                .workspace
                .read(cx)
                .last_flash()
                .map(|(text, error)| (text.to_string(), error))
        })
    };
    drive(&rt, cx, |cx| {
        cx.read(|cx| started.workspace.read(cx).note_task.is_none())
    });
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (text, error) = last(cx).expect("the failure is flashed");
    assert!(error, "{text}");
    assert_eq!(
        started.shown_modal(cx),
        Some(ModalKind::Finder),
        "the finder opened since stays"
    );
    assert!(on(&started, "a.md", cx));
}

// ── leader `n n`─────────────────────────────────────────────────────────

/// Leader `n n` shows FILES (its drawer was hidden) with the filter holding
/// the keyboard: typing a name offers `Create:`.
#[gpui_kit::test]
fn leader_n_n_focuses_the_files_filter(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], "a.md");
    // Hide the left drawer first.
    let shown = cx.read(|cx| {
        started
            .workspace
            .read(cx)
            .layout
            .is_visible(Panel::Drawer(Side::Left))
    });
    if shown {
        press(&started, "ctrl-t", cx);
    }
    assert!(!cx.read(|cx| {
        started
            .workspace
            .read(cx)
            .layout
            .is_visible(Panel::Drawer(Side::Left))
    }));
    press(&started, "ctrl-shift-b", cx);
    type_in(&started, "nn", cx);
    assert!(cx.read(|cx| {
        started
            .workspace
            .read(cx)
            .layout
            .is_visible(Panel::Drawer(Side::Left))
    }));
    assert_eq!(region(&started, cx), Panel::Drawer(Side::Left));
    assert!(files_filter_focused(&started, cx));
    assert_eq!(started.text(cx), "alpha", "nn typed nothing into the note");
}
