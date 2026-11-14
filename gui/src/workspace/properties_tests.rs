//! UI integration tests for the properties dialog (M7a G2): a real
//! `Workspace` on a real temp vault, the dialog driven through the window's
//! keys and pointer, every write read back from disk with core.
//!
//! Like the other vault tests, each test owns a current-thread tokio runtime
//! and drives it between GPUI's `run_until_parked` (`start_tests::drive`).
#![cfg(test)]

use std::time::Duration;

use gpui_kit::test::{TestQueryExt as _, TestWindowExt};
use gpui_kit::{App, AppContext as _, Entity, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_core::note::{PropertyKind, PropertyValue};
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::message::{AppMessage, FileOp, OverlayData};
use tokio::runtime::Runtime;

use super::file_dialogs::FileOpKind;
use super::modal::ModalKind;
use super::note_menu::{MenuItem, menu_item_element_id};
use super::properties::{
    ADD_ID, KEY_INPUT_ID, PropertiesDialog, REFUSED, STORE_ANYWAY_ID, VALUE_INPUT_ID, row_id,
    suggestion_id,
};
use super::route::NO_NOTE;
use super::start_tests::select_all_key;
use super::start_tests::{Started, drive, open_started, runtime, vault_start};
use crate::startup::FirstNote;
use crate::status_row::PROPERTIES_ID;
use crate::test_vault::{TestVault, test_vault};

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

/// A vault holding `notes`, started on `first`, driven until its text shows
/// and the index has caught up.
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

fn click(s: &Started, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
    let id = id.into();
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn shows(s: &Started, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> bool {
    let id = id.into();
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some_and(|e| e.visible())
    })
    .unwrap()
}

fn disabled(s: &Started, id: &'static str, cx: &mut TestAppContext) -> Option<bool> {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).disabled()
    })
    .unwrap()
}

fn kit_dialog_open(s: &Started, cx: &mut TestAppContext) -> bool {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        super::modal::dialog_open(window, cx)
    })
    .unwrap()
}

fn dialog(s: &Started, cx: &TestAppContext) -> Option<Entity<PropertiesDialog>> {
    cx.read(|cx| s.workspace.read(cx).properties_dialog())
}

fn with_dialog<R>(
    s: &Started,
    cx: &TestAppContext,
    f: impl FnOnce(&PropertiesDialog, &App) -> R,
) -> R {
    let d = dialog(s, cx).expect("the properties dialog is open");
    cx.read(|cx| f(d.read(cx), cx))
}

fn rows(s: &Started, cx: &TestAppContext) -> Vec<[String; 3]> {
    with_dialog(s, cx, |d, _| d.rows())
}

fn is_dirty(s: &Started, cx: &TestAppContext) -> bool {
    cx.read(|cx| {
        s.workspace
            .read(cx)
            .active_state()
            .editor
            .read(cx)
            .editor
            .is_dirty()
    })
}

/// Nothing holds the note: no write, no reload, no opening flush.
fn idle(s: &Started, cx: &TestAppContext) -> bool {
    cx.read(|cx| !s.workspace.read(cx).note_held())
}

fn disk(rt: &Runtime, v: &TestVault, p: &str) -> String {
    rt.block_on(v.vault.get_note_text(&path(p))).unwrap()
}

fn prop(rt: &Runtime, v: &TestVault, p: &str, key: &str) -> Option<Option<PropertyValue>> {
    rt.block_on(v.vault.get_property(&path(p), key)).unwrap()
}

/// Leader `n p`: the properties of the open note.
fn open_by_leader(s: &Started, cx: &mut TestAppContext) {
    press(s, "ctrl-shift-b", cx);
    press(s, "n", cx);
    press(s, "p", cx);
}

/// Open the dialog by the leader and wait for its rows.
fn open_loaded(rt: &Runtime, s: &Started, cx: &mut TestAppContext) {
    open_by_leader(s, cx);
    drive(rt, cx, |cx| {
        dialog(s, cx).is_some_and(|d| cx.read(|cx| !d.read(cx).is_loading()))
    });
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Properties));
}

/// Wait until the write under way, its reload and the dialog's own reload
/// have all landed.
fn settled(rt: &Runtime, s: &Started, cx: &mut TestAppContext) {
    drive(rt, cx, |cx| {
        idle(s, cx)
            && dialog(s, cx)
                .is_none_or(|d| cx.read(|cx| !d.read(cx).is_busy() && !d.read(cx).is_loading()))
    });
}

/// Let more than one autosave tick (1 s) pass on the runtime's real clock.
fn past_a_tick(rt: &Runtime, cx: &mut TestAppContext) {
    for _ in 0..30 {
        rt.block_on(tokio::time::sleep(Duration::from_millis(50)));
        cx.run_until_parked();
    }
}

/// Add `key` = `value` from the list: A, the key, the value, Enter.
fn add(s: &Started, key: &str, value: &str, cx: &mut TestAppContext) {
    press(s, "a", cx);
    assert!(with_dialog(s, cx, |d, _| d.in_form()), "A opens the form");
    type_in(s, key, cx);
    click(s, VALUE_INPUT_ID, cx);
    type_in(s, value, cx);
    press(s, "enter", cx);
}

const TOML: &str = "+++\ntitle = \"Plan\"\nstatus = \"draft\"\n+++\nbody\n";

// ── open, list, add ───────────────────────────────────────────────────────

/// Leader `n p` lists the note's properties; adding one writes it through
/// core, and the open note reloads clean with the new frontmatter, which no
/// later autosave reverts. The Footer's count follows.
#[gpui_kit::test]
fn adding_a_property_writes_it_and_reloads_the_note_clean(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", TOML)], "a.md");
    open_loaded(&rt, &started, cx);
    assert_eq!(
        rows(&started, cx),
        vec![
            ["title".to_string(), "text".into(), "Plan".into()],
            ["status".to_string(), "text".into(), "draft".into()],
        ]
    );
    assert!(shows(&started, row_id("title"), cx));

    add(&started, "count", "3", cx);
    settled(&rt, &started, cx);

    assert_eq!(
        prop(&rt, &v, "a.md", "count"),
        Some(Some(PropertyValue::Number(3.0)))
    );
    assert!(
        !with_dialog(&started, cx, |d, _| d.in_form()),
        "back to the list"
    );
    assert_eq!(rows(&started, cx).len(), 3);
    assert_eq!(started.flash(cx).as_deref(), Some("Property saved"));
    // The editor holds what is on disk, clean.
    let on_disk = disk(&rt, &v, "a.md");
    assert!(on_disk.contains("count = 3"), "{on_disk}");
    assert_eq!(started.text(cx), on_disk);
    assert!(!is_dirty(&started, cx));
    // The Footer's count follows the disk.
    drive(&rt, cx, |cx| {
        cx.read(|cx| {
            let w = started.workspace.read(cx);
            w.active_state().notes.as_ref().unwrap().meta.properties() == Some(3)
        })
    });
    // No tick writes the old frontmatter back.
    past_a_tick(&rt, cx);
    assert_eq!(disk(&rt, &v, "a.md"), on_disk);
    // And the note takes typing again once the dialog is closed.
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert!(!kit_dialog_open(&started, cx));
    type_in(&started, "Z", cx);
    press(&started, "ctrl-s", cx);
    drive(&rt, cx, |cx| !is_dirty(&started, cx));
    let saved = disk(&rt, &v, "a.md");
    assert!(
        saved.contains('Z') && saved.contains("count = 3"),
        "{saved}"
    );
}

/// An Add for a key the note has is refused with the TUI's words; nothing
/// is written.
#[gpui_kit::test]
fn adding_an_existing_key_is_refused(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", TOML)], "a.md");
    open_loaded(&rt, &started, cx);
    add(&started, "Status", "final", cx);
    cx.run_until_parked();
    assert_eq!(
        with_dialog(&started, cx, |d, _| d.form_error()).as_deref(),
        Some("'Status' already exists — edit it")
    );
    assert!(idle(&started, cx), "nothing was written");
    assert_eq!(disk(&rt, &v, "a.md"), TOML);
}

// ── edit, rename, no-op ───────────────────────────────────────────────────

/// Enter edits the selected row: a new value is written; saving it as
/// pre-filled writes nothing (the file stays byte-identical).
#[gpui_kit::test]
fn editing_writes_and_an_unchanged_edit_writes_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", TOML)], "a.md");
    open_loaded(&rt, &started, cx);

    // Unchanged: straight back to the list, nothing written.
    press(&started, "down", cx);
    press(&started, "enter", cx);
    assert!(with_dialog(&started, cx, |d, _| d.in_form()));
    press(&started, "enter", cx);
    assert!(!with_dialog(&started, cx, |d, _| d.in_form()));
    assert!(idle(&started, cx));
    assert_eq!(disk(&rt, &v, "a.md"), TOML);

    // A new value (Edit starts on the value).
    press(&started, "enter", cx);
    press(&started, select_all_key(), cx);
    type_in(&started, "final", cx);
    press(&started, "enter", cx);
    settled(&rt, &started, cx);
    assert_eq!(
        prop(&rt, &v, "a.md", "status"),
        Some(Some(PropertyValue::Text("final".into())))
    );
    assert_eq!(started.text(cx), disk(&rt, &v, "a.md"));
    assert!(!is_dirty(&started, cx));
}

/// Renaming a key in the form renames it on disk, its value kept.
#[gpui_kit::test]
fn renaming_a_key_moves_its_value(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", TOML)], "a.md");
    open_loaded(&rt, &started, cx);
    press(&started, "down", cx);
    press(&started, "enter", cx);
    click(&started, KEY_INPUT_ID, cx);
    press(&started, select_all_key(), cx);
    type_in(&started, "state", cx);
    press(&started, "enter", cx);
    settled(&rt, &started, cx);
    assert_eq!(prop(&rt, &v, "a.md", "status"), None);
    assert_eq!(
        prop(&rt, &v, "a.md", "state"),
        Some(Some(PropertyValue::Text("draft".into())))
    );
    assert!(rows(&started, cx).iter().any(|r| r[0] == "state"));
}

// ── delete ──────────────────────────────────────────────────────────────

/// D asks in an alert over the dialog; Cancel keeps the key, Delete removes
/// it and the dialog stays on the list.
#[gpui_kit::test]
fn deleting_asks_first_then_removes_the_key(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", TOML)], "a.md");
    open_loaded(&rt, &started, cx);
    press(&started, "d", cx);
    assert!(
        cx.update_window(started.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_all_by_role(gpui_kit::Role::AlertDialog).len() == 1
        })
        .unwrap(),
        "the delete asks in an alert over the dialog"
    );
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert!(kit_dialog_open(&started, cx), "only the alert closed");
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Properties));
    assert!(prop(&rt, &v, "a.md", "title").is_some());

    press(&started, "d", cx);
    press(&started, "enter", cx);
    settled(&rt, &started, cx);
    assert_eq!(prop(&rt, &v, "a.md", "title"), None);
    assert_eq!(rows(&started, cx).len(), 1);
    assert_eq!(started.flash(cx).as_deref(), Some("Property removed"));
    assert_eq!(started.text(cx), disk(&rt, &v, "a.md"));
}

// ── refusals and read-only ───────────────────────────────────────────────

/// A frontmatter block core will not write (a duplicate key) makes the
/// dialog read-only with core's message once a write finds it; the file is
/// untouched and nothing more can be added.
#[gpui_kit::test]
fn malformed_frontmatter_is_read_only(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let text = "+++\na = 1\na = 2\n+++\nbody";
    let (v, started) = started_on(cx, &rt, &[("a.md", text)], "a.md");
    open_loaded(&rt, &started, cx);
    add(&started, "b", "x", cx);
    settled(&rt, &started, cx);
    assert!(with_dialog(&started, cx, |d, _| d.load_error().is_some()));
    assert!(
        !with_dialog(&started, cx, |d, _| d.in_form()),
        "back to the list"
    );
    assert_eq!(disabled(&started, ADD_ID, cx), Some(true));
    press(&started, "a", cx);
    assert!(!with_dialog(&started, cx, |d, _| d.in_form()));
    assert_eq!(disk(&rt, &v, "a.md"), text);
}

/// A value the vault's type refuses offers `Store as text anyway`, which
/// forces the kind and writes.
#[gpui_kit::test]
fn a_type_mismatch_offers_store_anyway(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let other = "+++\npriority = 1\n+++\nx";
    let (v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "body"), ("o1.md", other), ("o2.md", other)],
        "a.md",
    );
    open_loaded(&rt, &started, cx);
    add(&started, "priority", "high", cx);
    settled(&rt, &started, cx);
    assert!(with_dialog(&started, cx, |d, _| d.form_error()).is_some());
    assert_eq!(
        with_dialog(&started, cx, |d, _| d.store_anyway_kind()),
        Some(PropertyKind::Text)
    );
    assert_eq!(disk(&rt, &v, "a.md"), "body", "nothing written");
    click(&started, STORE_ANYWAY_ID, cx);
    settled(&rt, &started, cx);
    assert_eq!(
        prop(&rt, &v, "a.md", "priority"),
        Some(Some(PropertyValue::Text("high".into())))
    );
}

/// A chosen type is written as that type.
#[gpui_kit::test]
fn the_type_select_forces_the_kind(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "body")], "a.md");
    open_loaded(&rt, &started, cx);
    press(&started, "a", cx);
    type_in(&started, "code", cx);
    let d = dialog(&started, cx).unwrap();
    cx.update_window(started.handle, |_, window, cx| {
        d.update(cx, |d, cx| {
            d.choose_kind(Some(PropertyKind::Text), window, cx)
        })
    })
    .unwrap();
    assert_eq!(cx.read(|cx| d.read(cx).chosen_kind(cx)), Some("text"));
    click(&started, VALUE_INPUT_ID, cx);
    type_in(&started, "42", cx);
    press(&started, "enter", cx);
    settled(&rt, &started, cx);
    assert_eq!(
        prop(&rt, &v, "a.md", "code"),
        Some(Some(PropertyValue::Text("42".into())))
    );
}

/// Known keys are suggested under the Key field; Down + Enter takes one.
#[gpui_kit::test]
fn known_keys_are_suggested(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[
            ("a.md", "body"),
            ("o.md", "+++\ndeadline = 2026-01-02\n+++\nx"),
        ],
        "a.md",
    );
    open_loaded(&rt, &started, cx);
    // The vault's keys load beside the note's properties: typing before they
    // land would suggest from nothing (and then, as they land, suggest).
    drive(&rt, cx, |cx| {
        with_dialog(&started, cx, |d, _| !d.known_keys().is_empty())
    });
    press(&started, "a", cx);
    type_in(&started, "dead", cx);
    assert_eq!(
        with_dialog(&started, cx, |d, _| d.suggestions()),
        vec!["deadline".to_string()]
    );
    assert!(shows(&started, suggestion_id("deadline"), cx));
    press(&started, "down", cx);
    press(&started, "enter", cx);
    let key = with_dialog(&started, cx, |d, cx| d.key_text(cx));
    assert_eq!(key, "deadline");
    assert!(
        with_dialog(&started, cx, |d, _| d.in_form()),
        "taking a key does not save"
    );
}

// ── write safety ─────────────────────────────────────────────────────────

/// Unsaved typing is flushed before the dialog opens.
#[gpui_kit::test]
fn unsaved_edits_are_saved_before_it_opens(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", TOML)], "a.md");
    type_in(&started, "X", cx);
    assert!(is_dirty(&started, cx));
    open_loaded(&rt, &started, cx);
    assert!(!is_dirty(&started, cx));
    assert!(disk(&rt, &v, "a.md").starts_with('X'));

    // A write then keeps that text and adds the key.
    add(&started, "count", "1", cx);
    settled(&rt, &started, cx);
    let on_disk = disk(&rt, &v, "a.md");
    assert!(on_disk.contains("count = 1"), "{on_disk}");
    assert_eq!(started.text(cx), on_disk);
}

/// A flush that fails refuses to open: no dialog, the TUI's flash, the
/// edits kept and the keyboard back in the note.
#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_flush_refuses_to_open(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", TOML)], "a.md");
    let file = v.workspace.as_path().join("a.md");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
    type_in(&started, "X", cx);
    open_by_leader(&started, cx);
    drive(&rt, cx, |cx| idle(&started, cx));
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

    assert_eq!(started.flash(cx).as_deref(), Some(REFUSED));
    assert!(!kit_dialog_open(&started, cx));
    assert!(dialog(&started, cx).is_none());
    assert!(is_dirty(&started, cx), "the edits are still in the buffer");
    assert_eq!(disk(&rt, &v, "a.md"), TOML);
    type_in(&started, "Y", cx);
    assert!(started.text(cx).starts_with("XY"), "typing works again");
}

/// Should the open note become dirty after the dialog opened (the dialog
/// holds the keyboard, so only something else could do it), a write is
/// refused and nothing is written.
#[gpui_kit::test]
fn a_write_under_unsaved_edits_is_refused(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", TOML)], "a.md");
    open_loaded(&rt, &started, cx);
    started.workspace.update(cx, |w, cx| {
        w.active_state_mut()
            .editor
            .update(cx, |view, _| view.editor.insert_at_cursor("typed"))
    });
    assert!(is_dirty(&started, cx));
    add(&started, "count", "1", cx);
    cx.run_until_parked();
    let error = with_dialog(&started, cx, |d, _| d.form_error()).unwrap_or_default();
    assert!(error.contains("unsaved edits"), "{error}");
    assert!(idle(&started, cx), "nothing was started");
    assert_eq!(disk(&rt, &v, "a.md"), TOML);
}

/// A file operation closing every modal while a write runs: the write still
/// lands and the open note still reloads clean (the reload is the window's
/// own message).
#[gpui_kit::test]
fn closing_during_a_write_still_reloads(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", TOML), ("z.md", "z")], "a.md");
    open_loaded(&rt, &started, cx);
    add(&started, "count", "7", cx);
    assert!(
        with_dialog(&started, cx, |d, _| d.is_busy()),
        "the write is under way"
    );
    // Escape waits for it.
    press(&started, "escape", cx);
    assert!(kit_dialog_open(&started, cx), "Escape waits for the write");
    // A file operation lands meanwhile and closes the modals.
    started.workspace.update(cx, |w, cx| {
        w.on_app_message(AppMessage::FileOp(FileOp::Created(path("z.md"))), cx)
    });
    started.settle_dialog(cx);
    assert!(!kit_dialog_open(&started, cx));
    drive(&rt, cx, |cx| idle(&started, cx));
    let on_disk = disk(&rt, &v, "a.md");
    assert!(on_disk.contains("count = 7"), "{on_disk}");
    assert_eq!(started.text(cx), on_disk, "reloaded without the dialog");
    assert!(!is_dirty(&started, cx));
    assert_eq!(started.flash(cx).as_deref(), Some("Property saved"));
}

/// Results for another note are dropped by the dialog.
#[gpui_kit::test]
fn results_for_another_note_are_ignored(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", TOML), ("b.md", "b")], "a.md");
    open_loaded(&rt, &started, cx);
    let before = rows(&started, cx);
    started.workspace.update(cx, |w, cx| {
        w.on_app_message(
            AppMessage::OverlayData(OverlayData::PropertiesLoaded {
                path: path("b.md"),
                result: Ok(vec![("other".into(), None)]),
            }),
            cx,
        );
        w.on_app_message(
            AppMessage::OverlayData(OverlayData::PropertiesLoaded {
                path: path("b.md"),
                result: Err("broken".into()),
            }),
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(rows(&started, cx), before);
    assert!(with_dialog(&started, cx, |d, _| d.load_error().is_none()));
}

// ── doors, Escape, one modal ─────────────────────────────────────────────

/// The Footer's property count opens the dialog.
#[gpui_kit::test]
fn the_footer_count_opens_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", TOML)], "a.md");
    drive(&rt, cx, |cx| shows(&started, PROPERTIES_ID, cx));
    click(&started, PROPERTIES_ID, cx);
    drive(&rt, cx, |cx| dialog(&started, cx).is_some());
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Properties));
}

/// The note menu offers `Properties…` first, from the right-click and
/// from F2 alike, and its letter opens the dialog.
#[gpui_kit::test]
fn the_note_menu_offers_properties_from_the_right_click_and_f2(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", TOML)], "a.md");
    let properties = menu_item_element_id(MenuItem::Note(FileOpKind::Properties));
    press(&started, "f2", cx);
    assert!(shows(&started, properties, cx));
    press(&started, "escape", cx);
    assert!(!shows(&started, properties, cx));

    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click("editor", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(shows(&started, properties, cx));
    press(&started, "p", cx);
    drive(&rt, cx, |cx| dialog(&started, cx).is_some());
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Properties));
}

/// Escape goes back one layer: the form to the list, the list closes. While
/// the dialog is open no other modal opens over it.
#[gpui_kit::test]
fn escape_goes_back_a_layer_and_nothing_opens_over_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", TOML)], "a.md");
    open_loaded(&rt, &started, cx);
    press(&started, "ctrl-shift-p", cx);
    press(&started, "f2", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Properties));

    press(&started, "a", cx);
    assert!(with_dialog(&started, cx, |d, _| d.in_form()));
    press(&started, "escape", cx);
    assert!(kit_dialog_open(&started, cx));
    assert!(
        !with_dialog(&started, cx, |d, _| d.in_form()),
        "back to the list"
    );
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert!(!kit_dialog_open(&started, cx), "the list closes");
}

/// With no note open, the leader's route and the Footer's door say so.
#[gpui_kit::test]
fn without_a_note_the_leader_says_so(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("a.md", "a")], &[]));
    let started = open_started(cx, vault_start(&v, FirstNote::Restore), v.settings.clone());
    drive(&rt, cx, |cx| !started.indexing(cx));
    // The leader leaf's route (the empty editor Panel has no keyboard to
    // type the sequence into): the palette and the leader run this.
    let workspace = started.workspace.clone();
    cx.update_window(started.handle, |_, window, cx| {
        workspace.update(cx, |w, cx| {
            let route = w.route_of(LeaderAction::NoteProperties);
            w.perform_route(route, window, cx);
            // The Footer's door says the same.
            w.open_properties_for_open_note(window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(started.flash(cx).as_deref(), Some(NO_NOTE));
    assert!(dialog(&started, cx).is_none());
}

/// YAML frontmatter stays YAML: a number, a date and a list land in it.
#[gpui_kit::test]
fn yaml_frontmatter_takes_new_properties(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let yaml = "---\nstatus: draft\n---\nbody\n";
    let (v, started) = started_on(cx, &rt, &[("a.md", yaml)], "a.md");
    open_loaded(&rt, &started, cx);
    add(&started, "count", "2", cx);
    settled(&rt, &started, cx);
    add(&started, "due", "2026-11-04", cx);
    settled(&rt, &started, cx);
    add(&started, "tags", "a, b", cx);
    settled(&rt, &started, cx);
    let on_disk = disk(&rt, &v, "a.md");
    assert!(on_disk.starts_with("---\n"), "{on_disk}");
    assert_eq!(
        prop(&rt, &v, "a.md", "count"),
        Some(Some(PropertyValue::Number(2.0)))
    );
    assert!(matches!(
        prop(&rt, &v, "a.md", "due"),
        Some(Some(PropertyValue::Date(_)))
    ));
    assert_eq!(
        prop(&rt, &v, "a.md", "tags"),
        Some(Some(PropertyValue::List(vec!["a".into(), "b".into()])))
    );
    assert_eq!(started.text(cx), on_disk);
    assert!(!is_dirty(&started, cx));
}
