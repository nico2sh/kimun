//! A real vault for tests: a temp workspace, a config file next to it whose
//! history and index live in the same temp dir (never the developer's own),
//! loaded and opened through the same calls `main` makes.
#![cfg(test)]

use std::sync::{Arc, RwLock};

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_core::system::SystemPath;
use kimun_notes::logic::startup::{VaultOpen, try_open_vault};
use kimun_notes::settings::{AppSettings, SharedSettings};

/// The workspace name the fixture configures.
pub(crate) const NAME: &str = "notes";

pub(crate) struct TestVault {
    pub(crate) settings: SharedSettings,
    pub(crate) vault: Arc<NoteVault>,
    pub(crate) workspace: SystemPath,
    _dir: tempfile::TempDir,
}

impl TestVault {
    /// The history, newest first.
    pub(crate) fn history(&self) -> Vec<VaultPath> {
        self.settings.read().unwrap().current_last_paths()
    }
}

/// Settings naming a workspace at `workspace` (which may not exist), with
/// the config, history and index under `dir`.
pub(crate) fn settings_for(dir: &std::path::Path, workspace: &std::path::Path) -> SharedSettings {
    let config = dir.join("config.toml");
    let path = toml::Value::String(workspace.to_string_lossy().into_owned());
    std::fs::write(
        &config,
        format!(
            "config_version = 7\n\
             cache_dir = \"cache\"\n\
             history_dir = \"history\"\n\
             [global]\n\
             current_workspace = \"{NAME}\"\n\
             [workspaces.{NAME}]\n\
             path = {path}\n\
             created = \"2026-01-01T00:00:00Z\"\n"
        ),
    )
    .unwrap();
    let settings = AppSettings::load_from_file(config).expect("the test config loads");
    Arc::new(RwLock::new(settings))
}

/// A vault holding `notes` (path, text), whose history is `history` (newest
/// first). Run inside a tokio runtime.
pub(crate) async fn test_vault(notes: &[(&str, &str)], history: &[&str]) -> TestVault {
    let dir = tempfile::TempDir::new().unwrap();
    let workspace = dir.path().join("ws");
    std::fs::create_dir(&workspace).unwrap();
    let settings = settings_for(dir.path(), &workspace);
    let VaultOpen::Opened(vault) = try_open_vault(&settings).await else {
        panic!("the test vault opens");
    };
    for (path, text) in notes {
        vault
            .create_note(&VaultPath::new(*path), *text)
            .await
            .expect("the note is created");
    }
    let history: Vec<VaultPath> = history.iter().map(|p| VaultPath::new(*p)).collect();
    settings.read().unwrap().edit_path_history(|paths| {
        *paths = history;
        true
    });
    let workspace = vault.workspace_path().clone();
    TestVault {
        settings,
        vault,
        workspace,
        _dir: dir,
    }
}
