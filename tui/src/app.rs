use std::sync::{Arc, RwLock};

use color_eyre::eyre;
use kimun_core::NoteVault;

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
}

impl App {
    pub async fn new(config_path: Option<std::path::PathBuf>) -> eyre::Result<Self> {
        let loaded_settings = match config_path {
            Some(path) => AppSettings::load_from_file(path)?,
            None => AppSettings::load_from_disk()?,
        };
        let settings: SharedSettings = Arc::new(RwLock::new(loaded_settings));

        let vault = {
            let workspace_path = settings.read().unwrap().resolve_workspace_path();
            if let Some(ref workspace) = workspace_path {
                NoteVault::new(workspace).await.ok().map(|mut v| {
                    let s = settings.read().unwrap();
                    if let Some(ref wc) = s.workspace_config
                        && let Some(entry) = wc.get_current_workspace()
                    {
                        v.set_inbox_path(kimun_core::nfs::VaultPath::new(
                            entry.effective_inbox_path(),
                        ));
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
        })
    }
}
