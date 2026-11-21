//! UI integration tests for the new note / new directory dialog: a real
//! `Workspace` on a real temp vault, the dialog driven through the window's
//! keys and pointer, every outcome checked on disk.
//!
//! Like the other vault tests, each test owns a current-thread tokio runtime
//! and drives it between GPUI's `run_until_parked` (`start_tests::drive`).
#![cfg(test)]

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::file_ops::ValidationState;
use kimun_notes::logic::layout::Panel;
use tokio::runtime::Runtime;

use super::directory_choice::choice_row_id;
use super::file_dialogs::CONFIRM_ID;
use super::files::{
    FILES_NEW_DIRECTORY_ID, FILES_NEW_ID, FILES_NEW_JOURNAL_ID, FILES_NEW_NOTE_ID,
    FILES_NEW_QUICK_NOTE_ID,
};
use super::modal::ModalKind;
use super::new_entry::{NEW_ENTRY_CHANGE_ID, NewEntryDialog};
use super::start_tests::{Started, drive, open_started, runtime, vault_start};
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

/// A vault holding `notes` and `dirs`, started on `first`, driven until it
/// shows.
fn started_on(
    cx: &mut TestAppContext,
    rt: &Runtime,
    notes: &[(&str, &str)],
    dirs: &[&str],
    first: &str,
) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(notes, &[]));
    for dir in dirs {
        rt.block_on(v.vault.create_directory(&path(dir))).unwrap();
    }
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path(first))),
        v.settings.clone(),
    );
    let want = notes.iter().find(|(p, _)| *p == first).unwrap().1;
    // Wait for the indexing too. Its `IndexingDone` message reloads the
    // folder counts on tokio. If it is still in flight when the test ends,
    // the harness drains it after the runtime guard is gone, and the spawn
    // panics with `no reactor running`.
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

fn click(s: &Started, id: &'static str, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn click_dir_row(s: &Started, dir: &str, cx: &mut TestAppContext) {
    let id = choice_row_id(&path(dir));
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn modal(s: &Started, cx: &mut TestAppContext) -> Option<ModalKind> {
    s.shown_modal(cx)
}

/// The dialog while it is up.
fn dialog(s: &Started, cx: &gpui_kit::App) -> Entity<NewEntryDialog> {
    s.workspace
        .read(cx)
        .new_entry
        .clone()
        .and_then(|d| d.upgrade())
        .expect("the new entry dialog is up")
}

fn target(s: &Started, cx: &TestAppContext) -> Option<Result<VaultPath, String>> {
    cx.read(|cx| dialog(s, cx).read(cx).target().cloned())
}

fn base(s: &Started, cx: &TestAppContext) -> VaultPath {
    cx.read(|cx| dialog(s, cx).read(cx).base().clone())
}

fn state(s: &Started, cx: &TestAppContext) -> ValidationState {
    cx.read(|cx| dialog(s, cx).read(cx).state())
}

fn focused_panel(s: &Started, cx: &TestAppContext) -> Panel {
    cx.read(|cx| s.workspace.read(cx).focused_panel())
}

/// The note the editor shows.
fn open_note(s: &Started, cx: &TestAppContext) -> Option<VaultPath> {
    cx.read(|cx| s.workspace.read(cx).status.note.clone())
}

/// The note chord opens the new note dialog and the directory chord the new
/// directory one, each on the open note's folder.
#[gpui_kit::test]
fn the_chords_open_the_dialogs_on_the_notes_folder(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("d/a.md", "alpha")], &[], "d/a.md");

    press(&started, "ctrl-n", cx);
    assert_eq!(modal(&started, cx), Some(ModalKind::NewNote));
    assert!(base(&started, cx).is_like(&path("d")));
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(modal(&started, cx), None);

    press(&started, "ctrl-shift-n", cx);
    assert_eq!(modal(&started, cx), Some(ModalKind::NewDirectory));
    assert!(base(&started, cx).is_like(&path("d")));
}

/// A name typed in the dialog is resolved against the folder as it is typed:
/// extension added, lowercased, shown.
#[gpui_kit::test]
fn typing_resolves_the_target(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("d/a.md", "alpha")], &[], "d/a.md");
    press(&started, "ctrl-n", cx);
    assert_eq!(
        target(&started, cx),
        None,
        "nothing typed, nothing resolved"
    );
    type_in(&started, "Fresh Idea", cx);
    assert_eq!(
        target(&started, cx),
        Some(Ok(path("/d/fresh idea.md"))),
        "{:?}",
        target(&started, cx)
    );
    drive(&rt, cx, |cx| {
        state(&started, cx) == ValidationState::Available
    });
}

/// Enter makes the note through core (the folders in a typed path too), the
/// dialog closes and the note opens with the keyboard.
#[gpui_kit::test]
fn enter_makes_the_note_and_opens_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("d/a.md", "alpha")], &[], "d/a.md");
    press(&started, "ctrl-n", cx);
    type_in(&started, "deeper/fresh", cx);
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| {
        open_note(&started, cx).is_some_and(|n| n.is_like(&path("d/deeper/fresh.md")))
    });
    assert_eq!(disk(&rt, &v, "d/deeper/fresh.md").as_deref(), Some(""));
    assert_eq!(disk(&rt, &v, "d/a.md").as_deref(), Some("alpha"));
    started.settle_dialog(cx);
    assert_eq!(modal(&started, cx), None);
    assert_eq!(focused_panel(&started, cx), Panel::Editor);
}

/// A name core refuses shows why, and Enter makes nothing.
#[gpui_kit::test]
fn a_refused_name_blocks_enter(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], &[], "a.md");
    press(&started, "ctrl-n", cx);
    type_in(&started, "bad?name", cx);
    assert!(matches!(target(&started, cx), Some(Err(reason)) if reason.contains('?')));
    press(&started, "enter", cx);
    cx.run_until_parked();
    assert_eq!(modal(&started, cx), Some(ModalKind::NewNote), "stays open");
    assert!(!exists(&rt, &v, "bad_name.md"));
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(CONFIRM_ID).disabled(), Some(true));
    })
    .unwrap();
}

/// A name that is taken is marked, and Enter leaves the note alone.
#[gpui_kit::test]
fn a_taken_name_cannot_be_made(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha"), ("b.md", "beta")], &[], "a.md");
    press(&started, "ctrl-n", cx);
    type_in(&started, "b", cx);
    drive(&rt, cx, |cx| state(&started, cx) == ValidationState::Taken);
    press(&started, "enter", cx);
    cx.run_until_parked();
    assert_eq!(modal(&started, cx), Some(ModalKind::NewNote));
    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta"));
}

/// The directory dialog makes the directory, closes, and FILES shows its
/// folder with the new row selected.
#[gpui_kit::test]
fn a_new_directory_is_made_and_pointed_out(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], &[], "a.md");
    press(&started, "ctrl-shift-n", cx);
    type_in(&started, "archive/old", cx);
    press(&started, "enter", cx);
    drive(&rt, cx, |_| exists(&rt, &v, "archive/old"));
    started.settle_dialog(cx);
    assert_eq!(modal(&started, cx), None);
    // FILES shows `archive` and has `old` selected.
    drive(&rt, cx, |cx| {
        cx.read(|cx| {
            let files = started.workspace.read(cx).files.clone().unwrap();
            let pane = files.read(cx);
            pane.current_dir().is_like(&path("archive"))
                && pane
                    .model()
                    .core()
                    .and_then(|core| core.selected_row())
                    .is_some_and(|row| row.path().is_like(&path("archive/old")))
        })
    });
}

/// `Change…` opens the folder choice on the folder the dialog opened on;
/// picking another re-resolves the name against it, and Enter makes the
/// note there.
#[gpui_kit::test]
fn the_folder_can_be_chosen(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(
        cx,
        &rt,
        &[("d/a.md", "alpha")],
        &["other", "other/inner"],
        "d/a.md",
    );
    press(&started, "ctrl-n", cx);
    type_in(&started, "sub/idea", cx);
    assert_eq!(target(&started, cx), Some(Ok(path("/d/sub/idea.md"))));

    click(&started, NEW_ENTRY_CHANGE_ID, cx);
    let choice = cx.read(|cx| dialog(&started, cx).read(cx).choice().cloned());
    let choice = choice.expect("the choice is open");
    drive(&rt, cx, |cx| !cx.read(|cx| choice.read(cx).is_loading()));
    assert_eq!(
        cx.read(|cx| choice.read(cx).highlighted()),
        Some(path("/d")),
        "the folder the dialog opened on"
    );

    click_dir_row(&started, "/other/inner", cx);
    assert_eq!(base(&started, cx), path("/other/inner"));
    assert!(
        cx.read(|cx| dialog(&started, cx).read(cx).choice().is_none()),
        "the choice closes on a pick"
    );
    assert_eq!(
        target(&started, cx),
        Some(Ok(path("/other/inner/sub/idea.md"))),
        "the name is kept and re-resolved"
    );

    press(&started, "enter", cx);
    drive(&rt, cx, |cx| {
        open_note(&started, cx).is_some_and(|n| n.is_like(&path("other/inner/sub/idea.md")))
    });
    assert!(exists(&rt, &v, "other/inner/sub/idea.md"));
    assert!(!exists(&rt, &v, "d/sub/idea.md"));
}

/// Escape in the folder choice closes the choice, not the dialog; Escape
/// again closes the dialog.
#[gpui_kit::test]
fn escape_closes_the_choice_first(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], &["other"], "a.md");
    press(&started, "ctrl-n", cx);
    click(&started, NEW_ENTRY_CHANGE_ID, cx);
    assert!(cx.read(|cx| dialog(&started, cx).read(cx).choice().is_some()));
    press(&started, "escape", cx);
    assert!(cx.read(|cx| dialog(&started, cx).read(cx).choice().is_none()));
    assert_eq!(modal(&started, cx), Some(ModalKind::NewNote));
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(modal(&started, cx), None);
}

/// The folder choice filters the folders as it is typed, and Enter picks
/// the best match.
#[gpui_kit::test]
fn the_choice_filters_and_enter_picks(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha")],
        &["archive", "projects", "projects/rust"],
        "a.md",
    );
    press(&started, "ctrl-n", cx);
    click(&started, NEW_ENTRY_CHANGE_ID, cx);
    let choice = cx
        .read(|cx| dialog(&started, cx).read(cx).choice().cloned())
        .unwrap();
    drive(&rt, cx, |cx| !cx.read(|cx| choice.read(cx).is_loading()));
    type_in(&started, "rust", cx);
    assert_eq!(
        cx.read(|cx| choice.read(cx).highlighted()),
        Some(path("/projects/rust"))
    );
    press(&started, "enter", cx);
    assert_eq!(base(&started, cx), path("/projects/rust"));
}

/// Leader `n n` and `n d` open the dialogs (leader `n n` used to focus the
/// FILES filter).
#[gpui_kit::test]
fn the_leader_opens_the_dialogs(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], &[], "a.md");
    press(&started, "ctrl-shift-b", cx);
    type_in(&started, "nn", cx);
    assert_eq!(modal(&started, cx), Some(ModalKind::NewNote));
    assert_eq!(started.text(cx), "alpha", "nn typed nothing into the note");
    press(&started, "escape", cx);
    started.settle_dialog(cx);

    press(&started, "ctrl-shift-b", cx);
    type_in(&started, "nd", cx);
    assert_eq!(modal(&started, cx), Some(ModalKind::NewDirectory));
}

/// FILES' `+` menu offers both, each opening its dialog.
#[gpui_kit::test]
fn the_files_plus_menu_opens_both_dialogs(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], &[], "a.md");
    for (item, kind) in [
        (FILES_NEW_NOTE_ID, ModalKind::NewNote),
        (FILES_NEW_DIRECTORY_ID, ModalKind::NewDirectory),
    ] {
        click(&started, FILES_NEW_ID, cx);
        click(&started, item, cx);
        cx.run_until_parked();
        assert_eq!(modal(&started, cx), Some(kind), "{item}");
        press(&started, "escape", cx);
        started.settle_dialog(cx);
        assert_eq!(modal(&started, cx), None);
    }
}

/// The `New` menu also holds the quick note and today's journal: each does
/// what its chord does.
#[gpui_kit::test]
fn the_files_new_menu_offers_the_quick_note_and_the_journal(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], &[], "a.md");
    click(&started, FILES_NEW_ID, cx);
    click(&started, FILES_NEW_QUICK_NOTE_ID, cx);
    cx.run_until_parked();
    assert_eq!(modal(&started, cx), Some(ModalKind::QuickNote));
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(modal(&started, cx), None);

    click(&started, FILES_NEW_ID, cx);
    click(&started, FILES_NEW_JOURNAL_ID, cx);
    drive(&rt, cx, |cx| {
        cx.read(|cx| {
            let w = started.workspace.read(cx);
            w.active_state()
                .notes
                .as_ref()
                .and_then(|h| h.note.clone())
                .is_some_and(|n| n.to_string().contains("journal"))
        })
    });
}

/// The dialog refuses to open over another modal, and its chord does
/// nothing then.
#[gpui_kit::test]
fn it_does_not_open_over_another_modal(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], &[], "a.md");
    press(&started, "ctrl-shift-w", cx);
    assert_eq!(modal(&started, cx), Some(ModalKind::QuickNote));
    press(&started, "ctrl-n", cx);
    assert_eq!(modal(&started, cx), Some(ModalKind::QuickNote));
}

/// A create core refuses (a parent that is a file) stays in the dialog,
/// which stays open and says why; the next keystroke clears it and the
/// dialog works again.
#[gpui_kit::test]
fn a_failed_create_stays_in_the_dialog(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], &[], "a.md");
    rt.block_on(v.vault.save_attachment(&path("pic.png"), b"\x89PNG"))
        .unwrap();
    press(&started, "ctrl-n", cx);
    type_in(&started, "pic.png/idea", cx);
    press(&started, "enter", cx);
    drive(&rt, cx, |cx| {
        cx.read(|cx| started.workspace.read(cx).note_error(cx))
            .is_some()
    });
    assert_eq!(modal(&started, cx), Some(ModalKind::NewNote), "stays open");
    assert!(!exists(&rt, &v, "pic.png/idea.md"));
    assert_eq!(
        open_note(&started, cx),
        Some(path("a.md")),
        "nothing opened"
    );
    type_in(&started, "x", cx);
    assert_eq!(
        cx.read(|cx| started.workspace.read(cx).note_error(cx)),
        None,
        "typing clears the error"
    );
}

/// Enter in a folder choice with nothing to choose closes the choice (the
/// keyboard is not left dead) and leaves the dialog and its folder alone.
#[gpui_kit::test]
fn enter_with_nothing_to_choose_closes_the_choice(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha")], &["other"], "a.md");
    press(&started, "ctrl-n", cx);
    click(&started, NEW_ENTRY_CHANGE_ID, cx);
    let choice = cx
        .read(|cx| dialog(&started, cx).read(cx).choice().cloned())
        .unwrap();
    drive(&rt, cx, |cx| !cx.read(|cx| choice.read(cx).is_loading()));
    type_in(&started, "zzzzqq", cx);
    assert_eq!(cx.read(|cx| choice.read(cx).highlighted()), None);
    press(&started, "enter", cx);
    assert!(cx.read(|cx| dialog(&started, cx).read(cx).choice().is_none()));
    assert_eq!(modal(&started, cx), Some(ModalKind::NewNote));
    assert_eq!(base(&started, cx), path("/"), "the folder is unchanged");
}
