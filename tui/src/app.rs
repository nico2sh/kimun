use std::sync::{Arc, RwLock};

use color_eyre::eyre;
use kimun_core::{NoteVault, VaultConfig};

use crate::{
    app_screen::{AppScreen, start::StartScreen},
    settings::{AppSettings, SharedSettings},
};

pub struct App {
    /// The currently active screen. Held as `Option` so we can temporarily
    /// `take()` it when calling screen methods (avoids double-borrow of `App`).
    pub current_screen: Option<Box<dyn AppScreen>>,

    pub settings: SharedSettings,

    /// The active vault. `None` until a workspace path is configured.
    /// Rebuilt only when the workspace path changes in settings.
    pub vault: Option<Arc<NoteVault>>,

    /// Monotonic counter bumped by every screen swap (see `switch_screen`
    /// in main.rs). The main event loop uses it to break its inner drain
    /// when the screen identity changes mid-batch — without this, queued
    /// events from the OLD screen instance can be routed to a fresh
    /// screen of the same `ScreenKind` (e.g. EditorScreen(A) → follow-link
    /// → EditorScreen(B)) and leak A's InsertAtCursor / dialog-result
    /// payloads into B.
    pub screen_generation: u64,
}

impl App {
    pub async fn new(config_path: Option<std::path::PathBuf>) -> eyre::Result<Self> {
        let loaded_settings = match config_path {
            Some(path) => AppSettings::load_from_file(path)?,
            None => AppSettings::load_from_disk()?,
        };
        let settings: SharedSettings = Arc::new(RwLock::new(loaded_settings));

        let vault = {
            let (workspace_path, cache_path, inbox) = {
                let s = settings.read().unwrap();
                let path = s.resolve_workspace_path();
                let name = s
                    .workspace_config
                    .as_ref()
                    .map(|wc| wc.global.current_workspace.clone())
                    .filter(|n| !n.is_empty());
                let cache = name.as_ref().map(|n| s.cache_path_for(n));
                let inbox = s
                    .workspace_config
                    .as_ref()
                    .and_then(|wc| wc.get_current_workspace())
                    .map(|entry| entry.effective_inbox_path());
                (path, cache, inbox)
            };
            if let Some(workspace) = workspace_path {
                let mut config = VaultConfig::new(&workspace);
                if let Some(cp) = cache_path {
                    config = config.with_db_path(cp);
                }
                NoteVault::new(config).await.ok().map(|mut v| {
                    if let Some(inbox) = inbox {
                        v.set_inbox_path(kimun_core::nfs::VaultPath::new(inbox));
                    }
                    Arc::new(v)
                })
            } else {
                None
            }
        };
        Ok(Self {
            current_screen: Some(Box::new(StartScreen::new(settings.clone(), vault.clone()))),
            settings,
            vault,
            screen_generation: 0,
        })
    }
}
