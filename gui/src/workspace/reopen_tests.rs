//! UI integration tests for the vault change at runtime (M8b, [`reopen`]):
//! a real `Workspace` on two real temp vaults, A and B, named in one temp
//! config, switched between through the shared settings.
//!
//! Like the startup tests, each test owns a current-thread tokio runtime and
//! drives it between GPUI's `run_until_parked` (see `start_tests::drive`).
//!
//! [`reopen`]: super::reopen
#![cfg(test)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, EntityId, TestAppContext, VisualTestContext, WindowOptions};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::layout::Side;
use kimun_notes::logic::message::{AppMessage, AppTx};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::startup::{VaultOpen, try_open_vault};
use kimun_notes::rag::RagStatus;
use kimun_notes::settings::gui::GuiSettings;
use kimun_notes::settings::themes::Theme;
use kimun_notes::settings::{AppSettings, SharedSettings};
use tokio::runtime::Runtime;

use super::modal::ModalKind;
use super::start_tests::{Started, drive, runtime, send};
use super::{EMPTY_STATE_ID, LayoutStore, Workspace};
use crate::editor_view::{self, EditorView};
use crate::keymap::{self, Keymap, Platform};
use crate::startup::{self, FirstNote, Start, VaultStart};

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

/// A's notes: `alpha` (open first), linked from `afriend`, labelled `atag`.
const A_NOTES: &[(&str, &str)] = &[
    ("alpha.md", "# Alpha\nthe alpha note #atag"),
    ("afriend.md", "# Afriend\nsee [[alpha]]"),
];
/// B's notes: `beta` (open first), linked from `bfriend`, labelled `btag`.
const B_NOTES: &[(&str, &str)] = &[
    ("beta.md", "# Beta\nthe beta note #btag"),
    ("bfriend.md", "# Bfriend\nsee [[beta]]"),
];
pub(super) const ALPHA: &str = "# Alpha\nthe alpha note #atag";
pub(super) const BETA: &str = "# Beta\nthe beta note #btag";

/// Two workspaces, `a` and `b`, in one config under a temp dir (history and
/// index there too), each holding its notes, each with its first note as
/// its history. The settings name `a`.
pub(super) struct Pair {
    pub(super) settings: SharedSettings,
    pub(super) config: PathBuf,
    pub(super) a: PathBuf,
    _dir: tempfile::TempDir,
}

/// Name `name` the current workspace in `settings` (in memory).
pub(super) fn set_current(settings: &SharedSettings, name: &str) {
    settings
        .write()
        .unwrap()
        .workspace_config
        .as_mut()
        .unwrap()
        .global
        .current_workspace = name.into();
}

pub(super) fn current(settings: &SharedSettings) -> Option<String> {
    settings.read().unwrap().current_workspace_name()
}

/// Fill the current workspace with `notes`, its history `[first]`.
async fn fill(settings: &SharedSettings, notes: &[(&str, &str)]) {
    let VaultOpen::Opened(vault) = try_open_vault(settings).await else {
        panic!("the test vault opens");
    };
    for (p, text) in notes {
        vault.create_note(&path(p), *text).await.unwrap();
    }
    settings.read().unwrap().edit_path_history(|paths| {
        *paths = vec![path(notes[0].0)];
        true
    });
}

pub(super) async fn pair() -> Pair {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let (a, b) = (root.join("a"), root.join("b"));
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let toml_path = |p: &Path| toml::Value::String(p.to_string_lossy().into_owned());
    let config = root.join("config.toml");
    std::fs::write(
        &config,
        format!(
            "config_version = 7\n\
             cache_dir = \"cache\"\n\
             history_dir = \"history\"\n\
             [global]\n\
             current_workspace = \"b\"\n\
             [workspaces.a]\n\
             path = {}\n\
             created = \"2026-01-01T00:00:00Z\"\n\
             [workspaces.b]\n\
             path = {}\n\
             created = \"2026-01-01T00:00:00Z\"\n",
            toml_path(&a),
            toml_path(&b),
        ),
    )
    .unwrap();
    let settings: SharedSettings = Arc::new(RwLock::new(
        AppSettings::load_from_file(config.clone()).unwrap(),
    ));
    fill(&settings, B_NOTES).await;
    set_current(&settings, "a");
    fill(&settings, A_NOTES).await;
    // On disk too: the config names `a`.
    settings.read().unwrap().save_to_disk().unwrap();
    Pair {
        settings,
        config,
        a,
        _dir: dir,
    }
}

/// The window as `main` builds it, on what the settings name now (as
/// `main` decides it), persisting to `store`.
pub(super) fn open(cx: &mut TestAppContext, rt: &Runtime, p: &Pair, store: LayoutStore) -> Started {
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
    });
    let name = current(&p.settings);
    let opened = rt.block_on(try_open_vault(&p.settings));
    let start = startup::decide(opened, name, None, None);
    let settings = p.settings.clone();
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

/// [`open`] on A, driven until alpha is restored and the index built.
pub(super) fn open_on_a(
    cx: &mut TestAppContext,
    rt: &Runtime,
    p: &Pair,
    store: LayoutStore,
) -> Started {
    let started = open(cx, rt, p, store);
    drive(rt, cx, |cx| {
        started.text(cx) == ALPHA && !started.indexing(cx)
    });
    started
}

impl Started {
    pub(super) fn sw_open_note(&self, cx: &TestAppContext) -> Option<VaultPath> {
        cx.read(|cx| self.workspace.read(cx).notes.as_ref()?.note.clone())
    }

    pub(super) fn sw_is_on(&self, p: &str, cx: &TestAppContext) -> bool {
        self.sw_open_note(cx).is_some_and(|n| n.is_like(&path(p)))
    }

    pub(super) fn sw_reopening(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).reopen_running())
    }

    fn sw_tx(&self, cx: &TestAppContext) -> AppTx {
        cx.read(|cx| self.workspace.read(cx).tx.clone().unwrap())
    }

    pub(super) fn sw_title(&self, cx: &mut TestAppContext) -> Option<String> {
        VisualTestContext::from_window(self.handle, cx).window_title()
    }

    pub(super) fn sw_typed(&self, text: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.input(text, cx))
            .unwrap();
        cx.run_until_parked();
    }

    pub(super) fn sw_key(&self, key: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn sw_reopen(&self, cx: &mut TestAppContext) {
        self.workspace.update(cx, |w, cx| w.reopen(cx));
    }

    /// Switch to the workspace the settings name now, driven until it
    /// landed (and its first note, when `text`).
    pub(super) fn sw_reopen_to(&self, rt: &Runtime, text: Option<&str>, cx: &mut TestAppContext) {
        self.sw_reopen(cx);
        drive(rt, cx, |cx| {
            !self.sw_reopening(cx) && !self.indexing(cx) && text.is_none_or(|t| self.text(cx) == t)
        });
    }

    /// Show `view` in the slot where it is placed, as the rail does.
    fn sw_reveal(&self, view: DrawerView, cx: &mut TestAppContext) {
        self.workspace.update(cx, |w, cx| {
            w.open_view(view);
            w.refresh_views(cx);
        });
        cx.run_until_parked();
    }

    /// The ids of every entity the vault's views are: none outlives a
    /// vault change.
    fn sw_view_ids(&self, cx: &TestAppContext) -> Vec<EntityId> {
        cx.read(|cx| {
            let w = self.workspace.read(cx);
            vec![
                w.files.as_ref().unwrap().entity_id(),
                w.links.as_ref().unwrap().entity_id(),
                w.outline.as_ref().unwrap().entity_id(),
                w.find.as_ref().unwrap().entity_id(),
                w.tags.as_ref().unwrap().entity_id(),
                w.semantic.as_ref().unwrap().entity_id(),
                w.ask.as_ref().unwrap().entity_id(),
                w.ask_sources.as_ref().unwrap().entity_id(),
            ]
        })
    }

    fn sw_subscriptions(&self, cx: &TestAppContext) -> usize {
        cx.read(|cx| self.workspace.read(cx)._views_events.len())
    }

    fn sw_flashed(&self, text: &str, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .flashes
                .iter()
                .any(|(t, _)| t.contains(text))
        })
    }
}

/// FILES' rows (file names, folders `name/`).
pub(super) fn files_rows(s: &Started, cx: &TestAppContext) -> Vec<String> {
    use kimun_notes::logic::files::FileListEntry;
    cx.read(|cx| {
        let files = s.workspace.read(cx).files.clone().unwrap();
        files
            .read(cx)
            .model()
            .core()
            .map(|core| {
                core.visible_rows()
                    .into_iter()
                    .map(|row| match row {
                        FileListEntry::Up { .. } => "..".to_string(),
                        FileListEntry::Directory { name, .. } => format!("{name}/"),
                        FileListEntry::Note { filename, .. }
                        | FileListEntry::Attachment { filename, .. } => filename.clone(),
                        FileListEntry::CreateNote { filename, .. } => format!("+{filename}"),
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// The rows of a drawer list once they landed, by `name`.
fn list_rows<R: super::drawer_list::DrawerRow>(
    list: Entity<super::drawer_list::DrawerList<R>>,
    rt: &Runtime,
    cx: &mut TestAppContext,
    name: impl Fn(&R) -> String,
) -> Vec<String> {
    drive(rt, cx, |cx| !cx.read(|cx| list.read(cx).is_loading()));
    let mut rows: Vec<String> = cx.read(|cx| list.read(cx).rows().iter().map(&name).collect());
    rows.sort();
    rows
}

fn tag_rows(s: &Started, rt: &Runtime, cx: &mut TestAppContext) -> Vec<String> {
    let list = cx.read(|cx| {
        let tags = s.workspace.read(cx).tags.clone().unwrap();
        tags.read(cx).list.clone()
    });
    list_rows(list, rt, cx, |r| r.label.clone())
}

fn link_rows(s: &Started, rt: &Runtime, cx: &mut TestAppContext) -> Vec<String> {
    let list = cx.read(|cx| {
        let links = s.workspace.read(cx).links.clone().unwrap();
        links.read(cx).list.clone()
    });
    list_rows(list, rt, cx, |r| r.path.get_clean_name())
}

fn outline_rows(s: &Started, cx: &TestAppContext) -> Vec<String> {
    cx.read(|cx| {
        let outline = s.workspace.read(cx).outline.clone().unwrap();
        let list = outline.read(cx).list.clone();
        list.read(cx)
            .rows()
            .iter()
            .map(|r| r.heading().to_string())
            .collect()
    })
}

/// Run `query` in FIND and return its result names.
fn find_rows(s: &Started, rt: &Runtime, query: &str, cx: &mut TestAppContext) -> Vec<String> {
    cx.update_window(s.handle, |_, window, cx| {
        s.workspace
            .update(cx, |w, cx| w.open_find_with_query(query.into(), window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let list = cx.read(|cx| {
        let find = s.workspace.read(cx).find.clone().unwrap();
        find.read(cx).list.clone()
    });
    list_rows(list, rt, cx, |r| r.entry.path.get_clean_name())
}

/// The config file as written on disk.
fn on_disk(config: &Path) -> Option<toml::Value> {
    let body = std::fs::read_to_string(config).ok()?;
    toml::from_str(&body).ok()
}

/// The config file's current workspace as written on disk.
pub(super) fn on_disk_current(config: &Path) -> Option<String> {
    on_disk(config)?
        .get("global")?
        .get("current_workspace")?
        .as_str()
        .map(str::to_string)
}

/// The config file's `[gui]` table as written on disk.
fn on_disk_gui(config: &Path) -> GuiSettings {
    on_disk(config)
        .and_then(|v| v.get("gui").cloned())
        .map(|gui| gui.try_into().unwrap())
        .unwrap_or_default()
}

// ── A → B ────────────────────────────────────────────────────────────────

/// A→B: every view lists B only, the editor holds B's restored note, the
/// title names B; nothing of A is left (the views are new entities).
#[gpui_kit::test]
fn a_switch_leaves_nothing_of_the_old_vault(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let started = open_on_a(cx, &rt, &p, LayoutStore::Off);
    assert_eq!(started.sw_title(cx).as_deref(), Some("a - kimün"));
    // A's views, each shown once so it has rows.
    started.sw_reveal(DrawerView::Tags, cx);
    assert_eq!(tag_rows(&started, &rt, cx), ["atag"]);
    started.sw_reveal(DrawerView::Links, cx);
    assert_eq!(link_rows(&started, &rt, cx), ["afriend"]);
    assert_eq!(find_rows(&started, &rt, "#atag", cx), ["alpha"]);
    let before = started.sw_view_ids(cx);

    set_current(&p.settings, "b");
    started.sw_reopen_to(&rt, Some(BETA), cx);

    assert!(started.sw_is_on("beta.md", cx));
    assert_eq!(
        cx.read(|cx| started.workspace.read(cx).status.note.clone()),
        Some(path("beta.md"))
    );
    assert_eq!(started.sw_title(cx).as_deref(), Some("b - kimün"));
    let after = started.sw_view_ids(cx);
    for id in &before {
        assert!(!after.contains(id), "a view of A survived the switch");
    }
    started.sw_reveal(DrawerView::Files, cx);
    drive(&rt, cx, |cx| !files_rows(&started, cx).is_empty());
    let files = files_rows(&started, cx);
    assert!(files.contains(&"beta.md".to_string()), "{files:?}");
    assert!(!files.iter().any(|r| r.contains("alpha")), "{files:?}");
    // FIND is new (no query of A's) and searches B.
    let query = cx.read(|cx| {
        let find = started.workspace.read(cx).find.clone().unwrap();
        find.read(cx).query(cx)
    });
    assert_ne!(query, "#atag");
    assert!(find_rows(&started, &rt, "#atag", cx).is_empty());
    assert_eq!(find_rows(&started, &rt, "#btag", cx), ["beta"]);
    started.sw_reveal(DrawerView::Tags, cx);
    assert_eq!(tag_rows(&started, &rt, cx), ["btag"]);
    started.sw_reveal(DrawerView::Links, cx);
    assert_eq!(link_rows(&started, &rt, cx), ["bfriend"]);
    started.sw_reveal(DrawerView::Outline, cx);
    drive(&rt, cx, |cx| outline_rows(&started, cx) == ["Beta"]);
    let sem_rows = cx.read(|cx| {
        let sem = started.workspace.read(cx).semantic.clone().unwrap();
        sem.read(cx).list.read(cx).rows().len()
    });
    assert_eq!(sem_rows, 0);
}

/// A dirty note in A is written to A's file before B is entered.
#[gpui_kit::test]
fn a_dirty_note_is_flushed_to_its_vault_first(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let started = open_on_a(cx, &rt, &p, LayoutStore::Off);
    started.sw_typed("X", cx);
    set_current(&p.settings, "b");
    started.sw_reopen_to(&rt, Some(BETA), cx);
    let on_disk = std::fs::read_to_string(p.a.join("alpha.md")).unwrap();
    assert_eq!(on_disk, format!("X{ALPHA}"));
    assert!(started.flash(cx).is_none(), "{:?}", started.flash(cx));
}

/// A flush that fails keeps the window on A, fully working: its note, its
/// edits, its views; an error flash says why.
#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_flush_stays_on_the_old_vault(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let started = open_on_a(cx, &rt, &p, LayoutStore::Off);
    let views = started.sw_view_ids(cx);
    let file = p.a.join("alpha.md");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
    started.sw_typed("X", cx);
    set_current(&p.settings, "b");
    started.sw_reopen(cx);
    drive(&rt, cx, |cx| !started.sw_reopening(cx));
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

    let flash = started.flash(cx).unwrap();
    assert!(
        flash.starts_with("Couldn't save alpha.md:")
            && flash.ends_with("The workspace was not changed."),
        "{flash}"
    );
    assert!(cx.read(|cx| started.workspace.read(cx).last_flash().unwrap().1));
    assert!(started.sw_is_on("alpha.md", cx));
    assert_eq!(started.text(cx), format!("X{ALPHA}"));
    assert_eq!(started.sw_view_ids(cx), views, "nothing was torn down");
    assert_eq!(started.sw_title(cx).as_deref(), Some("a - kimün"));
    // Still working: typing goes in, and it saves.
    started.sw_typed("Y", cx);
    assert_eq!(started.text(cx), format!("XY{ALPHA}"));
    started.sw_key("ctrl-s", cx);
    drive(&rt, cx, |_| {
        std::fs::read_to_string(&file).unwrap() == format!("XY{ALPHA}")
    });
}

// ── no workspace ─────────────────────────────────────────────────────────

/// A→none: the empty state for no workspace, no vault, no views; none→A
/// (the onboarding's way in) lands on A's restored note.
#[gpui_kit::test]
fn a_switch_to_no_workspace_and_back(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let started = open_on_a(cx, &rt, &p, LayoutStore::Off);

    set_current(&p.settings, "");
    started.sw_reopen_to(&rt, None, cx);
    // No workspace: the setup assistant takes the window (M8c), over the
    // empty state.
    assert!(started.shows(super::onboarding::ONBOARDING_ID, cx));
    assert!(!started.shows("editor", cx));
    let empty = cx.read(|cx| started.workspace.read(cx).empty.clone().unwrap());
    assert_eq!(empty, startup::empty_state(&Start::NoWorkspace).unwrap());
    cx.read(|cx| {
        let w = started.workspace.read(cx);
        assert!(w.vault().is_none());
        assert!(w.files.is_none() && w.find.is_none() && w.ask.is_none());
        assert!(w._views_events.is_empty());
    });
    assert_eq!(started.text(cx), "");
    assert_eq!(started.sw_title(cx).as_deref(), Some("kimün"));

    set_current(&p.settings, "a");
    started.sw_reopen_to(&rt, Some(ALPHA), cx);
    assert!(started.sw_is_on("alpha.md", cx));
    assert!(!started.shows(EMPTY_STATE_ID, cx));
    assert!(
        !started.shows(super::onboarding::ONBOARDING_ID, cx),
        "a vault: no setup"
    );
    assert_eq!(started.sw_title(cx).as_deref(), Some("a - kimün"));
}

/// A window started with no workspace enters one once the settings name it.
#[gpui_kit::test]
fn no_workspace_then_a_vault(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    set_current(&p.settings, "");
    let started = open(cx, &rt, &p, LayoutStore::Off);
    assert!(started.shows(super::onboarding::ONBOARDING_ID, cx));
    set_current(&p.settings, "b");
    send(&started, AppMessage::OnboardingFinished, cx);
    drive(&rt, cx, |cx| started.text(cx) == BETA);
    assert!(started.sw_is_on("beta.md", cx));
    assert_eq!(started.sw_title(cx).as_deref(), Some("b - kimün"));
}

// ── stale messages ───────────────────────────────────────────────────────

/// A message from A's tasks arriving after the switch is never applied:
/// A's channel has no listener any more.
#[gpui_kit::test]
fn a_late_message_from_the_old_vault_is_ignored(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let started = open_on_a(cx, &rt, &p, LayoutStore::Off);
    let old = started.sw_tx(cx);
    set_current(&p.settings, "b");
    started.sw_reopen_to(&rt, Some(BETA), cx);

    let late = [
        AppMessage::IndexingDone(Err("late".into())),
        AppMessage::FlashMessage("from A".into()),
        AppMessage::open(path("afriend.md")),
        AppMessage::RagStatus(RagStatus::Offline),
    ];
    for message in late {
        assert!(old.send(message).is_err(), "A's channel is closed");
    }
    drive(&rt, cx, |_| true);
    assert!(!started.sw_flashed("late", cx) && !started.sw_flashed("from A", cx));
    assert!(started.sw_is_on("beta.md", cx));
    assert_eq!(
        cx.read(|cx| started.workspace.read(cx).rag_status),
        RagStatus::Disabled
    );
    // The new channel is the one listened to.
    started
        .sw_tx(cx)
        .send(AppMessage::FlashMessage("from B".into()))
        .unwrap();
    cx.run_until_parked();
    assert!(started.sw_flashed("from B", cx));
}

// ── what is open at the switch ───────────────────────────────────────────

/// An open palette and a pending leader sequence close at a switch; the
/// Thread and the server sync go with their vault.
#[gpui_kit::test]
fn modals_the_leader_the_thread_and_the_sync_go(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    p.settings
        .write()
        .unwrap()
        .workspace_config
        .as_mut()
        .unwrap()
        .global
        .kimun_server_url = Some("http://127.0.0.1:9".into());
    let started = open_on_a(cx, &rt, &p, LayoutStore::Off);
    assert!(cx.read(|cx| started.workspace.read(cx).rag_sync_running()));

    // A palette open at the switch closes.
    cx.update_window(started.handle, |_, window, cx| {
        started
            .workspace
            .update(cx, |w, cx| w.open_palette(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(started.shown_modal(cx), Some(ModalKind::Palette));
    set_current(&p.settings, "b");
    started.sw_reopen_to(&rt, Some(BETA), cx);
    started.settle_dialog(cx);
    assert_eq!(started.shown_modal(cx), None);
    assert!(cx.read(|cx| started.workspace.read(cx).modal.is_none()));
    assert!(cx.read(|cx| started.workspace.read(cx).palette.is_none()));

    // A pending leader sequence ends; the Thread goes with its vault.
    started.sw_key("ctrl-shift-b", cx);
    assert!(cx.read(|cx| started.workspace.read(cx).leader.is_pending()));
    set_current(&p.settings, "");
    started.sw_reopen_to(&rt, None, cx);
    cx.read(|cx| {
        let w = started.workspace.read(cx);
        assert!(!w.leader.is_pending());
        assert!(w.ask.is_none() && !w.ask_shown());
        assert!(
            !w.rag_sync_running(),
            "the sync of the vault left was aborted"
        );
        assert_eq!(w.rag_status, RagStatus::Disabled);
        assert_eq!(w.status.rag, None);
    });
}

/// A→B→A→B: one listener (each old channel closed), the views rebuilt
/// once per entry, nothing accumulating.
#[gpui_kit::test]
fn repeated_switches_accumulate_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let started = open_on_a(cx, &rt, &p, LayoutStore::Off);
    let subscriptions = started.sw_subscriptions(cx);
    for (name, text) in [("b", BETA), ("a", ALPHA), ("b", BETA)] {
        let old = started.sw_tx(cx);
        let views = started.sw_view_ids(cx);
        set_current(&p.settings, name);
        started.sw_reopen_to(&rt, Some(text), cx);
        assert!(old.is_closed(), "the old listener is gone");
        assert_eq!(started.sw_subscriptions(cx), subscriptions);
        let now = started.sw_view_ids(cx);
        assert!(views.iter().all(|id| !now.contains(id)));
        assert_eq!(
            started.sw_title(cx),
            Some(format!("{name} - {}", startup::APP_NAME))
        );
    }
}

// ── the WorkspaceSwitched message and the settings write ────────────────

/// `WorkspaceSwitched("b")` on the window's channel: the shared settings
/// name `b`, the config on disk does too (its `[gui]` table kept), and the
/// window is on B.
#[gpui_kit::test]
fn workspace_switched_persists_and_reopens(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    AppSettings::update_gui_at(p.config.clone(), |gui| gui.set_layout_preset(Some("mine")))
        .unwrap();
    let started = open_on_a(cx, &rt, &p, LayoutStore::File(p.config.clone()));
    assert_eq!(on_disk_current(&p.config).as_deref(), Some("a"));
    started
        .sw_tx(cx)
        .send(AppMessage::WorkspaceSwitched("b".into()))
        .unwrap();
    drive(&rt, cx, |cx| {
        started.text(cx) == BETA && on_disk_current(&p.config).as_deref() == Some("b")
    });
    assert_eq!(current(&p.settings).as_deref(), Some("b"));
    assert!(started.sw_is_on("beta.md", cx));
    assert_eq!(
        on_disk_gui(&p.config).layout_preset().as_deref(),
        Some("mine")
    );
}

/// A settings write keeps the `[gui]` table on disk, and a layout saved
/// after it survives the next write.
#[gpui_kit::test]
fn the_gui_table_survives_a_settings_write(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let started = open_on_a(cx, &rt, &p, LayoutStore::File(p.config.clone()));
    AppSettings::update_gui_at(p.config.clone(), |gui| gui.set_layout_preset(Some("mine")))
        .unwrap();
    let persist = |theme: &str, cx: &mut TestAppContext| {
        p.settings.write().unwrap().theme = theme.into();
        started.workspace.update(cx, |w, cx| w.persist_settings(cx));
        drive(&rt, cx, |_| {
            on_disk(&p.config)
                .and_then(|v| v.get("theme")?.as_str().map(str::to_string))
                .as_deref()
                == Some(theme)
        });
    };
    persist("Nord", cx);
    assert_eq!(
        on_disk_gui(&p.config).layout_preset().as_deref(),
        Some("mine")
    );

    // The window saves its layout (the left side hidden), then the
    // settings are written again.
    let before = on_disk_gui(&p.config).layout();
    cx.update_window(started.handle, |_, window, cx| {
        started
            .workspace
            .update(cx, |w, cx| w.toggle_drawer(Side::Left, window, cx))
    })
    .unwrap();
    let toggled = on_disk_gui(&p.config).layout();
    assert!(toggled.is_some());
    assert_ne!(toggled, before);
    persist("Gruvbox Dark", cx);
    assert_eq!(on_disk_gui(&p.config).layout(), toggled);
    assert_eq!(
        on_disk_gui(&p.config).layout_preset().as_deref(),
        Some("mine")
    );
}

/// Two settings writes queued back to back, the first failing (its config
/// does not parse): its change reaches neither memory nor the second write,
/// which lands with only its own change. The second's answer is its
/// change's, on the shared settings.
#[test]
fn a_failed_write_does_not_poison_the_next() {
    let rt = runtime();
    let _guard = rt.enter();
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.toml");
    let good = dir.path().join("good.toml");
    std::fs::write(&bad, "this is [not toml").unwrap();
    let settings: SharedSettings = Arc::new(RwLock::new(AppSettings::default()));
    let before = settings.read().unwrap().clone();
    assert_ne!(before.theme, "Nord");
    assert_ne!(before.leader_timeout_ms, 1234);
    let first =
        super::reopen::write_settings(LayoutStore::File(bad.clone()), settings.clone(), |s| {
            s.theme = "Nord".into()
        });
    let second =
        super::reopen::write_settings(LayoutStore::File(good.clone()), settings.clone(), |s| {
            s.leader_timeout_ms = 1234;
            s.theme.clone()
        });
    let (first, second) = rt.block_on(async { (first.await, second.await) });
    assert!(first.is_err(), "the first fails: {first:?}");
    assert_eq!(second, Ok(before.theme.clone()), "seen without the first");
    let memory = settings.read().unwrap().clone();
    assert_eq!(
        memory.theme, before.theme,
        "the failed change is not in memory"
    );
    assert_eq!(memory.leader_timeout_ms, 1234);
    let disk = AppSettings::load_from_file(good).unwrap();
    assert_eq!(disk.theme, before.theme, "nor on disk");
    assert_eq!(disk.leader_timeout_ms, 1234);
    assert_eq!(std::fs::read_to_string(bad).unwrap(), "this is [not toml");
}

/// A switch while the window cannot change vault (the close flush runs)
/// changes nothing: settings, disk and window stay on A.
#[gpui_kit::test]
fn a_refused_switch_changes_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let started = open_on_a(cx, &rt, &p, LayoutStore::File(p.config.clone()));
    started.workspace.update(cx, |w, _| {
        w.closing = super::Closing::Flushing;
    });
    send(&started, AppMessage::WorkspaceSwitched("b".into()), cx);
    drive(&rt, cx, |_| true);
    assert_eq!(current(&p.settings).as_deref(), Some("a"));
    assert_eq!(
        on_disk_current(&p.config).as_deref(),
        Some("a"),
        "untouched"
    );
    assert!(started.sw_is_on("alpha.md", cx));
    let flash = started.flash(cx).unwrap();
    assert!(flash.starts_with("Couldn't switch to b:"), "{flash}");
    started.workspace.update(cx, |w, _| {
        w.closing = super::Closing::Open;
    });
}

/// A reopen decides like the startup, but never with the command line's
/// note: it lands on the history (the restore).
#[test]
fn a_reopen_never_reuses_the_command_line_note() {
    let rt = runtime();
    let p = rt.block_on(pair());
    let opened = rt.block_on(try_open_vault(&p.settings));
    match startup::decide(opened, current(&p.settings), None, None) {
        Start::Vault(VaultStart { first, name, .. }) => {
            assert_eq!(first, FirstNote::Restore);
            assert_eq!(name, "a");
        }
        _ => panic!("A opens"),
    }
}
