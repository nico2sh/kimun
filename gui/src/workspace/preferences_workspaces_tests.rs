//! UI integration tests for Preferences > Workspaces and Indexing (M9b-2,
//! [`workspaces`]): a real `Workspace` over the two real temp vaults of
//! `reopen_tests::pair()` (workspaces `a`, current, and `b`) whose config
//! file is the one written, driven by keys and clicks.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`): the draft, the folder
//! listings, the settings writes, the reindexes and the reopens run on
//! tokio. Left undriven, a reindex stays under way (the progress panel).
//!
//! [`workspaces`]: super::preferences::workspaces
#![cfg(test)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Context, Entity, EntityId, Focusable as _, TestAppContext, Window, WindowOptions,
    px, size,
};
use kimun_core::system::SystemPath;
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::leader_route::LeaderRoute;
use kimun_notes::logic::message::AppMessage;
use kimun_notes::settings::gui::GuiSettings;
use kimun_notes::settings::themes::Theme;
use kimun_notes::settings::{AppSettings, SharedSettings};
use tokio::runtime::Runtime;

use super::folder_chooser::{self, FolderChooser, Listing};
use super::preferences::workspaces::{
    ASK_GO_ID, CONFLICT_ID, FAST_ID, FULL_ID, FULL_YES_ID, Form, INDEX_INERT_ID, PROGRESS_ID,
    WS_CHOOSE_ID, WS_CREATE_ID, WS_DELETE_YES_ID, WS_EMPTY_ID, WS_FOLDER_ID, WS_NEW_ID,
    WS_SETUP_ID, WS_SWITCH_ID, delete_question, workspace_row_id,
};
use super::preferences::{
    CANCEL_ID, PREFERENCES_ID, PreferencesView, SAVE_ID, Section, section_id,
};
use super::reopen_tests::{Pair, current, files_rows, on_disk_current, open_on_a, pair};
use super::start_tests::{Started, drive, runtime, send};
use super::{LayoutStore, Workspace};
use crate::editor_view::{self, EditorView};
use crate::keymap::{self, Keymap, Platform};
use crate::startup::Start;

/// The window on A (alpha restored, indexed), Preferences open on
/// Workspaces with the draft read.
fn on_a_with_prefs(cx: &mut TestAppContext, rt: &Runtime, p: &Pair) -> Started {
    let s = open_on_a(cx, rt, p, LayoutStore::File(p.config.clone()));
    s.pw_open(rt, cx);
    s
}

/// The temp dir both vaults live in (the config's folder).
fn root(p: &Pair) -> PathBuf {
    p.config.parent().unwrap().to_path_buf()
}

/// The settings on disk now.
fn on_disk(p: &Pair) -> AppSettings {
    AppSettings::load_from_file(p.config.clone()).expect("the config on disk loads")
}

/// Workspace `name`'s configured folder in `s`.
fn path_of(s: &AppSettings, name: &str) -> Option<PathBuf> {
    s.workspace_config
        .as_ref()?
        .workspaces
        .get(name)
        .map(|e| e.path.clone())
}

fn names(s: &AppSettings) -> Vec<String> {
    s.workspace_config
        .as_ref()
        .map(|wc| wc.workspaces.keys().cloned().collect())
        .unwrap_or_default()
}

/// A window with no vault (`OpenFailed`) over a config with no workspace.
fn no_workspace(cx: &mut TestAppContext) -> (Started, SharedSettings, PathBuf, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let config = root.join("config.toml");
    std::fs::write(
        &config,
        "config_version = 7\ncache_dir = \"cache\"\nhistory_dir = \"history\"\n",
    )
    .unwrap();
    let settings: SharedSettings = Arc::new(RwLock::new(
        AppSettings::load_from_file(config.clone()).unwrap(),
    ));
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
    });
    let layout =
        crate::shell_tree::restore(&GuiSettings::default(), crate::shell_tree::RESTORE_CAPS);
    let store = LayoutStore::File(config.clone());
    let shared = settings.clone();
    let (handle, workspace) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                let editor = cx.new(|cx| EditorView::on_channel(false, tx.clone(), cx));
                let workspace = cx.new(|cx| {
                    let mut workspace =
                        Workspace::new(editor, Theme::default(), layout, store, window, cx);
                    workspace.start(
                        Start::OpenFailed("no workspace in this test".into()),
                        shared,
                        tx,
                        rx,
                        window,
                        cx,
                    );
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
    (Started { handle, workspace }, settings, root, dir)
}

/// Let the vault's background work land while the runtime is still up.
fn settle(rt: &Runtime, cx: &mut TestAppContext) {
    let mut rounds = 0;
    drive(rt, cx, |_| {
        rounds += 1;
        rounds >= 40
    });
}

impl Started {
    fn pw_view(&self, cx: &TestAppContext) -> Entity<PreferencesView> {
        cx.read(|cx| self.workspace.read(cx).preferences.clone())
            .expect("Preferences are open")
    }

    fn pw_is_open(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).preferences.is_some())
    }

    fn pw_read<R>(&self, cx: &TestAppContext, f: impl FnOnce(&PreferencesView) -> R) -> R {
        let view = self.pw_view(cx);
        cx.read(|cx| f(view.read(cx)))
    }

    fn pw_with_view(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut PreferencesView, &mut Window, &mut Context<PreferencesView>),
    ) {
        let view = self.pw_view(cx);
        cx.update_window(self.handle, |_, window, cx| {
            view.update(cx, |v, cx| f(v, window, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// Open Preferences through the leader route, driven until read.
    fn pw_open(&self, rt: &Runtime, cx: &mut TestAppContext) {
        let workspace = self.workspace.clone();
        cx.update_window(self.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| {
                assert_eq!(
                    w.route_block(&LeaderRoute::OpenPreferences, Panel::Editor, cx),
                    None
                );
                w.perform_route(LeaderRoute::OpenPreferences, window, cx)
            })
        })
        .unwrap();
        cx.run_until_parked();
        drive(rt, cx, |cx| self.pw_read(cx, |v| v.model().is_some()));
    }

    fn pw_press(&self, key: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn pw_typed(&self, text: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.input(text, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn pw_click(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn pw_disabled(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).disabled() == Some(true)
        })
        .unwrap()
    }

    fn pw_shows(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some_and(|e| e.visible())
        })
        .unwrap()
    }

    fn pw_bounds(
        &self,
        id: impl Into<gpui_kit::ElementId>,
        cx: &mut TestAppContext,
    ) -> gpui_kit::Bounds<gpui_kit::Pixels> {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).bounds()
        })
        .unwrap()
    }

    /// Whether `handle` has the keyboard.
    fn pw_has_focus(&self, handle: gpui_kit::FocusHandle, cx: &mut TestAppContext) -> bool {
        cx.update_window(self.handle, |_, window, _| handle.is_focused(window))
            .unwrap()
    }

    fn pw_section(&self, section: Section, cx: &mut TestAppContext) {
        self.pw_click(section_id(section), cx);
        assert_eq!(self.pw_read(cx, |v| v.section()), section);
    }

    /// Give the workspace list the keyboard (Tab from the section list).
    fn pw_list_focus(&self, cx: &mut TestAppContext) {
        self.pw_with_view(cx, |v, window, cx| {
            let handle = v.handles()[4].clone();
            window.focus(&handle, cx);
        });
    }

    /// Select workspace `name` with a click on its row.
    fn pw_pick(&self, name: &str, cx: &mut TestAppContext) {
        self.pw_click(workspace_row_id(name), cx);
        assert_eq!(self.pw_read(cx, |v| v.ws_selected()).as_deref(), Some(name));
    }

    fn pw_message(&self, cx: &TestAppContext) -> Option<(String, bool)> {
        self.pw_read(cx, |v| {
            v.ws_message().map(|n| (n.text.to_string(), n.error))
        })
    }

    /// The chooser up in the section.
    fn pw_chooser(&self, cx: &TestAppContext) -> Option<Entity<FolderChooser>> {
        self.pw_read(cx, |v| v.ws_chooser())
    }

    /// Show `dir` in the chooser up, driven until listed.
    fn pw_browse(&self, rt: &Runtime, dir: &Path, cx: &mut TestAppContext) {
        let chooser = self.pw_chooser(cx).expect("the chooser is up");
        let dir = SystemPath::try_absolute(dir).unwrap();
        cx.update_window(self.handle, |_, window, cx| {
            chooser.update(cx, |c, cx| c.browse(dir, window, cx))
        })
        .unwrap();
        cx.run_until_parked();
        drive(rt, cx, |cx| {
            cx.read(|cx| !matches!(chooser.read(cx).listing(), Listing::Loading))
        });
    }

    /// Drive until no workspace write runs.
    fn pw_written(&self, rt: &Runtime, cx: &mut TestAppContext) {
        drive(rt, cx, |cx| !self.pw_read(cx, |v| v.ws_writing()));
    }

    /// Drive until Preferences are gone and the window is calm.
    fn pw_gone(&self, rt: &Runtime, cx: &mut TestAppContext) {
        drive(rt, cx, |cx| {
            cx.read(|cx| {
                let w = self.workspace.read(cx);
                w.preferences.is_none() && !w.reopen_running() && !w.status.indexing
            })
        });
    }

    fn pw_files_id(&self, cx: &TestAppContext) -> EntityId {
        cx.read(|cx| self.workspace.read(cx).files.clone().unwrap().entity_id())
    }
}

/// The New name field: its text replaced by `text`, typed.
fn type_new_name(s: &Started, text: &str, cx: &mut TestAppContext) {
    let input = s.pw_read(cx, |v| v.ws_new_input()).expect("the New form");
    cx.update_window(s.handle, |_, window, cx| {
        input.update(cx, |i, cx| i.set_value("", window, cx));
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    })
    .unwrap();
    cx.run_until_parked();
    s.pw_typed(text, cx);
}

fn new_name_error(s: &Started, cx: &TestAppContext) -> Option<String> {
    s.pw_read(cx, |v| v.ws_new_error())
}

// ── sections ────────────────────────────────────────────────────────────────

/// Workspaces opens first and Indexing sits between Sorting and Editor;
/// keys and clicks reach both; the narrow window's tab row has them too.
#[gpui_kit::test]
fn workspaces_and_indexing_sit_in_the_tui_order(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let s = on_a_with_prefs(cx, &rt, &p);
    assert_eq!(s.pw_read(cx, |v| v.section()), Section::Workspaces);
    assert_eq!(
        Section::ALL,
        [
            Section::Workspaces,
            Section::Appearance,
            Section::Display,
            Section::Sorting,
            Section::Indexing,
            Section::Editor,
            Section::Server,
        ]
    );
    for _ in 0..4 {
        s.pw_press("down", cx);
    }
    assert_eq!(s.pw_read(cx, |v| v.section()), Section::Indexing);
    assert!(s.pw_shows(FAST_ID, cx));
    s.pw_section(Section::Workspaces, cx);
    assert!(s.pw_shows(workspace_row_id("a"), cx));

    // Narrow: one row of tabs above the controls, holding both.
    cx.simulate_window_resize(s.handle, size(px(400.), px(700.)));
    cx.run_until_parked();
    let ws = s.pw_bounds(section_id(Section::Workspaces), cx);
    let appearance = s.pw_bounds(section_id(Section::Appearance), cx);
    assert_eq!(ws.origin.y, appearance.origin.y, "side by side: a tab row");
    assert!(s.pw_shows(section_id(Section::Indexing), cx));
    s.pw_section(Section::Indexing, cx);
    s.pw_section(Section::Workspaces, cx);
    settle(&rt, cx);
}

// ── the list ────────────────────────────────────────────────────────────────

/// The list shows each workspace with its folder, the current one tagged
/// and selected; Up / Down walk it; Switch is off on the current one.
#[gpui_kit::test]
fn the_list_marks_the_current_workspace_and_walks(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let s = on_a_with_prefs(cx, &rt, &p);
    let rows = s.pw_read(cx, |v| v.ws_rows());
    assert_eq!(
        rows.iter()
            .map(|r| (r.name.as_str(), r.current))
            .collect::<Vec<_>>(),
        [("a", true), ("b", false)]
    );
    assert_eq!(s.pw_read(cx, |v| v.ws_selected()).as_deref(), Some("a"));
    assert!(s.pw_disabled(WS_SWITCH_ID, cx), "not on the current one");
    // Tab from the section list reaches the list.
    s.pw_press("tab", cx);
    let list = s.pw_read(cx, |v| v.handles()[4].clone());
    assert!(s.pw_has_focus(list, cx));
    s.pw_press("down", cx);
    assert_eq!(s.pw_read(cx, |v| v.ws_selected()).as_deref(), Some("b"));
    assert!(!s.pw_disabled(WS_SWITCH_ID, cx));
    s.pw_press("down", cx);
    assert_eq!(
        s.pw_read(cx, |v| v.ws_selected()).as_deref(),
        Some("b"),
        "clamped"
    );
    s.pw_press("up", cx);
    assert_eq!(s.pw_read(cx, |v| v.ws_selected()).as_deref(), Some("a"));
    // Enter on the current one does nothing.
    s.pw_press("enter", cx);
    assert!(s.pw_is_open(cx));
    settle(&rt, cx);
}

// ── new ─────────────────────────────────────────────────────────────────────

/// `n` opens the New form: the name is checked as typed; the folder comes
/// from the shared chooser (a new folder made there through core); Create
/// registers the workspace in memory and on disk, not current, no reopen.
#[gpui_kit::test]
fn a_new_workspace_is_checked_chosen_and_saved(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let base = root(&p).join("browse");
    std::fs::create_dir_all(&base).unwrap();
    let s = on_a_with_prefs(cx, &rt, &p);
    let files = s.pw_files_id(cx);
    s.pw_list_focus(cx);
    s.pw_press("n", cx);
    let input = s.pw_read(cx, |v| v.ws_new_input()).expect("the New form");
    let field = cx.read(|cx| input.read(cx).focus_handle(cx));
    assert!(s.pw_has_focus(field, cx), "the name field has the keyboard");

    type_new_name(&s, "A", cx);
    assert_eq!(
        new_name_error(&s, cx).as_deref(),
        Some("Workspace 'a' already exists.")
    );
    type_new_name(&s, "x:y", cx);
    assert!(new_name_error(&s, cx).is_some(), "not a valid name");
    type_new_name(&s, " ", cx);
    assert_eq!(
        new_name_error(&s, cx).as_deref(),
        Some("Workspace name is empty.")
    );
    type_new_name(&s, "Third", cx);
    assert_eq!(new_name_error(&s, cx), None);
    assert!(s.pw_disabled(WS_CREATE_ID, cx), "no folder yet");

    // Enter with no folder opens the chooser (the shared one).
    s.pw_press("enter", cx);
    assert!(s.pw_chooser(cx).is_some());
    assert!(s.pw_shows("preferences-chooser", cx));
    s.pw_browse(&rt, &base, cx);
    s.pw_press("n", cx);
    s.pw_typed("third-notes", cx);
    s.pw_press("enter", cx);
    let chooser = s.pw_chooser(cx).unwrap();
    drive(&rt, cx, |cx| {
        cx.read(|cx| {
            let c = chooser.read(cx);
            c.dir().as_path() == base.join("third-notes")
                && !matches!(c.listing(), Listing::Loading)
        })
    });
    assert!(base.join("third-notes").is_dir(), "made through core");
    s.pw_press("ctrl-enter", cx);
    assert!(s.pw_chooser(cx).is_none(), "chosen: back to the form");
    assert!(s.pw_shows(WS_FOLDER_ID, cx));
    assert!(!s.pw_disabled(WS_CREATE_ID, cx));
    assert!(!names(&p.settings.read().unwrap()).contains(&"third".to_string()));

    s.pw_click(WS_CREATE_ID, cx);
    s.pw_written(&rt, cx);
    let want = base.join("third-notes");
    for settings in [p.settings.read().unwrap().clone(), on_disk(&p)] {
        assert_eq!(path_of(&settings, "third").as_deref(), Some(want.as_path()));
        assert_eq!(settings.current_workspace_name().as_deref(), Some("a"));
    }
    assert_eq!(
        s.pw_message(cx),
        Some(("Workspace 'third' created.".to_string(), false))
    );
    assert!(s.pw_is_open(cx), "Preferences stay");
    assert_eq!(s.pw_read(cx, |v| v.ws_selected()).as_deref(), Some("third"));
    assert!(!s.pw_read(cx, |v| v.is_dirty()), "the draft is not touched");
    assert_eq!(s.pw_files_id(cx), files, "no reopen");
    // Creating it again is refused at once.
    s.pw_list_focus(cx);
    s.pw_press("n", cx);
    type_new_name(&s, "third", cx);
    assert!(new_name_error(&s, cx).is_some());
    s.pw_press("escape", cx);
    assert!(
        s.pw_read(cx, |v| v.ws_form().is_none()),
        "Esc closes the form"
    );
    assert!(s.pw_is_open(cx), "and only the form");
    settle(&rt, cx);
}

/// A write that fails (the config on disk does not parse) shows why in the
/// section and registers nothing.
#[gpui_kit::test]
fn a_failed_write_registers_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let base = root(&p).join("other");
    std::fs::create_dir_all(&base).unwrap();
    let s = on_a_with_prefs(cx, &rt, &p);
    std::fs::write(&p.config, "this is [not toml").unwrap();
    s.pw_list_focus(cx);
    s.pw_press("n", cx);
    type_new_name(&s, "third", cx);
    s.pw_click(WS_CHOOSE_ID, cx);
    s.pw_browse(&rt, &base, cx);
    s.pw_press("ctrl-enter", cx);
    s.pw_click(WS_CREATE_ID, cx);
    s.pw_written(&rt, cx);
    let (text, error) = s.pw_message(cx).expect("the reason is shown");
    assert!(
        error && text.contains("Couldn't save the workspaces"),
        "{text}"
    );
    assert!(!names(&p.settings.read().unwrap()).contains(&"third".to_string()));
    assert_eq!(
        std::fs::read_to_string(&p.config).unwrap(),
        "this is [not toml"
    );
    settle(&rt, cx);
}

/// With no workspace: the empty state offers New and the guided setup; the
/// first workspace's name and folder are suggested; created, it becomes
/// the current one and the window enters it.
#[gpui_kit::test]
fn the_first_workspace_becomes_current_and_opens(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (s, settings, root, _dir) = no_workspace(cx);
    s.pw_press("f4", cx);
    drive(&rt, cx, |cx| s.pw_read(cx, |v| v.model().is_some()));
    assert!(s.pw_shows(WS_EMPTY_ID, cx));
    assert!(s.pw_shows(WS_SETUP_ID, cx));
    s.pw_click(WS_NEW_ID, cx);
    let input = s.pw_read(cx, |v| v.ws_new_input()).unwrap();
    assert_eq!(cx.read(|cx| input.read(cx).value().to_string()), "default");
    let suggested = AppSettings::default_workspace_suggestion()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap();
    let shown = cx
        .update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(WS_FOLDER_ID).label().map(str::to_string)
        })
        .unwrap();
    assert_eq!(shown.as_deref(), Some(suggested.as_str()));

    // Somewhere in the temp dir instead of the home folder.
    let notes = root.join("first-notes");
    std::fs::create_dir_all(&notes).unwrap();
    type_new_name(&s, "first", cx);
    s.pw_click(WS_CHOOSE_ID, cx);
    s.pw_browse(&rt, &notes, cx);
    s.pw_press("ctrl-enter", cx);
    s.pw_click(WS_CREATE_ID, cx);
    s.pw_gone(&rt, cx);
    assert_eq!(current(&settings).as_deref(), Some("first"));
    let disk = AppSettings::load_from_file(root.join("config.toml")).unwrap();
    assert_eq!(disk.current_workspace_name().as_deref(), Some("first"));
    assert!(
        cx.read(|cx| s.workspace.read(cx).files.is_some()),
        "the window entered it"
    );
    settle(&rt, cx);
}

/// The empty state's `Run guided setup…` closes Preferences and opens the
/// setup assistant.
#[gpui_kit::test]
fn the_empty_state_runs_the_guided_setup(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (s, _settings, _root, _dir) = no_workspace(cx);
    s.pw_press("f4", cx);
    drive(&rt, cx, |cx| s.pw_read(cx, |v| v.model().is_some()));
    s.pw_click(WS_SETUP_ID, cx);
    assert!(!s.pw_is_open(cx));
    assert!(cx.read(|cx| s.workspace.read(cx).onboarding_open()));
    settle(&rt, cx);
}

// ── rename ──────────────────────────────────────────────────────────────────

/// F2 renames in place: an empty or taken name is refused in the field; a
/// new one is saved, the current workspace follows, and so do the window
/// title and FILES' root, without a reopen.
#[gpui_kit::test]
fn rename_saves_and_the_current_workspace_follows(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let s = on_a_with_prefs(cx, &rt, &p);
    let files = s.pw_files_id(cx);
    s.pw_list_focus(cx);
    s.pw_press("f2", cx);
    let input = s.pw_read(cx, |v| v.ws_rename_input()).expect("the field");
    assert_eq!(cx.read(|cx| input.read(cx).value().to_string()), "a");
    let set = |s: &Started, text: &str, cx: &mut TestAppContext| {
        cx.update_window(s.handle, |_, window, cx| {
            input.update(cx, |i, cx| i.set_value(text.to_string(), window, cx));
            let handle = input.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        })
        .unwrap();
        cx.run_until_parked();
        s.pw_press("enter", cx);
    };
    set(&s, "  ", cx);
    assert!(s.pw_read(cx, |v| v.ws_rename_input().is_some()), "refused");
    set(&s, "b", cx);
    assert!(s.pw_read(cx, |v| v.ws_rename_input().is_some()), "taken");
    assert_eq!(current(&p.settings).as_deref(), Some("a"));
    set(&s, "Alpha", cx);
    s.pw_written(&rt, cx);
    for settings in [p.settings.read().unwrap().clone(), on_disk(&p)] {
        assert_eq!(settings.current_workspace_name().as_deref(), Some("alpha"));
        assert_eq!(names(&settings), ["alpha", "b"]);
    }
    assert_eq!(s.sw_title(cx).as_deref(), Some("alpha - kimün"));
    let root_name = cx.read(|cx| {
        let files = s.workspace.read(cx).files.clone().unwrap();
        files.read(cx).root_name().to_string()
    });
    assert_eq!(root_name, "alpha");
    assert_eq!(s.pw_files_id(cx), files, "no reopen");
    settle(&rt, cx);
}

// ── delete ──────────────────────────────────────────────────────────────────

/// The current workspace is refused with the reason; another is asked
/// about (Esc keeps it), then removed with its index and history, its notes
/// left on disk.
#[gpui_kit::test]
fn delete_refuses_the_current_one_and_removes_another(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let (index, history) = p.settings.read().unwrap().workspace_artifacts("b");
    assert!(index.path().exists() && history.path().exists());
    let s = on_a_with_prefs(cx, &rt, &p);
    s.pw_list_focus(cx);
    s.pw_press("d", cx);
    let (text, error) = s.pw_message(cx).unwrap();
    assert!(error, "{text}");
    assert_eq!(
        text,
        "Workspace 'a' is the current workspace. Switch to another one first."
    );
    assert!(s.pw_read(cx, |v| v.ws_form().is_none()), "no question");

    s.pw_press("down", cx);
    s.pw_press("delete", cx);
    assert!(s.pw_shows(WS_DELETE_YES_ID, cx));
    assert!(s.pw_read(
        cx,
        |v| matches!(v.ws_form(), Some(Form::Delete(n)) if n == "b")
    ));
    assert_eq!(
        delete_question("b"),
        "Delete workspace 'b'? Its index and history are removed; your notes stay on disk."
    );
    s.pw_press("escape", cx);
    assert!(s.pw_read(cx, |v| v.ws_form().is_none()));
    assert!(names(&p.settings.read().unwrap()).contains(&"b".to_string()));

    s.pw_press("d", cx);
    s.pw_press("enter", cx);
    s.pw_written(&rt, cx);
    for settings in [p.settings.read().unwrap().clone(), on_disk(&p)] {
        assert_eq!(names(&settings), ["a"]);
    }
    assert!(!index.path().exists(), "the index is gone");
    assert!(!history.path().exists(), "the history is gone");
    assert!(
        root(&p).join("b").join("beta.md").exists(),
        "the notes stay"
    );
    assert_eq!(
        s.pw_message(cx),
        Some(("Workspace 'b' deleted.".to_string(), false))
    );
    settle(&rt, cx);
}

/// A delete whose write fails keeps the workspace and says why.
#[gpui_kit::test]
fn a_failed_delete_keeps_the_workspace(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let s = on_a_with_prefs(cx, &rt, &p);
    std::fs::write(&p.config, "this is [not toml").unwrap();
    s.pw_pick("b", cx);
    s.pw_press("d", cx);
    s.pw_click(WS_DELETE_YES_ID, cx);
    s.pw_written(&rt, cx);
    assert!(s.pw_message(cx).unwrap().1);
    assert!(names(&p.settings.read().unwrap()).contains(&"b".to_string()));
    settle(&rt, cx);
}

// ── switch ──────────────────────────────────────────────────────────────────

/// Enter on another workspace closes Preferences and opens it.
#[gpui_kit::test]
fn switch_opens_the_other_vault(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let s = on_a_with_prefs(cx, &rt, &p);
    s.pw_pick("b", cx);
    s.pw_press("enter", cx);
    assert!(!s.pw_is_open(cx), "closed at once");
    s.pw_gone(&rt, cx);
    drive(&rt, cx, |cx| {
        s.sw_is_on("beta.md", cx) && files_rows(&s, cx).contains(&"beta.md".to_string())
    });
    assert_eq!(current(&p.settings).as_deref(), Some("b"));
    assert_eq!(on_disk_current(&p.config).as_deref(), Some("b"));
    settle(&rt, cx);
}

/// With a changed draft, Switch asks first: Esc keeps editing, Enter
/// discards and switches; the `Switch` button does the same.
#[gpui_kit::test]
fn a_dirty_draft_asks_before_switching(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let s = on_a_with_prefs(cx, &rt, &p);
    s.pw_section(Section::Display, cx);
    s.pw_click(super::preferences::UPDATE_CHECK_ID, cx);
    assert!(s.pw_read(cx, |v| v.is_dirty()));
    s.pw_section(Section::Workspaces, cx);
    s.pw_pick("b", cx);
    s.pw_press("enter", cx);
    assert!(s.pw_read(cx, |v| v.ws_ask().is_some()), "it asks");
    assert!(s.pw_shows(ASK_GO_ID, cx));
    let root = s.pw_read(cx, |v| v.root_handle());
    assert!(s.pw_has_focus(root, cx), "the question has the keyboard");
    s.pw_press("escape", cx);
    assert!(s.pw_read(cx, |v| v.ws_ask().is_none()));
    assert!(s.pw_is_open(cx));
    assert_eq!(current(&p.settings).as_deref(), Some("a"));

    s.pw_click(WS_SWITCH_ID, cx);
    assert!(s.pw_read(cx, |v| v.ws_ask().is_some()));
    s.pw_press("enter", cx);
    s.pw_gone(&rt, cx);
    drive(&rt, cx, |cx| s.sw_is_on("beta.md", cx));
    assert_eq!(current(&p.settings).as_deref(), Some("b"));
    assert!(
        p.settings.read().unwrap().update_check(),
        "the draft was dropped"
    );
    settle(&rt, cx);
}

/// A switch the window refuses (it is closing) flashes, as the switcher's.
#[gpui_kit::test]
fn a_refused_switch_flashes(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let s = on_a_with_prefs(cx, &rt, &p);
    s.pw_pick("b", cx);
    s.workspace
        .update(cx, |w, _| w.closing = super::Closing::Flushing);
    s.pw_press("enter", cx);
    drive(&rt, cx, |_| true);
    assert_eq!(current(&p.settings).as_deref(), Some("a"));
    let flash = s.flash(cx).unwrap();
    assert!(flash.starts_with("Couldn't switch to b:"), "{flash}");
    s.workspace
        .update(cx, |w, _| w.closing = super::Closing::Open);
    settle(&rt, cx);
}

// ── change path ─────────────────────────────────────────────────────────────

/// Another workspace's folder is saved at once: no reindex, no reopen.
#[gpui_kit::test]
fn another_workspaces_path_is_saved_at_once(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let moved = root(&p).join("b2");
    std::fs::create_dir_all(&moved).unwrap();
    let s = on_a_with_prefs(cx, &rt, &p);
    let files = s.pw_files_id(cx);
    s.pw_pick("b", cx);
    s.pw_press("b", cx);
    assert!(s.pw_read(cx, |v| v.ws_path_chooser().is_some()));
    s.pw_browse(&rt, &moved, cx);
    s.pw_press("ctrl-enter", cx);
    assert!(!s.pw_read(cx, |v| v.reindexing()), "no reindex");
    s.pw_written(&rt, cx);
    for settings in [p.settings.read().unwrap().clone(), on_disk(&p)] {
        assert_eq!(path_of(&settings, "b").as_deref(), Some(moved.as_path()));
    }
    assert!(s.pw_is_open(cx));
    assert_eq!(s.pw_files_id(cx), files, "no reopen");
    settle(&rt, cx);
}

/// The current workspace's new folder: indexed first behind the progress
/// panel (everything waits, Esc too), then saved, and the window enters it.
#[gpui_kit::test]
fn the_current_workspaces_path_reindexes_then_reopens(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let moved = root(&p).join("a2");
    std::fs::create_dir_all(&moved).unwrap();
    std::fs::write(moved.join("moved.md"), "# Moved\n").unwrap();
    let s = on_a_with_prefs(cx, &rt, &p);
    s.pw_list_focus(cx);
    s.pw_press("b", cx);
    s.pw_browse(&rt, &moved, cx);
    s.pw_press("ctrl-enter", cx);
    // The reindex runs on tokio, not driven yet.
    assert!(s.pw_read(cx, |v| v.reindexing()));
    assert!(s.pw_shows(PROGRESS_ID, cx));
    assert!(s.pw_disabled(CANCEL_ID, cx) && s.pw_disabled(SAVE_ID, cx));
    s.pw_press("escape", cx);
    assert!(s.pw_is_open(cx), "Esc waits too");
    assert_eq!(
        path_of(&p.settings.read().unwrap(), "a").as_deref(),
        Some(p.a.as_path()),
        "nothing saved before the index is built"
    );
    s.pw_gone(&rt, cx);
    drive(&rt, cx, |cx| {
        cx.read(|cx| s.workspace.read(cx).files.is_some())
            && files_rows(&s, cx).contains(&"moved.md".to_string())
    });
    for settings in [p.settings.read().unwrap().clone(), on_disk(&p)] {
        assert_eq!(path_of(&settings, "a").as_deref(), Some(moved.as_path()));
    }
    assert!(
        !p.settings.read().unwrap().needs_indexing(),
        "reported indexed"
    );
    settle(&rt, cx);
}

/// A new folder whose files differ only by case: the conflict panel, and
/// nothing changes (settings, disk, the window's vault).
#[gpui_kit::test]
fn a_case_conflict_changes_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let clash = root(&p).join("clash");
    std::fs::create_dir_all(&clash).unwrap();
    std::fs::write(clash.join("Note.md"), "one").unwrap();
    std::fs::write(clash.join("note.md"), "two").unwrap();
    if std::fs::read_dir(&clash).unwrap().count() < 2 {
        // A case-insensitive filesystem cannot hold the clash.
        return;
    }
    let file = std::fs::read_to_string(&p.config).unwrap();
    let s = on_a_with_prefs(cx, &rt, &p);
    s.pw_list_focus(cx);
    s.pw_press("b", cx);
    s.pw_browse(&rt, &clash, cx);
    s.pw_press("ctrl-enter", cx);
    drive(&rt, cx, |cx| !s.pw_read(cx, |v| v.reindexing()));
    let conflict = s.pw_read(cx, |v| v.conflict().map(str::to_string)).unwrap();
    assert!(conflict.to_lowercase().contains("note.md"), "{conflict}");
    assert!(s.pw_shows(CONFLICT_ID, cx));
    assert!(s.pw_is_open(cx));
    assert_eq!(
        path_of(&p.settings.read().unwrap(), "a").as_deref(),
        Some(p.a.as_path())
    );
    assert_eq!(std::fs::read_to_string(&p.config).unwrap(), file);
    assert!(s.sw_is_on("alpha.md", cx), "still on A");
    s.pw_click(super::preferences::workspaces::CONFLICT_DISMISS_ID, cx);
    assert!(!s.pw_shows(CONFLICT_ID, cx));
    settle(&rt, cx);
}

/// A new folder the reindex cannot open: the reason, nothing changed.
#[gpui_kit::test]
fn a_failed_reindex_changes_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let s = on_a_with_prefs(cx, &rt, &p);
    s.pw_list_focus(cx);
    s.pw_press("b", cx);
    // A folder that is not there (the chooser shows its listing error and
    // can still take it).
    s.pw_browse(&rt, &root(&p).join("missing"), cx);
    s.pw_press("ctrl-enter", cx);
    drive(&rt, cx, |cx| !s.pw_read(cx, |v| v.reindexing()));
    let (text, error) = s.pw_message(cx).expect("the reason");
    assert!(
        error && text.starts_with("Couldn't index the workspace"),
        "{text}"
    );
    assert_eq!(
        path_of(&p.settings.read().unwrap(), "a").as_deref(),
        Some(p.a.as_path())
    );
    assert_eq!(path_of(&on_disk(&p), "a").as_deref(), Some(p.a.as_path()));
    settle(&rt, cx);
}

/// A Save whose vault moved under the draft takes the same flow: indexed,
/// then written and entered. One whose vault is gone says "No workspace
/// set".
#[gpui_kit::test]
fn a_save_that_needs_a_reindex_takes_the_flow(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let moved = root(&p).join("a3");
    std::fs::create_dir_all(&moved).unwrap();
    std::fs::write(moved.join("third.md"), "# Third\n").unwrap();
    let s = on_a_with_prefs(cx, &rt, &p);
    let updates = p.settings.read().unwrap().update_check();
    // The vault moved in memory since the draft was read.
    p.settings
        .write()
        .unwrap()
        .set_workspace_path("a", moved.clone());
    s.pw_section(Section::Display, cx);
    s.pw_click(super::preferences::UPDATE_CHECK_ID, cx);
    s.pw_click(SAVE_ID, cx);
    assert!(s.pw_read(cx, |v| v.reindexing()), "indexed first");
    s.pw_gone(&rt, cx);
    drive(&rt, cx, |cx| {
        cx.read(|cx| s.workspace.read(cx).files.is_some())
            && files_rows(&s, cx).contains(&"third.md".to_string())
    });
    let disk = on_disk(&p);
    assert_eq!(path_of(&disk, "a").as_deref(), Some(moved.as_path()));
    assert_eq!(disk.update_check(), !updates);

    // No workspace at all any more: said, nothing saved.
    s.pw_open(&rt, cx);
    p.settings
        .write()
        .unwrap()
        .workspace_config
        .as_mut()
        .unwrap()
        .global
        .current_workspace = String::new();
    s.pw_section(Section::Display, cx);
    s.pw_click(super::preferences::UPDATE_CHECK_ID, cx);
    s.pw_click(SAVE_ID, cx);
    assert_eq!(
        s.pw_read(cx, |v| v.message().map(str::to_string))
            .as_deref(),
        Some("No workspace set")
    );
    assert!(s.pw_is_open(cx));
    settle(&rt, cx);
}

// ── indexing ────────────────────────────────────────────────────────────────

/// Fast reindex runs and reports, and FILES lists a note written outside
/// kimün; Full asks first (Esc does nothing) and reports too. One at a
/// time; the window's close waits for it.
#[gpui_kit::test]
fn fast_and_full_reindex_run_and_report(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let s = on_a_with_prefs(cx, &rt, &p);
    let files = s.pw_files_id(cx);
    std::fs::write(p.a.join("outside.md"), "# Outside\n").unwrap();
    s.pw_section(Section::Indexing, cx);
    assert!(!s.pw_disabled(FAST_ID, cx));
    s.pw_click(FAST_ID, cx);
    assert!(s.pw_read(cx, |v| v.reindexing()));
    assert!(s.pw_shows(PROGRESS_ID, cx));
    // One at a time, and the window's close waits.
    s.pw_with_view(cx, |v, window, cx| {
        v.full_reindex(window, cx);
        v.fast_reindex(window, cx);
    });
    assert!(!s.pw_read(cx, |v| v.confirm_full()), "nothing else starts");
    assert!(!s.pw_on_close(cx), "the close waits for it");
    assert!(s.pw_read(cx, |v| v.message().is_some()), "and says so");
    drive(&rt, cx, |cx| !s.pw_read(cx, |v| v.reindexing()));
    let message = s.pw_read(cx, |v| v.index_message().cloned()).unwrap();
    assert!(
        !message.error && message.text.starts_with("Indexed in"),
        "{message:?}"
    );
    drive(&rt, cx, |cx| {
        files_rows(&s, cx).contains(&"outside.md".to_string())
    });
    assert_eq!(s.pw_files_id(cx), files, "picked up in place, no reopen");

    s.pw_click(FULL_ID, cx);
    assert!(s.pw_read(cx, |v| v.confirm_full()));
    assert!(s.pw_shows(FULL_YES_ID, cx));
    s.pw_press("escape", cx);
    assert!(!s.pw_read(cx, |v| v.confirm_full()));
    assert!(!s.pw_read(cx, |v| v.reindexing()), "cancel does nothing");
    assert!(s.pw_is_open(cx));
    s.pw_click(FULL_ID, cx);
    s.pw_press("enter", cx);
    assert!(s.pw_read(cx, |v| v.reindexing()));
    drive(&rt, cx, |cx| !s.pw_read(cx, |v| v.reindexing()));
    let message = s.pw_read(cx, |v| v.index_message().cloned()).unwrap();
    assert!(message.text.starts_with("Indexed in"), "{message:?}");
    assert!(s.pw_on_close(cx), "done: the close may go on");
    settle(&rt, cx);
}

/// Without a workspace the Indexing buttons are off and say why.
#[gpui_kit::test]
fn indexing_is_inert_without_a_workspace(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (s, _settings, _root, _dir) = no_workspace(cx);
    s.pw_press("f4", cx);
    drive(&rt, cx, |cx| s.pw_read(cx, |v| v.model().is_some()));
    s.pw_section(Section::Indexing, cx);
    assert!(s.pw_disabled(FAST_ID, cx) && s.pw_disabled(FULL_ID, cx));
    assert!(s.pw_shows(INDEX_INERT_ID, cx), "says why");
    s.pw_with_view(cx, |v, window, cx| v.fast_reindex(window, cx));
    assert!(!s.pw_read(cx, |v| v.reindexing()));
    settle(&rt, cx);
}

// ── VaultConflict ───────────────────────────────────────────────────────────

/// A vault that opens with a case conflict: Preferences > Workspaces with
/// the vault's message, the workspace config kept, and another workspace
/// can be picked from there.
#[gpui_kit::test]
fn a_vault_conflict_lands_in_workspaces(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let s = open_on_a(cx, &rt, &p, LayoutStore::File(p.config.clone()));
    let text = "Case-sensitivity conflicts detected in vault:\nx.md / X.md";
    send(&s, AppMessage::VaultConflict(text.into()), cx);
    assert!(s.pw_is_open(cx));
    assert!(s.shows(PREFERENCES_ID, cx));
    drive(&rt, cx, |cx| s.pw_read(cx, |v| v.model().is_some()));
    assert_eq!(s.pw_read(cx, |v| v.section()), Section::Workspaces);
    assert_eq!(
        s.pw_read(cx, |v| v.conflict().map(str::to_string))
            .as_deref(),
        Some(text)
    );
    assert!(s.pw_shows(CONFLICT_ID, cx));
    // Not cleared: memory and disk still name `a`, registered.
    for settings in [p.settings.read().unwrap().clone(), on_disk(&p)] {
        assert_eq!(settings.current_workspace_name().as_deref(), Some("a"));
        assert_eq!(names(&settings), ["a", "b"]);
    }
    s.pw_pick("b", cx);
    s.pw_press("enter", cx);
    s.pw_gone(&rt, cx);
    drive(&rt, cx, |cx| s.sw_is_on("beta.md", cx));
    assert_eq!(current(&p.settings).as_deref(), Some("b"));
    settle(&rt, cx);
}

// ── keys ────────────────────────────────────────────────────────────────────

/// Esc never saves here either: with a form up it closes the form, then
/// (clean) Preferences; the delete question is answered with the keys.
#[gpui_kit::test]
fn esc_closes_the_topmost_layer_and_never_saves(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let file = std::fs::read_to_string(&p.config).unwrap();
    let s = on_a_with_prefs(cx, &rt, &p);
    s.pw_pick("b", cx);
    s.pw_press("b", cx);
    assert!(s.pw_read(cx, |v| v.ws_path_chooser().is_some()));
    s.pw_press("escape", cx);
    assert!(s.pw_read(cx, |v| v.ws_form().is_none()), "the chooser goes");
    assert!(s.pw_is_open(cx));
    s.pw_press("r", cx);
    assert!(s.pw_read(cx, |v| v.ws_rename_input().is_some()));
    s.pw_press("escape", cx);
    assert!(s.pw_read(cx, |v| v.ws_form().is_none()), "the field goes");
    s.pw_press("escape", cx);
    assert!(!s.pw_is_open(cx), "then Preferences (clean)");
    assert_eq!(std::fs::read_to_string(&p.config).unwrap(), file);
    settle(&rt, cx);
}

/// The shared chooser keeps its ids under each host's prefix.
#[test]
fn the_chooser_ids_carry_the_hosts_prefix() {
    let dir = SystemPath::try_absolute(std::env::temp_dir().join("notes")).unwrap();
    assert_eq!(
        folder_chooser::row_id("preferences", &dir).as_ref(),
        "preferences-dir-notes"
    );
    assert_eq!(
        super::onboarding::chooser_row_id(&dir).as_ref(),
        "onboarding-dir-notes"
    );
}

impl Started {
    /// The window's close as the platform asks it: whether it may go on.
    fn pw_on_close(&self, cx: &mut TestAppContext) -> bool {
        let workspace = self.workspace.clone();
        let r = cx
            .update_window(self.handle, |_, _, cx| {
                workspace.update(cx, |w, cx| w.preferences_allow_close(cx))
            })
            .unwrap();
        cx.run_until_parked();
        r
    }
}
