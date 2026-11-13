//! UI integration tests for the saved searches (M7b G6): the Saved
//! Searches picker, the save-search dialog and FIND's name for a saved
//! search, on a real `Workspace` over a real, indexed temp vault whose
//! saved searches are core's.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`): the list, the names the
//! dialog checks and the save all land only when tokio runs, which is also
//! the proof that none of them is read on the UI thread.
#![cfg(test)]

use gpui_kit::test::{TestQueryExt as _, TestWindowExt};
use gpui_kit::{AppContext as _, Entity, Focusable as _, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::saved_search::{SearchItem, VIRTUAL_BACKLINKS_NAME};
use tokio::runtime::Runtime;

use super::file_dialogs::CONFIRM_ID;
use super::find::{FIND_DEBOUNCE, FIND_INPUT_ID, FIND_SAVE_ID, FindView};
use super::modal::ModalKind;
use super::palette::PaletteRun;
use super::picker::{NotePicker, PICKER_DELETE_ID, PICKER_INPUT_ID};
use super::route::NEEDS_FIND_FOCUS;
use super::saved_searches::{
    BACKLINKS_ROW_ID, SAVE_SEARCH_INPUT_ID, SaveSearchDialog, saved_row_id,
};
use super::start_tests::select_all_key;
use super::start_tests::{Started, drive, open_started, runtime, vault_start};
use crate::startup::FirstNote;
use crate::test_vault::{TestVault, test_vault};

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

const NOTES: &[(&str, &str)] = &[
    ("alpha.md", "# Alpha\nthe alpha note #rust, see [[beta]]"),
    ("beta.md", "# Beta\nback to [[alpha]] here #rust #go"),
    ("gamma.md", "# Gamma\nplain #go"),
];

/// A vault holding [`NOTES`] and the saved searches `saved` (name, query,
/// written through core first), started on alpha and indexed.
fn started_with(
    cx: &mut TestAppContext,
    rt: &Runtime,
    saved: &[(&str, &str)],
) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(NOTES, &[]));
    for (name, query) in saved {
        rt.block_on(v.vault.save_search(name, query)).unwrap();
    }
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("alpha.md"))),
        v.settings.clone(),
    );
    let want = NOTES[0].1;
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

fn focused(s: &Started, id: &'static str, cx: &mut TestAppContext) -> bool {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .is_some_and(|e| e.focused() == Some(true))
    })
    .unwrap()
}

/// The saved searches core keeps: (name, query), in stored order.
fn stored(rt: &Runtime, v: &TestVault) -> Vec<(String, String)> {
    rt.block_on(v.vault.list_saved_searches())
        .unwrap()
        .into_iter()
        .map(|s| (s.name, s.query))
        .collect()
}

fn list(s: &Started, cx: &TestAppContext) -> Entity<NotePicker<SearchItem>> {
    cx.read(|cx| {
        s.workspace
            .read(cx)
            .saved_searches
            .as_ref()
            .and_then(|p| p.upgrade())
            .expect("the saved searches are open")
    })
}

/// The rows once loaded: (number, name), the Backlinks entry first.
fn rows(s: &Started, rt: &Runtime, cx: &mut TestAppContext) -> Vec<(Option<u8>, String)> {
    let view = list(s, cx);
    drive(rt, cx, |cx| !cx.read(|cx| view.read(cx).is_loading()));
    cx.read(|cx| {
        view.read(cx)
            .rows()
            .into_iter()
            .map(|r| (r.index, r.name))
            .collect()
    })
}

fn selected_name(s: &Started, cx: &TestAppContext) -> Option<String> {
    let view = list(s, cx);
    cx.read(|cx| view.read(cx).selected().map(|r| r.name))
}

fn find(s: &Started, cx: &TestAppContext) -> Entity<FindView> {
    cx.read(|cx| s.workspace.read(cx).find.clone().expect("FIND is built"))
}

fn find_title(s: &Started, cx: &TestAppContext) -> String {
    let view = find(s, cx);
    cx.read(|cx| view.read(cx).shown_title(cx))
}

fn find_query(s: &Started, cx: &TestAppContext) -> String {
    let view = find(s, cx);
    cx.read(|cx| view.read(cx).query(cx))
}

/// FIND's result paths once its rows landed.
fn find_names(s: &Started, rt: &Runtime, cx: &mut TestAppContext) -> Vec<String> {
    let view = find(s, cx);
    let list = cx.read(|cx| view.read(cx).list.clone());
    drive(rt, cx, |cx| !cx.read(|cx| list.read(cx).is_loading()));
    cx.read(|cx| {
        list.read(cx)
            .rows()
            .iter()
            .map(|r| r.entry.path.get_clean_name())
            .collect()
    })
}

fn dialog(s: &Started, cx: &TestAppContext) -> Entity<SaveSearchDialog> {
    cx.read(|cx| {
        s.workspace
            .read(cx)
            .save_search
            .as_ref()
            .and_then(|open| open.dialog.upgrade())
            .expect("the save dialog is open")
    })
}

/// The save dialog's (confirm label, enabled) once the saved names are in.
fn confirm(s: &Started, rt: &Runtime, cx: &mut TestAppContext) -> (&'static str, bool) {
    let view = dialog(s, cx);
    drive(rt, cx, |cx| {
        !cx.read(|cx| view.read(cx).hint_text(cx))
            .starts_with("Checking")
    });
    cx.read(|cx| view.read(cx).confirm(cx))
}

fn hint(s: &Started, cx: &TestAppContext) -> String {
    let view = dialog(s, cx);
    cx.read(|cx| view.read(cx).hint_text(cx))
}

/// The save under way ended (the dialog closed, or it shows an error).
fn saved(s: &Started, rt: &Runtime, cx: &mut TestAppContext) {
    drive(rt, cx, |cx| {
        cx.read(|cx| s.workspace.read(cx).save_task.is_none())
    });
    s.settle_dialog(cx);
}

/// Show FIND (the rail) and run `query` in it.
fn find_on(s: &Started, rt: &Runtime, query: &str, cx: &mut TestAppContext) {
    if !focused(s, FIND_INPUT_ID, cx) {
        click_rail_find(s, cx);
    }
    press(s, select_all_key(), cx);
    press(s, "backspace", cx);
    type_in(s, query, cx);
    cx.executor().advance_clock(FIND_DEBOUNCE);
    cx.run_until_parked();
    find_names(s, rt, cx);
}

fn click_rail_find(s: &Started, cx: &mut TestAppContext) {
    let id = super::rail_cell_id(DrawerView::Find);
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

// ── The Saved Searches ───────────────────────────────────────────────────

/// F3 opens the Saved Searches with the filter focused: the Backlinks entry
/// on top (not numbered), then the saved searches core keeps, the first
/// nine numbered. The rows load on tokio: nothing is there before it runs.
#[gpui_kit::test]
fn f3_lists_the_backlinks_entry_then_the_saved_searches(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_with(cx, &rt, &[("todo", "#rust"), ("ideas", "#go")]);
    press(&started, "f3", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::SavedSearches));
    assert!(focused(&started, PICKER_INPUT_ID, cx), "the filter types");
    let view = list(&started, cx);
    assert!(
        cx.read(|cx| view.read(cx).rows().iter().all(|r| r.is_virtual)),
        "the saved searches are read off the UI thread, not yet here"
    );
    assert_eq!(
        rows(&started, &rt, cx),
        [
            (None, VIRTUAL_BACKLINKS_NAME.to_string()),
            (Some(1), "todo".to_string()),
            (Some(2), "ideas".to_string()),
        ]
    );
    assert_eq!(
        selected_name(&started, cx).as_deref(),
        Some(VIRTUAL_BACKLINKS_NAME)
    );
    assert!(started.shows(BACKLINKS_ROW_ID, cx));
    // F3 again closes it; the editor has the keyboard back.
    press(&started, "f3", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    let editor = cx.read(|cx| started.workspace.read(cx).active_state().editor.clone());
    let back = cx
        .update_window(started.handle, |_, window, cx| {
            editor.read(cx).focus_handle(cx).is_focused(window)
        })
        .unwrap();
    assert!(back, "the keyboard returns to the editor");
}

/// A digit types into the filter (it is still text) and puts the search
/// with that number under Enter; a name filters too; clearing the filter
/// puts the Backlinks entry back on top.
#[gpui_kit::test]
fn a_number_or_a_name_selects_the_search(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_with(
        cx,
        &rt,
        &[("todo", "#rust"), ("ideas", "#go"), ("2024 plans", "plans")],
    );
    press(&started, "f3", cx);
    rows(&started, &rt, cx);
    type_in(&started, "2", cx);
    let view = list(&started, cx);
    assert_eq!(cx.read(|cx| view.read(cx).query().to_string()), "2");
    assert_eq!(selected_name(&started, cx).as_deref(), Some("ideas"));
    let names: Vec<String> = rows(&started, &rt, cx).into_iter().map(|r| r.1).collect();
    assert_eq!(
        names,
        [VIRTUAL_BACKLINKS_NAME, "ideas", "2024 plans"],
        "the number first, then names holding it; Backlinks stays"
    );
    press(&started, "backspace", cx);
    type_in(&started, "TOD", cx);
    assert_eq!(selected_name(&started, cx).as_deref(), Some("todo"));
    press(&started, select_all_key(), cx);
    press(&started, "backspace", cx);
    assert_eq!(
        selected_name(&started, cx).as_deref(),
        Some(VIRTUAL_BACKLINKS_NAME)
    );
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
}

/// A click on a saved search runs it in FIND at once, as Enter does (C1);
/// a right click runs nothing.
#[gpui_kit::test]
fn a_click_runs_a_saved_search(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_with(cx, &rt, &[("rusty", "#rust"), ("go", "#go")]);
    press(&started, "f3", cx);
    rows(&started, &rt, cx);
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(saved_row_id("go"), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(started.shown_modal(cx), Some(ModalKind::SavedSearches));
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(saved_row_id("go"), cx)
    })
    .unwrap();
    cx.run_until_parked();
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert_eq!(find_query(&started, cx), "#go");
    assert_eq!(find_title(&started, cx), "go");
}

/// Enter runs the search in FIND, which is named after it; an edit marks
/// it "• edited"; saving it again under its name (offered by the dialog,
/// which says it updates it) drops the mark.
#[gpui_kit::test]
fn enter_runs_it_in_find_named_and_a_resave_drops_edited(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_with(cx, &rt, &[("rusty", "#rust"), ("go", "#go")]);
    press(&started, "f3", cx);
    rows(&started, &rt, cx);
    type_in(&started, "1", cx);
    press(&started, "enter", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert_eq!(find_query(&started, cx), "#rust");
    assert_eq!(find_names(&started, &rt, cx), ["alpha", "beta"]);
    assert_eq!(find_title(&started, cx), "rusty");
    assert!(
        focused(&started, FIND_INPUT_ID, cx),
        "FIND has the keyboard"
    );

    // Edited away from the saved text.
    type_in(&started, " #go", cx);
    assert_eq!(find_title(&started, cx), "rusty • edited");

    // Ctrl+D in FIND: the name is offered, and saving updates it.
    press(&started, "ctrl-d", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::SaveSearch));
    assert!(focused(&started, SAVE_SEARCH_INPUT_ID, cx));
    assert_eq!(confirm(&started, &rt, cx), ("Update", true));
    assert!(hint(&started, cx).starts_with("Updates \"rusty\""));
    press(&started, "enter", cx);
    saved(&started, &rt, cx);
    assert_eq!(started.shown_modal(cx), None);
    assert_eq!(find_title(&started, cx), "rusty", "saved: no longer edited");
    assert_eq!(
        stored(&rt, &v),
        [
            ("rusty".to_string(), "#rust #go".to_string()),
            ("go".to_string(), "#go".to_string())
        ]
    );
    assert_eq!(started.flash(cx).as_deref(), Some("Saved \"rusty\""));
}

/// The Backlinks entry runs `<{note}` in FIND (titled "Backlinks").
#[gpui_kit::test]
fn the_backlinks_entry_runs_the_open_notes_backlinks(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_with(cx, &rt, &[]);
    press(&started, "f3", cx);
    rows(&started, &rt, cx);
    press(&started, "enter", cx);
    started.settle_dialog(cx);
    assert_eq!(find_query(&started, cx), "<{note}");
    assert_eq!(find_names(&started, &rt, cx), ["beta"]);
    assert_eq!(find_title(&started, cx), "Backlinks");
}

/// Delete asks in an alert over the list (Escape keeps the search, the
/// list stays); Delete in it removes it through core and the list reloads.
/// The Backlinks entry is not a saved search: Delete does nothing on it and
/// the `Delete…` button is disabled.
#[gpui_kit::test]
fn deleting_asks_first_and_the_backlinks_entry_stays(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_with(cx, &rt, &[("todo", "#rust"), ("ideas", "#go")]);
    press(&started, "f3", cx);
    rows(&started, &rt, cx);
    let alerts = |cx: &mut TestAppContext| {
        cx.update_window(started.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_all_by_role(gpui_kit::Role::AlertDialog).len()
        })
        .unwrap()
    };
    // On the Backlinks entry: nothing to delete.
    press(&started, "delete", cx);
    assert_eq!(alerts(cx), 0);
    let disabled = cx
        .update_window(started.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(PICKER_DELETE_ID).disabled()
        })
        .unwrap();
    assert_eq!(disabled, Some(true));

    press(&started, "down", cx);
    assert_eq!(selected_name(&started, cx).as_deref(), Some("todo"));
    press(&started, "delete", cx);
    assert_eq!(alerts(cx), 1, "the delete asks in an alert over the list");
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(alerts(cx), 0);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::SavedSearches));
    assert_eq!(stored(&rt, &v).len(), 2);

    click(&started, PICKER_DELETE_ID, cx);
    assert_eq!(alerts(cx), 1, "the button asks too");
    press(&started, "enter", cx);
    started.settle_dialog(cx);
    let view = list(&started, cx);
    drive(&rt, cx, |cx| {
        cx.read(|cx| view.read(cx).rows().len()) == 2 && !cx.read(|cx| view.read(cx).is_loading())
    });
    assert_eq!(stored(&rt, &v), [("ideas".to_string(), "#go".to_string())]);
    let names: Vec<String> = rows(&started, &rt, cx).into_iter().map(|r| r.1).collect();
    assert_eq!(names, [VIRTUAL_BACKLINKS_NAME, "ideas"]);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::SavedSearches));
    assert!(focused(&started, PICKER_INPUT_ID, cx), "back to the filter");
}

// ── The save dialog ──────────────────────────────────────────────────────

/// A new name: the names are read off the UI thread first (the confirm
/// waits), a blank name is refused, then `Save` creates it through core,
/// FIND is named for it, and the Saved Searches list it.
#[gpui_kit::test]
fn saving_from_find_creates_a_search(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_with(cx, &rt, &[]);
    find_on(&started, &rt, "#go", cx);
    click(&started, FIND_SAVE_ID, cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::SaveSearch));
    // Blank (nothing offered: the query came from no saved search).
    let view = dialog(&started, cx);
    assert_eq!(cx.read(|cx| view.read(cx).confirm(cx)), ("Save", false));
    press(&started, "enter", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::SaveSearch));
    let disabled = cx
        .update_window(started.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(CONFIRM_ID).disabled()
        })
        .unwrap();
    assert_eq!(disabled, Some(true), "a blank name cannot be saved");
    type_in(&started, "go notes", cx);
    assert_eq!(confirm(&started, &rt, cx), ("Save", true));
    assert_eq!(hint(&started, cx), "Creates a new saved search");
    press(&started, "enter", cx);
    saved(&started, &rt, cx);
    assert_eq!(started.shown_modal(cx), None);
    assert_eq!(
        stored(&rt, &v),
        [("go notes".to_string(), "#go".to_string())],
        "saved through core: what the TUI's F3 lists"
    );
    assert_eq!(find_title(&started, cx), "go notes");
    press(&started, "f3", cx);
    let names: Vec<String> = rows(&started, &rt, cx).into_iter().map(|r| r.1).collect();
    assert_eq!(names, [VIRTUAL_BACKLINKS_NAME, "go notes"]);
}

/// Another search's name (any case) says it overwrites it, in words and on
/// the button; saving replaces that search's query, never silently.
#[gpui_kit::test]
fn an_existing_name_says_overwrite(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_with(cx, &rt, &[("todo", "#rust")]);
    find_on(&started, &rt, "#go", cx);
    press(&started, "ctrl-d", cx);
    type_in(&started, "TODO", cx);
    assert_eq!(confirm(&started, &rt, cx), ("Overwrite", true));
    assert_eq!(hint(&started, cx), "Overwrites the saved search \"todo\"");
    let label = cx
        .update_window(started.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(CONFIRM_ID).label().map(str::to_string)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("Overwrite"));
    press(&started, "enter", cx);
    saved(&started, &rt, cx);
    let all = stored(&rt, &v);
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(all[0].1, "#go");
}

/// A save core refuses stays in the dialog with the reason (and is
/// flashed); the name typed is kept.
#[gpui_kit::test]
fn a_failed_save_stays_in_the_dialog(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_with(cx, &rt, &[]);
    find_on(&started, &rt, "#go", cx);
    press(&started, "ctrl-d", cx);
    type_in(&started, "go", cx);
    assert_eq!(confirm(&started, &rt, cx), ("Save", true));
    // The store becomes unreadable: a folder where its file goes.
    let store = v
        .workspace
        .as_path()
        .join(".kimun")
        .join("saved-searches.toml");
    std::fs::create_dir_all(&store).unwrap();
    press(&started, "enter", cx);
    saved(&started, &rt, cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::SaveSearch));
    let view = dialog(&started, cx);
    let error = cx.read(|cx| {
        let dialog = view.read(cx);
        dialog.error(cx)
    });
    assert!(error.is_some(), "the reason shows in the dialog");
    assert_eq!(
        started.flash(cx).as_deref(),
        Some("Couldn't save the search \"go\"")
    );
    assert_eq!(
        cx.read(|cx| view.read(cx).input.read(cx).value().to_string()),
        "go"
    );
}

/// Ctrl+D in the search browser: the browser gives way to the save dialog
/// (one modal at a time) and comes back on its query when it closes, saved
/// or not. A browser save never touches FIND's name.
#[gpui_kit::test]
fn a_browser_save_comes_back_to_the_browser_and_leaves_find(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_with(cx, &rt, &[("rusty", "#rust")]);
    // FIND named for "rusty".
    press(&started, "f3", cx);
    rows(&started, &rt, cx);
    type_in(&started, "1", cx);
    press(&started, "enter", cx);
    started.settle_dialog(cx);
    assert_eq!(find_title(&started, cx), "rusty");

    let browse = |cx: &mut TestAppContext| {
        press(&started, "ctrl-k", cx);
        assert_eq!(started.shown_modal(cx), Some(ModalKind::Search));
        type_in(&started, "#go", cx);
    };
    // Cancel: back to the browser on its query.
    browse(cx);
    press(&started, "ctrl-d", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::SaveSearch));
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Search));
    let picker = cx.read(|cx| {
        started
            .workspace
            .read(cx)
            .picker
            .as_ref()
            .and_then(|p| p.upgrade())
            .unwrap()
    });
    assert_eq!(cx.read(|cx| picker.read(cx).query().to_string()), "#go");
    assert!(focused(&started, PICKER_INPUT_ID, cx));

    // Save: stored, the browser back, FIND still "rusty".
    press(&started, "ctrl-d", cx);
    type_in(&started, "go", cx);
    assert_eq!(confirm(&started, &rt, cx), ("Save", true));
    press(&started, "enter", cx);
    saved(&started, &rt, cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Search));
    assert_eq!(
        stored(&rt, &v),
        [
            ("rusty".to_string(), "#rust".to_string()),
            ("go".to_string(), "#go".to_string())
        ]
    );
    assert_eq!(find_title(&started, cx), "rusty", "FIND's name untouched");
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
}

// ── Doors ────────────────────────────────────────────────────────────────

/// Leader `f s` and the palette's `+find · saved searches` open it (the
/// row shows F3); the palette's `+find · save query…` needs FIND focused,
/// and runs from there. Ctrl+D in the editor is not a save.
#[gpui_kit::test]
fn the_leader_and_the_palette(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_with(cx, &rt, &[]);
    press(&started, "ctrl-d", cx);
    assert_eq!(started.shown_modal(cx), None, "the editor keeps Ctrl+D");

    press(&started, "ctrl-shift-b", cx);
    type_in(&started, "fs", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::SavedSearches));
    press(&started, "escape", cx);
    started.settle_dialog(cx);

    let entries = |opener: Panel, cx: &mut TestAppContext| {
        cx.read(|cx| started.workspace.read(cx).palette_entries(opener, cx))
    };
    let from_editor = entries(Panel::Editor, cx);
    let saved = from_editor
        .iter()
        .find(|e| *e.run() == PaletteRun::Leaf(LeaderAction::FindSaved))
        .expect("the leaf is listed");
    assert_eq!(saved.blocked(), None);
    let save = from_editor
        .iter()
        .find(|e| *e.run() == PaletteRun::Chord(ActionShortcuts::SaveCurrentQuery))
        .expect("the save row is listed");
    assert_eq!(save.blocked(), Some(NEEDS_FIND_FOCUS));

    find_on(&started, &rt, "#go", cx);
    let opener = cx.read(|cx| started.workspace.read(cx).focused_panel());
    let save = entries(opener, cx)
        .into_iter()
        .find(|e| *e.run() == PaletteRun::Chord(ActionShortcuts::SaveCurrentQuery))
        .unwrap();
    assert_eq!(save.blocked(), None, "FIND has the keyboard");
    cx.update_window(started.handle, |_, window, cx| {
        started.workspace.update(cx, |w, cx| {
            w.run_palette_row(
                PaletteRun::Chord(ActionShortcuts::SaveCurrentQuery),
                window,
                cx,
            )
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(started.shown_modal(cx), Some(ModalKind::SaveSearch));
}
