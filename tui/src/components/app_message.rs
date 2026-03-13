use std::path::PathBuf;

use tokio::sync::mpsc::UnboundedSender;

use crate::settings::AppSettings;

/// Messages screens send to the main loop. All variants must be `Send` so
/// they can travel through the tokio channel. Keep data simple — no vault
/// handles, no `Arc<…>`. The main loop reconstructs whatever it needs.
#[derive(Debug)]
pub enum AppMessage {
    Quit,
    OpenSettings,
    /// Navigate to the editor for the given vault root path.
    OpenEditor(PathBuf),
}

/// Convenience alias used throughout the codebase.
pub type AppTx = UnboundedSender<AppMessage>;
