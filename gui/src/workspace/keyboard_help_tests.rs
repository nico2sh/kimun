//! UI integration tests for the keyboard reference (M10d, [`keyboard_help`]):
//! a real `Workspace` over a real temp vault, driven by keys and clicks,
//! with the active keymap swapped where a test needs `[gui.keys]` overrides.
//!
//! [`keyboard_help`]: super::keyboard_help
#![cfg(test)]

use std::collections::BTreeMap;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Context, Entity, TestAppContext, Window};
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::actions::{ActionGroup, ActionShortcuts};
use kimun_notes::logic::keyboard_help::{HelpRow, HelpTarget, NOT_BOUND};
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::leader_route::LeaderRoute;
use kimun_notes::logic::overlay_open::OverlayOpen;
use kimun_notes::logic::text_action::TextAction;

use super::config::CONFIG_KEYS_ID;
use super::keyboard_help::{
    EMPTY_TEXT, KEYBOARD_HELP_EMPTY_ID, KEYBOARD_HELP_ID, KeyboardHelp, QUERY_NOTE, TITLE,
    help_heading_id, help_row_id,
};
use super::modal::ModalKind;
use super::palette::PaletteRun;
use super::preferences_tests::{config, on_vault, open, settle, vault};
use super::route::SOON;
use super::start_tests::{Started, runtime};
use super::{LayoutStore, Workspace};
use crate::keymap::{self, Keymap, Platform};
use crate::startup::Start;

/// The last keyboard reference view built in this app: the window keeps no
/// handle on it (the modal slot says whether it is open), so the tests watch
/// for it being built.
struct SeenKeyboardHelp(Option<gpui_kit::WeakEntity<KeyboardHelp>>);

impl gpui_kit::Global for SeenKeyboardHelp {}

/// Start watching for keyboard reference views (once per app).
fn kh_track(cx: &mut TestAppContext) {
    cx.update(|cx| {
        if cx.has_global::<SeenKeyboardHelp>() {
            return;
        }
        cx.set_global(SeenKeyboardHelp(None));
        cx.observe_new(|_: &mut KeyboardHelp, _, cx: &mut Context<KeyboardHelp>| {
            let view = cx.entity().downgrade();
            cx.set_global(SeenKeyboardHelp(Some(view)));
        })
        .detach();
    });
}

impl Started {
    fn kh_view(&self, cx: &TestAppContext) -> Entity<KeyboardHelp> {
        assert!(
            cx.read(|cx| self.workspace.read(cx).modal) == Some(ModalKind::KeyboardHelp),
            "the keyboard reference is the open modal"
        );
        cx.read(|cx| {
            cx.try_global::<SeenKeyboardHelp>()
                .and_then(|seen| seen.0.as_ref())
                .and_then(|view| view.upgrade())
        })
        .expect("the keyboard reference is open")
    }

    fn kh_is_open(&self, cx: &mut TestAppContext) -> bool {
        self.shown_modal(cx) == Some(ModalKind::KeyboardHelp)
    }

    fn kh_read<R>(&self, cx: &TestAppContext, f: impl FnOnce(&KeyboardHelp) -> R) -> R {
        let view = self.kh_view(cx);
        cx.read(|cx| f(view.read(cx)))
    }

    fn kh_rows(&self, cx: &TestAppContext) -> Vec<HelpRow> {
        self.kh_read(cx, |h| h.rows())
    }

    fn kh_on_window<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) -> R,
    ) -> R {
        kh_track(cx);
        let workspace = self.workspace.clone();
        let r = cx
            .update_window(self.handle, |_, window, cx| {
                workspace.update(cx, |w, cx| f(w, window, cx))
            })
            .unwrap();
        cx.run_until_parked();
        r
    }

    fn kh_press(&self, key: &str, cx: &mut TestAppContext) {
        kh_track(cx);
        cx.update_window(self.handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn kh_typed(&self, text: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.input(text, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn kh_click(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
        kh_track(cx);
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn kh_shows(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some_and(|e| e.visible())
        })
        .unwrap()
    }

    /// Close whatever modal is open and let it leave.
    fn kh_escape(&self, cx: &mut TestAppContext) {
        self.kh_press("escape", cx);
        self.settle_dialog(cx);
    }
}

/// The row for `action`, if listed.
fn action_row(rows: &[HelpRow], action: ActionShortcuts) -> Option<HelpRow> {
    rows.iter()
        .find(|r| r.target == HelpTarget::Action(action.clone()))
        .cloned()
}

/// Make a keymap with `overrides` the active one.
fn install(cx: &mut TestAppContext, overrides: &[(&str, &[&str])]) {
    let overrides: BTreeMap<String, Vec<String>> = overrides
        .iter()
        .map(|(a, cs)| (a.to_string(), cs.iter().map(|c| c.to_string()).collect()))
        .collect();
    let (keymap, errors) = Keymap::load(Platform::LinuxWindows, &overrides);
    assert!(errors.is_empty(), "{errors:?}");
    cx.update(|cx| keymap::init(cx, keymap));
    cx.run_until_parked();
}

// ── opening ─────────────────────────────────────────────────────────────────

/// F1 opens it over the window (the field focused: typing filters) and
/// toggles it closed; Escape closes it; the palette's row, leader `?` and
/// the CFG view's launcher (its `k` and a click) open it too.
#[gpui_kit::test]
fn f1_the_palette_the_leader_and_cfg_open_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = on_vault(cx, &rt, &v);

    s.kh_press("f1", cx);
    assert!(s.kh_is_open(cx));
    assert!(s.kh_shows(KEYBOARD_HELP_ID, cx));
    assert_eq!(TITLE, "Keyboard shortcuts");
    let all = s.kh_rows(cx).len();
    s.kh_typed("quit", cx);
    assert!(s.kh_rows(cx).len() < all, "the field has the keyboard");
    // Its own chord again closes it.
    s.kh_press("f1", cx);
    s.settle_dialog(cx);
    assert_eq!(s.shown_modal(cx), None);

    s.kh_press("f1", cx);
    assert!(s.kh_is_open(cx));
    s.kh_escape(cx);
    assert_eq!(s.shown_modal(cx), None);

    // The palette's row: runnable, with F1 as its hint.
    let entry = cx.read(|cx| {
        s.workspace
            .read(cx)
            .palette_entries(Panel::Editor, cx)
            .into_iter()
            .find(|e| *e.run() == PaletteRun::Leaf(LeaderAction::Help))
            .expect("the palette lists it")
    });
    assert_eq!(entry.blocked(), None);
    // One row, not a leaf and a chord twin.
    let twins = cx.read(|cx| {
        s.workspace
            .read(cx)
            .palette_entries(Panel::Editor, cx)
            .iter()
            .filter(|e| *e.run() == PaletteRun::Chord(ActionShortcuts::KeyboardHelp))
            .count()
    });
    assert_eq!(twins, 0, "the leaf row is the only route");
    s.kh_on_window(cx, |w, window, cx| {
        w.run_palette_row(PaletteRun::Leaf(LeaderAction::Help), window, cx)
    });
    assert!(s.kh_is_open(cx));
    s.kh_escape(cx);

    // Leader `?`: the cheatsheet route, now the reference.
    s.kh_on_window(cx, |w, window, cx| {
        let route = w.route_of(LeaderAction::Help);
        assert_eq!(route, LeaderRoute::Overlay(OverlayOpen::Cheatsheet));
        assert_eq!(w.route_block(&route, Panel::Editor, cx), None);
        w.perform_route(route, window, cx)
    });
    assert!(s.kh_is_open(cx));
    s.kh_escape(cx);

    // CFG: `k` with the view focused, and the launcher's click.
    s.kh_on_window(cx, |w, window, cx| {
        let route = w.route_of(LeaderAction::VaultConfig);
        w.perform_route(route, window, cx)
    });
    s.kh_press("k", cx);
    assert!(s.kh_is_open(cx));
    s.kh_escape(cx);
    s.kh_click(CONFIG_KEYS_ID, cx);
    assert!(s.kh_is_open(cx));
    s.kh_escape(cx);
    settle(&rt, cx);
}

/// Never over another modal, Preferences or the setup assistant: F1 and the
/// route do nothing there.
#[gpui_kit::test]
fn refused_over_modals_and_takeovers(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = on_vault(cx, &rt, &v);

    s.kh_press("ctrl-shift-p", cx);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Palette));
    s.kh_press("f1", cx);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Palette));
    s.kh_on_window(cx, |w, window, cx| w.open_keyboard_help(window, cx));
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Palette));
    s.kh_escape(cx);
    s.kh_escape(cx);
    assert_eq!(s.shown_modal(cx), None);

    s.kh_on_window(cx, |w, window, cx| w.open_preferences(window, cx));
    assert!(cx.read(|cx| s.workspace.read(cx).preferences.is_some()));
    s.kh_press("f1", cx);
    s.kh_on_window(cx, |w, window, cx| w.open_keyboard_help(window, cx));
    assert_ne!(s.shown_modal(cx), Some(ModalKind::KeyboardHelp));
    assert_eq!(s.shown_modal(cx), None);
    s.pf_loaded(&rt, cx);
    s.kh_press("escape", cx);
    assert!(cx.read(|cx| s.workspace.read(cx).preferences.is_none()));

    s.kh_on_window(cx, |w, window, cx| w.open_onboarding(window, cx));
    assert!(cx.read(|cx| s.workspace.read(cx).onboarding.is_some()));
    s.kh_press("f1", cx);
    s.kh_on_window(cx, |w, window, cx| w.open_keyboard_help(window, cx));
    assert_ne!(s.shown_modal(cx), Some(ModalKind::KeyboardHelp));
    assert_eq!(s.shown_modal(cx), None);
    settle(&rt, cx);
}

/// With no vault (a workspace that would not open) F1 opens it and lists
/// the chords and the leader tree.
#[gpui_kit::test]
fn it_works_without_a_vault(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = open(
        cx,
        Start::OpenFailed("no vault in this test".into()),
        v.settings.clone(),
        LayoutStore::File(config(&v)),
    );
    cx.run_until_parked();
    s.kh_press("f1", cx);
    assert!(s.kh_is_open(cx));
    let rows = s.kh_rows(cx);
    assert!(action_row(&rows, ActionShortcuts::OpenPreferences).is_some());
    assert!(rows.iter().any(|r| r.group == ActionGroup::LeaderTree));
    s.kh_escape(cx);
    assert_eq!(s.shown_modal(cx), None);
}

// ── what it lists ───────────────────────────────────────────────────────────

/// The effective keymap's chords under the headings: Preferences shows
/// `F4 / Ctrl+,`, every bound action is listed exactly once, nothing the app
/// ships unbound (formatting) is, the leader tree follows with its gateway
/// named once in the heading, and a leaf with no surface says so.
#[gpui_kit::test]
fn it_lists_every_bound_action_once_under_its_heading(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = on_vault(cx, &rt, &v);
    s.kh_press("f1", cx);
    let rows = s.kh_rows(cx);

    let prefs = action_row(&rows, ActionShortcuts::OpenPreferences).unwrap();
    assert_eq!(prefs.keys, "F4 / Ctrl+,");
    assert_eq!(prefs.label, "Preferences");
    assert_eq!(prefs.group, ActionGroup::Other);
    assert_eq!(
        action_row(&rows, ActionShortcuts::KeyboardHelp)
            .unwrap()
            .keys,
        "F1"
    );
    assert_eq!(
        action_row(&rows, ActionShortcuts::ToggleSidebar)
            .unwrap()
            .group,
        ActionGroup::WindowAndPanels
    );
    // The query syntax: once, with its chord and where it applies (F1 is
    // the reference's everywhere else).
    let syntax = action_row(&rows, ActionShortcuts::QuerySyntax).unwrap();
    assert_eq!(syntax.label, "Query syntax");
    assert_eq!(syntax.keys, "F1");
    assert_eq!(syntax.group, ActionGroup::SearchAndAsk);
    assert_eq!(syntax.note.as_deref(), Some(QUERY_NOTE));

    let keymap = cx.read(keymap::active);
    for action in ActionShortcuts::ALL {
        let listed = rows
            .iter()
            .filter(|r| r.target == HelpTarget::Action(action.clone()))
            .count();
        let bound = !keymap.chords_for(&action).is_empty();
        assert_eq!(
            listed,
            usize::from(bound),
            "{action}: listed {listed} times"
        );
    }
    assert!(action_row(&rows, ActionShortcuts::Text(TextAction::Bold)).is_none());

    let headings = s.kh_read(cx, |h| h.headings());
    assert_eq!(
        headings,
        [
            ActionGroup::WindowAndPanels,
            ActionGroup::Editor,
            ActionGroup::FilesAndNotes,
            ActionGroup::SearchAndAsk,
            ActionGroup::LeaderTree,
            ActionGroup::Other,
        ],
        "no Unbound heading by default"
    );
    assert!(s.kh_shows(help_heading_id(ActionGroup::WindowAndPanels), cx));

    let leader = |keys: &str| {
        rows.iter()
            .find(|r| r.group == ActionGroup::LeaderTree && r.keys == keys)
            .cloned()
            .unwrap_or_else(|| panic!("leader {keys} is listed"))
    };
    assert_eq!(leader("v p").label, "+vault · preferences");
    assert_eq!(leader("v p").note, None);
    assert_eq!(leader("n t").note.as_deref(), Some(SOON));
    assert_eq!(s.kh_read(cx, |h| h.gateway().to_string()), "Ctrl+Shift+B");
    // Selected: the first row, shown.
    let first = s.kh_read(cx, |h| h.selected()).unwrap();
    assert_eq!(first, rows[0]);
    assert!(s.kh_shows(help_row_id(&first), cx));
    settle(&rt, cx);
}

/// `[gui.keys]` overrides show as they are in effect: a moved chord, an
/// unbound default under "Unbound" saying "not bound", an unbound gateway
/// in the leader heading.
#[gpui_kit::test]
fn overrides_show_the_effective_chords(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = on_vault(cx, &rt, &v);
    install(
        cx,
        &[
            ("OpenSettings", &["f9", "ctrl+alt+p"]),
            ("QuickNote", &[]),
            ("Leader", &[]),
        ],
    );
    s.kh_press("f1", cx);
    assert!(s.kh_is_open(cx));
    let rows = s.kh_rows(cx);
    assert_eq!(
        action_row(&rows, ActionShortcuts::OpenPreferences)
            .unwrap()
            .keys,
        "F9 / Ctrl+Alt+P"
    );
    let quick = action_row(&rows, ActionShortcuts::QuickNote).unwrap();
    assert_eq!(quick.group, ActionGroup::Unbound);
    assert_eq!(quick.keys, NOT_BOUND);
    assert_eq!(
        s.kh_read(cx, |h| h.headings()).last(),
        Some(&ActionGroup::Unbound)
    );
    assert_eq!(
        s.kh_read(cx, |h| h.gateway().to_string()),
        "the Leader chord is not bound"
    );
    settle(&rt, cx);
}

/// In the vim editor the leader heading names the Space gateway too.
#[gpui_kit::test]
fn the_vim_editor_adds_the_space_gateway(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = super::start_tests::open_started_with(
        cx,
        super::start_tests::vault_start(&v, crate::startup::FirstNote::Restore),
        v.settings.clone(),
        true,
    );
    cx.run_until_parked();
    s.kh_on_window(cx, |w, window, cx| w.open_keyboard_help(window, cx));
    assert!(s.kh_is_open(cx));
    assert_eq!(
        s.kh_read(cx, |h| h.gateway().to_string()),
        "Ctrl+Shift+B, or Space in vim Normal mode"
    );
    settle(&rt, cx);
}

// ── filtering and keys ──────────────────────────────────────────────────────

/// Typing narrows the list (label or keys), the best match selected;
/// gibberish says "Nothing matches."; Up/Down move the selection; Enter
/// runs nothing and keeps it open.
#[gpui_kit::test]
fn the_filter_narrows_and_the_keys_move(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = on_vault(cx, &rt, &v);
    s.kh_press("f1", cx);
    let all = s.kh_rows(cx);

    s.kh_press("down", cx);
    assert_eq!(s.kh_read(cx, |h| h.selected()), Some(all[1].clone()));
    s.kh_press("up", cx);
    assert_eq!(s.kh_read(cx, |h| h.selected()), Some(all[0].clone()));

    s.kh_typed("preferences", cx);
    let narrowed = s.kh_rows(cx);
    assert!(!narrowed.is_empty() && narrowed.len() < all.len());
    assert!(
        narrowed
            .iter()
            .any(|r| r.target == HelpTarget::Action(ActionShortcuts::OpenPreferences))
    );
    assert!(
        narrowed
            .iter()
            .any(|r| r.target == HelpTarget::Leaf(LeaderAction::VaultPreferences))
    );
    assert_eq!(s.kh_read(cx, |h| h.selected()), Some(narrowed[0].clone()));
    // Grouped still: the headings in order.
    let groups: Vec<ActionGroup> = narrowed.iter().map(|r| r.group).collect();
    let mut sorted = groups.clone();
    sorted.sort();
    assert_eq!(groups, sorted);

    s.kh_press("enter", cx);
    assert!(s.kh_is_open(cx), "Enter runs nothing");
    assert!(cx.read(|cx| s.workspace.read(cx).preferences.is_none()));

    s.kh_typed("zzqqxxj", cx);
    assert!(s.kh_rows(cx).is_empty());
    assert!(s.kh_shows(KEYBOARD_HELP_EMPTY_ID, cx));
    assert_eq!(EMPTY_TEXT, "Nothing matches.");
    s.kh_press("down", cx);
    s.kh_press("enter", cx);
    assert!(s.kh_is_open(cx));
    s.kh_escape(cx);
    assert_eq!(s.shown_modal(cx), None);
    settle(&rt, cx);
}

/// A click selects a row; nothing runs.
#[gpui_kit::test]
fn a_click_selects_a_row(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = on_vault(cx, &rt, &v);
    s.kh_press("f1", cx);
    let rows = s.kh_rows(cx);
    s.kh_click(help_row_id(&rows[2]), cx);
    assert_eq!(s.kh_read(cx, |h| h.selected()), Some(rows[2].clone()));
    assert!(s.kh_is_open(cx));
    settle(&rt, cx);
}
