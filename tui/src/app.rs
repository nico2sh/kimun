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
        let vault = if let Some(ref workspace) = settings.workspace_dir {
            NoteVault::new(workspace).await.ok().map(Arc::new)
        } else {
            None
        };
        Ok(Self {
            current_screen: Some(Box::new(StartScreen::new(settings.clone()))),
            settings,
            vault,
        })
    }
}
