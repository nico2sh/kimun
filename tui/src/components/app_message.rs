use kimun_core::{NoteVault, nfs::VaultPath};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use crate::settings::AppSettings;

/// Messages screens send to the main loop. All variants must be `Send` so
/// they can travel through the tokio channel. Keep data simple — no vault
/// handles, no `Arc<…>`. The main loop reconstructs whatever it needs.
#[derive(Debug)]
pub enum AppMessage {
    Quit,
    Redraw,
    OpenSettings,
    /// Navigate to the editor for the given vault root path.
    OpenEditor(NoteVault, VaultPath),
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
}
