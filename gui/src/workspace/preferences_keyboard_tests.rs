//! UI integration tests for Preferences > **Keyboard** ([`keyboard`]): a
//! real `Workspace` over a real temp vault whose config file is the one
//! saved to, driven by keys and clicks (ADR-0064).
//!
//! The window runs the Linux and Windows table (`preferences_tests::open`),
//! so `primary` is Ctrl and a Ctrl+letter default has its Ctrl+Shift twin.
//!
//! [`keyboard`]: super::preferences::keyboard
#![cfg(test)]

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ScrollDelta, TestAppContext, point};
use kimun_notes::logic::actions::ActionShortcuts as A;
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::logic::layout::Dir;
use kimun_notes::logic::nav::DrawerView;
use tokio::runtime::Runtime;

use super::config::{CONFIG_ID, CONFIG_PREFERENCES_ID};
use super::preferences::keyboard::{
    EDITOR_NOTE, FILTER_EMPTY_ID, FILTER_ID, KEYBOARD_ID, NOTICE_ID, RESET_ALL_ID, VIM_NOTE,
    change_id, clear_id, group_id, reset_id, row_id, warning_id,
};
use super::preferences::{CANCEL_ID, CONTENT_ID, DISCARD_ID, PREFERENCES_ID, SAVE_ID, Section};
use super::preferences_tests::{on_disk, on_vault, settle, vault};
use super::rail_cell_id;
use super::start_tests::{Started, runtime};
use crate::keymap;
use crate::keymap::draft::{CaptureError, bindable_groups};
use crate::test_vault::TestVault;

/// The window on a fresh vault with Preferences open on Keyboard.
fn keyboard(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Started) {
    let v = vault(rt);
    let s = on_vault(cx, rt, &v);
    s.pf_route(cx);
    s.pf_loaded(rt, cx);
    s.pf_section(cx, Section::Keyboard);
    assert!(s.pf_shows(KEYBOARD_ID, cx));
    (v, s)
}

/// The accessible label of the element `id`, `None` when it is not drawn.
fn label(
    s: &Started,
    id: impl Into<gpui_kit::ElementId>,
    cx: &mut TestAppContext,
) -> Option<String> {
    let id = id.into();
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .and_then(|e| e.label().map(str::to_string))
    })
    .unwrap()
    .map(|text| spelled(&text))
}

/// Filter `text` as the rows read on this host: on a Mac `Ctrl+` is `⌃`,
/// the glyph the rows are searched by (see [`spelled`]).
fn typed(text: &str) -> String {
    if cfg!(target_os = "macos") {
        text.replace("Ctrl+", "⌃").replace("ctrl+", "⌃")
    } else {
        text.to_string()
    }
}

/// `text` with the macOS key glyphs written out (`⌃⇧E` as `Ctrl+Shift+E`).
/// The window runs the Linux and Windows table, but `Kbd::format` follows
/// the host, so on a Mac the rows read in glyphs. A no-op elsewhere.
fn spelled(text: &str) -> String {
    text.replace('⌃', "Ctrl+")
        .replace('⌥', "Alt+")
        .replace('⇧', "Shift+")
        .replace('⌘', "Cmd+")
}

/// Whether the element `id` is drawn disabled.
fn disabled(s: &Started, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> bool {
    let id = id.into();
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).disabled() == Some(true)
    })
    .unwrap()
}

/// Whether the element `id` has the keyboard.
fn focused(s: &Started, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> bool {
    let id = id.into();
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).focused() == Some(true)
    })
    .unwrap()
}

/// The row of `action` as it reads.
fn row(s: &Started, action: &A, cx: &mut TestAppContext) -> String {
    label(s, row_id(action), cx).unwrap_or_else(|| panic!("no row for {action:?}"))
}

/// The lines under the row of `action` (warnings, a refusal, notes).
fn lines(s: &Started, action: &A, cx: &mut TestAppContext) -> Option<String> {
    label(s, warning_id(action), cx)
}

/// Scroll the section with the wheel until the element `id` is wholly in
/// the pane, as a person scrolls to a row before clicking it.
pub(super) fn reveal(s: &Started, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
    let id = id.into();
    for _ in 0..300 {
        let dy = cx
            .update_window(s.handle, |_, window, cx| {
                window.render_frame(cx);
                let pane = window.find(CONTENT_ID).bounds();
                let target = window.find(id.clone()).bounds();
                if target.top() < pane.top() {
                    1.0
                } else if target.bottom() > pane.bottom() {
                    -1.0
                } else {
                    0.0
                }
            })
            .unwrap();
        if dy == 0.0 {
            return;
        }
        cx.update_window(s.handle, |_, window, cx| {
            window.scroll(CONTENT_ID, ScrollDelta::Lines(point(0.0, dy)), cx)
        })
        .unwrap();
        cx.run_until_parked();
    }
    panic!("{id:?} never scrolls into view");
}

/// Scroll to the element `id` and click it.
fn click(s: &Started, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
    let id = id.into();
    reveal(s, id.clone(), cx);
    s.pf_click(id, cx);
}

/// `Change` on the row of `action`, then `key`.
fn change(s: &Started, action: &A, key: &str, cx: &mut TestAppContext) {
    click(s, change_id(action), cx);
    assert_eq!(
        s.pf_read(cx, |v| v.keyboard_listening().cloned()),
        Some(action.clone()),
        "Change listens"
    );
    s.pf_press(key, cx);
}

fn overrides(s: &Started, cx: &TestAppContext) -> Vec<(String, Vec<String>)> {
    s.pf_model(cx, |m| m.gui_key_overrides.clone().into_iter().collect())
}

fn entry(name: &str, chords: &[&str]) -> (String, Vec<String>) {
    (
        name.to_string(),
        chords.iter().map(|c| c.to_string()).collect(),
    )
}

/// Every bindable action has a row under its group's heading, with its
/// chords from the keymap in effect (or "none") and its scope tag.
#[gpui_kit::test]
fn keyboard_section_lists_every_bindable_action_in_groups(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    for (ix, group) in bindable_groups().iter().enumerate() {
        assert_eq!(label(&s, group_id(ix), cx).as_deref(), Some(group.title));
        for r in &group.rows {
            let chords = cx.read(|cx| keymap::chord_labels(cx, &r.action));
            let chords = if chords.is_empty() {
                "none".to_string()
            } else {
                chords.join(" / ")
            };
            let mut want = format!("{}: {chords}", r.label);
            if let Some(tag) = r.scope_tag {
                want.push_str(&format!(" ({tag})"));
            }
            assert_eq!(row(&s, &r.action, cx), spelled(&want));
            reveal(&s, change_id(&r.action), cx);
            assert!(
                label(&s, reset_id(&r.action), cx).is_none(),
                "{:?}",
                r.action
            );
        }
    }
    assert_eq!(
        row(&s, &A::OpenFileBrowser, cx),
        "Open files: Ctrl+E / Ctrl+Shift+E"
    );
    assert_eq!(row(&s, &A::ToggleFacetView, cx), "Toggle facet view: none");
    assert_eq!(
        row(&s, &A::YankRow, cx),
        "Copy selected row: Ctrl+C / Ctrl+Shift+C (sidebar and rail only)"
    );
    settle(&rt, cx);
}

/// Change, then a free chord: the row has it, the draft is dirty, Reset
/// shows, nothing is written, and no warning shows.
#[gpui_kit::test]
fn change_then_a_key_assigns_it_and_marks_the_draft_dirty(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, s) = keyboard(cx, &rt);
    assert!(s.pf_disabled(SAVE_ID, cx));
    change(&s, &A::ToggleFacetView, "ctrl-g", cx);
    assert_eq!(s.pf_read(cx, |v| v.keyboard_listening().cloned()), None);
    assert_eq!(
        row(&s, &A::ToggleFacetView, cx),
        "Toggle facet view: Ctrl+G"
    );
    assert_eq!(
        overrides(&s, cx),
        [entry("ToggleFacetView", &["primary+g"])]
    );
    assert!(!s.pf_disabled(SAVE_ID, cx), "the draft is dirty");
    assert!(label(&s, reset_id(&A::ToggleFacetView), cx).is_some());
    assert_eq!(lines(&s, &A::ToggleFacetView, cx), None);
    assert!(
        on_disk(&v).gui.key_overrides().is_empty(),
        "nothing written"
    );
    assert!(s.pf_is_open(cx));
    settle(&rt, cx);
}

/// A chord another action holds moves: both rows say so, and the holder
/// reads "none".
#[gpui_kit::test]
fn a_chord_held_by_another_action_shows_the_warning_on_both_rows(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    change(&s, &A::OpenNote, "ctrl-e", cx);
    let warning = "Ctrl+E was Open files. Saving clears it there.";
    assert_eq!(lines(&s, &A::OpenNote, cx).as_deref(), Some(warning));
    assert_eq!(lines(&s, &A::OpenFileBrowser, cx).as_deref(), Some(warning));
    assert_eq!(row(&s, &A::OpenNote, cx), "Open note: Ctrl+E");
    assert_eq!(row(&s, &A::OpenFileBrowser, cx), "Open files: none");
    assert_eq!(
        overrides(&s, cx),
        [
            entry("OpenFileBrowser", &[]),
            entry("OpenNote", &["primary+e"]),
        ]
    );
    assert!(
        focused(&s, change_id(&A::OpenNote), cx),
        "the keyboard is back on the row's Change"
    );
    // A refused key, then Esc: the refusal goes, the warning stays.
    change(&s, &A::OpenNote, "g", cx);
    assert_eq!(
        lines(&s, &A::OpenNote, cx),
        Some(format!("{warning} {}", CaptureError::NeedsModifier))
    );
    s.pf_press("escape", cx);
    assert_eq!(lines(&s, &A::OpenNote, cx).as_deref(), Some(warning));
    assert!(focused(&s, change_id(&A::OpenNote), cx));
    // Picking the chord the row has already changes nothing and warns
    // about nothing.
    change(&s, &A::OpenNote, "ctrl-e", cx);
    assert_eq!(lines(&s, &A::OpenNote, cx), None);
    settle(&rt, cx);
}

/// Esc while listening ends the listening only; the next Esc closes the
/// clean view.
#[gpui_kit::test]
fn esc_while_listening_cancels_and_keeps_preferences_open(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    click(&s, change_id(&A::OpenNote), cx);
    assert!(s.pf_read(cx, |v| v.keyboard_listening().is_some()));
    s.pf_press("escape", cx);
    assert!(s.pf_is_open(cx), "Preferences stay");
    assert_eq!(s.pf_read(cx, |v| v.keyboard_listening().cloned()), None);
    assert_eq!(
        row(&s, &A::OpenNote, cx),
        "Open note: Ctrl+O / Ctrl+Shift+O"
    );
    assert!(overrides(&s, cx).is_empty());
    // Save ends the listening too, whatever the save does.
    click(&s, change_id(&A::OpenNote), cx);
    s.pf_with_view(cx, |v, window, cx| v.save(window, cx));
    assert_eq!(s.pf_read(cx, |v| v.keyboard_listening().cloned()), None);
    s.pf_press("escape", cx);
    assert!(!s.pf_is_open(cx), "a second Esc closes them");
    settle(&rt, cx);
}

/// A key that types text, or edits it, is refused with a message; the row
/// keeps listening, and Esc then takes the message away.
#[gpui_kit::test]
fn a_bare_letter_is_refused_with_a_message(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    change(&s, &A::OpenNote, "g", cx);
    assert_eq!(
        lines(&s, &A::OpenNote, cx),
        Some(CaptureError::NeedsModifier.to_string())
    );
    assert_eq!(
        CaptureError::NeedsModifier.to_string(),
        "A character key types text. Add Ctrl, Alt or Cmd."
    );
    assert!(s.pf_read(cx, |v| v.keyboard_listening().is_some()));
    s.pf_press("enter", cx);
    assert_eq!(
        lines(&s, &A::OpenNote, cx),
        Some(CaptureError::Reserved.to_string())
    );
    assert!(overrides(&s, cx).is_empty());
    assert!(s.pf_disabled(SAVE_ID, cx));
    s.pf_press("escape", cx);
    assert_eq!(lines(&s, &A::OpenNote, cx), None);
    assert!(s.pf_is_open(cx));
    settle(&rt, cx);
}

/// A Ctrl+letter vim reads is taken, with a note on the row.
#[gpui_kit::test]
fn a_vim_shadowed_chord_shows_the_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    change(&s, &A::OpenNote, "ctrl-v", cx);
    assert_eq!(row(&s, &A::OpenNote, cx), "Open note: Ctrl+V");
    assert_eq!(
        VIM_NOTE,
        "In the editor with vim mode, vim reads this key first."
    );
    assert_eq!(
        lines(&s, &A::OpenNote, cx),
        Some(format!("{EDITOR_NOTE} {VIM_NOTE}")),
        "Ctrl+V pastes in the plain editor too"
    );
    settle(&rt, cx);
}

/// A chord the plain editor edits with is taken, with a note on the row;
/// an ordinary chord gets none.
#[gpui_kit::test]
fn a_chord_the_editor_edits_with_shows_the_editor_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    change(&s, &A::ToggleFacetView, "ctrl-z", cx);
    assert_eq!(
        row(&s, &A::ToggleFacetView, cx),
        "Toggle facet view: Ctrl+Z"
    );
    assert_eq!(
        EDITOR_NOTE,
        "In the editor, this key then runs this shortcut and no longer edits text."
    );
    assert_eq!(
        lines(&s, &A::ToggleFacetView, cx).as_deref(),
        Some(EDITOR_NOTE)
    );
    change(&s, &A::ToggleFacetView, "ctrl-g", cx);
    assert_eq!(lines(&s, &A::ToggleFacetView, cx), None);
    settle(&rt, cx);
}

/// A chord a query action holds is shared, not taken: the row says where
/// the other action has it.
#[gpui_kit::test]
fn a_chord_a_query_action_holds_shows_where_it_wins(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    change(&s, &A::OpenAsk, "f7", cx);
    assert_eq!(row(&s, &A::OpenAsk, cx), "Ask: F7");
    assert_eq!(
        row(&s, &A::QueryBuilder, cx),
        "Query builder: F7 (where a query is typed)"
    );
    assert_eq!(
        lines(&s, &A::OpenAsk, cx).as_deref(),
        Some("Where a query is typed, F7 is Query builder.")
    );
    assert_eq!(lines(&s, &A::QueryBuilder, cx), None);
    change(&s, &A::QuerySyntax, "f5", cx);
    assert_eq!(
        lines(&s, &A::QuerySyntax, cx).as_deref(),
        Some("Everywhere else, F5 is Switch workspace.")
    );
    settle(&rt, cx);
}

/// Reset puts the row's default back and takes its message away. A reset
/// that takes a chord back warns as a change does.
#[gpui_kit::test]
fn reset_restores_the_default_and_clears_the_row_warning(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    change(&s, &A::OpenNote, "ctrl-e", cx);
    click(&s, reset_id(&A::OpenNote), cx);
    assert_eq!(
        row(&s, &A::OpenNote, cx),
        "Open note: Ctrl+O / Ctrl+Shift+O"
    );
    assert_eq!(lines(&s, &A::OpenNote, cx), None);
    assert!(label(&s, reset_id(&A::OpenNote), cx).is_none());
    assert!(
        focused(&s, change_id(&A::OpenNote), cx),
        "Reset hands the keyboard to the row's Change"
    );
    assert_eq!(overrides(&s, cx), [entry("OpenFileBrowser", &[])]);

    // Open files takes Ctrl+E back from the row that has it now.
    change(&s, &A::OpenNote, "ctrl-e", cx);
    assert_eq!(
        overrides(&s, cx),
        [
            entry("OpenFileBrowser", &[]),
            entry("OpenNote", &["primary+e"])
        ]
    );
    click(&s, reset_id(&A::OpenFileBrowser), cx);
    let warning = "Ctrl+E was Open note. Saving clears it there.";
    assert_eq!(lines(&s, &A::OpenFileBrowser, cx).as_deref(), Some(warning));
    assert_eq!(lines(&s, &A::OpenNote, cx).as_deref(), Some(warning));
    assert_eq!(
        row(&s, &A::OpenFileBrowser, cx),
        "Open files: Ctrl+E / Ctrl+Shift+E"
    );
    assert_eq!(row(&s, &A::OpenNote, cx), "Open note: none");
    settle(&rt, cx);
}

/// Reset all forgets every change: the draft is clean, the messages go.
#[gpui_kit::test]
fn reset_all_clears_every_change(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    change(&s, &A::OpenNote, "ctrl-e", cx);
    change(&s, &A::ToggleFacetView, "ctrl-g", cx);
    assert_eq!(overrides(&s, cx).len(), 3);
    click(&s, RESET_ALL_ID, cx);
    assert!(overrides(&s, cx).is_empty());
    assert!(s.pf_disabled(SAVE_ID, cx), "the draft is clean");
    assert_eq!(lines(&s, &A::OpenNote, cx), None);
    assert_eq!(lines(&s, &A::OpenFileBrowser, cx), None);
    assert_eq!(
        row(&s, &A::OpenFileBrowser, cx),
        "Open files: Ctrl+E / Ctrl+Shift+E"
    );
    assert_eq!(row(&s, &A::ToggleFacetView, cx), "Toggle facet view: none");
    assert!(
        label(&s, NOTICE_ID, cx).is_some(),
        "Reset all says what it did"
    );
    assert!(s.pf_disabled(RESET_ALL_ID, cx), "nothing left to reset");
    // The next edit takes the notice away: a change, a clear, a reset.
    change(&s, &A::ToggleFacetView, "ctrl-g", cx);
    assert_eq!(label(&s, NOTICE_ID, cx), None);
    click(&s, RESET_ALL_ID, cx);
    click(&s, clear_id(&A::OpenNote), cx);
    assert_eq!(label(&s, NOTICE_ID, cx), None);
    click(&s, RESET_ALL_ID, cx);
    change(&s, &A::OpenNote, "ctrl-e", cx);
    click(&s, RESET_ALL_ID, cx);
    click(&s, change_id(&A::OpenNote), cx);
    s.pf_press("ctrl-e", cx);
    click(&s, reset_id(&A::OpenNote), cx);
    assert_eq!(label(&s, NOTICE_ID, cx), None);
    settle(&rt, cx);
}

/// Clear leaves the row with no chord, written as `Action = []`; Reset
/// brings the default back. A row with no chord cannot be cleared.
#[gpui_kit::test]
fn clear_leaves_the_row_with_none_and_reset_brings_it_back(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    reveal(&s, clear_id(&A::ToggleFacetView), cx);
    assert!(disabled(&s, clear_id(&A::ToggleFacetView), cx), "none: off");
    click(&s, change_id(&A::OpenNote), cx);
    click(&s, clear_id(&A::OpenNote), cx);
    assert_eq!(s.pf_read(cx, |v| v.keyboard_listening().cloned()), None);
    assert_eq!(row(&s, &A::OpenNote, cx), "Open note: none");
    assert_eq!(overrides(&s, cx), [entry("OpenNote", &[])]);
    assert!(!s.pf_disabled(SAVE_ID, cx), "the draft is dirty");
    assert!(disabled(&s, clear_id(&A::OpenNote), cx));
    click(&s, reset_id(&A::OpenNote), cx);
    assert_eq!(
        row(&s, &A::OpenNote, cx),
        "Open note: Ctrl+O / Ctrl+Shift+O"
    );
    assert!(overrides(&s, cx).is_empty());
    assert!(s.pf_disabled(SAVE_ID, cx));
    settle(&rt, cx);
}

/// Cancel asks, Discard drops the key changes: nothing is written and the
/// keymap in effect stays.
#[gpui_kit::test]
fn cancel_discards_key_changes(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, s) = keyboard(cx, &rt);
    change(&s, &A::OpenNote, "ctrl-e", cx);
    s.pf_click(CANCEL_ID, cx);
    assert!(s.pf_read(cx, |v| v.confirm().is_some()), "it asks first");
    s.pf_click(DISCARD_ID, cx);
    assert!(!s.pf_is_open(cx));
    assert!(on_disk(&v).gui.key_overrides().is_empty());
    assert!(v.settings.read().unwrap().gui.key_overrides().is_empty());
    cx.read(|cx| {
        let active = keymap::active(cx);
        assert_eq!(active.chord_for(&A::OpenNote), Some(KeyInput::ctrl('o')));
        assert_eq!(
            active.chord_for(&A::OpenFileBrowser),
            Some(KeyInput::ctrl('e'))
        );
    });
    settle(&rt, cx);
}

/// With the leader chord and both Preferences chords cleared, the CFG view's
/// `Preferences…` button still opens them, and Reset all brings the
/// defaults back.
#[gpui_kit::test]
fn clearing_the_leader_and_preferences_chords_still_leaves_the_cfg_button(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, s) = keyboard(cx, &rt);
    click(&s, clear_id(&A::Leader), cx);
    click(&s, clear_id(&A::OpenPreferences), cx);
    assert_eq!(row(&s, &A::Leader, cx), "Leader: none");
    assert_eq!(row(&s, &A::OpenPreferences, cx), "Preferences: none");
    s.pf_save(&rt, cx);
    let saved = on_disk(&v).gui.key_overrides();
    assert_eq!(saved.get("Leader"), Some(&Vec::new()));
    assert_eq!(saved.get("OpenSettings"), Some(&Vec::new()));

    cx.run_until_parked();
    s.pf_press("f4", cx);
    s.pf_press("ctrl-,", cx);
    assert!(!s.pf_is_open(cx), "no chord opens them");
    s.pf_press("ctrl-shift-b", cx);
    assert!(
        cx.read(|cx| !s.workspace.read(cx).leader.is_pending()),
        "no chord arms the leader"
    );

    // The CFG view, through a click on the rail's CFG cell.
    assert!(!s.shows(CONFIG_ID, cx));
    s.pf_click(rail_cell_id(DrawerView::Config), cx);
    assert!(s.shows(CONFIG_ID, cx));
    s.pf_click(CONFIG_PREFERENCES_ID, cx);
    assert!(s.shows(PREFERENCES_ID, cx));
    s.pf_loaded(&rt, cx);
    s.pf_section(cx, Section::Keyboard);
    click(&s, RESET_ALL_ID, cx);
    assert!(overrides(&s, cx).is_empty());
    assert!(
        !s.pf_disabled(SAVE_ID, cx),
        "the saved changes are undone in the draft"
    );
    s.pf_save(&rt, cx);
    assert!(on_disk(&v).gui.key_overrides().is_empty());
    settle(&rt, cx);
}

/// The bounds of the element `id`, in a fresh frame.
fn bounds(
    s: &Started,
    id: impl Into<gpui_kit::ElementId>,
    cx: &mut TestAppContext,
) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    let id = id.into();
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).bounds()
    })
    .unwrap()
}

/// While a row listens its button reads Cancel; a click on it ends the
/// listening as Esc does, the chord stays, and the keyboard is back on the
/// row's button. The other rows keep their Change. Esc still works.
#[gpui_kit::test]
fn the_listening_row_offers_cancel(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    assert_eq!(
        label(&s, change_id(&A::OpenNote), cx).as_deref(),
        Some("Change the shortcut of Open note")
    );
    click(&s, change_id(&A::OpenNote), cx);
    assert!(s.pf_read(cx, |v| v.keyboard_listening().is_some()));
    assert_eq!(
        label(&s, change_id(&A::OpenNote), cx).as_deref(),
        Some("Cancel changing the shortcut of Open note")
    );
    assert_eq!(
        label(&s, change_id(&A::OpenFileBrowser), cx).as_deref(),
        Some("Change the shortcut of Open files"),
        "the other rows keep their Change"
    );
    s.pf_click(change_id(&A::OpenNote), cx);
    assert_eq!(s.pf_read(cx, |v| v.keyboard_listening().cloned()), None);
    assert_eq!(
        row(&s, &A::OpenNote, cx),
        "Open note: Ctrl+O / Ctrl+Shift+O"
    );
    assert!(overrides(&s, cx).is_empty());
    assert!(
        focused(&s, change_id(&A::OpenNote), cx),
        "the keyboard is back on the row's button"
    );
    assert_eq!(
        label(&s, change_id(&A::OpenNote), cx).as_deref(),
        Some("Change the shortcut of Open note")
    );
    // Esc still cancels.
    click(&s, change_id(&A::OpenNote), cx);
    s.pf_press("escape", cx);
    assert_eq!(s.pf_read(cx, |v| v.keyboard_listening().cloned()), None);
    assert!(focused(&s, change_id(&A::OpenNote), cx));
    assert!(s.pf_is_open(cx));
    settle(&rt, cx);
}

/// `Change` and `Clear` start at the same x on every row, whatever the
/// width of the chord text, and a `Reset` on a changed row moves neither.
#[gpui_kit::test]
fn the_row_buttons_line_up_across_rows(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    change(&s, &A::ToggleRail, "ctrl-alt-j", cx);
    assert!(label(&s, reset_id(&A::ToggleRail), cx).is_some());
    let change_x = bounds(&s, change_id(&A::OpenNote), cx).left();
    let clear_x = bounds(&s, clear_id(&A::OpenNote), cx).left();
    // A row's text starts where its group heading does: the row is pulled
    // out by its own padding (0.5rem), which leaves room for the listening
    // background.
    let rem = cx
        .update_window(s.handle, |_, window, _| window.rem_size())
        .unwrap();
    for (ix, group) in bindable_groups().iter().enumerate() {
        let heading = bounds(&s, group_id(ix), cx).left();
        for r in &group.rows {
            assert_eq!(
                bounds(&s, row_id(&r.action), cx).left() + rem * 0.5,
                heading,
                "label of {:?}",
                r.action
            );
        }
    }
    for group in bindable_groups() {
        for r in &group.rows {
            assert_eq!(
                bounds(&s, change_id(&r.action), cx).left(),
                change_x,
                "Change of {:?}",
                r.action
            );
            assert_eq!(
                bounds(&s, clear_id(&r.action), cx).left(),
                clear_x,
                "Clear of {:?}",
                r.action
            );
        }
    }
    // A listening row's Cancel starts where its Change did.
    click(&s, change_id(&A::SearchNotes), cx);
    assert_eq!(bounds(&s, change_id(&A::SearchNotes), cx).left(), change_x);
    settle(&rt, cx);
}

// ── The filter ──────────────────────────────────────────────────────────────

/// Give the filter field the keyboard and type `text` into it.
fn filter(s: &Started, text: &str, cx: &mut TestAppContext) {
    let text = typed(text);
    click(s, FILTER_ID, cx);
    cx.update_window(s.handle, |_, window, cx| window.input(&text, cx))
        .unwrap();
    cx.run_until_parked();
}

/// The filter field's text.
fn filter_text(s: &Started, cx: &mut TestAppContext) -> String {
    let input = s.pf_read(cx, |v| v.keyboard_filter_input());
    cx.read(|cx| input.read(cx).value().to_string())
}

/// Whether the row of `action` is drawn.
fn shown(s: &Started, action: &A, cx: &mut TestAppContext) -> bool {
    label(s, row_id(action), cx).is_some()
}

/// The indices of the group headings drawn.
fn headings(s: &Started, cx: &mut TestAppContext) -> Vec<usize> {
    (0..bindable_groups().len())
        .filter(|ix| label(s, group_id(*ix), cx).is_some())
        .collect()
}

/// A word of a row label keeps only the rows that have it, and only their
/// group's heading; a scope tag matches too. An empty field shows all.
#[gpui_kit::test]
fn the_filter_keeps_the_rows_whose_label_matches(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    assert_eq!(
        label(&s, FILTER_ID, cx).as_deref(),
        Some("Filter shortcuts")
    );
    assert_eq!(
        label(&s, KEYBOARD_ID, cx).as_deref(),
        Some("Keyboard shortcuts")
    );
    filter(&s, "SPLIT", cx);
    assert!(shown(&s, &A::SplitSlotRight, cx));
    assert!(shown(&s, &A::SplitSlotDown, cx));
    assert!(!shown(&s, &A::OpenNote, cx));
    assert!(!shown(&s, &A::ToggleRail, cx), "a row of the same group");
    assert_eq!(headings(&s, cx), [2], "only Layout and views");
    assert_eq!(label(&s, FILTER_EMPTY_ID, cx), None);
    // The scope tag is matched too.
    s.pf_with_view(cx, |v, window, cx| {
        let input = v.keyboard_filter_input();
        input.update(cx, |i, cx| i.set_value("sidebar and rail", window, cx));
    });
    cx.run_until_parked();
    assert!(shown(&s, &A::YankRow, cx));
    assert!(!shown(&s, &A::OpenNote, cx));
    // An empty field: every row, every heading.
    s.pf_with_view(cx, |v, window, cx| {
        let input = v.keyboard_filter_input();
        input.update(cx, |i, cx| i.set_value("", window, cx));
    });
    cx.run_until_parked();
    assert_eq!(
        headings(&s, cx),
        (0..bindable_groups().len()).collect::<Vec<_>>()
    );
    for group in bindable_groups() {
        for r in &group.rows {
            assert!(shown(&s, &r.action, cx), "{:?}", r.action);
        }
    }
    settle(&rt, cx);
}

/// The keys a row shows match, written either way.
#[gpui_kit::test]
fn the_filter_matches_the_chord_text(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    filter(&s, "ctrl+e", cx);
    assert!(shown(&s, &A::OpenFileBrowser, cx), "Open files has Ctrl+E");
    assert!(!shown(&s, &A::OpenNote, cx));
    s.pf_press("escape", cx);
    filter(&s, "Ctrl+E", cx);
    assert!(shown(&s, &A::OpenFileBrowser, cx));
    assert!(!shown(&s, &A::OpenNote, cx));
    settle(&rt, cx);
}

/// Every word must match.
#[gpui_kit::test]
fn every_word_of_the_filter_must_match(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    filter(&s, "move  left", cx);
    assert!(shown(&s, &A::MoveViewToNeighbour(Dir::Left), cx));
    assert!(!shown(&s, &A::FocusRegion(Dir::Left), cx), "no move");
    assert!(
        !shown(&s, &A::MoveViewToNeighbour(Dir::Right), cx),
        "no left"
    );
    settle(&rt, cx);
}

/// Set the filter field's text, the keyboard staying where it is.
fn set_filter(s: &Started, text: &str, cx: &mut TestAppContext) {
    let text = typed(text);
    s.pf_with_view(cx, |v, window, cx| {
        let input = v.keyboard_filter_input();
        input.update(cx, |i, cx| i.set_value(text, window, cx));
    });
    cx.run_until_parked();
}

/// A row never goes from under the user: the listening row stays while it
/// listens, and the rows an edit changed or warned stay while the filter
/// keeps the text it had then.
#[gpui_kit::test]
fn the_filter_keeps_the_rows_being_worked_on(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    click(&s, change_id(&A::ToggleFacetView), cx);
    set_filter(&s, "split", cx);
    assert!(s.pf_read(cx, |v| v.keyboard_listening().is_some()));
    assert!(
        shown(&s, &A::ToggleFacetView, cx),
        "the listening row stays"
    );
    assert!(!shown(&s, &A::FindInBuffer, cx));
    // Change then Esc edits nothing: the row goes.
    s.pf_press("escape", cx);
    assert!(!shown(&s, &A::ToggleFacetView, cx), "no edit, no keep");
    // An edit under "ctrl+o": the row no longer matches, nor does the row
    // it took the chord from, and both stay.
    set_filter(&s, "ctrl+o", cx);
    change(&s, &A::OpenNote, "ctrl-e", cx);
    assert!(shown(&s, &A::OpenNote, cx), "the row edited stays");
    assert!(shown(&s, &A::OpenFileBrowser, cx), "the warned row stays");
    assert!(!shown(&s, &A::NewNote, cx));
    settle(&rt, cx);
}

/// A new filter after an edit drops the rows the edit kept: with no match,
/// the note shows alone.
#[gpui_kit::test]
fn a_new_filter_drops_the_rows_an_edit_kept(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    set_filter(&s, "ctrl+o", cx);
    change(&s, &A::OpenNote, "ctrl-e", cx);
    assert!(shown(&s, &A::OpenNote, cx));
    set_filter(&s, "zqx", cx);
    assert!(!shown(&s, &A::OpenNote, cx));
    assert!(!shown(&s, &A::OpenFileBrowser, cx));
    assert!(headings(&s, cx).is_empty());
    assert_eq!(
        label(&s, FILTER_EMPTY_ID, cx).as_deref(),
        Some("No shortcut matches \"zqx\".")
    );
    settle(&rt, cx);
}

/// No row matches: a note says so, with the text typed.
#[gpui_kit::test]
fn a_filter_with_no_match_says_so(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    filter(&s, "Zqx", cx);
    assert_eq!(
        label(&s, FILTER_EMPTY_ID, cx).as_deref(),
        Some("No shortcut matches \"Zqx\".")
    );
    let role = cx
        .update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(FILTER_EMPTY_ID).role()
        })
        .unwrap();
    assert_eq!(role, Some(gpui_kit::Role::Status), "an empty state");
    assert!(headings(&s, cx).is_empty());
    assert!(!shown(&s, &A::OpenNote, cx));
    settle(&rt, cx);
}

/// Esc in the field clears its text and keeps Preferences; Esc in the
/// empty field closes the clean view, as Esc does elsewhere.
#[gpui_kit::test]
fn esc_in_the_filter_clears_it_then_closes(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    filter(&s, "split", cx);
    s.pf_press("escape", cx);
    assert_eq!(filter_text(&s, cx), "");
    assert!(s.pf_is_open(cx), "Preferences stay");
    assert!(shown(&s, &A::OpenNote, cx), "the full list is back");
    assert!(focused(&s, FILTER_ID, cx), "the field keeps the keyboard");
    s.pf_press("escape", cx);
    assert!(!s.pf_is_open(cx), "a second Esc closes them");
    settle(&rt, cx);
}

/// Tab goes from the field to the first row: the field comes first.
#[gpui_kit::test]
fn tab_goes_from_the_filter_to_the_list(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    click(&s, FILTER_ID, cx);
    assert!(focused(&s, FILTER_ID, cx));
    s.pf_press("tab", cx);
    let first = &bindable_groups()[0].rows[0].action;
    assert!(focused(&s, change_id(first), cx));
    settle(&rt, cx);
}

/// Another section shown, then Keyboard again: the full list.
#[gpui_kit::test]
fn leaving_the_section_clears_the_filter(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    filter(&s, "split", cx);
    assert!(!shown(&s, &A::OpenNote, cx));
    s.pf_section(cx, Section::Display);
    s.pf_section(cx, Section::Keyboard);
    assert_eq!(filter_text(&s, cx), "");
    assert!(shown(&s, &A::OpenNote, cx));
    assert_eq!(headings(&s, cx).len(), bindable_groups().len());
    settle(&rt, cx);
}

/// The filter is view state: the draft stays clean and Save off.
#[gpui_kit::test]
fn the_filter_leaves_the_draft_clean(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    filter(&s, "split", cx);
    assert!(s.pf_disabled(SAVE_ID, cx));
    assert!(overrides(&s, cx).is_empty());
    s.pf_press("enter", cx);
    assert!(s.pf_is_open(cx), "Enter does nothing special");
    assert!(s.pf_disabled(SAVE_ID, cx));
    settle(&rt, cx);
}

/// Keys typed in the field are text: a listening row stops listening and
/// takes no chord.
#[gpui_kit::test]
fn typing_in_the_filter_assigns_no_chord(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    click(&s, change_id(&A::ToggleFacetView), cx);
    click(&s, FILTER_ID, cx);
    s.pf_press("ctrl-g", cx);
    s.pf_press("g", cx);
    assert_eq!(s.pf_read(cx, |v| v.keyboard_listening().cloned()), None);
    assert!(overrides(&s, cx).is_empty(), "no chord for the row");
    assert_eq!(lines(&s, &A::ToggleFacetView, cx), None, "no refusal");
    assert_eq!(row(&s, &A::ToggleFacetView, cx), "Toggle facet view: none");
    settle(&rt, cx);
}

// ── Save installs the keymap in the running window ──────────────────────────

/// What `key` (a GPUI keystroke) does in the editor window now: the action
/// the keymap in effect resolves it to there.
fn resolves(s: &Started, key: KeyInput, cx: &mut TestAppContext) -> Option<A> {
    cx.read(|cx| {
        let ctx = s.workspace.read(cx).key_ctx(cx);
        keymap::active(cx).resolve(key, ctx).cloned()
    })
}

/// The CFG view's `Leader key` row, as it draws now.
fn cfg_leader_row(s: &Started, cx: &mut TestAppContext) -> String {
    let view = cx.read(|cx| s.workspace.read(cx).config.clone());
    cx.update(|cx| view.update(cx, |v, cx| v.rows(cx)))
        .expect("the window has its settings")
        .into_iter()
        .find(|r| r.label == "Leader key")
        .expect("a leader row")
        .value
}

/// After Save the new chord runs its action in the editor window at once.
#[gpui_kit::test]
fn save_applies_the_new_chord_without_a_restart(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    change(&s, &A::OpenNote, "ctrl-e", cx);
    assert_eq!(
        cx.read(|cx| keymap::active(cx).chord_for(&A::OpenNote)),
        Some(KeyInput::ctrl('o')),
        "not applied before Save"
    );
    s.pf_save(&rt, cx);
    assert!(!s.pf_is_open(cx), "Save closes Preferences");
    // Save rebuilds the keymap for the host, where `primary` is Cmd on a Mac.
    let saved = if cfg!(target_os = "macos") {
        KeyInput::new(Key::Char('e'), Mods::SUPER)
    } else {
        KeyInput::ctrl('e')
    };
    assert_eq!(resolves(&s, saved, cx), Some(A::OpenNote));
    settle(&rt, cx);
}

/// The chord an action gave up, and the one it took from another action,
/// stop doing their old work.
#[gpui_kit::test]
fn save_stops_the_old_chord(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    assert_eq!(
        resolves(&s, KeyInput::ctrl('o'), cx),
        Some(A::OpenNote),
        "the default, before Save"
    );
    change(&s, &A::OpenNote, "ctrl-e", cx);
    s.pf_save(&rt, cx);
    assert_eq!(resolves(&s, KeyInput::ctrl('o'), cx), None, "given up");
    cx.read(|cx| {
        assert!(
            keymap::active(cx)
                .chords_for(&A::OpenFileBrowser)
                .iter()
                .all(|c| *c != KeyInput::ctrl('e')),
            "the browser lost the chord it was stolen from"
        );
    });
    settle(&rt, cx);
}

/// `[gui.keys]` is written, the rest of `[gui]` stays.
#[gpui_kit::test]
fn save_writes_gui_keys_and_keeps_other_gui_keys(cx: &mut TestAppContext) {
    use crate::workspace::preferences_tests::config;
    use kimun_notes::settings::AppSettings;
    use kimun_notes::settings::gui::WindowDecorations;

    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let system = WindowDecorations::System;
    AppSettings::update_gui_at(config(&v), |gui| gui.set_window_decorations(system)).unwrap();
    v.settings
        .write()
        .unwrap()
        .gui
        .set_window_decorations(system);
    let s = on_vault(cx, &rt, &v);
    s.pf_route(cx);
    s.pf_loaded(&rt, cx);
    s.pf_section(cx, Section::Keyboard);
    change(&s, &A::OpenNote, "ctrl-e", cx);
    s.pf_save(&rt, cx);

    let text = std::fs::read_to_string(config(&v)).unwrap();
    assert!(text.contains("[gui.keys]"), "{text}");
    let table: toml::Table = text.parse().unwrap();
    let gui = table["gui"].as_table().expect("a [gui] table");
    assert!(gui["keys"].as_table().unwrap().contains_key("OpenNote"));
    assert!(gui.contains_key("window_decorations"), "{text}");
    assert_eq!(on_disk(&v).gui.window_decorations(), system);
    assert_eq!(
        on_disk(&v).gui.key_overrides().get("OpenNote"),
        Some(&vec!["primary+e".to_string()])
    );
    settle(&rt, cx);
}

/// The CFG row follows the saved leader chord without a reopen.
#[gpui_kit::test]
fn the_cfg_row_shows_the_new_leader_chord_after_save(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = keyboard(cx, &rt);
    let before = cfg_leader_row(&s, cx);
    change(&s, &A::Leader, "f9", cx);
    s.pf_save(&rt, cx);
    let after = cfg_leader_row(&s, cx);
    assert_ne!(after, before);
    assert_eq!(after, "F9");
    settle(&rt, cx);
}

// ── Export and import ───────────────────────────────────────────────────────

mod file {
    use super::*;
    use crate::workspace::folder_chooser::{self, FolderChooser, Listing};
    use crate::workspace::preferences::keyboard::file::{
        CHOOSER_AREA_ID, CHOOSER_PREFIX, EXPORT_CHOOSE_ID, EXPORT_ID, EXPORT_NOTICE_ID,
        EXPORT_PATH_ID, IMPORT_CHOOSE_ID, IMPORT_ID, IMPORT_NOTICE_ID, IMPORT_PATH_ID,
    };
    use crate::workspace::preferences_tests::config;
    use crate::workspace::start_tests::drive;
    use gpui_kit::{Entity, Modifiers, Point, ScrollEvent, SharedString, VisualTestContext, px};
    use kimun_core::system::{self, SystemPath};
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    /// Set the Export (`export`) or Import path field to `path`.
    fn set_path(s: &Started, export: bool, path: &Path, cx: &mut TestAppContext) {
        let input = s.pf_read(cx, |v| v.keyboard_path_input(export));
        let text = system::path_to_string(path);
        cx.update_window(s.handle, |_, window, cx| {
            input.update(cx, |i, cx| i.set_value(text, window, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// The Export (`export`) or Import path field's text.
    fn path_text(s: &Started, export: bool, cx: &mut TestAppContext) -> String {
        let input = s.pf_read(cx, |v| v.keyboard_path_input(export));
        cx.read(|cx| input.read(cx).value().to_string())
    }

    /// Click `id` and wait until the file work is done.
    fn run(s: &Started, rt: &Runtime, id: &'static str, cx: &mut TestAppContext) {
        click(s, id, cx);
        drive(rt, cx, |cx| !s.pf_read(cx, |v| v.keyboard_file_working()));
    }

    /// The chooser of the Keyboard section, once it is listed.
    fn chooser(s: &Started, rt: &Runtime, cx: &mut TestAppContext) -> Entity<FolderChooser> {
        let chooser = s
            .pf_read(cx, |v| v.keyboard_chooser())
            .expect("the chooser is up");
        drive(rt, cx, |cx| {
            cx.read(|cx| !matches!(chooser.read(cx).listing(), Listing::Loading))
        });
        chooser
    }

    /// The names the chooser lists.
    fn rows(chooser: &Entity<FolderChooser>, cx: &TestAppContext) -> Vec<String> {
        cx.read(|cx| match chooser.read(cx).listing() {
            Listing::Loaded(rows) => rows
                .iter()
                .map(crate::workspace::folder_chooser::folder_name)
                .collect(),
            other => panic!("not listed: {other:?}"),
        })
    }

    /// A temp folder with `sub/`, `.hidden/`, `keys.toml`, `.hidden.toml`,
    /// `notes.txt`.
    fn folder() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = SystemPath::try_absolute(tmp.path())
            .unwrap()
            .into_path_buf();
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::create_dir(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join("keys.toml"), "[keys]\n").unwrap();
        std::fs::write(dir.join(".hidden.toml"), "[keys]\n").unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        (tmp, dir)
    }

    fn config_bytes(v: &TestVault) -> Option<Vec<u8>> {
        std::fs::read(config(v)).ok()
    }

    /// The Keymap file area comes after the list; Reset all and its notice
    /// stay above it. The chooser takes the area's place, still below the
    /// list.
    #[gpui_kit::test]
    fn the_keymap_file_area_is_below_the_list(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        change(&s, &A::ToggleFacetView, "ctrl-g", cx);
        click(&s, RESET_ALL_ID, cx);
        let list = bounds(&s, KEYBOARD_ID, cx);
        assert!(bounds(&s, RESET_ALL_ID, cx).bottom() <= list.top());
        assert!(bounds(&s, NOTICE_ID, cx).bottom() <= list.top());
        assert!(
            bounds(&s, EXPORT_PATH_ID, cx).top() >= list.bottom(),
            "Export under the list"
        );
        assert!(bounds(&s, IMPORT_PATH_ID, cx).top() >= list.bottom());
        let (_tmp, dir) = folder();
        set_path(&s, false, &dir.join("missing.toml"), cx);
        click(&s, IMPORT_CHOOSE_ID, cx);
        chooser(&s, &rt, cx);
        let top = bounds(
            &s,
            SharedString::from(format!("{CHOOSER_PREFIX}-chooser")),
            cx,
        )
        .top();
        assert!(
            top >= bounds(&s, KEYBOARD_ID, cx).bottom(),
            "the chooser too"
        );
        settle(&rt, cx);
    }

    /// Tab goes from the last row's buttons to the Export field: the list
    /// first, then the file area.
    #[gpui_kit::test]
    fn tab_goes_from_the_last_row_to_the_keymap_file_area(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let groups = bindable_groups();
        let last = &groups.last().unwrap().rows.last().unwrap().action;
        click(&s, change_id(last), cx);
        s.pf_press("escape", cx);
        assert!(focused(&s, change_id(last), cx));
        let mut reached = false;
        for _ in 0..3 {
            s.pf_press("tab", cx);
            if focused(&s, EXPORT_PATH_ID, cx) {
                reached = true;
                break;
            }
        }
        assert!(reached, "Tab reaches the Export field after the last row");
        settle(&rt, cx);
    }

    /// The section's intro and the Keymap file note, as they read.
    #[test]
    fn the_section_copy_reads_plainly() {
        use crate::workspace::preferences::keyboard::INTRO;
        use crate::workspace::preferences::keyboard::file::FILE_NOTE;
        assert_eq!(
            INTRO,
            "Change a shortcut, then press the new keys. Esc cancels. If another \
             action already has those keys, it loses them. Save to keep your changes."
        );
        assert_eq!(
            FILE_NOTE,
            "Export saves all your shortcuts to a file. Import loads a file in \
             their place."
        );
    }

    /// The chooser, much taller than the area it replaces at the foot of
    /// the page, opens wholly in view with no further scroll: its commands
    /// show. After Cancel the area is back in view.
    #[gpui_kit::test]
    fn the_chooser_opens_in_view_and_cancel_keeps_the_area_in_view(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let (_tmp, dir) = folder();
        let choose = format!("{CHOOSER_PREFIX}-chooser-choose");
        let cancel = format!("{CHOOSER_PREFIX}-chooser-cancel");
        set_path(&s, true, &dir.join("mine.toml"), cx);
        click(&s, EXPORT_CHOOSE_ID, cx);
        chooser(&s, &rt, cx);
        assert!(s.pf_shows(choose.clone(), cx), "Choose this folder shows");
        assert!(s.pf_shows(cancel.clone(), cx), "Cancel shows");
        s.pf_click(cancel.clone(), cx);
        assert!(s.pf_read(cx, |v| v.keyboard_chooser().is_none()));
        assert!(
            s.pf_shows(EXPORT_CHOOSE_ID, cx),
            "the Export row is in view"
        );
        set_path(&s, false, &dir.join("missing.toml"), cx);
        click(&s, IMPORT_CHOOSE_ID, cx);
        chooser(&s, &rt, cx);
        assert!(s.pf_shows(cancel.clone(), cx), "Cancel shows in files mode");
        s.pf_click(cancel, cx);
        assert!(
            s.pf_shows(IMPORT_CHOOSE_ID, cx),
            "the Import row is in view"
        );
        settle(&rt, cx);
    }

    /// The bounds of `id` once the frames after an open are drawn: the
    /// pane's scroll to the chooser lands in the frame after the one that
    /// first lays the chooser out, as in the app.
    fn settled_bounds(
        s: &Started,
        id: impl Into<gpui_kit::ElementId>,
        cx: &mut TestAppContext,
    ) -> gpui_kit::Bounds<gpui_kit::Pixels> {
        let id = id.into();
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            window.find(id).bounds()
        })
        .unwrap()
    }

    /// In a window where the whole chooser fits, it opens wholly in the
    /// pane: heading, field, list, commands and hints.
    #[gpui_kit::test]
    fn the_keyboard_chooser_is_fully_in_view_when_it_fits(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        cx.simulate_window_resize(s.handle, gpui_kit::size(px(1000.), px(1000.)));
        cx.run_until_parked();
        let (_tmp, dir) = folder();
        set_path(&s, true, &dir.join("mine.toml"), cx);
        click(&s, EXPORT_CHOOSE_ID, cx);
        chooser(&s, &rt, cx);
        let area = settled_bounds(&s, CHOOSER_AREA_ID, cx);
        let pane = settled_bounds(&s, CONTENT_ID, cx);
        assert!(
            area.size.height < pane.size.height,
            "the chooser {area:?} fits the pane {pane:?}"
        );
        assert!(
            area.top() >= pane.top() && area.bottom() <= pane.bottom(),
            "the chooser {area:?} is inside the pane {pane:?}"
        );
        settle(&rt, cx);
    }

    /// In a short window the chooser is taller than the pane: the pane
    /// shows its top, so the heading, the Location field and the list
    /// show. The list keeps its floor, and the wheel still reaches the
    /// commands.
    #[gpui_kit::test]
    fn a_chooser_taller_than_the_pane_shows_its_top(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        cx.simulate_window_resize(s.handle, gpui_kit::size(px(1000.), px(420.)));
        cx.run_until_parked();
        let (_tmp, dir) = folder();
        set_path(&s, true, &dir.join("mine.toml"), cx);
        click(&s, EXPORT_CHOOSE_ID, cx);
        chooser(&s, &rt, cx);
        let area = settled_bounds(&s, CHOOSER_AREA_ID, cx);
        let pane = settled_bounds(&s, CONTENT_ID, cx);
        assert!(
            area.size.height > pane.size.height,
            "the chooser {area:?} is taller than the pane {pane:?}"
        );
        assert!(
            (area.top() - pane.top()).abs() < px(1.),
            "the chooser's top {area:?} is at the pane's top {pane:?}"
        );
        let location = settled_bounds(&s, format!("{CHOOSER_PREFIX}-chooser-location"), cx);
        assert!(location.bottom() <= pane.bottom(), "the field shows");
        let rows = settled_bounds(&s, format!("{CHOOSER_PREFIX}-chooser-rows"), cx);
        let rem = cx
            .update_window(s.handle, |_, window, _| window.rem_size())
            .unwrap();
        assert_eq!(rows.size.height, rem * folder_chooser::LIST_MIN_REMS);
        assert!(rows.top() < pane.bottom(), "the list shows");
        let choose = format!("{CHOOSER_PREFIX}-chooser-choose");
        reveal(&s, choose.clone(), cx);
        assert!(s.pf_shows(choose, cx), "the wheel reaches the commands");
        settle(&rt, cx);
    }

    /// Each chooser row names its kind for a screen reader, and the rows
    /// keep their height.
    #[gpui_kit::test]
    fn chooser_rows_name_their_kind(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let (_tmp, dir) = folder();
        set_path(&s, false, &dir.join("missing.toml"), cx);
        click(&s, IMPORT_CHOOSE_ID, cx);
        chooser(&s, &rt, cx);
        let sub = SystemPath::try_absolute(dir.join("sub")).unwrap();
        let file = SystemPath::try_absolute(dir.join("keys.toml")).unwrap();
        let folder_row = folder_chooser::row_id(CHOOSER_PREFIX, &sub);
        let file_row = folder_chooser::file_row_id(CHOOSER_PREFIX, &file);
        assert_eq!(
            label(&s, folder_row.clone(), cx).as_deref(),
            Some("sub, folder")
        );
        assert_eq!(
            label(&s, file_row.clone(), cx).as_deref(),
            Some("keys.toml, file")
        );
        let rem = cx
            .update_window(s.handle, |_, window, _| window.rem_size())
            .unwrap();
        assert_eq!(bounds(&s, folder_row, cx).size.height, rem * 1.75);
        assert_eq!(bounds(&s, file_row, cx).size.height, rem * 1.75);
        settle(&rt, cx);
    }

    /// `New folder…` (and its Ctrl+N key) is only in folder mode: Export's
    /// chooser has it, Import's file chooser does not. Cancel stays.
    #[gpui_kit::test]
    fn new_folder_shows_only_in_folder_mode(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let (_tmp, dir) = folder();
        let new = format!("{CHOOSER_PREFIX}-chooser-new");
        let cancel = format!("{CHOOSER_PREFIX}-chooser-cancel");
        set_path(&s, true, &dir.join("mine.toml"), cx);
        click(&s, EXPORT_CHOOSE_ID, cx);
        chooser(&s, &rt, cx);
        assert!(s.pf_shows(new.clone(), cx), "folder mode has New folder");
        assert!(s.pf_shows(cancel.clone(), cx));
        s.pf_press("escape", cx);
        set_path(&s, false, &dir.join("missing.toml"), cx);
        click(&s, IMPORT_CHOOSE_ID, cx);
        let files = chooser(&s, &rt, cx);
        let drawn = cx
            .update_window(s.handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find(SharedString::from(new)).is_some()
            })
            .unwrap();
        assert!(!drawn, "files mode has none");
        assert!(s.pf_shows(cancel, cx));
        s.pf_press("ctrl-n", cx);
        assert!(
            !cx.read(|cx| files.read(cx).new_folder_open()),
            "Ctrl+N opens nothing"
        );
        assert!(
            !cx.read(|cx| files.read(cx).hints(cx))
                .contains("new folder")
        );
        settle(&rt, cx);
    }

    /// Esc in the Keyboard section's chooser closes only the chooser: the
    /// Location field has the keyboard as it opens, its keys are never a
    /// shortcut for the section (a row that listened stops), Esc with text
    /// clears it, Esc from the empty field gives the keyboard back to the
    /// path field it was for. Preferences stay open, with no question.
    #[gpui_kit::test]
    fn the_keyboard_section_chooser_esc_closes_only_the_chooser(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let (_tmp, dir) = folder();
        let location = format!("{CHOOSER_PREFIX}-chooser-location");
        set_path(&s, true, &dir.join("mine.toml"), cx);
        click(&s, change_id(&A::ToggleFacetView), cx);
        click(&s, EXPORT_CHOOSE_ID, cx);
        let chooser = chooser(&s, &rt, cx);
        assert!(
            focused(&s, location.clone(), cx),
            "the field has the keyboard"
        );
        assert_eq!(s.pf_read(cx, |v| v.keyboard_listening().cloned()), None);
        s.pf_typed("su", cx);
        s.pf_press("ctrl-g", cx);
        assert!(
            overrides(&s, cx).is_empty(),
            "no chord taken from the field"
        );
        let text = |cx: &TestAppContext| {
            cx.read(|cx| {
                chooser
                    .read(cx)
                    .location_input()
                    .read(cx)
                    .value()
                    .to_string()
            })
        };
        assert_eq!(text(cx), "su");
        s.pf_press("escape", cx);
        assert_eq!(text(cx), "", "Esc clears the text first");
        assert!(s.pf_read(cx, |v| v.keyboard_chooser().is_some()));
        s.pf_press("escape", cx);
        assert!(s.pf_read(cx, |v| v.keyboard_chooser().is_none()));
        assert!(focused(&s, EXPORT_PATH_ID, cx), "back to the path field");
        assert!(s.pf_is_open(cx), "Preferences stay");
        assert!(s.pf_read(cx, |v| v.confirm().is_none()), "no question");
        assert_eq!(
            path_text(&s, true, cx),
            system::path_to_string(dir.join("mine.toml"))
        );
        settle(&rt, cx);
    }

    /// A row stops listening once the keyboard moves to a path field: its
    /// keys are the field's, never a refusal or a chord for the row.
    #[gpui_kit::test]
    fn a_row_stops_listening_when_a_path_field_takes_the_keyboard(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        click(&s, change_id(&A::ToggleFacetView), cx);
        click(&s, EXPORT_PATH_ID, cx);
        s.pf_press("g", cx);
        assert_eq!(lines(&s, &A::ToggleFacetView, cx), None, "no refusal");
        s.pf_press("ctrl-g", cx);
        assert_eq!(s.pf_read(cx, |v| v.keyboard_listening().cloned()), None);
        assert!(overrides(&s, cx).is_empty(), "no chord for the row");
        assert_eq!(row(&s, &A::ToggleFacetView, cx), "Toggle facet view: none");
        settle(&rt, cx);
    }

    /// `Choose file…` lists the folders (hidden ones too) and the `.toml`
    /// files (hidden ones left out); Enter on a file puts it in the Import
    /// field and closes the chooser.
    #[gpui_kit::test]
    fn chooser_in_files_mode_chooses_a_file_on_enter(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let (_tmp, dir) = folder();
        set_path(&s, false, &dir.join("missing.toml"), cx);
        click(&s, IMPORT_CHOOSE_ID, cx);
        let chooser = chooser(&s, &rt, cx);
        assert_eq!(
            cx.read(|cx| chooser.read(cx).dir().as_path().to_path_buf()),
            dir,
            "it starts in the field's folder"
        );
        assert_eq!(rows(&chooser, cx), [".hidden", "sub", "keys.toml"]);
        s.pf_press("down", cx);
        s.pf_press("down", cx);
        s.pf_press("enter", cx);
        assert!(s.pf_read(cx, |v| v.keyboard_chooser().is_none()));
        assert_eq!(
            path_text(&s, false, cx),
            system::path_to_string(dir.join("keys.toml"))
        );
        settle(&rt, cx);
    }

    /// The chooser up on a folder of 40 subfolders, in the pane's view.
    fn long_chooser(
        s: &Started,
        rt: &Runtime,
        cx: &mut TestAppContext,
    ) -> (tempfile::TempDir, PathBuf, Entity<FolderChooser>) {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = SystemPath::try_absolute(tmp.path())
            .unwrap()
            .into_path_buf();
        for i in 0..40 {
            std::fs::create_dir(dir.join(format!("d{i:02}"))).unwrap();
        }
        set_path(s, false, &dir.join("missing.toml"), cx);
        click(s, IMPORT_CHOOSE_ID, cx);
        let chooser = chooser(s, rt, cx);
        let first = SystemPath::try_absolute(dir.join("d00")).unwrap();
        reveal(s, folder_chooser::row_id(CHOOSER_PREFIX, &first), cx);
        (tmp, dir, chooser)
    }

    /// The chooser's top in the window: it moves only when the pane scrolls.
    fn chooser_top(s: &Started, cx: &mut TestAppContext) -> gpui_kit::Pixels {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .find(SharedString::from(format!("{CHOOSER_PREFIX}-chooser")))
                .bounds()
                .top()
        })
        .unwrap()
    }

    /// The centre of the element `id`, in a fresh frame.
    fn centre(
        s: &Started,
        id: impl Into<gpui_kit::ElementId>,
        cx: &mut TestAppContext,
    ) -> Point<gpui_kit::Pixels> {
        let id = id.into();
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).bounds().center()
        })
        .unwrap()
    }

    /// The wheel `dy` lines at `at`, as the platform sends it.
    fn wheel(s: &Started, at: Point<gpui_kit::Pixels>, dy: f32, cx: &mut TestAppContext) {
        let mut visual = VisualTestContext::from_window(s.handle, cx);
        visual.simulate_mouse_move(at, None, Modifiers::default());
        visual.simulate_event(ScrollEvent {
            position: at,
            delta: ScrollDelta::Lines(point(0.0, dy)),
            ..Default::default()
        });
        cx.run_until_parked();
    }

    /// The wheel over the chooser's list scrolls the list only, also at
    /// its top or bottom edge: the pane under it stays put.
    #[gpui_kit::test]
    fn the_wheel_over_the_chooser_list_scrolls_the_list_only(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let (_tmp, dir, chooser) = long_chooser(&s, &rt, cx);
        let first = SystemPath::try_absolute(dir.join("d00")).unwrap();
        // The chooser opens with the pane at its bottom. A line up, so
        // that the pane could scroll either way.
        let path = centre(
            &s,
            SharedString::from(format!("{CHOOSER_PREFIX}-chooser-location")),
            cx,
        );
        let before = chooser_top(&s, cx);
        wheel(&s, path, 1.0, cx);
        let pane = chooser_top(&s, cx);
        assert!(pane > before, "the pane is off its bottom");
        let over = centre(&s, folder_chooser::row_id(CHOOSER_PREFIX, &first), cx);
        let list = |cx: &TestAppContext| cx.read(|cx| chooser.read(cx).list_scroll());

        wheel(&s, over, 3.0, cx);
        assert_eq!(list(cx), px(0.), "the list is at its top edge");
        assert_eq!(chooser_top(&s, cx), pane, "the pane stays at the top edge");

        wheel(&s, over, -3.0, cx);
        assert!(list(cx) > px(0.), "the list scrolls down");
        assert_eq!(chooser_top(&s, cx), pane, "the pane stays");

        for _ in 0..40 {
            wheel(&s, over, -3.0, cx);
        }
        let bottom = list(cx);
        wheel(&s, over, -3.0, cx);
        assert_eq!(list(cx), bottom, "the list is at its bottom edge");
        assert_eq!(
            chooser_top(&s, cx),
            pane,
            "the pane stays at the bottom edge"
        );
        settle(&rt, cx);
    }

    /// The wheel outside the list, over the chooser's Location field, still
    /// scrolls the pane.
    #[gpui_kit::test]
    fn the_wheel_outside_the_chooser_list_scrolls_the_pane(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let (_tmp, _dir, chooser) = long_chooser(&s, &rt, cx);
        let path = centre(
            &s,
            SharedString::from(format!("{CHOOSER_PREFIX}-chooser-location")),
            cx,
        );
        // The chooser opens with the pane at its bottom: up, then down.
        let pane = chooser_top(&s, cx);
        wheel(&s, path, 1.0, cx);
        let up = chooser_top(&s, cx);
        assert!(up > pane, "the pane scrolls up: {pane:?} to {up:?}");
        let path = centre(
            &s,
            SharedString::from(format!("{CHOOSER_PREFIX}-chooser-location")),
            cx,
        );
        wheel(&s, path, -1.0, cx);
        assert_eq!(chooser_top(&s, cx), pane, "the pane scrolls back down");
        assert_eq!(cx.read(|cx| chooser.read(cx).list_scroll()), px(0.));
        settle(&rt, cx);
    }

    /// A click on a file row picks it at once (a folder row opens).
    #[gpui_kit::test]
    fn a_click_on_a_file_row_picks_it(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let (_tmp, dir) = folder();
        set_path(&s, false, &dir.join("missing.toml"), cx);
        click(&s, IMPORT_CHOOSE_ID, cx);
        chooser(&s, &rt, cx);
        let file = SystemPath::try_absolute(dir.join("keys.toml")).unwrap();
        click(&s, folder_chooser::file_row_id(CHOOSER_PREFIX, &file), cx);
        assert!(s.pf_read(cx, |v| v.keyboard_chooser().is_none()));
        assert_eq!(
            path_text(&s, false, cx),
            system::path_to_string(dir.join("keys.toml"))
        );
        settle(&rt, cx);
    }

    /// Esc with the chooser up closes the chooser only; Preferences stay,
    /// and the field keeps its path.
    #[gpui_kit::test]
    fn esc_closes_only_the_chooser(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let (_tmp, dir) = folder();
        let start = dir.join("mine.toml");
        set_path(&s, true, &start, cx);
        click(&s, EXPORT_CHOOSE_ID, cx);
        chooser(&s, &rt, cx);
        s.pf_press("escape", cx);
        assert!(s.pf_read(cx, |v| v.keyboard_chooser().is_none()));
        assert!(s.pf_is_open(cx), "Preferences stay open");
        assert_eq!(path_text(&s, true, cx), system::path_to_string(&start));
        settle(&rt, cx);
    }

    /// Esc in the empty filter while the chooser is up closes the chooser
    /// only, as Esc does elsewhere in the section.
    #[gpui_kit::test]
    fn esc_in_the_empty_filter_closes_only_the_chooser(cx: &mut TestAppContext) {
        use crate::workspace::preferences::keyboard::FILTER_ID;
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let (_tmp, dir) = folder();
        set_path(&s, false, &dir.join("missing.toml"), cx);
        click(&s, IMPORT_CHOOSE_ID, cx);
        chooser(&s, &rt, cx);
        click(&s, FILTER_ID, cx);
        s.pf_press("escape", cx);
        assert!(s.pf_read(cx, |v| v.keyboard_chooser().is_none()));
        assert!(s.pf_is_open(cx), "Preferences stay open");
        settle(&rt, cx);
    }

    /// Another section shown: the chooser is gone, and the area is back on
    /// return.
    #[gpui_kit::test]
    fn leaving_the_section_closes_its_chooser(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let (_tmp, dir) = folder();
        set_path(&s, false, &dir.join("missing.toml"), cx);
        click(&s, IMPORT_CHOOSE_ID, cx);
        chooser(&s, &rt, cx);
        s.pf_section(cx, Section::Display);
        assert!(s.pf_read(cx, |v| v.keyboard_chooser().is_none()));
        s.pf_section(cx, Section::Keyboard);
        // The area is under the list: scroll to it.
        reveal(&s, IMPORT_CHOOSE_ID, cx);
        assert!(s.pf_shows(IMPORT_CHOOSE_ID, cx));
        settle(&rt, cx);
    }

    /// `Choose folder…` is the chooser as before: folders only (hidden ones
    /// too), Enter opens one, Ctrl+Enter takes the folder shown, which goes
    /// before the file name in the Export field.
    #[gpui_kit::test]
    fn chooser_in_folder_mode_is_unchanged(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let (_tmp, dir) = folder();
        set_path(&s, true, &dir.join("mine.toml"), cx);
        click(&s, EXPORT_CHOOSE_ID, cx);
        let chooser = chooser(&s, &rt, cx);
        assert_eq!(rows(&chooser, cx), [".hidden", "sub"]);
        let choose = format!("{CHOOSER_PREFIX}-chooser-choose");
        assert!(s.pf_shows(choose, cx));
        s.pf_press("down", cx);
        s.pf_press("enter", cx);
        let chooser = super::file::chooser(&s, &rt, cx);
        assert_eq!(
            cx.read(|cx| chooser.read(cx).dir().as_path().to_path_buf()),
            dir.join("sub")
        );
        s.pf_press("ctrl-enter", cx);
        assert!(s.pf_read(cx, |v| v.keyboard_chooser().is_none()));
        assert_eq!(
            path_text(&s, true, cx),
            system::path_to_string(dir.join("sub").join("mine.toml"))
        );
        settle(&rt, cx);
    }

    /// The Export field starts as `kimun-keys.toml` in the home folder;
    /// Export writes the file for the draft, into a folder it makes, and
    /// says where.
    #[gpui_kit::test]
    fn export_writes_the_keymap_file_for_the_draft(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let home = system::browse_root()
            .unwrap()
            .join(super::super::preferences::keyboard::file::file_name());
        assert_eq!(path_text(&s, true, cx), system::path_to_string(&home));
        change(&s, &A::OpenNote, "ctrl-e", cx);
        let (_tmp, dir) = folder();
        let path = dir.join("new").join("keys.toml");
        set_path(&s, true, &path, cx);
        run(&s, &rt, EXPORT_ID, cx);
        let text = std::fs::read_to_string(&path).unwrap();
        let got = crate::keymap::file::import(&text, crate::keymap::Platform::LinuxWindows)
            .unwrap()
            .overrides;
        // Every bindable action is written, defaults too.
        assert_eq!(got["OpenNote"], vec!["primary+e".to_string()]);
        assert_eq!(got["OpenFileBrowser"], Vec::<String>::new());
        assert!(got.len() > 10, "defaults are exported too: {got:?}");
        assert!(got.values().any(|chords| !chords.is_empty()));
        assert_eq!(
            label(&s, EXPORT_NOTICE_ID, cx),
            Some(format!("Saved to {}.", system::path_to_string(&path)))
        );
        // A path that is a folder: the error, nothing else.
        set_path(&s, true, &dir, cx);
        run(&s, &rt, EXPORT_ID, cx);
        assert_eq!(
            label(&s, EXPORT_NOTICE_ID, cx),
            Some(format!(
                "This is a folder. Add a file name, such as {}.",
                super::super::preferences::keyboard::file::file_name()
            ))
        );
        // Reset all takes the result line away.
        click(&s, RESET_ALL_ID, cx);
        assert_eq!(
            label(&s, EXPORT_NOTICE_ID, cx),
            None,
            "stale after Reset all"
        );
        // A leading `~` is the home folder: `~` alone is a folder.
        set_path(&s, true, Path::new("~"), cx);
        run(&s, &rt, EXPORT_ID, cx);
        assert_eq!(
            label(&s, EXPORT_NOTICE_ID, cx),
            Some(format!(
                "This is a folder. Add a file name, such as {}.",
                super::super::preferences::keyboard::file::file_name()
            ))
        );
        // A path that is not a full path.
        set_path(&s, true, Path::new("keys.toml"), cx);
        run(&s, &rt, EXPORT_ID, cx);
        assert_eq!(
            label(&s, EXPORT_NOTICE_ID, cx).as_deref(),
            Some("Type a full path, from the top folder.")
        );
        settle(&rt, cx);
    }

    /// Import replaces the draft: the rows and the model follow the file,
    /// Save turns on, and nothing is written.
    #[gpui_kit::test]
    fn import_replaces_the_draft_and_does_not_save(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (v, s) = keyboard(cx, &rt);
        change(&s, &A::ToggleFacetView, "ctrl-g", cx);
        let before = config_bytes(&v);
        let (_tmp, dir) = folder();
        let path = dir.join("keys.toml");
        std::fs::write(
            &path,
            "[keys]\nOpenFileBrowser = []\nOpenNote = [\"primary+e\"]\n",
        )
        .unwrap();
        set_path(&s, false, &path, cx);
        run(&s, &rt, IMPORT_ID, cx);
        assert_eq!(
            overrides(&s, cx),
            [
                entry("OpenFileBrowser", &[]),
                entry("OpenNote", &["primary+e"]),
            ]
        );
        assert_eq!(row(&s, &A::OpenNote, cx), "Open note: Ctrl+E");
        assert_eq!(row(&s, &A::ToggleFacetView, cx), "Toggle facet view: none");
        assert_eq!(
            label(&s, IMPORT_NOTICE_ID, cx).as_deref(),
            Some("Loaded 2 changes.")
        );
        assert!(!s.pf_disabled(SAVE_ID, cx), "the draft is dirty");
        assert_eq!(config_bytes(&v), before, "config.toml is untouched");
        assert!(on_disk(&v).gui.key_overrides().is_empty());
        // The next edit takes the result line away.
        change(&s, &A::ToggleFacetView, "ctrl-g", cx);
        assert_eq!(label(&s, IMPORT_NOTICE_ID, cx), None, "stale after an edit");
        settle(&rt, cx);
    }

    /// A file line takes its chord from the action that holds it by
    /// default, as an edit does. Nothing is listed as dropped.
    #[gpui_kit::test]
    fn import_takes_a_chord_from_its_holder(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let (_tmp, dir) = folder();
        let path = dir.join("keys.toml");
        std::fs::write(&path, "[keys]\nToggleRail = [\"primary+o\"]\n").unwrap();
        set_path(&s, false, &path, cx);
        run(&s, &rt, IMPORT_ID, cx);
        assert_eq!(row(&s, &A::ToggleRail, cx), "Toggle rail: Ctrl+O");
        assert!(
            !row(&s, &A::OpenNote, cx).contains("Ctrl+O /"),
            "Open note lost Ctrl+O: {}",
            row(&s, &A::OpenNote, cx)
        );
        let notice = label(&s, IMPORT_NOTICE_ID, cx).unwrap();
        assert!(!notice.contains("Skipped"), "{notice}");
        settle(&rt, cx);
    }

    /// A file from the other family: the lines that equal that system's
    /// defaults are left out, and one line says how many.
    #[gpui_kit::test]
    fn import_from_another_system_counts_the_skipped_defaults(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let (_tmp, dir) = folder();
        let path = dir.join("keys.toml");
        std::fs::write(
            &path,
            "platform = \"macos\"\n[keys]\nToggleSidebar = [\"primary+alt+l\"]\n\
             OpenNote = [\"primary+alt+o\"]\n",
        )
        .unwrap();
        set_path(&s, false, &path, cx);
        run(&s, &rt, IMPORT_ID, cx);
        assert_eq!(overrides(&s, cx), [entry("OpenNote", &["primary+alt+o"])]);
        assert_eq!(
            label(&s, IMPORT_NOTICE_ID, cx).as_deref(),
            Some("Loaded 1 change. Skipped 1 line that matches the macOS defaults.")
        );
        settle(&rt, cx);
    }

    /// What the loader drops is listed under the count, one line each, and
    /// left out of the draft.
    #[gpui_kit::test]
    fn import_lists_what_it_dropped(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let (_tmp, dir) = folder();
        let path = dir.join("keys.toml");
        std::fs::write(
            &path,
            "[keys]\nNoSuchAction = [\"primary+alt+r\"]\n\
             ToggleRail = [\"primary+alt+j\", \"primary+nonsense\"]\n",
        )
        .unwrap();
        set_path(&s, false, &path, cx);
        run(&s, &rt, IMPORT_ID, cx);
        assert_eq!(overrides(&s, cx), [entry("ToggleRail", &["primary+alt+j"])]);
        assert_eq!(
            label(&s, IMPORT_NOTICE_ID, cx).as_deref(),
            Some(
                "Loaded 1 change. \
                 Skipped NoSuchAction: no such action. \
                 Skipped ToggleRail = \"primary+nonsense\": unknown key `nonsense`."
            )
        );
        settle(&rt, cx);
    }

    /// After a chooser closes the pane rests at its bottom. The import
    /// result that then grows the page shows whole, with no further scroll.
    #[gpui_kit::test]
    fn the_import_result_shows_without_a_scroll(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        let (_tmp, dir) = folder();
        let path = dir.join("keys.toml");
        std::fs::write(
            &path,
            "[keys]\nNoSuchAction = [\"primary+alt+r\"]\nOtherAction = []\n\
             ToggleRail = [\"primary+alt+j\", \"primary+nonsense\"]\n",
        )
        .unwrap();
        set_path(&s, false, &path, cx);
        click(&s, IMPORT_CHOOSE_ID, cx);
        chooser(&s, &rt, cx);
        s.pf_press("escape", cx);
        assert!(s.pf_read(cx, |v| v.keyboard_chooser().is_none()));
        // No reveal: the button is in view after the chooser.
        s.pf_click(IMPORT_ID, cx);
        drive(&rt, cx, |cx| !s.pf_read(cx, |v| v.keyboard_file_working()));
        let notice = label(&s, IMPORT_NOTICE_ID, cx).unwrap();
        assert!(notice.contains("Skipped OtherAction"), "{notice}");
        let pane = bounds(&s, CONTENT_ID, cx);
        let lines = bounds(&s, IMPORT_NOTICE_ID, cx);
        assert!(
            lines.bottom() <= pane.bottom() && lines.top() >= pane.top(),
            "the result {lines:?} is inside the pane {pane:?}"
        );
        settle(&rt, cx);
    }

    /// A file that cannot be read changes nothing and says why.
    #[gpui_kit::test]
    fn import_of_a_missing_file_shows_an_error_and_changes_nothing(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        change(&s, &A::ToggleFacetView, "ctrl-g", cx);
        let (_tmp, dir) = folder();
        set_path(&s, false, &dir.join("gone.toml"), cx);
        run(&s, &rt, IMPORT_ID, cx);
        let notice = label(&s, IMPORT_NOTICE_ID, cx).unwrap();
        assert!(notice.starts_with("Could not read the file: "), "{notice}");
        assert_eq!(
            overrides(&s, cx),
            [entry("ToggleFacetView", &["primary+g"])]
        );
        // `~/…` is read from the home folder, not refused as relative.
        set_path(
            &s,
            false,
            &Path::new("~").join("kimun-no-such-keymap-6f1e.toml"),
            cx,
        );
        run(&s, &rt, IMPORT_ID, cx);
        let notice = label(&s, IMPORT_NOTICE_ID, cx).unwrap();
        assert!(notice.starts_with("Could not read the file: "), "{notice}");
        // Not a keymap file: the same.
        set_path(&s, false, &dir.join("notes.txt"), cx);
        run(&s, &rt, IMPORT_ID, cx);
        let notice = label(&s, IMPORT_NOTICE_ID, cx).unwrap();
        assert!(notice.starts_with("This is not a keymap file"), "{notice}");
        assert_eq!(
            overrides(&s, cx),
            [entry("ToggleFacetView", &["primary+g"])]
        );
        settle(&rt, cx);
    }

    /// An empty `[keys]` table is a keymap with no changes: every row goes
    /// back to its default.
    #[gpui_kit::test]
    fn import_of_an_empty_keys_table_resets_the_draft(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = keyboard(cx, &rt);
        change(&s, &A::OpenNote, "ctrl-e", cx);
        let (_tmp, dir) = folder();
        set_path(&s, false, &dir.join("keys.toml"), cx);
        run(&s, &rt, IMPORT_ID, cx);
        assert!(overrides(&s, cx).is_empty());
        assert_eq!(
            row(&s, &A::OpenFileBrowser, cx),
            "Open files: Ctrl+E / Ctrl+Shift+E"
        );
        assert_eq!(
            label(&s, IMPORT_NOTICE_ID, cx).as_deref(),
            Some("Loaded 0 changes.")
        );
        assert!(s.pf_disabled(SAVE_ID, cx), "back to what is saved");
        settle(&rt, cx);
    }

    /// A `[gui.keys]` line for an action this build does not know survives
    /// an edit and Save.
    #[gpui_kit::test]
    fn an_unknown_action_in_the_config_survives_an_edit_and_save(cx: &mut TestAppContext) {
        use kimun_notes::settings::AppSettings;
        let rt = runtime();
        let _guard = rt.enter();
        let v = vault(&rt);
        let mut keys = BTreeMap::new();
        keys.insert(
            "FromANewerBuild".to_string(),
            vec!["primary+alt+j".to_string()],
        );
        AppSettings::update_gui_at(config(&v), |gui| gui.set_key_overrides(&keys)).unwrap();
        v.settings.write().unwrap().gui.set_key_overrides(&keys);
        let s = on_vault(cx, &rt, &v);
        s.pf_route(cx);
        s.pf_loaded(&rt, cx);
        s.pf_section(cx, Section::Keyboard);
        change(&s, &A::ToggleFacetView, "ctrl-g", cx);
        s.pf_save(&rt, cx);
        let saved = on_disk(&v).gui.key_overrides();
        assert_eq!(saved.get("FromANewerBuild"), keys.get("FromANewerBuild"));
        assert_eq!(
            saved.get("ToggleFacetView"),
            Some(&vec!["primary+g".to_string()])
        );
        settle(&rt, cx);
    }
}
