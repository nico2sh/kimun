use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use tokio::sync::mpsc::UnboundedSender;

use kimun_core::{NoteVault, nfs::VaultPath};

use crate::components::file_list::{SortField, SortOrder};

/// Which panel a sort selection applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortTarget {
    Sidebar,
    Query,
}

/// The surface a save-current-query action sourced its query from. Carried
/// through the save-search dialog so the editor knows whether the Query
/// panel's breadcrumb should re-pin after the save — by identity, not by
/// comparing query text (equal text from different surfaces must not collide).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveSource {
    QueryPanel,
    NoteBrowser,
}

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
    /// Background autosave task finished. `saved_revision` carries the
    /// editor's `content_revision` at the moment the save was *issued*
    /// on success, `None` if the write failed. The editor screen uses
    /// `path` to ignore stale completions for notes the user has
    /// already navigated away from, and `saved_revision` to clear the
    /// dirty flag iff the buffer is still at that revision (i.e. no
    /// edits during the save). `NonZeroU64` because the editor's
    /// `content_revision` is never zero.
    AutosaveCompleted {
        path: VaultPath,
        saved_revision: Option<NonZeroU64>,
    },
    OpenPath(VaultPath),
    FocusSidebar,
    /// Sent by SettingsScreen when user confirms Save. The shared settings
    /// reference already contains the updated values.
    SettingsSaved,
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
    /// Dismiss the active editor overlay (note browser, Saved Searches modal,
    /// or dialog). The single close path for everything owned by `OverlayHost`.
    CloseOverlay,
    /// Follow the link under the editor cursor: note name/path or external URL.
    FollowLink(String),
    /// Open the search modal pre-filled with `#<name>` to browse notes by label.
    FollowLabel(String),
    /// Insert raw text at the editor's cursor (replacing any active selection).
    /// Used by the screen layer to deliver async results back to the editor —
    /// e.g. the markdown link generated after a clipboard image is saved as an attachment.
    InsertAtCursor(String),

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
    EntryRenamed {
        from: VaultPath,
        to: VaultPath,
    },
    /// Confirmation that an entry was successfully moved.
    EntryMoved {
        from: VaultPath,
        to: VaultPath,
    },
    /// A new note was just created and should be opened; sidebar should reflect it.
    EntryCreated(VaultPath),
    /// A dialog operation failed; carries a human-readable error message.
    DialogError(String),

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
    RenameValidation {
        available: bool,
    },
    /// Move dialog: directory list has loaded.
    MoveDirectoriesLoaded(Vec<VaultPath>),
    /// Move dialog: fuzzy filter results are ready.
    MoveFilterResults(Vec<VaultPath>),
    /// Move dialog: destination existence check result.
    MoveDestValidation {
        available: bool,
    },
    /// Save-search dialog: existing saved-search names have loaded (drives
    /// the update/overwrite/save-new hint).
    SavedSearchNamesLoaded(Vec<String>),

    // ── Workspace messages ──────────────────────────────────────────────
    /// User switched to a different workspace. Carries the workspace name.
    /// Handled by main.rs to rebuild the vault and navigate to StartScreen.
    WorkspaceSwitched(String),

    /// Persist a saved search (emitted by the save-search dialog on submit).
    /// `source` is the surface the query was sourced from, decided when the
    /// dialog opened — it drives whether the panel breadcrumb re-pins.
    SaveSearchConfirmed {
        name: String,
        query: String,
        source: SaveSource,
    },

    /// A saved search was written to disk (success path of
    /// `SaveSearchConfirmed`). The editor re-pins the panel breadcrumb here —
    /// only once the write actually succeeded.
    SavedSearchPersisted {
        name: String,
        query: String,
        source: SaveSource,
    },

    /// The background saved-search write failed; surface it to the user.
    SavedSearchSaveFailed {
        name: String,
    },

    /// A saved search was chosen in the Saved Searches modal.
    SavedSearchSelected {
        query: String,
        name: String,
    },

    /// Sort selection changed in the sort dialog — apply live to `target`.
    /// When `persist` is set (sidebar's "save as default"), also write the
    /// choice to settings. `group_directories` is sidebar-only (the query panel
    /// ignores it).
    SortChanged {
        target: SortTarget,
        field: SortField,
        order: SortOrder,
        group_directories: bool,
        persist: bool,
    },
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
    /// Bracketed-paste payload from the terminal. On macOS this is what
    /// Cmd+V delivers, since the terminal intercepts Cmd combos before they
    /// reach the TUI. The string may be empty when the clipboard holds only
    /// non-text content (e.g. an image).
    Paste(String),
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

/// Build a `Send + Sync` callback that fires `AppEvent::Redraw` on the
/// app event bus. Used by long-lived components (autocomplete query
/// task, etc.) that need to wake the render loop from a background
/// thread but should not be aware of `AppEvent` themselves.
pub fn redraw_callback(tx: AppTx) -> Arc<dyn Fn() + Send + Sync + 'static> {
    Arc::new(move || {
        let _ = tx.send(AppEvent::Redraw);
    })
}

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
            _ => {}
        }
    }

    #[test]
    fn sort_events_construct() {
        use crate::components::file_list::{SortField, SortOrder};
        let _ = AppEvent::SortChanged {
            target: SortTarget::Sidebar,
            field: SortField::Name,
            order: SortOrder::Ascending,
            group_directories: true,
            persist: false,
        };
        let _ = AppEvent::SortChanged {
            target: SortTarget::Query,
            field: SortField::Title,
            order: SortOrder::Descending,
            group_directories: false,
            persist: true,
        };
    }
}
