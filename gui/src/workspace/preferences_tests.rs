//! UI integration tests for Preferences (M9b-1, [`preferences`]): a real
//! `Workspace` over a real temp vault whose config file is the one saved
//! to, driven by keys and clicks.
//!
//! Like the startup tests, each test owns a current-thread tokio runtime and
//! drives it between GPUI's `run_until_parked` (`start_tests::drive`): the
//! model, the settings write and a reopen run on tokio.
//!
//! [`preferences`]: super::preferences
#![cfg(test)]

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Context, Entity, EntityId, Focusable as _, TestAppContext, Window, WindowOptions,
};
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::layout::{Panel, Side};
use kimun_notes::logic::leader_route::LeaderRoute;
use kimun_notes::logic::message::AppMessage;
use kimun_notes::logic::preferences::{PreferencesModel, SortSlot};
use kimun_notes::logic::sort::{SortField, SortOrder};
use kimun_notes::settings::gui::GuiSettings;
use kimun_notes::settings::themes::Theme;
use kimun_notes::settings::{AppSettings, EditorBackendSetting, SharedSettings, SortOrderSetting};
use tokio::runtime::Runtime;

use super::modal::ModalKind;
use super::palette::PaletteRun;
use super::preferences::{
    AUTOSAVE_ID, BACKEND_KEPT_ID, CANCEL_ID, DELAY_ID, DISCARD_ID, FONT_DEFAULT_LABEL,
    FONT_SIZE_ID, FONT_SIZE_UP_ID, KEEP_EDITING_ID, MESSAGE_ID, PREFERENCES_ID, PreferencesView,
    REDUCE_MOTION_HELP, REDUCE_MOTION_HELP_ID, REDUCE_MOTION_ID, SAVE_ID, SERVER_ERROR_ID,
    SERVER_ID, Section, UPDATE_CHECK_ID, needs_reopen, section_id, theme_row_id,
};
use super::start_tests::{Started, drive, runtime};
use super::{LayoutStore, Workspace};
use crate::editor_font::{self, EditorFontSetting};
use crate::editor_view::{self, EditorView};
use crate::keymap::{self, Keymap, Platform};
use crate::mono_families::{self, MonoFamilies};
use crate::startup::{FirstNote, Start, VaultStart};
use crate::test_vault::{NAME, TestVault, test_vault};
use crate::theme;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::select::SelectEvent;

/// The window as `main` builds it, on `start`, saving to `store`.
pub(super) fn open(
    cx: &mut TestAppContext,
    start: Start,
    settings: SharedSettings,
    store: LayoutStore,
) -> Started {
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
    });
    let layout =
        crate::shell_tree::restore(&GuiSettings::default(), crate::shell_tree::RESTORE_CAPS);
    let (handle, workspace) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                let editor = cx.new(|cx| EditorView::on_channel(false, tx.clone(), cx));
                let workspace = cx.new(|cx| {
                    let mut workspace =
                        Workspace::new(editor, Theme::default(), layout, store, window, cx);
                    workspace.start(start, settings, tx, rx, window, cx);
                    workspace
                });
                workspace.update(cx, |w, cx| {
                    w.apply_focus(window, cx);
                    w.guard_close(window, cx);
                });
                workspace
            })
        })
        .expect("open test window");
    Started { handle, workspace }
}

/// The config file `v`'s settings came from (and are saved to).
pub(super) fn config(v: &TestVault) -> PathBuf {
    v.settings.read().unwrap().config_file.clone().unwrap()
}

/// The settings as they are on disk now.
pub(super) fn on_disk(v: &TestVault) -> AppSettings {
    AppSettings::load_from_file(config(v)).expect("the config on disk loads")
}

/// A vault with `a.md` ("# A") open, the window saving to its config.
/// Its theme is named (the window's), so going back to it is clean.
pub(super) fn vault(rt: &Runtime) -> TestVault {
    let v = rt.block_on(test_vault(&[("a.md", "# A"), ("b.md", "# B")], &["a.md"]));
    v.settings.write().unwrap().theme = Theme::default().name;
    v
}

/// The window on `v`, indexed, `a.md` restored.
pub(super) fn on_vault(cx: &mut TestAppContext, rt: &Runtime, v: &TestVault) -> Started {
    let start = Start::Vault(VaultStart {
        vault: v.vault.clone(),
        name: NAME.into(),
        first: FirstNote::Restore,
    });
    let s = open(cx, start, v.settings.clone(), LayoutStore::File(config(v)));
    drive(rt, cx, |cx| !s.indexing(cx) && s.text(cx) == "# A");
    s
}

/// The window on `v` with Preferences open (the leader route), the draft
/// read.
fn with_prefs(cx: &mut TestAppContext, rt: &Runtime, v: &TestVault) -> Started {
    let s = on_vault(cx, rt, v);
    s.pf_route(cx);
    s.pf_loaded(rt, cx);
    s
}

/// Let the vault's background work land while the runtime is still up.
pub(super) fn settle(rt: &Runtime, cx: &mut TestAppContext) {
    let mut rounds = 0;
    drive(rt, cx, |_| {
        rounds += 1;
        rounds >= 40
    });
}

impl Started {
    fn pf_view(&self, cx: &TestAppContext) -> Entity<PreferencesView> {
        cx.read(|cx| self.workspace.read(cx).preferences.clone())
            .expect("Preferences are open")
    }

    pub(super) fn pf_is_open(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).preferences.is_some())
    }

    pub(super) fn pf_read<R>(
        &self,
        cx: &TestAppContext,
        f: impl FnOnce(&PreferencesView) -> R,
    ) -> R {
        let view = self.pf_view(cx);
        cx.read(|cx| f(view.read(cx)))
    }

    fn pf_model<R>(&self, cx: &TestAppContext, f: impl FnOnce(&PreferencesModel) -> R) -> R {
        self.pf_read(cx, |v| f(v.model().expect("the draft is read")))
    }

    fn pf_with_view(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut PreferencesView, &mut Window, &mut Context<PreferencesView>),
    ) {
        let view = self.pf_view(cx);
        cx.update_window(self.handle, |_, window, cx| {
            view.update(cx, |v, cx| f(v, window, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn pf_on_window<R>(
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

    /// Open Preferences through the leader route (`v p`).
    pub(super) fn pf_route(&self, cx: &mut TestAppContext) {
        self.pf_on_window(cx, |w, window, cx| {
            assert_eq!(
                w.route_block(&LeaderRoute::OpenPreferences, Panel::Editor, cx),
                None
            );
            w.perform_route(LeaderRoute::OpenPreferences, window, cx)
        });
    }

    pub(super) fn pf_loaded(&self, rt: &Runtime, cx: &mut TestAppContext) {
        drive(rt, cx, |cx| self.pf_read(cx, |v| v.model().is_some()));
    }

    fn pf_press(&self, key: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn pf_typed(&self, text: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.input(text, cx))
            .unwrap();
        cx.run_until_parked();
    }

    pub(super) fn pf_click(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn pf_disabled(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).disabled() == Some(true)
        })
        .unwrap()
    }

    fn pf_shows(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some_and(|e| e.visible())
        })
        .unwrap()
    }

    /// Whether the handle `ix` of [`PreferencesView::handles`] has the
    /// keyboard.
    fn pf_focused(&self, ix: usize, cx: &mut TestAppContext) -> bool {
        let handle = self.pf_read(cx, |v| v.handles()[ix].clone());
        cx.update_window(self.handle, |_, window, _| handle.is_focused(window))
            .unwrap()
    }

    pub(super) fn pf_section(&self, cx: &mut TestAppContext, section: Section) {
        self.pf_click(section_id(section), cx);
        assert_eq!(self.pf_read(cx, |v| v.section()), section);
    }

    /// Select the next theme with the theme list's Down key; its name.
    fn pf_pick_next_theme(&self, cx: &mut TestAppContext) -> String {
        let next = next_theme(self, cx);
        self.pf_section(cx, Section::Appearance);
        self.pf_with_view(cx, |v, window, cx| {
            let handle = v.handles()[1].clone();
            window.focus(&handle, cx);
        });
        self.pf_press("down", cx);
        assert_eq!(self.pf_model(cx, |m| m.theme_name.clone()), next);
        next
    }

    fn pf_window_theme(&self, cx: &TestAppContext) -> String {
        cx.read(|cx| crate::theme::current(cx).name.clone())
    }

    /// Give the server field the keyboard and replace its text by typing.
    fn pf_type_server(&self, text: &str, cx: &mut TestAppContext) {
        self.pf_section(cx, Section::Server);
        let input = self.pf_read(cx, |v| v.server_input().clone());
        cx.update_window(self.handle, |_, window, cx| {
            input.update(cx, |i, cx| i.set_value("", window, cx));
            let handle = input.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        })
        .unwrap();
        cx.run_until_parked();
        self.pf_typed(text, cx);
    }

    /// Save, and wait until Preferences are gone and the window is calm.
    fn pf_save(&self, rt: &Runtime, cx: &mut TestAppContext) {
        assert!(!self.pf_disabled(SAVE_ID, cx), "Save is enabled");
        self.pf_click(SAVE_ID, cx);
        drive(rt, cx, |cx| {
            cx.read(|cx| {
                let w = self.workspace.read(cx);
                w.preferences.is_none() && !w.reopen_running() && !w.status.indexing
            })
        });
    }

    /// The FILES entity: a reopen builds a new one.
    fn files_id(&self, cx: &TestAppContext) -> EntityId {
        cx.read(|cx| self.workspace.read(cx).files.clone().unwrap().entity_id())
    }

    fn autosave_interval(&self, cx: &TestAppContext) -> Option<u64> {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .notes
                .as_ref()
                .and_then(|h| h.session.autosave_interval())
        })
    }
}

/// The index of the theme after the selected one (the list wraps).
fn next_theme(s: &Started, cx: &TestAppContext) -> String {
    s.pf_model(cx, |m| {
        m.themes[(m.theme_idx + 1) % m.themes.len()].name.clone()
    })
}

// ── entry ───────────────────────────────────────────────────────────────────

/// The leader route, F4, Ctrl+, and the palette each open Preferences over
/// the whole window, the section list holding the keyboard; a clean Esc
/// closes them.
#[gpui_kit::test]
fn opens_from_the_route_the_chords_and_the_palette(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_prefs(cx, &rt, &v);
    assert!(s.shows(PREFERENCES_ID, cx));
    assert!(!s.shows("rail", cx) && !s.shows("editor-panel", cx));
    assert!(s.pf_focused(0, cx), "the section list has the keyboard");
    assert_eq!(
        s.pf_read(cx, |v| v.section()),
        Section::Workspaces,
        "they open on Workspaces, as the TUI's"
    );
    assert!(s.pf_disabled(SAVE_ID, cx), "nothing to save yet");
    s.pf_press("escape", cx);
    assert!(!s.pf_is_open(cx), "clean: Esc closes at once");
    assert!(s.shows("rail", cx));

    for chord in ["f4", "ctrl-,"] {
        s.pf_press(chord, cx);
        assert!(s.pf_is_open(cx), "{chord} opens them");
        s.pf_loaded(&rt, cx);
        s.pf_click(CANCEL_ID, cx);
        assert!(!s.pf_is_open(cx));
    }

    let entry = cx.read(|cx| {
        s.workspace
            .read(cx)
            .palette_entries(Panel::Editor, cx)
            .into_iter()
            .find(|e| *e.run() == PaletteRun::Leaf(LeaderAction::VaultPreferences))
            .expect("the leaf is listed")
    });
    assert_eq!(entry.blocked(), None);
    assert_eq!(entry.label(), "+vault · preferences");
    s.pf_on_window(cx, |w, window, cx| {
        w.run_palette_row(PaletteRun::Leaf(LeaderAction::VaultPreferences), window, cx)
    });
    assert!(s.pf_is_open(cx));
    s.pf_loaded(&rt, cx);
    settle(&rt, cx);
}

/// Without a vault (a workspace that would not open) they open too.
#[gpui_kit::test]
fn opens_with_no_vault(cx: &mut TestAppContext) {
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
    s.pf_press("f4", cx);
    s.pf_loaded(&rt, cx);
    assert!(s.shows(PREFERENCES_ID, cx));
    s.pf_press("down", cx);
    s.pf_press("right", cx);
    s.pf_on_window(cx, |_, _, _| ());
}

/// Under the palette or the setup assistant they do not open.
#[gpui_kit::test]
fn refused_over_a_modal_or_the_setup_assistant(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = on_vault(cx, &rt, &v);
    s.pf_press("ctrl-shift-p", cx);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Palette));
    s.pf_press("f4", cx);
    s.pf_on_window(cx, |w, window, cx| w.open_preferences(window, cx));
    assert!(!s.pf_is_open(cx));
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Palette));
    s.pf_press("escape", cx);
    s.settle_dialog(cx);

    s.pf_on_window(cx, |w, window, cx| w.open_onboarding(window, cx));
    assert!(cx.read(|cx| s.workspace.read(cx).onboarding_open()));
    s.pf_press("f4", cx);
    s.pf_on_window(cx, |w, window, cx| w.open_preferences(window, cx));
    assert!(!s.pf_is_open(cx));
    settle(&rt, cx);
}

/// While they show, the window's chords do nothing, and a leader leaf sent
/// on the channel is dropped.
#[gpui_kit::test]
fn shell_chords_are_inert_while_they_show(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_prefs(cx, &rt, &v);
    let left = |cx: &TestAppContext| cx.read(|cx| s.workspace.read(cx).side_drawn(Side::Left));
    let before = left(cx);
    s.pf_press("ctrl-t", cx);
    assert_eq!(left(cx), before, "the side toggle is inert");
    s.pf_press("ctrl-shift-p", cx);
    assert_eq!(s.shown_modal(cx), None, "no palette");
    s.pf_press("ctrl-shift-b", cx);
    assert!(cx.read(|cx| !s.workspace.read(cx).leader.is_pending()));
    s.pf_press("f5", cx);
    assert_eq!(s.shown_modal(cx), None, "no switcher");
    super::start_tests::send(
        &s,
        AppMessage::ExecuteLeaderAction(LeaderAction::VaultSwitch),
        cx,
    );
    cx.run_until_parked();
    assert_eq!(s.shown_modal(cx), None, "the leaf is dropped");
    assert!(s.pf_is_open(cx));
    settle(&rt, cx);
}

// ── editing ─────────────────────────────────────────────────────────────────

/// Up / Down walk the sections from the section list; a click shows one.
#[gpui_kit::test]
fn the_sections_walk_with_keys_and_clicks(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_prefs(cx, &rt, &v);
    let mut seen = vec![s.pf_read(cx, |v| v.section())];
    for _ in 0..7 {
        s.pf_press("down", cx);
        seen.push(s.pf_read(cx, |v| v.section()));
    }
    assert_eq!(
        seen,
        [
            Section::Workspaces,
            Section::Appearance,
            Section::Display,
            Section::Sorting,
            Section::Indexing,
            Section::Editor,
            Section::Server,
            Section::Server,
        ]
    );
    for section in Section::ALL {
        assert!(s.pf_shows(section_id(section), cx), "{section:?} is listed");
    }
    s.pf_press("up", cx);
    assert_eq!(s.pf_read(cx, |v| v.section()), Section::Editor);
    s.pf_section(cx, Section::Display);
    assert!(s.pf_shows(UPDATE_CHECK_ID, cx));
    assert!(
        s.pf_focused(0, cx),
        "a click leaves the keyboard on the list"
    );
    settle(&rt, cx);
}

/// Every section edits the draft only: the shared settings and the file stay
/// as they were until Save, which is enabled once the draft differs.
#[gpui_kit::test]
fn each_section_edits_the_draft_only(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let before = v.settings.read().unwrap().clone();
    let file = std::fs::read_to_string(config(&v)).unwrap();
    let s = with_prefs(cx, &rt, &v);

    // Appearance: Tab to the theme list, Down selects the next theme.
    let next = next_theme(&s, cx);
    s.pf_section(cx, Section::Appearance);
    s.pf_with_view(cx, |v, window, cx| {
        let handle = v.handles()[1].clone();
        window.focus(&handle, cx);
    });
    s.pf_press("down", cx);
    assert_eq!(s.pf_model(cx, |m| m.theme_name.clone()), next);
    assert!(!s.pf_disabled(SAVE_ID, cx), "a change enables Save");
    s.pf_press("up", cx);
    assert!(s.pf_disabled(SAVE_ID, cx), "back to clean: disabled again");

    // Display: the update check (a click), the which-key delay (keys and
    // buttons).
    s.pf_section(cx, Section::Display);
    let checked = s.pf_model(cx, |m| m.update_check);
    s.pf_click(UPDATE_CHECK_ID, cx);
    assert_eq!(s.pf_model(cx, |m| m.update_check), !checked);
    let delay = s.pf_model(cx, |m| m.leader_timeout_ms);
    s.pf_click(DELAY_ID, cx);
    s.pf_press("right", cx);
    s.pf_press("right", cx);
    assert_eq!(s.pf_model(cx, |m| m.leader_timeout_ms), delay + 100);
    s.pf_click(super::preferences::DELAY_DOWN_ID, cx);
    assert_eq!(s.pf_model(cx, |m| m.leader_timeout_ms), delay + 50);

    // Sorting: the choices, as the radios set them.
    s.pf_section(cx, Section::Sorting);
    s.pf_with_view(cx, |v, _, cx| {
        v.set_sort(SortSlot::DefaultOrder, 1, cx);
        v.set_sort(SortSlot::JournalField, 1, cx);
    });
    assert_eq!(
        s.pf_model(cx, |m| (
            m.default_sort_order,
            m.sort_label(SortSlot::JournalField)
        )),
        (SortOrderSetting::Descending, "Title")
    );

    // Editor: the autosave stepper, the backend.
    s.pf_section(cx, Section::Editor);
    let autosave = s.pf_model(cx, |m| m.autosave_interval_secs);
    s.pf_click(AUTOSAVE_ID, cx);
    s.pf_press("right", cx);
    s.pf_press("right", cx);
    s.pf_press("left", cx);
    assert_eq!(s.pf_model(cx, |m| m.autosave_interval_secs), autosave + 5);
    s.pf_click(super::preferences::AUTOSAVE_UP_ID, cx);
    assert_eq!(s.pf_model(cx, |m| m.autosave_interval_secs), autosave + 10);
    s.pf_with_view(cx, |v, _, cx| v.select_backend(1, cx));
    assert_eq!(
        s.pf_model(cx, |m| m.editor_backend),
        EditorBackendSetting::Vim
    );

    // Server: the address.
    s.pf_type_server("http://localhost:7573", cx);
    assert_eq!(
        s.pf_model(cx, |m| m.server_url.clone()).as_deref(),
        Some("http://localhost:7573")
    );

    assert!(!s.pf_disabled(SAVE_ID, cx));
    assert_eq!(*v.settings.read().unwrap(), before, "memory untouched");
    assert_eq!(std::fs::read_to_string(config(&v)).unwrap(), file);
    settle(&rt, cx);
}

// ── leaving ─────────────────────────────────────────────────────────────────

/// A theme previews on the whole window at once. Esc asks, Esc again keeps
/// editing; Cancel asks, Discard puts the theme back. Nothing is saved.
#[gpui_kit::test]
fn the_theme_previews_live_and_cancel_restores_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let before = v.settings.read().unwrap().clone();
    let file = std::fs::read_to_string(config(&v)).unwrap();
    let s = with_prefs(cx, &rt, &v);
    let original = s.pf_window_theme(cx);
    let next = s.pf_pick_next_theme(cx);
    assert_eq!(s.pf_window_theme(cx), next, "the window shows it");
    let picked = s.pf_model(cx, |m| m.selected_theme().clone());
    cx.read(|cx| {
        let kit = gpui_kit::component::Theme::global(cx);
        let want: gpui_kit::Hsla = theme::panel_background(&picked).into();
        assert_eq!(kit.background, want, "the kit's components too");
    });

    // A click on a row previews too: back on the window's own theme (clean).
    s.pf_click(theme_row_id(&original), cx);
    assert_eq!(s.pf_window_theme(cx), original);
    assert!(s.pf_disabled(SAVE_ID, cx));
    s.pf_press("down", cx);
    assert_eq!(s.pf_window_theme(cx), next);
    s.pf_press("escape", cx);
    assert!(s.pf_read(cx, |v| v.confirm()).is_some(), "dirty: it asks");
    assert!(s.pf_shows(DISCARD_ID, cx));
    s.pf_press("escape", cx);
    assert!(
        s.pf_read(cx, |v| v.confirm()).is_none(),
        "Esc: keep editing"
    );
    assert!(s.pf_is_open(cx));
    s.pf_click(CANCEL_ID, cx);
    assert!(s.pf_read(cx, |v| v.confirm()).is_some());
    s.pf_click(KEEP_EDITING_ID, cx);
    assert!(s.pf_read(cx, |v| v.confirm()).is_none());
    assert_eq!(s.pf_window_theme(cx), next, "still previewing");
    s.pf_click(CANCEL_ID, cx);
    s.pf_click(DISCARD_ID, cx);
    assert!(!s.pf_is_open(cx));
    assert_eq!(s.pf_window_theme(cx), original, "the theme is back");
    cx.read(|cx| {
        let kit = gpui_kit::component::Theme::global(cx);
        let want: gpui_kit::Hsla = theme::panel_background(&Theme::default()).into();
        assert_eq!(kit.background, want);
    });
    settle(&rt, cx);
    assert_eq!(*v.settings.read().unwrap(), before, "Esc never saves");
    assert_eq!(std::fs::read_to_string(config(&v)).unwrap(), file);
}

/// With no theme configured, picking a theme enables Save and going back
/// to the one in effect disables it again: the draft is clean (M10d).
#[gpui_kit::test]
fn an_unset_theme_picked_and_put_back_is_clean(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    v.settings.write().unwrap().theme = String::new();
    let s = with_prefs(cx, &rt, &v);
    let original = s.pf_window_theme(cx);
    assert_eq!(original, Theme::default().name, "unset: the default");
    assert!(s.pf_disabled(SAVE_ID, cx));
    let next = s.pf_pick_next_theme(cx);
    assert_ne!(next, original);
    assert!(!s.pf_disabled(SAVE_ID, cx), "another theme: Save");
    s.pf_click(theme_row_id(&original), cx);
    assert_eq!(s.pf_model(cx, |m| m.theme_name.clone()), original);
    assert!(
        s.pf_disabled(SAVE_ID, cx),
        "back on the effective theme: clean"
    );
    // Clean: Esc closes without asking.
    s.pf_press("escape", cx);
    assert!(!s.pf_is_open(cx));
    settle(&rt, cx);
    assert_eq!(v.settings.read().unwrap().theme, "", "still unset");
}

/// Enter on the question discards.
#[gpui_kit::test]
fn enter_on_the_question_discards(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_prefs(cx, &rt, &v);
    s.pf_section(cx, Section::Display);
    s.pf_click(UPDATE_CHECK_ID, cx);
    s.pf_press("escape", cx);
    s.pf_press("enter", cx);
    assert!(!s.pf_is_open(cx));
    assert!(v.settings.read().unwrap().update_check(), "nothing applied");
    settle(&rt, cx);
}

/// The window's close with a changed draft asks first and keeps the window;
/// Discard then closes it. With a clean draft it closes as usual.
#[gpui_kit::test]
fn the_window_close_asks_when_dirty(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_prefs(cx, &rt, &v);
    assert!(
        s.pf_on_window(cx, |w, _, cx| w.preferences_allow_close(cx)),
        "clean: nothing to ask"
    );
    s.pf_section(cx, Section::Display);
    s.pf_click(UPDATE_CHECK_ID, cx);
    assert!(!s.pf_on_window(cx, |w, _, cx| w.should_close(cx)));
    assert_eq!(
        s.pf_read(cx, |v| v.confirm()).map(|c| c.close_window),
        Some(true)
    );
    assert!(s.pf_is_open(cx));
    s.pf_click(DISCARD_ID, cx);
    drive(&rt, cx, |cx| {
        cx.update_window(s.handle, |_, _, _| ()).is_err()
    });
}

/// `is_dirty` is cached between frames: every edit of the draft, and the
/// edit that undoes it, is seen at once.
#[gpui_kit::test]
fn the_dirty_answer_follows_every_edit(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_prefs(cx, &rt, &v);
    assert!(!s.pf_read(cx, |v| v.is_dirty()));
    assert!(!s.pf_read(cx, |v| v.is_dirty()), "the cached answer agrees");
    s.pf_section(cx, Section::Display);
    s.pf_click(UPDATE_CHECK_ID, cx);
    assert!(s.pf_read(cx, |v| v.is_dirty()));
    s.pf_click(UPDATE_CHECK_ID, cx);
    assert!(!s.pf_read(cx, |v| v.is_dirty()), "undone");
    s.pf_section(cx, Section::Sorting);
    s.pf_with_view(cx, |v, _, cx| v.set_sort(SortSlot::DefaultOrder, 1, cx));
    assert!(s.pf_read(cx, |v| v.is_dirty()));
}

// ── saving ──────────────────────────────────────────────────────────────────

/// Save writes every field to the file and applies it at once, without
/// entering the vault again: theme (window, editor, FILES), which-key delay
/// (the next leader sequence), autosave interval (the running tick), the
/// FILES sort default, the update check (stored).
#[gpui_kit::test]
fn save_persists_and_applies_without_a_reopen(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_prefs(cx, &rt, &v);
    let files = s.files_id(cx);
    let editor = cx.read(|cx| s.workspace.read(cx).editor.entity_id());
    let updates = s.pf_model(cx, |m| m.update_check);
    let autosave = s.autosave_interval(cx).expect("the tick runs");
    let next = s.pf_pick_next_theme(cx);

    s.pf_section(cx, Section::Display);
    s.pf_click(UPDATE_CHECK_ID, cx);
    s.pf_click(DELAY_ID, cx);
    for _ in 0..40 {
        s.pf_press("left", cx);
    }
    assert_eq!(s.pf_model(cx, |m| m.leader_timeout_ms), 0);

    s.pf_section(cx, Section::Sorting);
    s.pf_with_view(cx, |v, _, cx| v.set_sort(SortSlot::DefaultOrder, 1, cx));

    s.pf_section(cx, Section::Editor);
    s.pf_click(AUTOSAVE_ID, cx);
    s.pf_press("right", cx);
    s.pf_press("ctrl-s", cx);
    drive(&rt, cx, |cx| !s.pf_is_open(cx));

    // Memory and disk.
    for settings in [v.settings.read().unwrap().clone(), on_disk(&v)] {
        assert_eq!(settings.theme, next);
        assert_eq!(settings.update_check(), !updates);
        assert_eq!(settings.leader_timeout_ms, 0);
        assert_eq!(settings.default_sort_order, SortOrderSetting::Descending);
        assert_eq!(settings.autosave_interval_secs, autosave + 5);
    }
    // No reopen: the same views, the same note.
    assert_eq!(s.files_id(cx), files, "FILES was not rebuilt");
    assert!(!cx.read(|cx| s.workspace.read(cx).reopen_running()));
    assert_eq!(
        cx.read(|cx| s.workspace.read(cx).editor.entity_id()),
        editor
    );
    assert_eq!(s.text(cx), "# A");
    // Live: theme, autosave, sort, leader delay.
    assert_eq!(s.pf_window_theme(cx), next);
    assert_eq!(s.autosave_interval(cx), Some(autosave + 5));
    let sort = cx.read(|cx| {
        let files = s.workspace.read(cx).files.clone().unwrap();
        files.read(cx).model().current_sort()
    });
    assert_eq!(sort, (SortField::Name, SortOrder::Descending));
    s.pf_press("ctrl-shift-b", cx);
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert!(
        cx.read(|cx| s.workspace.read(cx).leader.which_key().is_some()),
        "a zero delay shows the which-key at once"
    );
    s.pf_press("escape", cx);
    settle(&rt, cx);
}

/// Plain → vim: the vault is entered again (the note flushed and restored)
/// and the editor runs vim.
#[gpui_kit::test]
fn an_engine_change_reopens_on_the_new_engine(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_prefs(cx, &rt, &v);
    let files = s.files_id(cx);
    s.pf_section(cx, Section::Editor);
    s.pf_with_view(cx, |v, _, cx| v.select_backend(1, cx));
    s.pf_save(&rt, cx);
    drive(&rt, cx, |cx| s.text(cx) == "# A");
    assert_ne!(s.files_id(cx), files, "reopened");
    let vim = cx.read(|cx| {
        let editor = s.workspace.read(cx).editor.clone();
        editor.read(cx).editor.is_vim()
    });
    assert!(vim);
    assert_eq!(on_disk(&v).editor_backend, EditorBackendSetting::Vim);
    settle(&rt, cx);
}

/// A new server address is `PreferencesSaved`: the vault is entered again
/// (the sync restarts); a theme-only save is not.
#[gpui_kit::test]
fn a_server_change_reopens_and_a_theme_change_does_not(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_prefs(cx, &rt, &v);
    let files = s.files_id(cx);
    s.pf_pick_next_theme(cx);
    s.pf_save(&rt, cx);
    assert_eq!(s.files_id(cx), files, "a theme change: no reopen");

    s.pf_route(cx);
    s.pf_loaded(&rt, cx);
    s.pf_type_server("http://127.0.0.1:9", cx);
    s.pf_save(&rt, cx);
    assert_ne!(s.files_id(cx), files, "a server change reopens");
    drive(&rt, cx, |cx| s.text(cx) == "# A");
    let url = |settings: &AppSettings| {
        settings
            .workspace_config
            .as_ref()
            .and_then(|wc| wc.global.kimun_server_url.clone())
    };
    assert_eq!(
        url(&v.settings.read().unwrap()).as_deref(),
        Some("http://127.0.0.1:9")
    );
    assert_eq!(url(&on_disk(&v)).as_deref(), Some("http://127.0.0.1:9"));
    s.pf_on_window(cx, |w, _, _| w.rag_sync.stop());
    settle(&rt, cx);
}

/// The reopen rule itself.
#[test]
fn only_the_server_and_the_engine_need_a_reopen() {
    let before = AppSettings::default();
    let mut after = before.clone();
    after.theme = "Other".into();
    after.leader_timeout_ms = 0;
    after.autosave_interval_secs = 120;
    after.default_sort_order = SortOrderSetting::Descending;
    assert!(!needs_reopen(&before, &after));
    after.editor_backend = EditorBackendSetting::Nvim;
    assert!(!needs_reopen(&before, &after), "nvim runs the plain editor");
    after.editor_backend = EditorBackendSetting::Vim;
    assert!(needs_reopen(&before, &after));
    let mut after = before.clone();
    after
        .workspace_config
        .get_or_insert_with(kimun_notes::settings::workspace_config::WorkspaceConfig::new_empty)
        .global
        .kimun_server_url = Some("http://h:1".into());
    assert!(needs_reopen(&before, &after));
}

/// A configured nvim is shown as kept and survives a save untouched; so do
/// Nerd Fonts and mouse capture, which the desktop app does not show.
#[gpui_kit::test]
fn nvim_nerd_fonts_and_mouse_survive_a_save(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    {
        let mut settings = v.settings.write().unwrap();
        settings.editor_backend = EditorBackendSetting::Nvim;
        settings.use_nerd_fonts = !settings.use_nerd_fonts;
        let config = settings.workspace_config.get_or_insert_with(
            kimun_notes::settings::workspace_config::WorkspaceConfig::new_empty,
        );
        config.global.mouse = !config.global.mouse;
    }
    let fonts = v.settings.read().unwrap().use_nerd_fonts;
    let mouse = v.settings.read().unwrap().mouse();
    let s = with_prefs(cx, &rt, &v);
    assert_eq!(s.pf_model(cx, |m| m.backends().len()), 2, "plain and vim");
    s.pf_section(cx, Section::Editor);
    assert!(s.pf_shows(BACKEND_KEPT_ID, cx), "nvim is shown as kept");
    let next = s.pf_pick_next_theme(cx);
    s.pf_save(&rt, cx);
    for settings in [v.settings.read().unwrap().clone(), on_disk(&v)] {
        assert_eq!(settings.editor_backend, EditorBackendSetting::Nvim);
        assert_eq!(settings.use_nerd_fonts, fonts);
        assert_eq!(settings.mouse(), mouse);
        assert_eq!(settings.theme, next);
    }
    settle(&rt, cx);
}

/// Display > Reduce motion: part of the draft (dirty enables Save, toggling
/// back is clean), saved as `[gui] reduce_motion` with the rest of `[gui]`
/// kept, and applied to GPUI's switch at once, without a restart or reopen.
#[gpui_kit::test]
fn reduce_motion_is_a_draft_setting_applied_on_save(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    // Another `[gui]` key, on disk and in memory: kept by the save.
    let system = kimun_notes::settings::gui::WindowDecorations::System;
    AppSettings::update_gui_at(config(&v), |gui| gui.set_window_decorations(system)).unwrap();
    v.settings
        .write()
        .unwrap()
        .gui
        .set_window_decorations(system);
    let s = with_prefs(cx, &rt, &v);
    let files = s.files_id(cx);
    assert!(!cx.read(|cx| cx.reduce_motion()));
    s.pf_section(cx, Section::Display);
    assert!(s.pf_shows(REDUCE_MOTION_HELP_ID, cx));
    assert!(REDUCE_MOTION_HELP.contains("Not read from your system settings"));
    assert!(s.pf_disabled(SAVE_ID, cx), "clean: Save is off");

    s.pf_click(REDUCE_MOTION_ID, cx);
    assert!(s.pf_model(cx, |m| m.reduce_motion));
    assert!(!s.pf_disabled(SAVE_ID, cx), "dirty: Save is on");
    assert!(!cx.read(|cx| cx.reduce_motion()), "not applied before Save");
    s.pf_click(REDUCE_MOTION_ID, cx);
    assert!(s.pf_disabled(SAVE_ID, cx), "toggled back: clean");

    s.pf_click(REDUCE_MOTION_ID, cx);
    s.pf_save(&rt, cx);
    for settings in [v.settings.read().unwrap().clone(), on_disk(&v)] {
        assert!(settings.gui.reduce_motion());
        assert_eq!(
            settings.gui.window_decorations(),
            system,
            "the rest of [gui] is kept"
        );
    }
    assert!(cx.read(|cx| cx.reduce_motion()), "applied at once");
    assert_eq!(s.files_id(cx), files, "no reopen");
    settle(&rt, cx);
}

// ── Display > Font ──────────────────────────────────────────────────────────

/// The theme's monospace size, rounded to whole points (the stepper's start).
fn theme_font_size(cx: &TestAppContext) -> u8 {
    cx.read(|cx| f32::from(cx.theme().mono_font_size).round() as u8)
}

/// The family list as the probe would leave it.
fn families_ready(cx: &mut TestAppContext, names: &[&str]) {
    let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    cx.update(|cx| cx.set_global(MonoFamilies::Ready(names.into())));
}

/// `family` saved as `[gui] editor_font_family`, in memory and on disk.
fn saved_family(v: &TestVault, family: &str) {
    AppSettings::update_gui_at(config(v), |gui| gui.set_editor_font_family(Some(family))).unwrap();
    v.settings
        .write()
        .unwrap()
        .gui
        .set_editor_font_family(Some(family));
}

impl Started {
    /// The family dropdown's item labels and its selected label.
    fn pf_font_choices(&self, cx: &TestAppContext) -> (Vec<String>, Option<String>) {
        let view = self.pf_view(cx);
        cx.read(|cx| view.read(cx).font_choices(cx))
    }
}

/// Stepping the size dirties the draft; Save writes `editor_font_size`.
#[gpui_kit::test]
fn stepping_the_font_size_makes_the_draft_dirty_and_save_writes_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    families_ready(cx, &["Alpha Mono"]);
    let s = with_prefs(cx, &rt, &v);
    let default = theme_font_size(cx);
    s.pf_section(cx, Section::Display);
    assert!(s.pf_disabled(SAVE_ID, cx), "clean");
    s.pf_click(FONT_SIZE_UP_ID, cx);
    assert_eq!(s.pf_model(cx, |m| m.editor_font_size), Some(default + 1));
    assert!(!s.pf_disabled(SAVE_ID, cx), "dirty: Save is on");
    s.pf_save(&rt, cx);
    for settings in [v.settings.read().unwrap().clone(), on_disk(&v)] {
        assert_eq!(settings.gui.editor_font_size(), Some(default + 1));
    }
    settle(&rt, cx);
}

/// The size stepper takes the arrow keys, as the other steppers do.
#[gpui_kit::test]
fn the_font_size_stepper_takes_the_arrows(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    families_ready(cx, &[]);
    let s = with_prefs(cx, &rt, &v);
    let default = theme_font_size(cx);
    s.pf_section(cx, Section::Display);
    s.pf_click(FONT_SIZE_ID, cx);
    s.pf_press("left", cx);
    assert_eq!(s.pf_model(cx, |m| m.editor_font_size), Some(default - 1));
    settle(&rt, cx);
}

/// A family chosen in the dropdown is the draft's, and Save writes it.
#[gpui_kit::test]
fn choosing_a_family_is_written_on_save(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    families_ready(cx, &["Alpha Mono", "X"]);
    let s = with_prefs(cx, &rt, &v);
    s.pf_section(cx, Section::Display);
    // Through the dropdown's own event, as a pick in the menu sends it.
    let select = s.pf_read(cx, |v| v.font_select().clone());
    cx.update(|cx| {
        select.update(cx, |_, cx| {
            cx.emit(SelectEvent::Confirm(Some(Some("Alpha Mono".to_string()))))
        })
    });
    cx.run_until_parked();
    assert_eq!(
        s.pf_model(cx, |m| m.editor_font_family.clone()).as_deref(),
        Some("Alpha Mono")
    );
    s.pf_with_view(cx, |v, _, cx| {
        v.set_editor_font_family(Some("X".into()), cx)
    });
    assert_eq!(s.pf_font_choices(cx).1.as_deref(), Some("X"));
    s.pf_save(&rt, cx);
    for settings in [v.settings.read().unwrap().clone(), on_disk(&v)] {
        assert_eq!(settings.gui.editor_font_family().as_deref(), Some("X"));
    }
    settle(&rt, cx);
}

/// "Default (theme)" is `None`: Save removes the key.
#[gpui_kit::test]
fn choosing_default_removes_the_family_key(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    saved_family(&v, "Alpha Mono");
    families_ready(cx, &["Alpha Mono"]);
    let s = with_prefs(cx, &rt, &v);
    s.pf_section(cx, Section::Display);
    assert_eq!(s.pf_font_choices(cx).1.as_deref(), Some("Alpha Mono"));
    s.pf_with_view(cx, |v, _, cx| v.set_editor_font_family(None, cx));
    assert_eq!(s.pf_font_choices(cx).1.as_deref(), Some(FONT_DEFAULT_LABEL));
    s.pf_save(&rt, cx);
    for settings in [v.settings.read().unwrap().clone(), on_disk(&v)] {
        assert_eq!(settings.gui.editor_font_family(), None);
    }
    assert!(
        !std::fs::read_to_string(config(&v))
            .unwrap()
            .contains("editor_font_family")
    );
    settle(&rt, cx);
}

/// A saved family the system no longer has is shown, labelled, selected; a
/// save of something else keeps it.
#[gpui_kit::test]
fn a_saved_family_that_is_not_installed_is_labeled_and_kept(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    saved_family(&v, "Gone");
    families_ready(cx, &["Alpha Mono"]);
    let s = with_prefs(cx, &rt, &v);
    s.pf_section(cx, Section::Display);
    let (labels, selected) = s.pf_font_choices(cx);
    assert_eq!(
        labels,
        [FONT_DEFAULT_LABEL, "Gone (not installed)", "Alpha Mono"]
    );
    assert_eq!(selected.as_deref(), Some("Gone (not installed)"));
    assert!(s.pf_disabled(SAVE_ID, cx), "nothing changed");
    // Away and back: the entry stays, and choosing it again is clean.
    s.pf_with_view(cx, |v, _, cx| v.set_editor_font_family(None, cx));
    assert!(
        s.pf_font_choices(cx)
            .0
            .contains(&"Gone (not installed)".to_string())
    );
    s.pf_with_view(cx, |v, _, cx| {
        v.set_editor_font_family(Some("Gone".into()), cx)
    });
    assert!(s.pf_disabled(SAVE_ID, cx), "back to the saved value");
    s.pf_click(UPDATE_CHECK_ID, cx);
    s.pf_save(&rt, cx);
    for settings in [v.settings.read().unwrap().clone(), on_disk(&v)] {
        assert_eq!(settings.gui.editor_font_family().as_deref(), Some("Gone"));
    }
    settle(&rt, cx);
}

/// While the list is being read the dropdown holds the default and the
/// saved family; once it is ready, the dropdown takes the whole list.
#[gpui_kit::test]
fn the_family_list_fills_in_when_it_is_ready(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    saved_family(&v, "Fira Code");
    // The probe is under way: opening Display starts no other.
    cx.update(|cx| cx.set_global(MonoFamilies::Loading));
    let s = with_prefs(cx, &rt, &v);
    s.pf_section(cx, Section::Display);
    let (labels, selected) = s.pf_font_choices(cx);
    assert_eq!(labels, [FONT_DEFAULT_LABEL, "Fira Code"]);
    assert_eq!(selected.as_deref(), Some("Fira Code"), "never blank");
    families_ready(cx, &["Alpha Mono", "Fira Code"]);
    cx.run_until_parked();
    let (labels, selected) = s.pf_font_choices(cx);
    assert_eq!(labels, [FONT_DEFAULT_LABEL, "Alpha Mono", "Fira Code"]);
    assert_eq!(selected.as_deref(), Some("Fira Code"));
    settle(&rt, cx);
}

/// The saved family is read from the settings once, when the view loads,
/// not on every frame (a wrong-typed value would log a warning per frame):
/// the settings changed behind the view's back leave the dropdown as it was.
#[gpui_kit::test]
fn the_saved_family_is_read_once_not_per_frame(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    saved_family(&v, "Gone");
    families_ready(cx, &["Alpha Mono"]);
    let s = with_prefs(cx, &rt, &v);
    s.pf_section(cx, Section::Display);
    let before = s.pf_font_choices(cx);
    assert!(before.0.contains(&"Gone (not installed)".to_string()));
    s.pf_with_view(cx, |v, _, cx| {
        if let Some(opened) = v.opened_mut() {
            opened.gui.set_editor_font_family(Some("Elsewhere"));
        }
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(s.pf_font_choices(cx), before);
    settle(&rt, cx);
}

/// Opening Display starts the family probe.
#[gpui_kit::test]
fn opening_display_loads_the_family_list(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_prefs(cx, &rt, &v);
    assert!(matches!(cx.read(mono_families::state), MonoFamilies::Idle));
    s.pf_section(cx, Section::Display);
    assert!(!matches!(cx.read(mono_families::state), MonoFamilies::Idle));
    settle(&rt, cx);
}

/// Cancel and Discard: no font key is written, the editor font unchanged.
#[gpui_kit::test]
fn cancel_discards_the_font_draft(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    families_ready(cx, &["Alpha Mono"]);
    let s = with_prefs(cx, &rt, &v);
    let before = cx.read(|cx| cx.try_global::<EditorFontSetting>().cloned());
    s.pf_section(cx, Section::Display);
    s.pf_click(FONT_SIZE_UP_ID, cx);
    s.pf_with_view(cx, |v, _, cx| {
        v.set_editor_font_family(Some("Alpha Mono".into()), cx)
    });
    s.pf_click(CANCEL_ID, cx);
    s.pf_click(DISCARD_ID, cx);
    assert!(!s.pf_is_open(cx));
    for settings in [v.settings.read().unwrap().clone(), on_disk(&v)] {
        assert_eq!(settings.gui.editor_font_family(), None);
        assert_eq!(settings.gui.editor_font_size(), None);
    }
    assert_eq!(
        cx.read(|cx| cx.try_global::<EditorFontSetting>().cloned()),
        before
    );
    settle(&rt, cx);
}

/// Save hands the new font to the Edit surface at once.
#[gpui_kit::test]
fn saving_applies_the_font_without_a_restart(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    families_ready(cx, &["Alpha Mono"]);
    let s = with_prefs(cx, &rt, &v);
    let default = theme_font_size(cx);
    s.pf_section(cx, Section::Display);
    s.pf_click(FONT_SIZE_UP_ID, cx);
    s.pf_with_view(cx, |v, _, cx| {
        v.set_editor_font_family(Some("Alpha Mono".into()), cx)
    });
    s.pf_save(&rt, cx);
    let (_, family, size) = cx.read(editor_font::face_inputs);
    assert_eq!(family.as_deref(), Some("Alpha Mono"));
    assert_eq!(size, gpui_kit::px(f32::from(default + 1)));
    settle(&rt, cx);
}

/// A save that leaves the font alone writes neither key.
#[gpui_kit::test]
fn an_untouched_save_never_adds_the_keys(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    families_ready(cx, &["Alpha Mono"]);
    let s = with_prefs(cx, &rt, &v);
    s.pf_section(cx, Section::Display);
    s.pf_click(UPDATE_CHECK_ID, cx);
    s.pf_save(&rt, cx);
    let text = std::fs::read_to_string(config(&v)).unwrap();
    assert!(!text.contains("editor_font"), "{text}");
    let settings = v.settings.read().unwrap().clone();
    assert_eq!(settings.gui.editor_font_family(), None);
    assert_eq!(settings.gui.editor_font_size(), None);
    settle(&rt, cx);
}

/// A write that fails (the config on disk no longer parses) keeps the view
/// open with the reason, and the settings as they were.
#[gpui_kit::test]
fn a_failed_write_keeps_the_view_open(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let before = v.settings.read().unwrap().clone();
    let s = with_prefs(cx, &rt, &v);
    std::fs::write(config(&v), "this is [not toml").unwrap();
    s.pf_section(cx, Section::Display);
    s.pf_click(UPDATE_CHECK_ID, cx);
    s.pf_click(SAVE_ID, cx);
    drive(&rt, cx, |cx| !s.pf_read(cx, |v| v.saving()));
    assert!(s.pf_is_open(cx), "the view stays");
    let message = s.pf_read(cx, |v| v.message().map(str::to_string)).unwrap();
    assert!(
        message.contains("Couldn't save the preferences"),
        "{message}"
    );
    assert!(s.pf_shows(MESSAGE_ID, cx));
    assert_eq!(*v.settings.read().unwrap(), before, "nothing applied");
    assert_eq!(
        std::fs::read_to_string(config(&v)).unwrap(),
        "this is [not toml",
        "the unreadable config is left alone"
    );
    assert!(!s.pf_disabled(SAVE_ID, cx), "it can be tried again");
    settle(&rt, cx);
}

/// An address that is not http(s) is shown as wrong and cannot be saved,
/// by the button or the chord.
#[gpui_kit::test]
fn an_invalid_address_cannot_be_saved(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_prefs(cx, &rt, &v);
    s.pf_section(cx, Section::Display);
    s.pf_click(UPDATE_CHECK_ID, cx);
    s.pf_type_server("localhost:7573", cx);
    assert!(s.pf_read(cx, |v| v.server_error().is_some()));
    assert!(s.pf_shows(SERVER_ERROR_ID, cx));
    assert!(
        s.pf_disabled(SAVE_ID, cx),
        "an invalid draft cannot be saved"
    );
    s.pf_press("ctrl-s", cx);
    s.pf_press("enter", cx);
    drive(&rt, cx, |_| true);
    assert!(s.pf_is_open(cx));
    assert!(v.settings.read().unwrap().update_check(), "nothing saved");
    // Fixed: it saves (Enter in the field).
    s.pf_type_server("http://localhost:7573", cx);
    assert!(s.pf_read(cx, |v| v.server_error().is_none()));
    assert!(s.pf_shows(SERVER_ID, cx));
    s.pf_press("enter", cx);
    drive(&rt, cx, |cx| {
        !s.pf_is_open(cx) && !cx.read(|cx| s.workspace.read(cx).reopen_running())
    });
    assert!(!v.settings.read().unwrap().update_check());
    s.pf_on_window(cx, |w, _, _| w.rag_sync.stop());
    settle(&rt, cx);
}

/// Tab moves the keyboard from the section list to the next control.
#[gpui_kit::test]
fn tab_leaves_the_section_list(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = with_prefs(cx, &rt, &v);
    assert!(s.pf_focused(0, cx));
    s.pf_press("tab", cx);
    assert!(!s.pf_focused(0, cx), "Tab moved on");
    assert!(
        s.pf_focused(4, cx),
        "to the workspace list (Workspaces shows)"
    );
    s.pf_section(cx, Section::Appearance);
    s.pf_press("tab", cx);
    assert!(s.pf_focused(1, cx), "to the theme list");
    s.pf_press("down", cx);
    assert!(!s.pf_disabled(SAVE_ID, cx), "its arrows work there");
    settle(&rt, cx);
}
