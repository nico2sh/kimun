//! UI integration tests for the modal host and the command palette (M6 G1):
//! a real `Workspace` in a headless window, the palette opened by its chord
//! or vim `:`, filtered, run and dismissed through the keyboard.
#![cfg(test)]

use std::time::Duration;

use gpui_kit::component::IndexPath;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::layout::{Panel, Side};
use kimun_notes::logic::message::AppMessage;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::settings::gui::GuiSettings;

use super::modal::ModalKind;
use super::palette::{Palette, PaletteRun};
use super::route::{NEEDS_NOTE_FOCUS, NEEDS_SLOT_FOCUS, NO_NOTE, NO_WORKSPACE};
use super::start_tests::{drive, open_started, runtime, send, vault_start};
use super::tests::{Shell, open_default, open_shell};
use super::{LayoutStore, Workspace, rail_cell_id};
use crate::startup::FirstNote;
use crate::test_vault::test_vault;

const SAVE: &str = "+note · write (save now)";

/// The palette's view, while it is open.
fn palette(workspace: &Entity<Workspace>, cx: &gpui_kit::App) -> Option<Entity<Palette>> {
    workspace.read(cx).palette.as_ref()?.upgrade()
}

/// Whether the kit's dialog layer shows a dialog (the palette).
fn dialog_open(shell: &Shell, cx: &mut TestAppContext) -> bool {
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.has_active_dialog(cx)
    })
    .unwrap()
}

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

/// Let a closing dialog finish leaving: the kit restores the opener's
/// focus once its exit transition ends.
fn settle(shell: &Shell, cx: &mut TestAppContext) {
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    cx.update_window(shell.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

/// The labels the palette shows, best first.
fn visible(shell: &Shell, cx: &mut TestAppContext) -> Vec<String> {
    cx.read(|cx| {
        palette(&shell.workspace, cx)
            .map(|p| {
                p.read(cx)
                    .visible()
                    .map(|e| e.label().to_string())
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn query_focused(shell: &Shell, cx: &mut TestAppContext) -> bool {
    let workspace = shell.workspace.clone();
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        palette(&workspace, cx).is_some_and(|p| p.read(cx).query_focused(window, cx))
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

fn open_palette(shell: &Shell, cx: &mut TestAppContext) {
    press(shell, "ctrl-shift-p", cx);
    assert!(dialog_open(shell, cx), "the chord opens the palette");
}

/// The layout-tree commands are palette rows; the drawer-era ones are
/// gone.
#[gpui_kit::test]
fn the_palette_lists_the_layout_tree_commands(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    open_palette(&shell, cx);
    let rows = visible(&shell, cx);
    for row in [
        "+window · split slot right (pick a view)",
        "+window · split slot down (pick a view)",
        "+window · close slot",
        "+window · move view left (new slot if none)",
        "+window · move view right (new slot if none)",
        "+window · move view up (new slot if none)",
        "+window · move view down (new slot if none)",
        "+window · toggle zoom",
    ] {
        assert_eq!(
            rows.iter().filter(|r| *r == row).count(),
            1,
            "{row}: {rows:?}"
        );
    }
    for gone in [
        "+window · move view to left slot",
        "+window · move view to right slot",
        "+window · split slot",
    ] {
        assert!(!rows.iter().any(|r| r == gone), "{gone}");
    }
}

/// The palette's zoom row zooms the editor that opened it, and leaves zoom
/// again.
#[gpui_kit::test]
fn the_palette_toggles_zoom(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    for zoomed in [true, false] {
        open_palette(&shell, cx);
        type_text(&shell, "toggle zoom", cx);
        assert_eq!(
            visible(&shell, cx).first().map(String::as_str),
            Some("+window · toggle zoom")
        );
        press(&shell, "enter", cx);
        settle(&shell, cx);
        let now = cx.read(|cx| shell.workspace.read(cx).layout.zoomed());
        assert_eq!(now.is_some(), zoomed);
    }
}

/// The chord opens the palette over the editor with its query field
/// focused; every leader leaf and the GUI's own rows are listed.
#[gpui_kit::test]
fn the_chord_opens_the_palette_with_the_query_focused(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    open_palette(&shell, cx);
    assert!(query_focused(&shell, cx));
    assert_eq!(
        cx.read(|cx| shell.workspace.read(cx).modal),
        Some(ModalKind::Palette)
    );
    let rows = visible(&shell, cx);
    assert!(rows.iter().any(|r| r == SAVE), "{rows:?}");
    assert!(rows.iter().any(|r| r == "+window · toggle right side"));
    assert!(
        !rows.iter().any(|r| r == "command palette"),
        "the palette never lists itself"
    );
    // The kit names each row after its label.
    cx.update_window(shell.handle, |_, window, _| {
        assert_eq!(
            window.find(IndexPath::new(0)).label(),
            Some(rows[0].as_str())
        );
    })
    .unwrap();
}

/// Typing filters fuzzily (gaps allowed), best match first.
#[gpui_kit::test]
fn typing_filters_fuzzily(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    open_palette(&shell, cx);
    type_text(&shell, "wrsave", cx);
    let rows = visible(&shell, cx);
    assert_eq!(rows.first().map(String::as_str), Some(SAVE), "{rows:?}");
    type_text(&shell, "qqqzz", cx);
    assert!(visible(&shell, cx).is_empty());
}

/// Escape clears a typed query first, then closes the palette, and the
/// keyboard goes back to the editor that opened it.
#[gpui_kit::test]
fn escape_clears_then_closes_and_focus_returns(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    assert!(editor_focused(&shell, cx));
    open_palette(&shell, cx);
    type_text(&shell, "zen", cx);
    press(&shell, "escape", cx);
    assert!(dialog_open(&shell, cx), "the first Escape only clears");
    let query = cx.read(|cx| palette(&shell.workspace, cx).map(|p| p.read(cx).query(cx)));
    assert_eq!(query.as_deref(), Some(""));
    press(&shell, "escape", cx);
    settle(&shell, cx);
    assert!(!dialog_open(&shell, cx));
    assert!(editor_focused(&shell, cx));
    assert_eq!(cx.read(|cx| shell.focused(cx)), Panel::Editor);
}

/// The chord over an open palette closes it (the TUI's toggle).
#[gpui_kit::test]
fn the_chord_toggles_the_palette_closed(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    open_palette(&shell, cx);
    press(&shell, "ctrl-shift-p", cx);
    settle(&shell, cx);
    assert!(!dialog_open(&shell, cx));
    assert!(editor_focused(&shell, cx));
}

/// Vim `:` opens the palette, and the chord works under vim too.
#[gpui_kit::test]
fn vim_colon_opens_the_palette(cx: &mut TestAppContext) {
    let shell = open_shell(
        cx,
        1000.,
        &GuiSettings::default(),
        "abc",
        true,
        LayoutStore::Off,
    );
    type_text(&shell, ":", cx);
    assert!(dialog_open(&shell, cx));
    assert!(query_focused(&shell, cx));
    assert_eq!(cx.read(|cx| shell.text(cx)), "abc", "`:` typed nothing");
    press(&shell, "escape", cx);
    settle(&shell, cx);
    assert!(!dialog_open(&shell, cx));
    assert!(editor_focused(&shell, cx));
    open_palette(&shell, cx);
}

/// A row the window cannot run is disabled and says why; Enter on it does
/// nothing and the palette stays open.
#[gpui_kit::test]
fn a_disabled_row_cannot_run(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    open_palette(&shell, cx);
    let blocked = cx.read(|cx| {
        let palette = palette(&shell.workspace, cx).unwrap();
        let palette = palette.read(cx);
        let reason = |action: LeaderAction| {
            palette
                .entries()
                .iter()
                .find(|e| *e.run() == PaletteRun::Leaf(action))
                .and_then(|e| e.blocked())
        };
        (
            reason(LeaderAction::NoteDaily),
            reason(LeaderAction::NoteRename),
            reason(LeaderAction::TextBold),
            reason(LeaderAction::NoteSave),
        )
    });
    // There is no workspace (no note session) in this shell.
    assert_eq!(
        blocked,
        (Some(NO_WORKSPACE), Some(NO_NOTE), None, Some(NO_NOTE))
    );
    type_text(&shell, "daily", cx);
    assert_eq!(
        visible(&shell, cx).first().map(String::as_str),
        Some("+note · daily")
    );
    press(&shell, "enter", cx);
    assert!(dialog_open(&shell, cx), "a disabled row does not run");
    cx.update_window(shell.handle, |_, window, _| {
        assert_eq!(window.find(IndexPath::new(0)).selected(), Some(false));
    })
    .unwrap();
}

/// A click on a disabled row is inert (C1): the palette stays.
#[gpui_kit::test]
fn a_click_on_a_disabled_row_is_inert(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    open_palette(&shell, cx);
    // No workspace in this shell: the daily note is disabled.
    type_text(&shell, "daily", cx);
    assert_eq!(
        visible(&shell, cx).first().map(String::as_str),
        Some("+note · daily")
    );
    let click_first = |cx: &mut TestAppContext| {
        cx.update_window(shell.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(IndexPath::new(0), cx)
        })
        .unwrap();
        cx.run_until_parked();
    };
    click_first(cx);
    assert!(dialog_open(&shell, cx), "a disabled row does not run");
}

/// A click on a runnable row runs it at once: the right side's toggle.
#[gpui_kit::test]
fn a_click_runs_a_runnable_row(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    let right =
        |cx: &mut TestAppContext| cx.read(|cx| shell.workspace.read(cx).side_drawn(Side::Right));
    assert!(!right(cx));
    open_palette(&shell, cx);
    type_text(&shell, "toggle right side", cx);
    assert_eq!(
        visible(&shell, cx).first().map(String::as_str),
        Some("+window · toggle right side")
    );
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(IndexPath::new(0), cx)
    })
    .unwrap();
    cx.run_until_parked();
    settle(&shell, cx);
    // Nothing placed on the right: the toggle asks which view to show.
    assert!(!right(cx));
    press(&shell, "enter", cx);
    settle(&shell, cx);
    assert!(!dialog_open(&shell, cx));
    assert!(right(cx));
}

/// A text action needs the note to have had the keyboard: from the rail its
/// row is disabled, from the editor it formats the selection.
#[gpui_kit::test]
fn formatting_runs_on_the_note_that_had_the_keyboard(cx: &mut TestAppContext) {
    let shell = open_default(cx, "word");
    press(&shell, "shift-end", cx);
    open_palette(&shell, cx);
    type_text(&shell, "text bold", cx);
    assert_eq!(
        visible(&shell, cx).first().map(String::as_str),
        Some("+text · bold")
    );
    press(&shell, "enter", cx);
    settle(&shell, cx);
    assert!(!dialog_open(&shell, cx));
    assert_eq!(cx.read(|cx| shell.text(cx)), "**word**");
    assert!(editor_focused(&shell, cx));

    // From the rail: disabled, with the reason.
    press(&shell, "alt-left", cx);
    assert_eq!(cx.read(|cx| shell.focused(cx)), Panel::Drawer(Side::Left));
    press(&shell, "alt-left", cx);
    assert_eq!(cx.read(|cx| shell.focused(cx)), Panel::Rail);
    open_palette(&shell, cx);
    let reason = cx.read(|cx| {
        let palette = palette(&shell.workspace, cx).unwrap();
        palette
            .read(cx)
            .entries()
            .iter()
            .find(|e| *e.run() == PaletteRun::Leaf(LeaderAction::TextBold))
            .and_then(|e| e.blocked())
    });
    assert_eq!(reason, Some(NEEDS_NOTE_FOCUS));
    press(&shell, "escape", cx);
    settle(&shell, cx);
    assert_eq!(cx.read(|cx| shell.focused(cx)), Panel::Rail);
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("rail").focused(),
            Some(true),
            "focus is back on the rail"
        );
    })
    .unwrap();
}

/// A GUI-only row runs the chord's command in the opener's context: the
/// right side's toggle.
#[gpui_kit::test]
fn a_gui_row_runs_its_chord(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    let right =
        |cx: &mut TestAppContext| cx.read(|cx| shell.workspace.read(cx).side_drawn(Side::Right));
    assert!(!right(cx));
    open_palette(&shell, cx);
    type_text(&shell, "toggle right side", cx);
    press(&shell, "enter", cx);
    settle(&shell, cx);
    // Nothing placed on the right: the toggle asks which view to show.
    assert!(!right(cx));
    press(&shell, "enter", cx);
    settle(&shell, cx);
    assert!(!dialog_open(&shell, cx));
    assert!(right(cx));
}

/// A leaf that shows a drawer view shows it (never toggles it away) and
/// gives it the keyboard.
#[gpui_kit::test]
fn a_drawer_leaf_shows_its_view(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    open_palette(&shell, cx);
    type_text(&shell, "open drawer tags", cx);
    press(&shell, "enter", cx);
    settle(&shell, cx);
    assert_eq!(cx.read(|cx| shell.left_slot_view(cx)), DrawerView::Tags);
    assert_eq!(cx.read(|cx| shell.focused(cx)), Panel::Drawer(Side::Left));
    cx.update_window(shell.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find(super::slot_id(DrawerView::Tags)).focused(),
            Some(true)
        );
        assert!(window.find(rail_cell_id(DrawerView::Tags)).visible());
    })
    .unwrap();
}

/// While a modal is open nothing else takes the keyboard: a focus the
/// window would apply (an async open landing) waits for it to close.
#[gpui_kit::test]
fn async_focus_does_not_steal_from_the_palette(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    open_palette(&shell, cx);
    let workspace = shell.workspace.clone();
    cx.update_window(shell.handle, |_, window, cx| {
        workspace.update(cx, |w, cx| {
            w.set_focus_panel(Panel::Rail);
            w.apply_focus(window, cx);
        });
    })
    .unwrap();
    assert!(query_focused(&shell, cx), "the palette keeps the keyboard");
    // Another modal is refused while one is open.
    let workspace = shell.workspace.clone();
    cx.update_window(shell.handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_palette(window, cx));
    })
    .unwrap();
    assert!(query_focused(&shell, cx));
}

/// The palette's quit ends the app (nothing to save in this shell).
#[gpui_kit::test]
fn quit_ends_the_app(cx: &mut TestAppContext) {
    let shell = open_default(cx, "hello");
    open_palette(&shell, cx);
    type_text(&shell, "quit", cx);
    assert_eq!(
        visible(&shell, cx).first().map(String::as_str),
        Some("quit kimün")
    );
    press(&shell, "enter", cx);
    assert_eq!(cx.read(|cx| shell.workspace.read(cx).quits), 1);
}

// ── with a vault ─────────────────────────────────────────────────────────

/// "write (save now)" from the palette saves the open note through the
/// session, as the Save chord does, and the keyboard is back in the editor.
#[gpui_kit::test]
fn the_save_row_writes_the_open_note(cx: &mut TestAppContext) {
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
    open_palette(&shell, cx);
    type_text(&shell, "write save", cx);
    assert_eq!(visible(&shell, cx).first().map(String::as_str), Some(SAVE));
    press(&shell, "enter", cx);
    drive(&rt, cx, |cx| !dirty(cx));
    assert_eq!(
        rt.block_on(v.vault.get_note_text(&VaultPath::new("a.md")))
            .unwrap(),
        "Xalpha"
    );
    settle(&shell, cx);
    assert!(editor_focused(&shell, cx));
}

/// A leader leaf asked for on the app channel runs with the window (the
/// channel's dispatch has none of its own); under an open modal it is
/// dropped.
#[gpui_kit::test]
fn a_leaf_sent_on_the_channel_runs_unless_a_modal_is_open(cx: &mut TestAppContext) {
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
    open_palette(&shell, cx);
    send(
        &started,
        AppMessage::ExecuteLeaderAction(LeaderAction::OpenDrawer(DrawerView::Tags)),
        cx,
    );
    cx.run_until_parked();
    assert_ne!(
        cx.read(|cx| shell.left_slot_view(cx)),
        DrawerView::Tags,
        "dropped under the palette"
    );
    press(&shell, "escape", cx);
    settle(&shell, cx);
    send(
        &started,
        AppMessage::ExecuteLeaderAction(LeaderAction::OpenDrawer(DrawerView::Tags)),
        cx,
    );
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| shell.left_slot_view(cx)), DrawerView::Tags);
    assert_eq!(cx.read(|cx| shell.focused(cx)), Panel::Drawer(Side::Left));
}

/// A file operation landing under an open palette closes it.
#[gpui_kit::test]
fn a_file_op_closes_the_palette(cx: &mut TestAppContext) {
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
    open_palette(&shell, cx);
    send(
        &started,
        AppMessage::FileOp(kimun_notes::logic::message::FileOp::Created(
            VaultPath::new("b.md"),
        )),
        cx,
    );
    cx.run_until_parked();
    settle(&shell, cx);
    assert!(!dialog_open(&shell, cx));
    assert_eq!(cx.read(|cx| started.workspace.read(cx).modal), None);
}

/// "yank note path" copies the open note's path to the clipboard and says
/// so in a flash.
#[gpui_kit::test]
fn the_yank_row_copies_the_note_path(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("ideas/a.md", "alpha")], &[]));
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(VaultPath::new("ideas/a.md"))),
        v.settings.clone(),
    );
    drive(&rt, cx, |cx| started.text(cx) == "alpha");
    let shell = Shell {
        handle: started.handle,
        workspace: started.workspace.clone(),
    };
    open_palette(&shell, cx);
    type_text(&shell, "yank note path", cx);
    press(&shell, "enter", cx);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    let note = cx
        .read(|cx| started.workspace.read(cx).status.note.clone())
        .unwrap();
    assert_eq!(copied, Some(note.to_string()));
    assert_eq!(started.flash(cx).as_deref(), Some("note path copied"));
}

/// `+window · move view to…` opens the move chooser for the slot that had
/// the keyboard (FILES, over FIND); from the editor it says a slot needs
/// the keyboard.
#[gpui_kit::test]
fn the_palette_row_opens_the_move_chooser(cx: &mut TestAppContext) {
    const ROW: &str = "+window · move view to…";
    let settings = super::tests::gui(
        "[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, split = \"vertical\", children = [\n\
           { weight = 0.5, views = [\"files\"], shown = \"files\" },\n\
           { weight = 0.5, views = [\"find\"], shown = \"find\" },\n] },\n\
         { weight = 0.8, editor = true },\n]\n",
    );
    let shell = open_shell(cx, 1000., &settings, "", false, LayoutStore::Off);
    let blocked = |shell: &Shell, cx: &mut TestAppContext| {
        cx.read(|cx| {
            palette(&shell.workspace, cx)
                .unwrap()
                .read(cx)
                .entries()
                .iter()
                .find(|e| e.label() == ROW)
                .map(|e| e.blocked())
        })
    };
    open_palette(&shell, cx);
    assert_eq!(blocked(&shell, cx), Some(Some(NEEDS_SLOT_FOCUS)));
    press(&shell, "escape", cx);
    settle(&shell, cx);

    cx.update_window(shell.handle, |_, window, cx| {
        window.click(super::slot_id(DrawerView::Files), cx)
    })
    .unwrap();
    open_palette(&shell, cx);
    assert_eq!(blocked(&shell, cx), Some(None));
    type_text(&shell, "move view to", cx);
    assert_eq!(visible(&shell, cx).first().map(String::as_str), Some(ROW));
    press(&shell, "enter", cx);
    settle(&shell, cx);
    assert!(dialog_open(&shell, cx), "the chooser is open");
    cx.read(|cx| {
        let workspace = shell.workspace.read(cx);
        assert_eq!(workspace.modal, Some(ModalKind::ViewChooser));
        assert_eq!(
            workspace.move_chooser_rows(cx),
            Some(vec!["Into Find slot".to_string()])
        );
    });
}

/// The highlight follows the query: after the arrows moved it down the
/// list, a longer query leaves several rows and Enter runs the first
/// runnable one, not whatever row now sits where the highlight was.
#[gpui_kit::test]
fn enter_runs_the_first_row_after_the_query_changes(cx: &mut TestAppContext) {
    let settings = super::tests::gui(
        "[gui.layout]\nsplit = \"horizontal\"\nchildren = [\n\
         { weight = 0.2, split = \"vertical\", children = [\n\
           { weight = 0.5, views = [\"files\"], shown = \"files\" },\n\
           { weight = 0.5, views = [\"find\"], shown = \"find\" },\n] },\n\
         { weight = 0.8, editor = true },\n]\n",
    );
    let shell = open_shell(cx, 1000., &settings, "", false, LayoutStore::Off);
    cx.update_window(shell.handle, |_, window, cx| {
        window.click(super::slot_id(DrawerView::Files), cx)
    })
    .unwrap();
    open_palette(&shell, cx);
    type_text(&shell, "move view", cx);
    assert!(visible(&shell, cx).len() > 3, "{:?}", visible(&shell, cx));
    for _ in 0..3 {
        press(&shell, "down", cx);
    }
    type_text(&shell, " to", cx);
    let rows = visible(&shell, cx);
    assert!(rows.len() > 1, "several rows stay: {rows:?}");
    assert_eq!(rows[0], "+window · move view to…");
    press(&shell, "enter", cx);
    settle(&shell, cx);
    assert!(
        dialog_open(&shell, cx),
        "the first row ran: the chooser is open"
    );
    cx.read(|cx| {
        assert_eq!(shell.workspace.read(cx).modal, Some(ModalKind::ViewChooser));
    });
}
