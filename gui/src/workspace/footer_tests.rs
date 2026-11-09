//! UI integration tests for the Footer's document segments (M5b-2 G5): the
//! caret's line and column, the find bar's matches, and the open note's
//! backlink and property counts, read off a real `Workspace` on a real temp
//! vault through the rendered frame (role, label, visibility).
#![cfg(test)]

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::layout::Panel;
use tokio::runtime::Runtime;

use super::start_tests::{Started, drive, open_started, runtime, vault_start};
use crate::startup::FirstNote;
use crate::status_row::{BACKLINKS_ID, CARET_ID, MATCHES_ID, PROPERTIES_ID};
use crate::test_vault::{TestVault, test_vault};

impl Started {
    /// The spoken label of the Footer segment `id` in a fresh frame, `None`
    /// when it is not shown.
    fn segment(&self, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(id)
                .filter(|e| e.visible())
                .and_then(|e| e.label().map(str::to_string))
        })
        .unwrap()
    }

    fn key(&self, key: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.press(key, cx))
            .unwrap();
    }

    fn input(&self, text: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.input(text, cx))
            .unwrap();
    }

    fn focused_panel(&self, cx: &TestAppContext) -> Panel {
        cx.read(|cx| self.workspace.read(cx).focused_panel())
    }
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
        vault_start(&v, FirstNote::Note(VaultPath::new(first))),
        v.settings.clone(),
    );
    let want = notes.iter().find(|(p, _)| *p == first).unwrap().1;
    drive(rt, cx, |cx| {
        started.text(cx) == want && !started.indexing(cx)
    });
    (v, started)
}

// ── the caret ────────────────────────────────────────────────────────────

/// The caret's line and column follow typing while the editor has the
/// keyboard, and go when another panel takes it.
#[gpui_kit::test]
fn the_caret_shows_only_while_the_editor_has_the_keyboard(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "alpha\nbeta")], "a.md");
    assert_eq!(started.focused_panel(cx), Panel::Editor);
    assert_eq!(
        started.segment(CARET_ID, cx).as_deref(),
        Some("Line 1, column 1")
    );
    started.key("down", cx);
    started.input("XY", cx);
    assert_eq!(
        started.segment(CARET_ID, cx).as_deref(),
        Some("Line 2, column 3")
    );

    started.key("alt-left", cx);
    assert_ne!(started.focused_panel(cx), Panel::Editor);
    assert_eq!(started.segment(CARET_ID, cx), None);
    for _ in 0..4 {
        if started.focused_panel(cx) == Panel::Editor {
            break;
        }
        started.key("ctrl-tab", cx);
    }
    assert_eq!(started.focused_panel(cx), Panel::Editor);
    assert_eq!(
        started.segment(CARET_ID, cx).as_deref(),
        Some("Line 2, column 3")
    );
}

// ── the find bar ─────────────────────────────────────────────────────────

/// While the find bar is open the Footer shows where it stands among its
/// matches instead of the caret; closing it brings the caret back.
#[gpui_kit::test]
fn the_find_bar_shows_its_place_among_the_matches(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "foo bar\nfoo")], "a.md");
    assert_eq!(started.segment(MATCHES_ID, cx), None);

    started.key("ctrl-f", cx);
    assert_eq!(started.segment(CARET_ID, cx), None, "the bar has the caret");
    assert_eq!(started.segment(MATCHES_ID, cx), None, "nothing typed yet");
    started.input("foo", cx);
    assert_eq!(
        started.segment(MATCHES_ID, cx).as_deref(),
        Some("Match 1 of 2")
    );
    started.key("enter", cx);
    assert_eq!(
        started.segment(MATCHES_ID, cx).as_deref(),
        Some("Match 2 of 2")
    );
    started.input("q", cx);
    assert_eq!(started.segment(MATCHES_ID, cx), None, "no match");

    started.key("escape", cx);
    assert_eq!(started.segment(MATCHES_ID, cx), None);
    assert!(started.segment(CARET_ID, cx).is_some());
}

// ── the open note's counts ───────────────────────────────────────────────

/// Opening a note loads its backlink and property counts; opening another
/// replaces them.
#[gpui_kit::test]
fn the_open_notes_counts_load_on_open(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[
            ("a.md", "---\ntitle: A\nstatus: draft\n---\nalpha"),
            ("b.md", "see [[a]]"),
            ("c.md", "and [a](a.md)"),
        ],
        "a.md",
    );
    drive(&rt, cx, |cx| {
        started.segment(BACKLINKS_ID, cx).is_some() && started.segment(PROPERTIES_ID, cx).is_some()
    });
    assert_eq!(
        started.segment(BACKLINKS_ID, cx).as_deref(),
        Some("2 backlinks")
    );
    assert_eq!(
        started.segment(PROPERTIES_ID, cx).as_deref(),
        Some("2 properties")
    );

    started.workspace.update(cx, |w, cx| {
        w.open_note(VaultPath::new("b.md"), cx);
    });
    drive(&rt, cx, |cx| {
        started.text(cx) == "see [[a]]"
            && started.segment(BACKLINKS_ID, cx).as_deref() == Some("0 backlinks")
    });
    assert_eq!(
        started.segment(PROPERTIES_ID, cx).as_deref(),
        Some("0 properties")
    );
}

/// Saving re-reads the open note's property count.
#[gpui_kit::test]
fn a_save_refreshes_the_property_count(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, &[("a.md", "---\nk: v\n---\nbody")], "a.md");
    drive(&rt, cx, |cx| {
        started.segment(PROPERTIES_ID, cx).as_deref() == Some("1 property")
    });
    started.key("down", cx);
    started.input("j: w", cx);
    started.key("enter", cx);
    started.key("ctrl-s", cx);
    drive(&rt, cx, |cx| {
        started.segment(PROPERTIES_ID, cx).as_deref() == Some("2 properties")
    });
    let disk = rt
        .block_on(v.vault.get_note_text(&VaultPath::new("a.md")))
        .unwrap();
    assert!(disk.contains("j: w\nk: v"), "{disk}");
}

/// No note open: no caret, matches or counts (the editor is not even shown).
#[gpui_kit::test]
fn no_note_open_shows_no_document_segments(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("a.md", "alpha")], &[]));
    let started = open_started(cx, vault_start(&v, FirstNote::Restore), v.settings.clone());
    drive(&rt, cx, |cx| !started.indexing(cx));
    for id in [CARET_ID, MATCHES_ID, BACKLINKS_ID, PROPERTIES_ID] {
        assert_eq!(started.segment(id, cx), None, "{id}");
    }
}

// ── narrow windows ───────────────────────────────────────────────────────

/// With the flash gone from the Footer (N1), a narrow window still cuts
/// the path short first: the save state keeps its room inside the window
/// and the path ends before it.
#[gpui_kit::test]
fn a_narrow_window_shrinks_the_path_not_the_save_state(cx: &mut TestAppContext) {
    use crate::status_row::{PATH_ID, ROW_ID, SAVE_ID};
    let rt = runtime();
    let _guard = rt.enter();
    const WIDTH: f32 = 640.;
    let note = "a-long-folder-name/another-long-folder/a-rather-long-note-name.md";
    let (_v, started) = started_on(cx, &rt, &[(note, "body")], note);
    cx.simulate_window_resize(
        started.handle,
        gpui_kit::size(gpui_kit::px(WIDTH), gpui_kit::px(480.)),
    );
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.viewport_size().width, gpui_kit::px(WIDTH));
        let row = window.find(ROW_ID).bounds();
        let path = window.find(PATH_ID).bounds();
        let save = window.find(SAVE_ID).bounds();
        assert!(save.right() <= row.right(), "{save:?} in {row:?}");
        assert!(path.right() <= save.left(), "{path:?} before {save:?}");
    })
    .unwrap();
}
