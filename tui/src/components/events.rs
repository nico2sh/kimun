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
    SettingsSaved(Box<AppSettings>),
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
    /// Sent by NoteBrowserModal on Esc or after Enter+open.
    CloseNoteBrowser,
    /// Follow the link under the editor cursor: note name/path or external URL.
    FollowLink(String),

    // ── File-operation dialog messages ───────────────────────────────────────
    /// Request to show the file-operations menu (delete / rename / move).
    ShowFileOpsMenu(VaultPath),
    /// Request to show the delete confirmation dialog for the given entry.
    ShowDeleteDialog(VaultPath),
    /// Request to show the rename dialog for the given entry.
    ShowRenameDialog(VaultPath),
    /// Request to show the move dialog for the given entry.
    ShowMoveDialog(VaultPath),
    /// Confirmation that the given entry was successfully deleted.
    EntryDeleted(VaultPath),
    /// Confirmation that an entry was successfully renamed.
    EntryRenamed { from: VaultPath, to: VaultPath },
    /// Confirmation that an entry was successfully moved.
    EntryMoved { from: VaultPath, to: VaultPath },
    /// A new note was just created and should be opened; sidebar should reflect it.
    EntryCreated(VaultPath),
    /// A dialog operation failed; carries a human-readable error message.
    DialogError(String),
    /// Dismiss the currently visible dialog without taking action.
    CloseDialog,

    /// A vault was found to be structurally unusable (conflicts, invalid layout, etc.).
    /// Carries a formatted, human-readable error message.
    ///
    /// Handled by `handle_app_message` in `main.rs`, which clears the workspace,
    /// saves settings, and opens the settings screen with an error overlay.
    /// To add a new conflict source: emit this event from the detection site; no
    /// other files need to change.
    VaultConflict(String),

    // ── Dialog async result messages ─────────────────────────────────────────
    /// Rename dialog: name availability check result.
    RenameValidation { available: bool },
    /// Move dialog: directory list has loaded.
    MoveDirectoriesLoaded(Vec<VaultPath>),
    /// Move dialog: fuzzy filter results are ready.
    MoveFilterResults(Vec<VaultPath>),
    /// Move dialog: destination existence check result.
    MoveDestValidation { available: bool },
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
    /// Open the settings screen with an error overlay already shown.
    OpenSettingsWithError(String),
    /// Navigate to the editor for the given vault root path.
    OpenEditor(Arc<NoteVault>, VaultPath),
    /// Navigate to the browse screen for the given vault root and directory path.
    OpenBrowse(Arc<NoteVault>, VaultPath),
}

/// Convenience alias used throughout the codebase.
pub type AppTx = UnboundedSender<AppEvent>;

#[cfg(test)]
mod tests {
    use super::*;

    fn _assert_new_variants_exist(e: AppEvent) {
        match e {
            AppEvent::ShowDeleteDialog(_) => {}
            AppEvent::ShowRenameDialog(_) => {}
            AppEvent::ShowMoveDialog(_) => {}
            AppEvent::EntryDeleted(_) => {}
            AppEvent::EntryRenamed { from: _, to: _ } => {}
            AppEvent::EntryMoved { from: _, to: _ } => {}
            AppEvent::DialogError(_) => {}
            AppEvent::CloseDialog => {}
            _ => {}
        }
    }
}
