//! UI integration tests for the theme picker (M9c, [`theme_picker`]): a real
//! `Workspace` over a real temp vault whose config file is the one written,
//! driven by keys and clicks.
//!
//! Like the Preferences tests, each test owns a current-thread tokio runtime
//! and drives it between GPUI's `run_until_parked`: the theme list and the
//! settings write run on it.
//!
//! [`theme_picker`]: super::theme_picker
#![cfg(test)]

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Context, Entity, EntityId, TestAppContext, Window, point, px};
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::leader_route::LeaderRoute;
use kimun_notes::logic::overlay_open::OverlayOpen;
use kimun_notes::logic::sort::{SortField, SortOrder, SortState};
use kimun_notes::settings::themes::Theme;
use kimun_notes::settings::{SortFieldSetting, SortOrderSetting};
use tokio::runtime::Runtime;

use super::modal::ModalKind;
use super::palette::PaletteRun;
use super::preferences::PREFERENCES_ID;
use super::preferences_tests::{config, on_disk, on_vault, open, settle, vault};
use super::start_tests::{Started, drive, runtime};
use super::theme_picker::{
    EMPTY_TEXT, LOADING_TEXT, THEME_PICKER_APPLY_ID, THEME_PICKER_CANCEL_ID, THEME_PICKER_EMPTY_ID,
    THEME_PICKER_ERROR_ID, THEME_PICKER_ID, TITLE, ThemePicker, theme_pick_row_id,
};
use super::{LayoutStore, Workspace};
use crate::startup::Start;
use crate::test_vault::TestVault;

impl Started {
    fn tp_view(&self, cx: &TestAppContext) -> Entity<ThemePicker> {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .theme_picker
                .as_ref()
                .and_then(|open| open.view.upgrade())
        })
        .expect("the theme picker is open")
    }

    fn tp_is_open(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).theme_picker.is_some())
    }

    fn tp_read<R>(&self, cx: &TestAppContext, f: impl FnOnce(&ThemePicker) -> R) -> R {
        let view = self.tp_view(cx);
        cx.read(|cx| f(view.read(cx)))
    }

    fn tp_on_window<R>(
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

    fn tp_press(&self, key: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn tp_typed(&self, text: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.input(text, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn tp_click(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn tp_double_click(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.double_click(id, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn tp_shows(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some_and(|e| e.visible())
        })
        .unwrap()
    }

    fn tp_window_theme(&self, cx: &TestAppContext) -> String {
        cx.read(|cx| crate::theme::current(cx).name.clone())
    }

    /// Open the picker through the leader route (`v t`), checked unblocked.
    fn tp_route(&self, cx: &mut TestAppContext) {
        self.tp_on_window(cx, |w, window, cx| {
            let route = w.route_of(LeaderAction::VaultTheme);
            assert_eq!(route, LeaderRoute::Overlay(OverlayOpen::ThemePicker));
            assert_eq!(w.route_block(&route, Panel::Editor, cx), None);
            w.perform_route(route, window, cx)
        });
    }

    fn tp_loaded(&self, rt: &Runtime, cx: &mut TestAppContext) {
        drive(rt, cx, |cx| self.tp_read(cx, |p| p.is_loaded()));
    }

    fn tp_selected(&self, cx: &TestAppContext) -> Option<String> {
        self.tp_read(cx, |p| p.selected())
    }

    fn tp_rows(&self, cx: &TestAppContext) -> Vec<(String, bool)> {
        self.tp_read(cx, |p| p.rows())
    }

    /// The FILES entity: a reopen builds a new one.
    fn tp_files(&self, cx: &TestAppContext) -> EntityId {
        cx.read(|cx| self.workspace.read(cx).files.clone().unwrap().entity_id())
    }
}

/// The window on `v` with the picker open and its list read.
fn with_picker(cx: &mut TestAppContext, rt: &Runtime, v: &TestVault) -> Started {
    let s = on_vault(cx, rt, v);
    s.tp_route(cx);
    s.tp_loaded(rt, cx);
    s
}

// ── opening ─────────────────────────────────────────────────────────────────

/// The leader route opens it: "Loading themes…" until the list lands, then
/// every theme, the current one marked and selected, the field focused.
#[gpui_kit::test]
fn the_leader_lists_the_themes_with_the_current_one_selected(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = on_vault(cx, &rt, &v);
    s.tp_route(cx);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::ThemePicker));
    assert!(!s.tp_read(cx, |p| p.is_loaded()), "read off the UI thread");
    assert!(s.tp_shows(THEME_PICKER_EMPTY_ID, cx));
    assert_eq!(LOADING_TEXT, "Loading themes…");
    s.tp_loaded(&rt, cx);
    let rows = s.tp_rows(cx);
    assert!(
        rows.len() >= Theme::builtins().len(),
        "every built-in theme"
    );
    let current: Vec<&str> = rows
        .iter()
        .filter(|(_, current)| *current)
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(current, [Theme::default().name.as_str()]);
    assert_eq!(s.tp_selected(cx), Some(Theme::default().name));
    assert!(s.tp_shows(theme_pick_row_id(&Theme::default().name), cx));
    assert!(s.tp_shows(THEME_PICKER_ID, cx));
    assert_eq!(TITLE, "Theme");
    // The field has the keyboard: typing filters.
    s.tp_typed("x", cx);
    assert!(s.tp_rows(cx).len() < rows.len());
    settle(&rt, cx);
}

/// Leader `v t` (the keys), the palette's `+vault · theme picker` and the
/// same route again: it opens; the palette row is not blocked.
#[gpui_kit::test]
fn the_leader_keys_and_the_palette_open_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = on_vault(cx, &rt, &v);
    s.tp_press("ctrl-shift-b", cx);
    s.tp_press("v", cx);
    s.tp_press("t", cx);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::ThemePicker));
    s.tp_loaded(&rt, cx);
    s.tp_press("escape", cx);
    s.settle_dialog(cx);
    assert_eq!(s.shown_modal(cx), None);
    assert!(!s.tp_is_open(cx));

    let entry = cx.read(|cx| {
        s.workspace
            .read(cx)
            .palette_entries(Panel::Editor, cx)
            .into_iter()
            .find(|e| *e.run() == PaletteRun::Leaf(LeaderAction::VaultTheme))
            .expect("the leaf is listed")
    });
    assert_eq!(entry.blocked(), None);
    assert_eq!(entry.label(), "+vault · theme picker");
    s.tp_on_window(cx, |w, window, cx| {
        w.run_palette_row(PaletteRun::Leaf(LeaderAction::VaultTheme), window, cx)
    });
    assert_eq!(s.shown_modal(cx), Some(ModalKind::ThemePicker));
    // Its route again while open: the toggle closes it.
    s.tp_on_window(cx, |w, window, cx| {
        w.perform_route(LeaderRoute::Overlay(OverlayOpen::ThemePicker), window, cx)
    });
    s.settle_dialog(cx);
    assert_eq!(s.shown_modal(cx), None);
    settle(&rt, cx);
}

/// With no vault (a workspace that would not open) it opens and a pick is
/// written.
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
    s.tp_route(cx);
    s.tp_loaded(&rt, cx);
    s.tp_press("down", cx);
    let next = s.tp_selected(cx).unwrap();
    s.tp_press("enter", cx);
    drive(&rt, cx, |cx| !s.tp_is_open(cx));
    assert_eq!(v.settings.read().unwrap().theme, next);
    assert_eq!(on_disk(&v).theme, next);
}

// ── preview, keep, revert ───────────────────────────────────────────────────

/// Up/Down restyle the whole window live (window theme and the kit's
/// colours); nothing is written, in memory or on disk. Escape puts the
/// opening theme back and closes.
#[gpui_kit::test]
fn moving_previews_live_and_escape_reverts(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_picker(cx, &rt, &v);
    let original = s.tp_window_theme(cx);
    let kit_before = cx.read(|cx| gpui_kit::component::Theme::global(cx).background);
    let before = v.settings.read().unwrap().clone();
    let disk_before = std::fs::read_to_string(config(&v)).unwrap();

    s.tp_press("down", cx);
    let next = s.tp_selected(cx).unwrap();
    assert_ne!(next, original);
    assert_eq!(s.tp_window_theme(cx), next, "the window previews it");
    let kit_after = cx.read(|cx| gpui_kit::component::Theme::global(cx).background);
    assert_ne!(kit_before, kit_after, "the kit's colours follow");
    s.tp_press("up", cx);
    assert_eq!(s.tp_window_theme(cx), original, "back on the opening row");
    s.tp_press("down", cx);
    s.tp_press("down", cx);
    assert_ne!(s.tp_window_theme(cx), original);
    assert_eq!(*v.settings.read().unwrap(), before, "memory untouched");
    assert_eq!(std::fs::read_to_string(config(&v)).unwrap(), disk_before);

    s.tp_press("escape", cx);
    s.settle_dialog(cx);
    assert_eq!(s.shown_modal(cx), None);
    assert!(!s.tp_is_open(cx));
    assert_eq!(s.tp_window_theme(cx), original, "Escape reverts");
    assert_eq!(*v.settings.read().unwrap(), before);
    assert_eq!(std::fs::read_to_string(config(&v)).unwrap(), disk_before);
    settle(&rt, cx);
}

/// Enter writes the theme (only the theme) to disk and memory, keeps it on
/// the window and closes; the vault is not entered again.
#[gpui_kit::test]
fn enter_keeps_the_theme_without_a_reopen(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_picker(cx, &rt, &v);
    let files = s.tp_files(cx);
    let before = v.settings.read().unwrap().clone();
    s.tp_press("down", cx);
    let next = s.tp_selected(cx).unwrap();
    s.tp_press("enter", cx);
    drive(&rt, cx, |cx| !s.tp_is_open(cx));
    s.settle_dialog(cx);
    assert_eq!(s.shown_modal(cx), None);
    assert_eq!(s.tp_window_theme(cx), next, "kept on the window");
    let after = v.settings.read().unwrap().clone();
    assert_eq!(after.theme, next, "in memory");
    let mut rest = after.clone();
    rest.theme = before.theme.clone();
    assert_eq!(rest, before, "nothing else changed");
    let disk = on_disk(&v);
    assert_eq!(disk.theme, next, "on disk");
    assert_eq!(disk.leader_timeout_ms, before.leader_timeout_ms);
    assert_eq!(s.tp_files(cx), files, "no reopen: FILES is the same entity");
    assert!(cx.read(|cx| !s.workspace.read(cx).reopen_running()));
    settle(&rt, cx);
}

/// Typing narrows the list (fuzzy, best match selected and previewed); a
/// filter matching nothing says so, and the window goes back to the opening
/// theme; Enter then does nothing.
#[gpui_kit::test]
fn the_filter_narrows_and_says_when_nothing_matches(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_picker(cx, &rt, &v);
    let original = s.tp_window_theme(cx);
    let all = s.tp_rows(cx).len();
    s.tp_typed("nord", cx);
    let rows = s.tp_rows(cx);
    assert!(!rows.is_empty() && rows.len() < all, "{rows:?}");
    assert!(rows.iter().any(|(name, _)| name == "Nord"), "{rows:?}");
    assert_eq!(s.tp_selected(cx), Some(rows[0].0.clone()), "best match");
    assert_eq!(s.tp_window_theme(cx), rows[0].0, "previewed");

    s.tp_typed("zzzqqq", cx);
    assert!(s.tp_rows(cx).is_empty());
    assert_eq!(s.tp_selected(cx), None);
    assert!(s.tp_shows(THEME_PICKER_EMPTY_ID, cx));
    assert_eq!(EMPTY_TEXT, "No themes match.");
    assert_eq!(
        s.tp_window_theme(cx),
        original,
        "nothing selected: reverted"
    );
    let disabled = cx
        .update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(THEME_PICKER_APPLY_ID).disabled()
        })
        .unwrap();
    assert_eq!(disabled, Some(true));
    s.tp_press("enter", cx);
    drive(&rt, cx, |_| true);
    assert!(s.tp_is_open(cx), "nothing to keep");

    // Emptied: the current theme is selected again.
    s.tp_on_window(cx, |w, window, cx| {
        let view = w.theme_picker.as_ref().unwrap().view.upgrade().unwrap();
        let input = view.read(cx).input.clone();
        input.update(cx, |i, cx| i.set_value("", window, cx));
    });
    // `set_value` reports no change: type and erase one letter instead.
    s.tp_typed("a", cx);
    s.tp_press("backspace", cx);
    assert_eq!(s.tp_selected(cx), Some(Theme::default().name));
    settle(&rt, cx);
}

/// A failed write keeps the dialog open with the error, the previewed
/// theme on the window and the settings untouched; Escape still reverts.
#[gpui_kit::test]
fn a_failed_write_keeps_the_dialog_with_the_error(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_picker(cx, &rt, &v);
    let original = s.tp_window_theme(cx);
    let before = v.settings.read().unwrap().clone();
    std::fs::write(config(&v), "this is [not toml").unwrap();
    s.tp_press("down", cx);
    let next = s.tp_selected(cx).unwrap();
    s.tp_press("enter", cx);
    drive(&rt, cx, |cx| !s.tp_read(cx, |p| p.is_saving()));
    assert!(s.tp_is_open(cx), "the dialog stays");
    assert_eq!(s.shown_modal(cx), Some(ModalKind::ThemePicker));
    let error = s.tp_read(cx, |p| p.error().map(str::to_string)).unwrap();
    assert!(error.contains("Couldn't save the theme"), "{error}");
    assert!(s.tp_shows(THEME_PICKER_ERROR_ID, cx));
    assert_eq!(*v.settings.read().unwrap(), before, "memory untouched");
    assert_eq!(
        std::fs::read_to_string(config(&v)).unwrap(),
        "this is [not toml",
        "the unreadable config is left alone"
    );
    assert_eq!(s.tp_window_theme(cx), next, "the preview stays");
    s.tp_press("escape", cx);
    s.settle_dialog(cx);
    assert!(!s.tp_is_open(cx));
    assert_eq!(s.tp_window_theme(cx), original);
    settle(&rt, cx);
}

/// A kept theme and a sort default queued back to back (`theme_first`
/// says which first), both written before the UI thread hears of either:
/// neither write undoes the other, on disk or in memory.
fn queued_writes_both_land(theme_first: bool, cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_picker(cx, &rt, &v);
    let before = v.settings.read().unwrap().clone();
    let state = SortState {
        field: SortField::Title,
        order: SortOrder::Descending,
        group_dirs: None,
    };
    assert_ne!(before.default_sort_field, SortFieldSetting::Title);
    s.tp_press("down", cx);
    let next = s.tp_selected(cx).unwrap();
    assert_ne!(next, before.theme);
    let sort = |cx: &mut TestAppContext| {
        s.tp_on_window(cx, |w, _, cx| w.save_sort_default(state.clone(), false, cx))
    };
    // Neither write has started: the runtime is not driven yet.
    if theme_first {
        s.tp_press("enter", cx);
        sort(cx);
    } else {
        sort(cx);
        s.tp_press("enter", cx);
    }
    // Both run now, one after the other, while the UI thread is not run:
    // what each callback applies lands only after both are on disk.
    rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(500)));
    drive(&rt, cx, |cx| !s.tp_is_open(cx));
    settle(&rt, cx);
    let memory = v.settings.read().unwrap().clone();
    let disk = on_disk(&v);
    for (what, got) in [("memory", &memory), ("disk", &disk)] {
        assert_eq!(got.theme, next, "the theme in {what}");
        assert_eq!(
            got.default_sort_field,
            SortFieldSetting::Title,
            "the sort default in {what}"
        );
        assert_eq!(got.default_sort_order, SortOrderSetting::Descending);
        assert_eq!(got.journal_sort_field, before.journal_sort_field);
    }
    assert_eq!(s.tp_window_theme(cx), next);
}

#[gpui_kit::test]
fn a_theme_then_a_sort_default_queued_both_land(cx: &mut TestAppContext) {
    queued_writes_both_land(true, cx);
}

#[gpui_kit::test]
fn a_sort_default_then_a_theme_queued_both_land(cx: &mut TestAppContext) {
    queued_writes_both_land(false, cx);
}

/// Enter on the opening theme, nothing moved, keeps nothing to write: the
/// picker closes, the config is not touched (an unreadable one raises no
/// error) and an unset theme stays unset.
#[gpui_kit::test]
fn enter_on_the_opening_theme_writes_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_picker(cx, &rt, &v);
    let original = s.tp_window_theme(cx);
    let before = v.settings.read().unwrap().clone();
    std::fs::write(config(&v), "this is [not toml").unwrap();
    s.tp_press("enter", cx);
    s.settle_dialog(cx);
    assert!(!s.tp_is_open(cx), "closed");
    assert_eq!(s.tp_window_theme(cx), original);
    assert_eq!(*v.settings.read().unwrap(), before, "settings untouched");
    assert_eq!(
        std::fs::read_to_string(config(&v)).unwrap(),
        "this is [not toml",
        "the config was not written"
    );
    settle(&rt, cx);
}

/// A click selects a row and previews it, the dialog staying open; a
/// double-click keeps it (`ClickPolicy::Select`). `Cancel` reverts.
#[gpui_kit::test]
fn a_click_previews_and_a_double_click_keeps(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_picker(cx, &rt, &v);
    let original = s.tp_window_theme(cx);
    // The row above the current one: on screen with it.
    let rows = s.tp_rows(cx);
    let at = rows.iter().position(|(_, current)| *current).unwrap();
    let target = rows[if at > 0 { at - 1 } else { at + 1 }].0.clone();
    s.tp_click(theme_pick_row_id(&target), cx);
    assert!(s.tp_is_open(cx), "a click only selects");
    assert_eq!(s.tp_selected(cx), Some(target.clone()));
    assert_eq!(s.tp_window_theme(cx), target);
    assert_eq!(v.settings.read().unwrap().theme, original);
    s.tp_click(THEME_PICKER_CANCEL_ID, cx);
    s.settle_dialog(cx);
    assert!(!s.tp_is_open(cx));
    assert_eq!(s.tp_window_theme(cx), original, "Cancel reverts");

    s.tp_route(cx);
    s.tp_loaded(&rt, cx);
    s.tp_double_click(theme_pick_row_id(&target), cx);
    drive(&rt, cx, |cx| !s.tp_is_open(cx));
    assert_eq!(s.tp_window_theme(cx), target);
    assert_eq!(v.settings.read().unwrap().theme, target);
    assert_eq!(on_disk(&v).theme, target);
    settle(&rt, cx);
}

/// A click outside the dialog reverts and closes, as Escape does.
#[gpui_kit::test]
fn a_click_outside_reverts(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_picker(cx, &rt, &v);
    let original = s.tp_window_theme(cx);
    s.tp_press("down", cx);
    assert_ne!(s.tp_window_theme(cx), original);
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click_at("dialog-0", point(px(4.), px(300.)), cx)
    })
    .unwrap();
    cx.run_until_parked();
    s.settle_dialog(cx);
    assert!(!s.tp_is_open(cx));
    assert_eq!(s.shown_modal(cx), None);
    assert_eq!(s.tp_window_theme(cx), original);
    settle(&rt, cx);
}

// ── interplay ───────────────────────────────────────────────────────────────

/// Refused over another modal, Preferences and the setup assistant; with it
/// open, Preferences are refused; leaving the vault closes it and puts the
/// opening theme back, and so does the setup assistant.
#[gpui_kit::test]
fn refused_over_modals_and_takeovers_and_restored_when_they_win(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = on_vault(cx, &rt, &v);
    let original = s.tp_window_theme(cx);

    // Over the palette: refused.
    s.tp_press("ctrl-shift-p", cx);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Palette));
    s.tp_on_window(cx, |w, window, cx| w.open_theme_picker(window, cx));
    assert!(!s.tp_is_open(cx));
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Palette));
    s.tp_press("escape", cx);
    s.settle_dialog(cx);

    // Over Preferences: refused.
    s.tp_press("f4", cx);
    assert!(s.shows(PREFERENCES_ID, cx));
    s.tp_on_window(cx, |w, window, cx| w.open_theme_picker(window, cx));
    assert!(!s.tp_is_open(cx));
    assert_eq!(s.shown_modal(cx), None);
    drive(&rt, cx, |cx| {
        cx.read(|cx| {
            s.workspace
                .read(cx)
                .preferences
                .as_ref()
                .is_some_and(|p| p.read(cx).model().is_some())
        })
    });
    s.tp_press("escape", cx);
    assert!(!s.shows(PREFERENCES_ID, cx));

    // With it open: Preferences are refused.
    s.tp_route(cx);
    s.tp_loaded(&rt, cx);
    s.tp_press("down", cx);
    s.tp_press("f4", cx);
    s.tp_on_window(cx, |w, window, cx| w.open_preferences(window, cx));
    assert!(!s.shows(PREFERENCES_ID, cx));
    assert_eq!(s.shown_modal(cx), Some(ModalKind::ThemePicker));

    // Leaving the vault: the picker goes, its preview with it.
    assert_ne!(s.tp_window_theme(cx), original);
    s.tp_on_window(cx, |w, window, cx| w.leave_vault(window, cx));
    assert!(!s.tp_is_open(cx));
    assert_eq!(s.tp_window_theme(cx), original, "the preview is put back");

    // The setup assistant wins: the picker goes, its preview with it.
    s.tp_route(cx);
    s.tp_loaded(&rt, cx);
    s.tp_press("down", cx);
    assert_ne!(s.tp_window_theme(cx), original);
    s.tp_on_window(cx, |w, window, cx| w.open_onboarding(window, cx));
    assert!(!s.tp_is_open(cx));
    assert_eq!(s.tp_window_theme(cx), original, "the preview is put back");
    s.tp_on_window(cx, |w, window, cx| w.open_theme_picker(window, cx));
    assert!(!s.tp_is_open(cx), "refused over the setup assistant");
    settle(&rt, cx);
}
