use std::sync::Arc;

use color_eyre::eyre;
use kimun_core::NoteVault;

use crate::{
    app_screen::{AppScreen, start::StartScreen},
    settings::AppSettings,
};

pub struct App {
    /// The currently active screen. Held as `Option` so we can temporarily
    /// `take()` it when calling screen methods (avoids double-borrow of `App`).
    pub current_screen: Option<Box<dyn AppScreen>>,

    pub settings: AppSettings,

    /// The active vault. `None` until a workspace path is configured.
    /// Rebuilt only when the workspace path changes in settings.
    pub vault: Option<Arc<NoteVault>>,
}

impl App {
    pub async fn new(config_path: Option<std::path::PathBuf>) -> eyre::Result<Self> {
        let settings = match config_path {
            Some(path) => AppSettings::load_from_file(path)?,
            None => AppSettings::load_from_disk()?,
        };
        // Phase 1 configs store the workspace in `workspace_dir`.
        // Phase 2 configs store it in `workspace_config[current_workspace].path`.
        let workspace_path = settings.workspace_dir.clone().or_else(|| {
            settings
                .workspace_config
                .as_ref()
                .and_then(|wc| wc.get_current_workspace())
                .map(|entry| entry.path.clone())
        });
        let vault = if let Some(ref workspace) = workspace_path {
            NoteVault::new(workspace).await.ok().map(|mut v| {
                if let Some(ref wc) = settings.workspace_config
                    && let Some(entry) = wc.get_current_workspace()
                {
                    v.set_inbox_path(kimun_core::nfs::VaultPath::new(entry.effective_inbox_path()));
                }
                Arc::new(v)
            })
        } else {
            None
        };
        Ok(Self {
            current_screen: Some(Box::new(StartScreen::new(settings.clone(), vault.clone()))),
            settings,
            vault,
        })
    }
}
