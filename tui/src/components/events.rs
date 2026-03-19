use std::sync::Arc;
use std::time::Duration;

use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use tokio::sync::mpsc::UnboundedSender;

use crate::settings::AppSettings;
use kimun_core::{NoteVault, nfs::VaultPath};

/// All events that flow through the system — both input events (from crossterm)
/// and app-level messages sent by components / screens to the main loop.
#[derive(Debug, Clone)]
pub enum AppEvent {
    Input(InputEvent),
    OpenScreen(ScreenEvent),

    // ── App-level messages ───────────────────────────────────────────────────
    Quit,
    Redraw,
    Autosave,
    OpenPath(VaultPath),
    FocusEditor,
    FocusSidebar,
    /// Sent by SettingsScreen when user confirms Save.
    SettingsSaved(AppSettings),
    /// Sent by SettingsScreen when user discards or closes unchanged.
    CloseSettings,
    /// Sent by VaultSection; SettingsScreen::handle_app_message intercepts.
    OpenFileBrowser,
    /// Sent by IndexingSection; SettingsScreen intercepts.
    TriggerFastReindex,
    TriggerFullReindex,
    /// Sent by indexing tokio task on completion.
    IndexingDone(Result<Duration, String>),
    /// Open (or create) today's journal entry and switch to it in the editor.
    OpenJournal,
}

impl AppEvent {
    pub fn send_input(event: InputEvent) -> Self {
        AppEvent::Input(event)
    }
}

// ── Input events ────────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub enum InputEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
}

// ── Screen events ────────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub enum ScreenEvent {
    Start,
    OpenSettings,
    /// Navigate to the editor for the given vault root path.
    OpenEditor(Arc<NoteVault>, VaultPath),
    /// Navigate to the browse screen for the given vault root and directory path.
    OpenBrowse(Arc<NoteVault>, VaultPath),
}

/// Convenience alias used throughout the codebase.
pub type AppTx = UnboundedSender<AppEvent>;
