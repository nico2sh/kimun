use kimun_core::{NoteVault, nfs::VaultPath};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use crate::settings::AppSettings;

/// Messages screens send to the main loop. All variants must be `Send` so
/// they can travel through the tokio channel.
#[derive(Debug)]
pub enum AppMessage {
    Quit,
    Redraw,
    Autosave,
    OpenSettings,
    /// Navigate to the editor for the given vault root path.
    OpenEditor(Arc<NoteVault>, VaultPath),
    /// Navigate to the browse screen for the given vault root and directory path.
    OpenBrowse(Arc<NoteVault>, VaultPath),
    OpenPath(VaultPath),
    FocusEditor,
    FocusSidebar,
    /// Sent by SettingsScreen when user confirms Save.
    /// Main loop updates App::settings and navigates back.
    SettingsSaved(AppSettings),
    /// Sent by SettingsScreen when user discards or closes unchanged.
    /// Main loop navigates back without updating App::settings.
    CloseSettings,
    /// Sent by VaultSection; SettingsScreen::handle_app_message intercepts.
    OpenFileBrowser,
    /// Sent by IndexingSection; SettingsScreen intercepts.
    /// NOTE: does NOT start indexing directly — opens ConfirmFullReindex overlay.
    TriggerFastReindex,
    TriggerFullReindex,
    /// Sent by indexing tokio task on completion.
    IndexingDone(Result<Duration, String>),
}

/// Convenience alias used throughout the codebase.
pub type AppTx = UnboundedSender<AppMessage>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use crate::settings::AppSettings;

    #[test]
    fn settings_saved_variant_exists() {
        // This test fails to compile until SettingsSaved(AppSettings) is added.
        let _msg = AppMessage::SettingsSaved(AppSettings::default());
    }

    #[test]
    fn indexing_done_variant_exists() {
        // This test fails to compile until IndexingDone(Result<Duration, String>) is added.
        let _msg = AppMessage::IndexingDone(Ok(Duration::from_secs(1)));
    }

    #[test]
    fn open_browse_variant_exists() {
        // Fails to compile until OpenBrowse(Arc<NoteVault>, VaultPath) is added.
        // NoteVault requires a real path at runtime, so we just verify the type compiles.
        let _: fn(Arc<NoteVault>, VaultPath) -> AppMessage = AppMessage::OpenBrowse;
    }

    #[test]
    fn autosave_variant_exists() {
        let _msg = AppMessage::Autosave;
    }
}
