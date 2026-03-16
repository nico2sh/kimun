use kimun_core::{NoteVault, nfs::VaultPath};
use tokio::sync::mpsc::UnboundedSender;

/// Messages screens send to the main loop. All variants must be `Send` so
/// they can travel through the tokio channel. Keep data simple — no vault
/// handles, no `Arc<…>`. The main loop reconstructs whatever it needs.
#[derive(Debug)]
pub enum AppMessage {
    Quit,
    OpenSettings,
    /// Navigate to the editor for the given vault root path.
    OpenEditor(NoteVault, VaultPath),
    OpenPath(VaultPath),
}

/// Convenience alias used throughout the codebase.
pub type AppTx = UnboundedSender<AppMessage>;
