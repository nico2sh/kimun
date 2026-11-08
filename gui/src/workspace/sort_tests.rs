//! UI integration tests for the sort dialog (M7b G7) on a real `Workspace`
//! over a real, indexed temp vault: FILES re-sorts in place (a property
//! sort once its values land), FIND's and the search browser's queries
//! get the order directive, the doors (chord, chips, palette) and where the
//! chord is not live.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`): the property values
//! and the known keys land only when tokio runs, which is also the proof
//! that they are not read on the UI thread.
#![cfg(test)]

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Context, Entity, TestAppContext, Window};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::files::FileListEntry;
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::message::SortTarget;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::sort::{SortField, SortOrder, order_of_query};
use tokio::runtime::Runtime;

use super::LayoutStore;
use super::Workspace;
use super::files::{FILES_SORT_ID, FILTER_ID};
use super::find::{FIND_DEBOUNCE, FIND_INPUT_ID, FIND_SORT_ID, FindView};
use super::modal::ModalKind;
use super::palette::PaletteRun;
use super::picker::{BROWSER_SORT_ID, NotePicker, PICKER_INPUT_ID};
use super::preferences_tests::{config, on_disk, open as open_saving, settle};
use super::sort::{
    CLOSE_LABEL, FILES_HINT, FILES_PROPERTY_HINT, FILES_SAVE_HINT, QUERY_HINT, SAVE_JOURNAL_LABEL,
    SAVE_LABEL, SAVE_NOTES_LABEL, SORT_CLOSE_ID, SORT_DIALOG_ID, SORT_HINT_ID, SORT_SAVE_ID,
    SortDialog,
};
use super::start_tests::{Started, drive, open_started, runtime, vault_start};
use crate::startup::FirstNote;
use crate::test_vault::{TestVault, test_vault};
use kimun_notes::logic::leader_route::LeaderRoute;
use kimun_notes::settings::{AppSettings, SortFieldSetting, SortOrderSetting};

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

/// Notes with titles that sort differently from their names, a `priority`
/// on two of them, a `rank` on two others, and a folder.
const NOTES: &[(&str, &str)] = &[
    (
        "a.md",
        "---\npriority: 3\nrank: 1\n---\n# Zed\nthe a note #rust",
    ),
    ("b.md", "---\npriority: 1\n---\n# Alpha\nthe b note #rust"),
    ("c.md", "---\nrank: 2\n---\n# Mid\nthe c note #rust"),
    ("sub/d.md", "# Deep\nin a folder"),
];

/// Started on a.md (the editor has the keyboard), indexed, FILES listing
/// the root.
fn started(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(NOTES, &["c.md", "a.md", "b.md"]));
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("a.md"))),
        v.settings.clone(),
    );
    let want = NOTES[0].1;
    drive(rt, cx, |cx| {
        started.text(cx) == want && !started.indexing(cx) && files_listed(&started, cx)
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

/// Run `f` on the workspace with the window.
fn on_workspace<R>(
    s: &Started,
    cx: &mut TestAppContext,
    f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) -> R,
) -> R {
    let workspace = s.workspace.clone();
    let out = cx
        .update_window(s.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| f(w, window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    out
}

fn dialog(s: &Started, cx: &TestAppContext) -> Entity<SortDialog> {
    cx.read(|cx| s.workspace.read(cx).sort_dialog())
        .expect("the sort dialog is open")
}

fn with_dialog<R>(s: &Started, cx: &TestAppContext, f: impl FnOnce(&SortDialog) -> R) -> R {
    let view = dialog(s, cx);
    cx.read(|cx| f(view.read(cx)))
}

fn sort_open(s: &Started, cx: &TestAppContext) -> bool {
    cx.read(|cx| s.workspace.read(cx).sort_dialog().is_some())
}

/// Run `f` on the open sort dialog with the window (a pick, as its select,
/// switch or key field makes it).
fn pick(
    s: &Started,
    cx: &mut TestAppContext,
    f: impl FnOnce(&mut SortDialog, &mut Window, &mut Context<SortDialog>),
) {
    let view = dialog(s, cx);
    cx.update_window(s.handle, |_, window, cx| {
        view.update(cx, |d, cx| f(d, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn files_listed(s: &Started, cx: &TestAppContext) -> bool {
    cx.read(|cx| {
        s.workspace.read(cx).files.as_ref().is_some_and(|files| {
            files
                .read(cx)
                .model()
                .core()
                .is_some_and(|core| !core.is_loading() && core.visible_len() > 0)
        })
    })
}

/// FILES' rows, top to bottom: a note's file name, a folder as `name/`.
fn files_rows(s: &Started, cx: &TestAppContext) -> Vec<String> {
    cx.read(|cx| {
        let files = s.workspace.read(cx).files.clone().expect("FILES");
        files
            .read(cx)
            .model()
            .core()
            .map(|core| {
                core.visible_rows()
                    .into_iter()
                    .map(|row| match row {
                        FileListEntry::Directory { name, .. } => format!("{name}/"),
                        FileListEntry::Note { filename, .. } => filename.clone(),
                        other => other.path().to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// FILES with the keyboard (`OpenFileBrowser`).
fn focus_files(s: &Started, cx: &mut TestAppContext) {
    press(s, "ctrl-e", cx);
    assert!(focused(s, FILTER_ID, cx), "FILES' filter has the keyboard");
}

fn find(s: &Started, cx: &TestAppContext) -> Entity<FindView> {
    cx.read(|cx| s.workspace.read(cx).find.clone().expect("FIND is built"))
}

fn find_query(s: &Started, cx: &TestAppContext) -> String {
    let view = find(s, cx);
    cx.read(|cx| view.read(cx).query(cx))
}

/// FIND's result names, in order, once its rows landed.
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

/// Show FIND (the rail) and run `query` in it.
fn find_on(s: &Started, rt: &Runtime, query: &str, cx: &mut TestAppContext) {
    let id = super::rail_cell_id(DrawerView::Find);
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
    press(s, "ctrl-a", cx);
    press(s, "backspace", cx);
    type_in(s, query, cx);
    cx.executor().advance_clock(FIND_DEBOUNCE);
    cx.run_until_parked();
    find_names(s, rt, cx);
}

fn browser(s: &Started, cx: &TestAppContext) -> Entity<NotePicker> {
    cx.read(|cx| {
        s.workspace
            .read(cx)
            .picker
            .as_ref()
            .and_then(|p| p.upgrade())
            .expect("the search browser is open")
    })
}

/// The browser's query and its rows' names once they landed.
fn browser_rows(s: &Started, rt: &Runtime, cx: &mut TestAppContext) -> (String, Vec<String>) {
    let view = browser(s, cx);
    drive(rt, cx, |cx| !cx.read(|cx| view.read(cx).is_loading()));
    cx.read(|cx| {
        let picker = view.read(cx);
        let rows = picker
            .rows()
            .iter()
            .filter(|r| matches!(r, FileListEntry::Note { .. }))
            .map(|r| r.path().get_clean_name())
            .collect();
        (picker.query().to_string(), rows)
    })
}

// ── FILES ────────────────────────────────────────────────────────────────

/// Ctrl+R in FILES opens the dialog on FILES' sort; picks re-sort the rows
/// at once (Title descending, then folders first), the hint says it is not
/// saved; Escape closes it keeping the order, and FILES has the keyboard
/// back.
#[gpui_kit::test]
fn files_re_sorts_live_and_escape_keeps_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    assert_eq!(files_rows(&started, cx), ["a.md", "b.md", "c.md", "sub/"]);
    focus_files(&started, cx);
    press(&started, "ctrl-r", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Sort));
    assert!(started.shows(SORT_DIALOG_ID, cx));
    let view = dialog(&started, cx);
    cx.read(|cx| {
        let d = view.read(cx);
        assert_eq!(d.target(), SortTarget::Sidebar);
        assert_eq!(d.state().field, SortField::Name);
        assert_eq!(d.state().group_dirs, Some(false));
    });
    let hint = cx
        .update_window(started.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(SORT_HINT_ID).label().map(str::to_string)
        })
        .unwrap();
    assert_eq!(hint.as_deref(), Some(FILES_HINT));

    pick(&started, cx, |d, window, cx| {
        d.pick_order(SortOrder::Descending, window, cx)
    });
    assert_eq!(files_rows(&started, cx), ["sub/", "c.md", "b.md", "a.md"]);
    pick(&started, cx, |d, window, cx| {
        d.pick_field("Title", window, cx)
    });
    let notes: Vec<String> = files_rows(&started, cx)
        .into_iter()
        .filter(|r| r.ends_with(".md"))
        .collect();
    assert_eq!(notes, ["a.md", "c.md", "b.md"], "Zed, Mid, Alpha");
    pick(&started, cx, |d, window, cx| {
        d.set_group_dirs(true, window, cx)
    });
    assert_eq!(files_rows(&started, cx), ["sub/", "a.md", "c.md", "b.md"]);

    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(!sort_open(&started, cx));
    assert_eq!(
        files_rows(&started, cx),
        ["sub/", "a.md", "c.md", "b.md"],
        "Escape keeps what was applied"
    );
    assert!(
        focused(&started, FILTER_ID, cx),
        "the opener has the keyboard"
    );
    // The chip says the order, and opens the dialog again.
    click(&started, FILES_SORT_ID, cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Sort));
    let state = with_dialog(&started, cx, |d| d.state().clone());
    assert_eq!(state.field, SortField::Title);
    assert_eq!(state.order, SortOrder::Descending);
    // The chord again closes it.
    press(&started, "ctrl-r", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
}

/// A property sort reads its values from the index off the UI thread:
/// nothing moves until tokio runs, then the notes with the key lead in its
/// order and the rest (a note without it, the folder) follow by name.
#[gpui_kit::test]
fn files_property_sort_fetches_values_missing_last(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    focus_files(&started, cx);
    press(&started, "ctrl-r", cx);
    // The known keys land on tokio; typing a prefix offers them.
    pick(&started, cx, |d, window, cx| {
        d.pick_field("Property", window, cx)
    });
    assert_eq!(
        with_dialog(&started, cx, |d| d.state().field.clone()),
        SortField::Property(String::new()),
        "no key yet: nothing applied"
    );
    assert_eq!(files_rows(&started, cx), ["a.md", "b.md", "c.md", "sub/"]);
    type_in(&started, "pri", cx);
    let view = dialog(&started, cx);
    drive(&rt, cx, |cx| {
        cx.read(|cx| view.read(cx).suggestions()) == ["priority"]
    });
    press(&started, "down", cx);
    press(&started, "enter", cx);
    assert_eq!(
        cx.read(|cx| view.read(cx).state().field.clone()),
        SortField::Property("priority".into())
    );
    drive(&rt, cx, |cx| files_rows(&started, cx)[0] == "b.md");
    assert_eq!(files_rows(&started, cx), ["b.md", "a.md", "c.md", "sub/"]);
    // Descending: the values reverse, the missing stay last by name.
    pick(&started, cx, |d, window, cx| {
        d.pick_order(SortOrder::Descending, window, cx)
    });
    drive(&rt, cx, |cx| files_rows(&started, cx)[0] == "a.md");
    assert_eq!(files_rows(&started, cx), ["a.md", "b.md", "c.md", "sub/"]);
}

/// Two keys in a row: the first key's values never land; the newest key's
/// order is the one FILES shows.
#[gpui_kit::test]
fn a_stale_property_fetch_is_dropped(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    focus_files(&started, cx);
    press(&started, "ctrl-r", cx);
    pick(&started, cx, |d, window, cx| {
        d.pick_key("priority".into(), window, cx);
        d.pick_key("rank".into(), window, cx);
    });
    drive(&rt, cx, |cx| files_rows(&started, cx)[1] == "c.md");
    // rank: a (1), c (2), then b and the folder without it.
    assert_eq!(files_rows(&started, cx), ["a.md", "c.md", "b.md", "sub/"]);
    // Let anything else in flight land: the order stays rank's.
    for _ in 0..20 {
        rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(5)));
        cx.run_until_parked();
    }
    assert_eq!(files_rows(&started, cx), ["a.md", "c.md", "b.md", "sub/"]);
}

// ── FIND ─────────────────────────────────────────────────────────────────

/// FIND's chip opens the dialog on the query's order; a pick writes the
/// order directive into the query and runs it again; a saved search's name
/// then reads "• edited".
#[gpui_kit::test]
fn find_gets_the_order_directive_and_reads_edited(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    find_on(&started, &rt, "#rust", cx);
    assert_eq!(find_names(&started, &rt, cx), ["a", "b", "c"]);
    on_workspace(&started, cx, |w, window, cx| {
        w.open_find_named("#rust".into(), Some("rusty".into()), window, cx)
    });
    find_names(&started, &rt, cx);
    let title = |cx: &TestAppContext| {
        let view = find(&started, cx);
        cx.read(|cx| view.read(cx).shown_title(cx))
    };
    assert_eq!(title(cx), "rusty");

    click(&started, FIND_SORT_ID, cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Sort));
    let view = dialog(&started, cx);
    assert_eq!(cx.read(|cx| view.read(cx).target()), SortTarget::Query);
    let hint = cx
        .update_window(started.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(SORT_HINT_ID).label().map(str::to_string)
        })
        .unwrap();
    assert_eq!(hint.as_deref(), Some(QUERY_HINT));
    pick(&started, cx, |d, window, cx| {
        d.pick_field("Title", window, cx)
    });
    assert_eq!(
        order_of_query(&find_query(&started, cx)),
        (SortField::Title, SortOrder::Ascending)
    );
    assert_eq!(
        find_names(&started, &rt, cx),
        ["b", "c", "a"],
        "Alpha, Mid, Zed"
    );
    pick(&started, cx, |d, window, cx| {
        d.pick_order(SortOrder::Descending, window, cx)
    });
    assert_eq!(
        order_of_query(&find_query(&started, cx)),
        (SortField::Title, SortOrder::Descending)
    );
    assert_eq!(find_names(&started, &rt, cx), ["a", "c", "b"]);
    assert!(
        find_query(&started, cx).contains("#rust"),
        "the rest of the query stays"
    );
    let shown = title(cx);
    assert!(
        shown.starts_with("rusty") && shown.contains("edited"),
        "{shown}"
    );
    // Escape keeps the sort; FIND has the keyboard back.
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert_eq!(find_names(&started, &rt, cx), ["a", "c", "b"]);
    assert!(
        cx.read(|cx| started.workspace.read(cx).focused_panel()) != Panel::Editor,
        "the slot keeps the keyboard"
    );
}

/// A property sort in FIND goes into the query: notes with the key first
/// in its order, the one without it last.
#[gpui_kit::test]
fn find_sorts_by_a_property(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    find_on(&started, &rt, "#rust", cx);
    press(&started, "ctrl-r", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Sort));
    pick(&started, cx, |d, window, cx| {
        d.pick_key("priority".into(), window, cx)
    });
    assert_eq!(
        order_of_query(&find_query(&started, cx)),
        (SortField::Property("priority".into()), SortOrder::Ascending)
    );
    assert_eq!(find_names(&started, &rt, cx), ["b", "a", "c"]);
}

// ── The search browser ───────────────────────────────────────────────────

/// Ctrl+R in the browser stacks the dialog over it (the slot stays the
/// browser's); a pick rewrites the browser's query; Escape closes only the
/// dialog and the browser's query field has the keyboard. On the empty
/// query (recent notes, unsorted) an Order pick sorts the recents by name.
#[gpui_kit::test]
fn the_browser_sort_stacks_and_rewrites_its_query(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    press(&started, "ctrl-k", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Search));
    let (_, mut recents) = browser_rows(&started, &rt, cx);
    recents.sort();
    assert_eq!(recents, ["a", "b", "c"], "the recent notes");

    press(&started, "ctrl-r", cx);
    assert!(sort_open(&started, cx));
    assert_eq!(
        started.shown_modal(cx),
        Some(ModalKind::Search),
        "stacked: the slot stays the browser's"
    );
    with_dialog(&started, cx, |d| {
        assert_eq!(d.target(), SortTarget::Browser);
        assert!(d.is_unsorted(), "the recents are in their own order");
    });
    pick(&started, cx, |d, window, cx| {
        d.pick_order(SortOrder::Descending, window, cx)
    });
    let (query, rows) = browser_rows(&started, &rt, cx);
    assert_eq!(
        order_of_query(&query),
        (SortField::Name, SortOrder::Descending)
    );
    assert_eq!(rows, ["c", "b", "a"]);

    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert!(!sort_open(&started, cx));
    assert_eq!(
        started.shown_modal(cx),
        Some(ModalKind::Search),
        "the browser stays"
    );
    assert!(focused(&started, PICKER_INPUT_ID, cx));

    // A query: its chip opens the dialog; Title rewrites it.
    press(&started, "ctrl-a", cx);
    press(&started, "backspace", cx);
    type_in(&started, "#rust", cx);
    browser_rows(&started, &rt, cx);
    click(&started, BROWSER_SORT_ID, cx);
    assert!(sort_open(&started, cx));
    assert!(!with_dialog(&started, cx, |d| d.is_unsorted()));
    pick(&started, cx, |d, window, cx| {
        d.pick_field("Title", window, cx)
    });
    let (query, rows) = browser_rows(&started, &rt, cx);
    assert!(query.contains("#rust"), "{query}");
    assert_eq!(
        order_of_query(&query),
        (SortField::Title, SortOrder::Ascending)
    );
    assert_eq!(rows, ["b", "c", "a"]);
    // Escape twice: the dialog, then the browser.
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Search));
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
}

// ── Doors ────────────────────────────────────────────────────────────────

/// Ctrl+R in the editor is not the sort (vim's redo, the plain editor's
/// own); from the rail it sorts the rail's cell (FILES).
#[gpui_kit::test]
fn the_chord_is_inert_in_the_editor_and_works_from_the_rail(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    assert_eq!(
        cx.read(|cx| started.workspace.read(cx).focused_panel()),
        Panel::Editor
    );
    press(&started, "ctrl-r", cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(!sort_open(&started, cx));

    on_workspace(&started, cx, |w, window, cx| {
        w.place_rail_cursor(Some(DrawerView::Files));
        w.set_focus_panel(Panel::Rail);
        w.apply_focus(window, cx);
    });
    press(&started, "ctrl-r", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Sort));
    assert_eq!(
        with_dialog(&started, cx, |d| d.target()),
        SortTarget::Sidebar
    );
}

/// The palette's `+view · sort…` runs from anywhere with a workspace: from
/// the editor it sorts FILES, from FIND it sorts FIND.
#[gpui_kit::test]
fn the_palette_row_opens_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let row = |opener: Panel, cx: &mut TestAppContext| {
        cx.read(|cx| started.workspace.read(cx).palette_entries(opener, cx))
            .into_iter()
            .find(|e| *e.run() == PaletteRun::Chord(ActionShortcuts::OpenSortDialog))
            .expect("the sort row is listed")
    };
    assert_eq!(row(Panel::Editor, cx).blocked(), None);
    let run = |cx: &mut TestAppContext| {
        on_workspace(&started, cx, |w, window, cx| {
            w.run_palette_row(
                PaletteRun::Chord(ActionShortcuts::OpenSortDialog),
                window,
                cx,
            )
        })
    };
    run(cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Sort));
    assert_eq!(
        with_dialog(&started, cx, |d| d.target()),
        SortTarget::Sidebar
    );
    press(&started, "escape", cx);
    started.settle_dialog(cx);

    find_on(&started, &rt, "#rust", cx);
    assert!(focused(&started, FIND_INPUT_ID, cx));
    run(cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Sort));
    assert_eq!(with_dialog(&started, cx, |d| d.target()), SortTarget::Query);
}

// ── Save as default (M9d) ───────────────────────────────────────────────

/// Started on a.md with the window saving to the vault's own config, FILES
/// listing the root, and two notes in the journal folder.
fn started_saving(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(NOTES, &["c.md", "a.md", "b.md"]));
    let journal = v.vault.journal_path().clone();
    for (name, text) in [
        ("2026-01-02.md", "# Alpha day"),
        ("2026-01-01.md", "# Zed day"),
    ] {
        rt.block_on(v.vault.create_note(&journal.append(&path(name)), text))
            .expect("the journal note is created");
    }
    let started = open_saving(
        cx,
        vault_start(&v, FirstNote::Note(path("a.md"))),
        v.settings.clone(),
        LayoutStore::File(config(&v)),
    );
    let want = NOTES[0].1;
    drive(rt, cx, |cx| {
        started.text(cx) == want && !started.indexing(cx) && files_listed(&started, cx)
    });
    (v, started)
}

fn settings_now(v: &TestVault) -> AppSettings {
    v.settings.read().unwrap().clone()
}

fn files_entity(s: &Started, cx: &TestAppContext) -> gpui_kit::EntityId {
    cx.read(|cx| {
        s.workspace
            .read(cx)
            .files
            .as_ref()
            .expect("FILES")
            .entity_id()
    })
}

fn files_sort(s: &Started, cx: &TestAppContext) -> (SortField, SortOrder) {
    cx.read(|cx| {
        let files = s.workspace.read(cx).files.clone().expect("FILES");
        files.read(cx).model().current_sort()
    })
}

/// FILES opens `dir`, listed.
fn files_open(s: &Started, rt: &Runtime, dir: VaultPath, cx: &mut TestAppContext) {
    let files = cx.read(|cx| s.workspace.read(cx).files.clone().expect("FILES"));
    cx.update(|cx| files.update(cx, |pane, cx| pane.navigate(dir.clone(), cx)));
    drive(rt, cx, |cx| {
        files_listed(s, cx) && cx.read(|cx| files.read(cx).current_dir() == &dir)
    });
}

/// The element `id`'s accessible label in the last frame.
fn label_of(s: &Started, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .and_then(|e| e.label().map(str::to_string))
    })
    .unwrap()
}

/// The checkbox's ticked and disabled states in the last frame.
fn save_box(s: &Started, cx: &mut TestAppContext) -> (Option<bool>, bool) {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        let e = window.find(SORT_SAVE_ID);
        (e.checked(), e.disabled() == Some(true))
    })
    .unwrap()
}

/// Open Preferences (the route) and read its draft's four sort choices.
fn preferences_sorting(
    s: &Started,
    rt: &Runtime,
    cx: &mut TestAppContext,
) -> (
    SortFieldSetting,
    SortOrderSetting,
    SortFieldSetting,
    SortOrderSetting,
) {
    on_workspace(s, cx, |w, window, cx| {
        w.perform_route(LeaderRoute::OpenPreferences, window, cx)
    });
    let loaded = |cx: &TestAppContext| {
        cx.read(|cx| {
            s.workspace.read(cx).preferences.as_ref().and_then(|p| {
                p.read(cx).model().map(|m| {
                    (
                        m.default_sort_field,
                        m.default_sort_order,
                        m.journal_sort_field,
                        m.journal_sort_order,
                    )
                })
            })
        })
    };
    drive(rt, cx, |cx| loaded(cx).is_some());
    loaded(cx).unwrap()
}

/// Ticked with `s` and confirmed with `Save` on a notes folder: the sort
/// stays, the notes' default is written (disk and memory), the journal's
/// untouched, no reopen; later folder opens use it, Preferences show it.
#[gpui_kit::test]
fn save_as_default_for_notes_writes_the_notes_default(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_saving(cx, &rt);
    let before = settings_now(&v);
    let files = files_entity(&started, cx);
    focus_files(&started, cx);
    press(&started, "ctrl-r", cx);
    assert_eq!(
        with_dialog(&started, cx, |d| d.save_default_label()),
        Some(SAVE_NOTES_LABEL)
    );
    assert_eq!(
        label_of(&started, SORT_SAVE_ID, cx).as_deref(),
        Some(SAVE_NOTES_LABEL)
    );
    assert_eq!(save_box(&started, cx), (Some(false), false));
    assert_eq!(
        label_of(&started, SORT_CLOSE_ID, cx).as_deref(),
        Some(CLOSE_LABEL)
    );
    pick(&started, cx, |d, window, cx| {
        d.pick_field("Title", window, cx)
    });
    pick(&started, cx, |d, window, cx| {
        d.pick_order(SortOrder::Descending, window, cx)
    });
    // `s` ticks it (the field select has the keyboard), again clears it.
    press(&started, "s", cx);
    assert!(with_dialog(&started, cx, |d| d.saves_default()));
    assert_eq!(save_box(&started, cx).0, Some(true));
    press(&started, "s", cx);
    assert!(!with_dialog(&started, cx, |d| d.saves_default()));
    press(&started, "s", cx);
    assert_eq!(
        label_of(&started, SORT_CLOSE_ID, cx).as_deref(),
        Some(SAVE_LABEL)
    );
    assert_eq!(
        label_of(&started, SORT_HINT_ID, cx).as_deref(),
        Some(FILES_SAVE_HINT)
    );

    click(&started, SORT_CLOSE_ID, cx);
    started.settle_dialog(cx);
    assert!(!sort_open(&started, cx), "the confirm closes it");
    // Memory takes it in the write; the flash once the UI thread hears.
    drive(&rt, cx, |cx| {
        settings_now(&v).default_sort_order == SortOrderSetting::Descending
            && started.flash(cx).is_some()
    });
    let memory = settings_now(&v);
    let disk = on_disk(&v);
    for s in [&memory, &disk] {
        assert_eq!(s.default_sort_field, SortFieldSetting::Title);
        assert_eq!(s.default_sort_order, SortOrderSetting::Descending);
        assert_eq!(s.journal_sort_field, before.journal_sort_field);
        assert_eq!(s.journal_sort_order, before.journal_sort_order);
        assert_eq!(s.group_directories, before.group_directories);
    }
    assert_eq!(
        started.flash(cx).as_deref(),
        Some("Sort saved as the default for notes")
    );
    assert_eq!(files_entity(&started, cx), files, "no reopen");
    assert_eq!(
        files_sort(&started, cx),
        (SortField::Title, SortOrder::Descending)
    );
    // Later opens of a notes folder use it; the journal keeps its own.
    files_open(&started, &rt, path("sub"), cx);
    assert_eq!(
        files_sort(&started, cx),
        (SortField::Title, SortOrder::Descending)
    );
    files_open(&started, &rt, v.vault.journal_path().clone(), cx);
    assert_eq!(
        files_sort(&started, cx),
        (
            SortField::from(before.journal_sort_field),
            SortOrder::from(before.journal_sort_order)
        )
    );
    // Preferences > Sorting reads the saved values.
    assert_eq!(
        preferences_sorting(&started, &rt, cx),
        (
            SortFieldSetting::Title,
            SortOrderSetting::Descending,
            before.journal_sort_field,
            before.journal_sort_order,
        )
    );
    settle(&rt, cx);
}

/// On the journal folder the box says so and saves the journal's default
/// (ticked with a click), plus the grouping the dialog set; the notes'
/// default is untouched.
#[gpui_kit::test]
fn save_as_default_in_the_journal_writes_the_journal_default(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_saving(cx, &rt);
    let before = settings_now(&v);
    let journal = v.vault.journal_path().clone();
    // FILES with the keyboard first: its chord reveals the note's folder.
    focus_files(&started, cx);
    files_open(&started, &rt, journal.clone(), cx);
    press(&started, "ctrl-r", cx);
    assert_eq!(
        with_dialog(&started, cx, |d| d.save_default_label()),
        Some(SAVE_JOURNAL_LABEL)
    );
    assert_eq!(
        label_of(&started, SORT_SAVE_ID, cx).as_deref(),
        Some(SAVE_JOURNAL_LABEL)
    );
    pick(&started, cx, |d, window, cx| {
        d.pick_field("Title", window, cx)
    });
    pick(&started, cx, |d, window, cx| {
        d.set_group_dirs(!before.group_directories, window, cx)
    });
    click(&started, SORT_SAVE_ID, cx);
    assert_eq!(save_box(&started, cx).0, Some(true), "a click ticks it");
    click(&started, SORT_CLOSE_ID, cx);
    started.settle_dialog(cx);
    drive(&rt, cx, |cx| {
        settings_now(&v).journal_sort_field == SortFieldSetting::Title
            && started.flash(cx).is_some()
    });
    for s in [settings_now(&v), on_disk(&v)] {
        assert_eq!(s.journal_sort_field, SortFieldSetting::Title);
        assert_eq!(s.journal_sort_order, before.journal_sort_order);
        assert_eq!(s.default_sort_field, before.default_sort_field);
        assert_eq!(s.default_sort_order, before.default_sort_order);
        assert_eq!(s.group_directories, !before.group_directories);
    }
    assert_eq!(
        started.flash(cx).as_deref(),
        Some("Sort saved as the default for the journal")
    );
    // The notes keep theirs; back in the journal, its new default.
    files_open(&started, &rt, VaultPath::root(), cx);
    assert_eq!(
        files_sort(&started, cx),
        (SortField::Name, SortOrder::Ascending)
    );
    files_open(&started, &rt, journal, cx);
    assert_eq!(
        files_sort(&started, cx),
        (SortField::Title, SortOrder::from(before.journal_sort_order))
    );
    settle(&rt, cx);
}

/// Unticked, `Close` changes nothing on disk or in memory; ticked, Escape
/// still saves nothing; `s` in the Key field is typed, not a toggle; a
/// property sort disables the box. The query lists have no box.
#[gpui_kit::test]
fn nothing_is_saved_unticked_on_escape_or_for_a_property_sort(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_saving(cx, &rt);
    let before = settings_now(&v);
    // The window writes its layout (`[gui]`) on its own: the sort fields
    // are what must not move on disk.
    let sorting = |s: &AppSettings| {
        (
            s.default_sort_field,
            s.default_sort_order,
            s.journal_sort_field,
            s.journal_sort_order,
            s.group_directories,
        )
    };
    let disk = sorting(&on_disk(&v));
    let unchanged = |cx: &mut TestAppContext| {
        settle(&rt, cx);
        assert_eq!(settings_now(&v), before, "memory unchanged");
        assert_eq!(sorting(&on_disk(&v)), disk, "disk unchanged");
    };
    focus_files(&started, cx);
    press(&started, "ctrl-r", cx);
    pick(&started, cx, |d, window, cx| {
        d.pick_order(SortOrder::Descending, window, cx)
    });
    click(&started, SORT_CLOSE_ID, cx);
    started.settle_dialog(cx);
    assert!(!sort_open(&started, cx));
    assert_eq!(
        files_sort(&started, cx),
        (SortField::Name, SortOrder::Descending)
    );
    unchanged(cx);

    // Ticked, then Escape: the sort stays, nothing is saved.
    press(&started, "ctrl-r", cx);
    press(&started, "s", cx);
    assert!(with_dialog(&started, cx, |d| d.saves_default()));
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    assert!(!sort_open(&started, cx));
    unchanged(cx);

    // A property sort: the Key field takes `s` as text, the box is
    // disabled and the hint says why; a confirm saves nothing.
    press(&started, "ctrl-r", cx);
    press(&started, "s", cx);
    pick(&started, cx, |d, window, cx| {
        d.pick_field("Property", window, cx)
    });
    type_in(&started, "s", cx);
    press(&started, "s", cx);
    let view = dialog(&started, cx);
    let typed = cx.read(|cx| view.read(cx).state().clone());
    assert!(matches!(typed.field, SortField::Property(_)));
    assert!(!with_dialog(&started, cx, |d| d.saves_default()));
    assert_eq!(save_box(&started, cx), (Some(false), true));
    assert_eq!(
        label_of(&started, SORT_HINT_ID, cx).as_deref(),
        Some(FILES_PROPERTY_HINT)
    );
    assert_eq!(
        label_of(&started, SORT_CLOSE_ID, cx).as_deref(),
        Some(CLOSE_LABEL)
    );
    // The tick went with the property sort: back on Name it stays clear.
    pick(&started, cx, |d, window, cx| {
        d.pick_field("Name", window, cx)
    });
    assert!(!with_dialog(&started, cx, |d| d.saves_default()));
    assert_eq!(save_box(&started, cx), (Some(false), false));
    pick(&started, cx, |d, window, cx| {
        d.pick_key("priority".into(), window, cx);
        d.set_save_default(true, cx);
        d.confirm(window, cx);
    });
    started.settle_dialog(cx);
    assert!(!sort_open(&started, cx));
    unchanged(cx);

    // FIND: no box, and nothing it could save.
    find_on(&started, &rt, "#rust", cx);
    press(&started, "ctrl-r", cx);
    assert_eq!(with_dialog(&started, cx, |d| d.target()), SortTarget::Query);
    assert_eq!(with_dialog(&started, cx, |d| d.save_default_label()), None);
    assert!(!started.shows(SORT_SAVE_ID, cx));
    pick(&started, cx, |d, window, cx| {
        d.pick_field("Title", window, cx);
        d.set_save_default(true, cx);
        d.confirm(window, cx);
    });
    started.settle_dialog(cx);
    unchanged(cx);
}

/// The search browser's dialog has no box either.
#[gpui_kit::test]
fn the_browser_sort_has_no_save_as_default(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    press(&started, "ctrl-k", cx);
    browser_rows(&started, &rt, cx);
    press(&started, "ctrl-r", cx);
    assert_eq!(
        with_dialog(&started, cx, |d| d.target()),
        SortTarget::Browser
    );
    assert_eq!(with_dialog(&started, cx, |d| d.save_default_label()), None);
    assert!(!started.shows(SORT_SAVE_ID, cx));
    press(&started, "s", cx);
    assert!(!with_dialog(&started, cx, |d| d.saves_default()));
}

/// A failed write (an unparsable config): the sort stays on the folder
/// shown, a flash says it couldn't be saved, memory is untouched and the
/// default is not remembered (the next open is on the old one).
#[gpui_kit::test]
fn a_failed_write_keeps_the_sort_and_remembers_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_saving(cx, &rt);
    let before = settings_now(&v);
    std::fs::write(config(&v), "this is [not toml").unwrap();
    focus_files(&started, cx);
    press(&started, "ctrl-r", cx);
    pick(&started, cx, |d, window, cx| {
        d.pick_order(SortOrder::Descending, window, cx)
    });
    press(&started, "s", cx);
    click(&started, SORT_CLOSE_ID, cx);
    started.settle_dialog(cx);
    drive(&rt, cx, |cx| started.flash(cx).is_some());
    let flash = started.flash(cx).unwrap();
    assert!(
        flash.starts_with("Couldn't save the sort default"),
        "{flash}"
    );
    assert!(
        cx.read(|cx| started.workspace.read(cx).last_flash().unwrap().1),
        "an error"
    );
    assert_eq!(settings_now(&v), before, "memory untouched");
    assert_eq!(
        std::fs::read_to_string(config(&v)).unwrap(),
        "this is [not toml",
        "the unreadable config is left alone"
    );
    assert_eq!(
        files_sort(&started, cx),
        (SortField::Name, SortOrder::Descending)
    );
    files_open(&started, &rt, path("sub"), cx);
    assert_eq!(
        files_sort(&started, cx),
        (SortField::Name, SortOrder::Ascending),
        "not remembered"
    );
    settle(&rt, cx);
}
