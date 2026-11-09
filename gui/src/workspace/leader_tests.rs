//! UI integration tests for the leader and the which-key (M6 G2): a real
//! `Workspace` in a headless window, sequences typed through GPUI's key
//! dispatch (so the workspace's keystroke interceptor sees them as it does
//! in the app), the which-key's wait driven by the test clock.
#![cfg(test)]

use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::layout::{Panel, Side};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::settings::gui::GuiSettings;

use super::leader::{DEFAULT_TIMEOUT, WHICH_KEY_ID};
use super::palette::{Palette, PaletteRun};
use super::route::{NEEDS_NOTE_FOCUS, NO_WORKSPACE};
use super::start_tests::chords;
use super::start_tests::{drive, open_started, runtime, vault_start};
use super::tests::{Shell, open_default, open_shell};
use super::{LayoutStore, Workspace};
use crate::startup::FirstNote;
use crate::test_vault::test_vault;

const CHORD: &str = "ctrl-shift-b";

fn press(shell: &Shell, key: &str, cx: &mut TestAppContext) {
    cx.update_window(shell.handle, |_, window, cx| window.press(key, cx))
        .unwrap();
    cx.run_until_parked();
}

fn type_text(shell: &Shell, text: &str, cx: &mut TestAppContext) {
    cx.update_window(shell.handle, |_, window, cx| window.input(text, cx))
        .unwrap();
    cx.run_until_parked();
}

fn pending(shell: &Shell, cx: &TestAppContext) -> bool {
    cx.read(|cx| shell.workspace.read(cx).leader.is_pending())
}

fn revealed(shell: &Shell, cx: &TestAppContext) -> bool {
    cx.read(|cx| shell.workspace.read(cx).leader.revealed())
}

/// The which-key's header (the pressed keys) and row count, while it shows.
fn which_key(shell: &Shell, cx: &TestAppContext) -> Option<(String, usize)> {
    cx.read(|cx| {
        shell
            .workspace
            .read(cx)
            .leader
            .which_key()
            .map(|m| (m.pressed().to_string(), m.row_count()))
    })
}

fn which_key_drawn(shell: &Shell, cx: &mut TestAppContext) -> bool {
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(WHICH_KEY_ID).is_some_and(|e| e.visible())
    })
    .unwrap()
}

fn flash(shell: &Shell, cx: &TestAppContext) -> Option<String> {
    cx.read(|cx| {
        shell
            .workspace
            .read(cx)
            .last_flash()
            .map(|(text, _)| text.to_string())
    })
}

fn wait(cx: &mut TestAppContext, ms: u64) {
    cx.executor().advance_clock(Duration::from_millis(ms));
    cx.run_until_parked();
}

fn dialog_open(shell: &Shell, cx: &mut TestAppContext) -> bool {
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.has_active_dialog(cx)
    })
    .unwrap()
}

fn editor_focused(shell: &Shell, cx: &mut TestAppContext) -> bool {
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.find("editor").focused() == Some(true)
    })
    .unwrap()
}

fn vim_shell(cx: &mut TestAppContext, text: &str) -> Shell {
    open_shell(
        cx,
        1000.,
        &GuiSettings::default(),
        text,
        true,
        LayoutStore::Off,
    )
}

/// The palette's view, while it is open.
fn palette(workspace: &Entity<Workspace>, cx: &gpui_kit::App) -> Option<Entity<Palette>> {
    workspace.read(cx).palette.as_ref()?.upgrade()
}

/// The leader sequence the palette shows for `action`'s row.
fn sequence_of(shell: &Shell, action: LeaderAction, cx: &TestAppContext) -> Option<String> {
    cx.read(|cx| {
        palette(&shell.workspace, cx)?
            .read(cx)
            .entries()
            .iter()
            .find(|e| *e.run() == PaletteRun::Leaf(action))
            .and_then(|e| e.sequence().map(str::to_string))
    })
}

/// The chord arms a sequence and `p` opens the command palette; nothing
/// is typed into the note.
#[gpui_kit::test]
fn the_chord_then_p_opens_the_palette(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    press(&shell, CHORD, cx);
    assert!(pending(&shell, cx));
    type_text(&shell, "p", cx);
    assert!(!pending(&shell, cx));
    assert!(dialog_open(&shell, cx));
    assert_eq!(cx.read(|cx| shell.text(cx)), "hello");
}

/// In vim Normal mode a bare Space arms the leader (it types nothing); in
/// Insert mode it types a space.
#[gpui_kit::test]
fn space_leads_in_vim_normal_and_types_in_insert(cx: &mut TestAppContext) {
    let shell = vim_shell(cx, "abc");
    type_text(&shell, " ", cx);
    assert!(pending(&shell, cx), "Normal-mode Space arms the leader");
    press(&shell, "escape", cx);
    assert!(!pending(&shell, cx));
    assert_eq!(cx.read(|cx| shell.text(cx)), "abc");
    type_text(&shell, "i ", cx);
    assert!(!pending(&shell, cx), "Insert-mode Space is a space");
    assert_eq!(cx.read(|cx| shell.text(cx)), " abc");
}

/// Space is never a leader in the plain editor, nor on the rail (where it
/// activates the cell under the cursor), even under vim.
#[gpui_kit::test]
fn space_does_not_lead_in_plain_mode_or_off_the_editor(cx: &mut TestAppContext) {
    let shell = open_default(cx, "abc");
    type_text(&shell, " ", cx);
    assert!(!pending(&shell, cx));
    assert_eq!(cx.read(|cx| shell.text(cx)), " abc");

    let shell = vim_shell(cx, "abc");
    press(&shell, "alt-left", cx);
    press(&shell, "alt-left", cx);
    assert_eq!(cx.read(|cx| shell.focused(cx)), Panel::Rail);
    press(&shell, "space", cx);
    assert!(!pending(&shell, cx));
}

/// Typed fluently, a sequence never shows the which-key; once it stalls for
/// the timeout it shows, docked over the note; each step hides it again,
/// and it goes when the sequence fires.
#[gpui_kit::test]
fn the_which_key_shows_after_a_hesitation(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    let timeout = DEFAULT_TIMEOUT.as_millis() as u64;
    press(&shell, CHORD, cx);
    wait(cx, timeout - 50);
    assert!(!revealed(&shell, cx), "fluent: hidden");
    assert!(!which_key_drawn(&shell, cx));
    type_text(&shell, "w", cx);
    wait(cx, timeout - 50);
    assert!(!revealed(&shell, cx), "each step restarts the wait");
    wait(cx, 60);
    assert!(revealed(&shell, cx));
    assert!(which_key_drawn(&shell, cx));
    let (pressed, rows) = which_key(&shell, cx).unwrap();
    assert_eq!(pressed, chords("Ctrl+Shift+B w"));
    assert_eq!(rows, 4, "the +window group's leaves");
    // `z` (zen) fires: the popup goes, and the slot with it.
    type_text(&shell, "z", cx);
    assert!(!pending(&shell, cx));
    assert!(!revealed(&shell, cx));
    assert!(!which_key_drawn(&shell, cx));
    assert!(!cx.read(|cx| shell.left_side_shown(cx)));
    assert_eq!(cx.read(|cx| shell.text(cx)), "hello");
}

/// Escape cancels and gives the note the keyboard, from wherever the
/// sequence started (the rail here).
#[gpui_kit::test]
fn escape_cancels_and_focuses_the_note(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    press(&shell, "alt-left", cx);
    press(&shell, "alt-left", cx);
    assert_eq!(cx.read(|cx| shell.focused(cx)), Panel::Rail);
    press(&shell, CHORD, cx);
    assert!(pending(&shell, cx));
    press(&shell, "escape", cx);
    assert!(!pending(&shell, cx));
    assert_eq!(cx.read(|cx| shell.focused(cx)), Panel::Editor);
    assert!(editor_focused(&shell, cx));
}

/// Backspace steps up a level; at the root it cancels.
#[gpui_kit::test]
fn backspace_steps_up_then_cancels(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    press(&shell, CHORD, cx);
    type_text(&shell, "f", cx);
    wait(cx, DEFAULT_TIMEOUT.as_millis() as u64);
    assert_eq!(
        which_key(&shell, cx).map(|w| w.0).as_deref(),
        Some(chords("Ctrl+Shift+B f").as_str())
    );
    press(&shell, "backspace", cx);
    assert!(pending(&shell, cx));
    assert!(!revealed(&shell, cx), "a step hides the which-key");
    wait(cx, DEFAULT_TIMEOUT.as_millis() as u64);
    assert_eq!(
        which_key(&shell, cx).map(|w| w.0).as_deref(),
        Some("Ctrl+Shift+B")
    );
    press(&shell, "backspace", cx);
    assert!(!pending(&shell, cx));
    assert_eq!(cx.read(|cx| shell.text(cx)), "hello");
}

/// A key the tree does not have says so and the sequence stays pending;
/// digits and letters of a sequence never reach the note.
#[gpui_kit::test]
fn an_invalid_key_flashes_and_stays_pending(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    press(&shell, CHORD, cx);
    type_text(&shell, "x", cx);
    assert!(pending(&shell, cx));
    assert_eq!(
        flash(&shell, cx).as_deref(),
        Some("leader: no entry for 'x'")
    );
    type_text(&shell, "f5", cx);
    assert!(pending(&shell, cx));
    assert_eq!(
        flash(&shell, cx).as_deref(),
        Some("leader: no entry for '5'")
    );
    press(&shell, "escape", cx);
    // A root digit is a pinned jump: no workspace here, so it says why.
    press(&shell, CHORD, cx);
    type_text(&shell, "3", cx);
    assert!(!pending(&shell, cx));
    assert_eq!(flash(&shell, cx).as_deref(), Some(NO_WORKSPACE));
    assert_eq!(cx.read(|cx| shell.text(cx)), "hello");
}

/// The chord while a sequence is pending cancels it (a toggle).
#[gpui_kit::test]
fn the_chord_toggles_the_sequence(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    press(&shell, CHORD, cx);
    type_text(&shell, "n", cx);
    press(&shell, CHORD, cx);
    assert!(!pending(&shell, cx));
    type_text(&shell, "w", cx);
    assert_eq!(cx.read(|cx| shell.text(cx)), "whello", "keys type again");
}

/// No sequence while a dialog is open: the chord does nothing under the
/// palette.
#[gpui_kit::test]
fn the_leader_is_refused_under_a_dialog(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    press(&shell, "ctrl-shift-p", cx);
    assert!(dialog_open(&shell, cx));
    press(&shell, CHORD, cx);
    assert!(!pending(&shell, cx));
    assert!(dialog_open(&shell, cx));
}

/// `t b` bolds the selection from the note, and does nothing to it while
/// the find bar has the keyboard (the leaf says why; the find field gets
/// none of the keys either).
#[gpui_kit::test]
fn text_leaves_yield_to_the_find_bar(cx: &mut TestAppContext) {
    let shell = open_default(cx, "word");
    press(&shell, "shift-end", cx);
    press(&shell, CHORD, cx);
    type_text(&shell, "tb", cx);
    assert_eq!(cx.read(|cx| shell.text(cx)), "**word**");

    press(&shell, "ctrl-f", cx);
    let find_open =
        |cx: &TestAppContext| cx.read(|cx| shell.editor(cx).read(cx).editor.find_open());
    assert!(find_open(cx));
    let query = |cx: &TestAppContext| {
        cx.read(|cx| {
            shell
                .editor(cx)
                .read(cx)
                .editor
                .find_bar()
                .map(|bar| bar.query().to_string())
        })
    };
    let before = query(cx);
    press(&shell, CHORD, cx);
    assert!(pending(&shell, cx));
    type_text(&shell, "tb", cx);
    assert!(!pending(&shell, cx));
    assert_eq!(flash(&shell, cx).as_deref(), Some(NEEDS_NOTE_FOCUS));
    assert_eq!(cx.read(|cx| shell.text(cx)), "**word**");
    assert!(find_open(cx));
    assert_eq!(query(cx), before, "the bar's field took none of the keys");
}

/// A leaf that opens a drawer view opens it and gives it the keyboard.
#[gpui_kit::test]
fn a_drawer_leaf_shows_its_view(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    press(&shell, CHORD, cx);
    type_text(&shell, "ot", cx);
    assert_eq!(cx.read(|cx| shell.left_slot_view(cx)), DrawerView::Tags);
    assert_eq!(cx.read(|cx| shell.focused(cx)), Panel::Drawer(Side::Left));
}

/// The palette's leaf rows show their leader sequence from the gateway
/// that works where the keyboard was: the chord in the plain editor, Space
/// in vim Normal.
#[gpui_kit::test]
fn palette_rows_show_the_leader_sequence(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    press(&shell, "ctrl-shift-p", cx);
    assert_eq!(
        sequence_of(&shell, LeaderAction::NoteSave, cx).as_deref(),
        Some(chords("Ctrl+Shift+B n w").as_str())
    );
    assert_eq!(
        sequence_of(&shell, LeaderAction::TextBold, cx).as_deref(),
        Some("Ctrl+Shift+B t b")
    );

    let shell = vim_shell(cx, "abc");
    type_text(&shell, ":", cx);
    assert!(dialog_open(&shell, cx));
    assert_eq!(
        sequence_of(&shell, LeaderAction::NoteSave, cx).as_deref(),
        Some("Space n w")
    );
}

// ── with a vault ─────────────────────────────────────────────────────────

/// Ctrl+S in the middle of a sequence cancels it and saves, as it would
/// without one.
#[gpui_kit::test]
fn ctrl_s_mid_sequence_cancels_and_saves(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("a.md", "alpha")], &[]));
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(VaultPath::new("a.md"))),
        v.settings.clone(),
    );
    drive(&rt, cx, |cx| started.text(cx) == "alpha");
    let shell = Shell {
        handle: started.handle,
        workspace: started.workspace.clone(),
    };
    type_text(&shell, "X", cx);
    let dirty = |cx: &TestAppContext| {
        cx.read(|cx| started.workspace.read(cx).editor.read(cx).editor.is_dirty())
    };
    assert!(dirty(cx));
    press(&shell, CHORD, cx);
    type_text(&shell, "n", cx);
    press(&shell, "ctrl-s", cx);
    assert!(!pending(&shell, cx));
    drive(&rt, cx, |cx| !dirty(cx));
    assert_eq!(
        rt.block_on(v.vault.get_note_text(&VaultPath::new("a.md")))
            .unwrap(),
        "Xalpha"
    );
}

/// The shell tests' hesitation is the settings' own default.
#[test]
fn the_default_timeout_is_the_settings_default() {
    assert_eq!(
        DEFAULT_TIMEOUT,
        Duration::from_millis(kimun_notes::settings::AppSettings::default().leader_timeout_ms)
    );
}
