//! UI integration tests for the CFG drawer view (M9c, [`config`]): a real
//! `Workspace` over a real temp vault whose config file is the one written,
//! driven by keys and clicks.
//!
//! [`config`]: super::config
#![cfg(test)]

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Context, FocusHandle, TestAppContext, Window};
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::leader_route::LeaderRoute;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::settings::themes::Theme;
use tokio::runtime::Runtime;

use super::config::{
    CONFIG_ID, CONFIG_PATH_ID, CONFIG_PREFERENCES_ID, CONFIG_THEME_ID, ConfigRow, NOT_SET,
};
use super::modal::ModalKind;
use super::palette::PaletteRun;
use super::preferences::{PREFERENCES_ID, SAVE_ID};
use super::preferences_tests::{config, on_disk, on_vault, open, settle, vault};
use super::start_tests::{Started, drive, runtime};
use super::{LayoutStore, Workspace, rail_cell_id};
use crate::keymap;
use crate::startup::Start;

impl Started {
    fn cf_on_window<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) -> R,
    ) -> R {
        let workspace = self.workspace.clone();
        let r = cx
            .update_window(self.handle, |_, window, cx| {
                workspace.update(cx, |w, cx| f(w, window, cx))
            })
            .unwrap();
        cx.run_until_parked();
        r
    }

    fn cf_press(&self, key: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn cf_click(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// The CFG rows as the view draws them now.
    fn cf_rows(&self, cx: &mut TestAppContext) -> Vec<ConfigRow> {
        let view = cx.read(|cx| self.workspace.read(cx).config.clone());
        cx.update(|cx| view.update(cx, |v, cx| v.rows(cx)))
            .expect("the window has its settings")
    }

    fn cf_value(&self, label: &str, cx: &mut TestAppContext) -> String {
        self.cf_rows(cx)
            .into_iter()
            .find(|r| r.label == label)
            .map(|r| r.value)
            .unwrap_or_else(|| panic!("no {label} row"))
    }

    /// The view's root and its two launchers.
    fn cf_handles(&self, cx: &TestAppContext) -> [FocusHandle; 3] {
        cx.read(|cx| self.workspace.read(cx).config.read(cx).handles())
    }

    fn cf_focused(&self, ix: usize, cx: &mut TestAppContext) -> bool {
        let handle = self.cf_handles(cx)[ix].clone();
        cx.update_window(self.handle, |_, window, _| handle.is_focused(window))
            .unwrap()
    }

    /// Show CFG through the leader route `v c`, checked unblocked.
    fn cf_route(&self, cx: &mut TestAppContext) {
        self.cf_on_window(cx, |w, window, cx| {
            let route = w.route_of(LeaderAction::VaultConfig);
            assert_eq!(route, LeaderRoute::Drawer(DrawerView::Config));
            assert_eq!(w.route_block(&route, Panel::Editor, cx), None);
            w.perform_route(route, window, cx)
        });
    }

    fn cf_theme_picker_open(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).theme_picker.is_some())
    }

    fn cf_preferences_open(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).preferences.is_some())
    }
}

/// Drive until the CFG theme row reads `name` (its resolution lands on the
/// blocking pool).
fn theme_row_reads(s: &Started, rt: &Runtime, name: &str, cx: &mut TestAppContext) {
    drive(rt, cx, |cx| s.cf_value("Theme", cx) == name);
}

// ── reaching it ─────────────────────────────────────────────────────────────

/// The leader route (`v c`), the palette's `+vault · config` and the rail
/// cell show it; the view takes the keyboard.
#[gpui_kit::test]
fn the_leader_the_palette_and_the_rail_reach_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = on_vault(cx, &rt, &v);
    assert!(!s.shows(CONFIG_ID, cx));
    s.cf_route(cx);
    assert!(s.shows(CONFIG_ID, cx));
    assert!(s.cf_focused(0, cx), "the view has the keyboard");

    // Away (FILES), then the palette leaf.
    s.cf_click(rail_cell_id(DrawerView::Files), cx);
    assert!(!s.shows(CONFIG_ID, cx));
    let entry = cx.read(|cx| {
        s.workspace
            .read(cx)
            .palette_entries(Panel::Editor, cx)
            .into_iter()
            .find(|e| *e.run() == PaletteRun::Leaf(LeaderAction::VaultConfig))
            .expect("the leaf is listed")
    });
    assert_eq!(entry.blocked(), None);
    s.cf_on_window(cx, |w, window, cx| {
        w.run_palette_row(PaletteRun::Leaf(LeaderAction::VaultConfig), window, cx)
    });
    assert!(s.shows(CONFIG_ID, cx));

    // The leader keys.
    s.cf_click(rail_cell_id(DrawerView::Files), cx);
    s.cf_press("ctrl-shift-b", cx);
    s.cf_press("v", cx);
    s.cf_press("c", cx);
    assert!(s.shows(CONFIG_ID, cx));

    // The rail cell (FILES first, then the CFG cell).
    s.cf_click(rail_cell_id(DrawerView::Files), cx);
    assert!(!s.shows(CONFIG_ID, cx));
    s.cf_click(rail_cell_id(DrawerView::Config), cx);
    assert!(s.shows(CONFIG_ID, cx));
    settle(&rt, cx);
}

// ── what it shows ───────────────────────────────────────────────────────────

/// The rows: the resolved theme, the desktop app's chords (every one of
/// Preferences'), the delay and the config file (its full path in the
/// tooltip target).
#[gpui_kit::test]
fn the_rows_show_the_settings(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    v.settings.write().unwrap().leader_timeout_ms = 350;
    let s = on_vault(cx, &rt, &v);
    s.cf_route(cx);
    theme_row_reads(&s, &rt, &Theme::default().name, cx);
    let leader = cx.read(|cx| keymap::chord_labels(cx, &ActionShortcuts::Leader));
    let prefs = cx.read(|cx| keymap::chord_labels(cx, &ActionShortcuts::OpenPreferences));
    assert_eq!(prefs.len(), 2, "F4 and Ctrl+,");
    assert_eq!(s.cf_value("Leader key", cx), leader.join(" / "));
    assert_eq!(s.cf_value("Preferences", cx), prefs.join(" / "));
    assert!(s.cf_value("Preferences", cx).starts_with("F4"));
    assert_eq!(s.cf_value("Which-key delay", cx), "350 ms");
    assert_eq!(
        s.cf_value("Config file", cx),
        config(&v).display().to_string()
    );
    assert!(s.shows(CONFIG_PATH_ID, cx));
    let labels: Vec<&str> = s.cf_rows(cx).iter().map(|r| r.label).collect();
    assert_eq!(
        labels,
        [
            "Theme",
            "Leader key",
            "Preferences",
            "Which-key delay",
            "Config file"
        ]
    );
    assert_eq!(NOT_SET, "not set");
    settle(&rt, cx);
}

/// An unbound chord reads "not set".
#[gpui_kit::test]
fn an_unbound_chord_reads_not_set(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = on_vault(cx, &rt, &v);
    let mut overrides = std::collections::BTreeMap::new();
    overrides.insert("Leader".to_string(), Vec::new());
    let (unbound, _) = keymap::Keymap::load(keymap::Platform::LinuxWindows, &overrides);
    cx.update(|cx| keymap::init(cx, unbound));
    cx.run_until_parked();
    s.cf_route(cx);
    assert_eq!(s.cf_value("Leader key", cx), NOT_SET);
    settle(&rt, cx);
}

/// A theme picked in the picker and a Preferences save show in the rows
/// without reopening the view.
#[gpui_kit::test]
fn a_theme_pick_and_a_save_update_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = on_vault(cx, &rt, &v);
    s.cf_route(cx);
    theme_row_reads(&s, &rt, &Theme::default().name, cx);

    // The theme picker, from the launcher: Down, Enter.
    s.cf_press("t", cx);
    assert!(s.cf_theme_picker_open(cx));
    drive(&rt, cx, |cx| {
        cx.read(|cx| {
            s.workspace
                .read(cx)
                .theme_picker
                .as_ref()
                .and_then(|o| o.view.upgrade())
                .is_some_and(|p| p.read(cx).is_loaded())
        })
    });
    s.cf_press("down", cx);
    let picked = cx.read(|cx| {
        s.workspace
            .read(cx)
            .theme_picker
            .as_ref()
            .and_then(|o| o.view.upgrade())
            .and_then(|p| p.read(cx).selected())
            .unwrap()
    });
    s.cf_press("enter", cx);
    drive(&rt, cx, |cx| !s.cf_theme_picker_open(cx));
    s.settle_dialog(cx);
    theme_row_reads(&s, &rt, &picked, cx);
    assert!(s.shows(CONFIG_ID, cx));

    // A Preferences save of the which-key delay.
    let before = s.cf_value("Which-key delay", cx);
    s.cf_press("p", cx);
    assert!(s.cf_preferences_open(cx));
    drive(&rt, cx, |cx| {
        cx.read(|cx| {
            s.workspace
                .read(cx)
                .preferences
                .as_ref()
                .is_some_and(|p| p.read(cx).model().is_some())
        })
    });
    let view = cx.read(|cx| s.workspace.read(cx).preferences.clone().unwrap());
    cx.update(|cx| view.update(cx, |v, cx| v.step_delay(true, cx)));
    s.cf_click(SAVE_ID, cx);
    drive(&rt, cx, |cx| !s.cf_preferences_open(cx));
    let after = s.cf_value("Which-key delay", cx);
    assert_ne!(after, before);
    assert_eq!(
        after,
        format!("{} ms", on_disk(&v).leader_timeout_ms),
        "what was saved"
    );
    settle(&rt, cx);
}

// ── launchers ───────────────────────────────────────────────────────────────

/// `Theme…` and `Preferences…` open the picker and Preferences with the
/// mouse; `t`, Enter and `p` with the keyboard.
#[gpui_kit::test]
fn the_launchers_open_the_picker_and_preferences(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = on_vault(cx, &rt, &v);
    s.cf_route(cx);

    s.cf_click(CONFIG_THEME_ID, cx);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::ThemePicker));
    s.cf_press("escape", cx);
    s.settle_dialog(cx);
    assert!(!s.cf_theme_picker_open(cx));
    assert!(s.cf_focused(0, cx), "the keyboard is back on the view");

    s.cf_press("t", cx);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::ThemePicker));
    s.cf_press("escape", cx);
    s.settle_dialog(cx);

    s.cf_press("enter", cx);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::ThemePicker));
    s.cf_press("escape", cx);
    s.settle_dialog(cx);

    s.cf_click(CONFIG_PREFERENCES_ID, cx);
    assert!(s.shows(PREFERENCES_ID, cx));
    drive(&rt, cx, |cx| {
        cx.read(|cx| {
            s.workspace
                .read(cx)
                .preferences
                .as_ref()
                .is_some_and(|p| p.read(cx).model().is_some())
        })
    });
    s.cf_press("escape", cx);
    assert!(!s.cf_preferences_open(cx));

    s.cf_on_window(cx, |w, window, cx| w.apply_focus(window, cx));
    assert!(s.cf_focused(0, cx));
    s.cf_press("p", cx);
    assert!(s.shows(PREFERENCES_ID, cx));
    settle(&rt, cx);
}

/// Tab goes from the view to `Theme…`, then `Preferences…`; Enter on a
/// launcher is the launcher's.
#[gpui_kit::test]
fn the_launchers_follow_the_view_in_the_focus_order(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = on_vault(cx, &rt, &v);
    s.cf_route(cx);
    assert!(s.cf_focused(0, cx));
    s.cf_press("tab", cx);
    assert!(s.cf_focused(1, cx), "Theme… is next");
    s.cf_press("tab", cx);
    assert!(s.cf_focused(2, cx), "then Preferences…");
    s.cf_press("enter", cx);
    assert!(
        s.shows(PREFERENCES_ID, cx),
        "Enter on Preferences… opens them"
    );
    assert!(!s.cf_theme_picker_open(cx));
    settle(&rt, cx);
}

/// With no vault (a workspace that would not open) the view shows its rows
/// and its launchers work.
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
    s.cf_route(cx);
    assert!(s.shows(CONFIG_ID, cx));
    theme_row_reads(&s, &rt, &Theme::default().name, cx);
    assert_eq!(
        s.cf_value("Config file", cx),
        config(&v).display().to_string()
    );
    s.cf_press("t", cx);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::ThemePicker));
    s.cf_press("escape", cx);
    s.settle_dialog(cx);
    s.cf_click(CONFIG_PREFERENCES_ID, cx);
    assert!(s.shows(PREFERENCES_ID, cx));
    settle(&rt, cx);
}
