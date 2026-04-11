use std::sync::Arc;

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::components::Component;
use crate::components::dialogs::{
    ActiveDialog, CreateNoteDialog, DeleteConfirmDialog, FileOpsMenuDialog, HelpDialog, MoveDialog,
    QuickNoteModal, RenameDialog, ValidationState, WorkspaceSwitcherModal,
};
use crate::settings::AppSettings;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::keys::KeyBindings;
use crate::settings::themes::Theme;

/// Manages dialog lifecycle: open/close, focus save/restore, input routing,
/// rendering, and dialog-related `AppEvent` handling.
///
/// The `focus_token` is an opaque `u8` that the owning screen maps to/from its
/// own focus enum. This keeps `DialogManager` screen-agnostic.
pub struct DialogManager {
    active: Option<ActiveDialog>,
    /// The focus token saved when the *first* dialog in a chain opens.
    /// Subsequent chained dialogs (e.g. Menu → Delete) don't overwrite it.
    saved_focus: Option<u8>,
}

impl Default for DialogManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DialogManager {
    pub fn new() -> Self {
        Self {
            active: None,
            saved_focus: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.active.is_some()
    }

    /// Open a dialog, saving the current focus token if this is the first dialog
    /// in a chain (i.e. no dialog is currently open).
    pub fn open(&mut self, dialog: ActiveDialog, current_focus: u8) {
        if self.saved_focus.is_none() {
            self.saved_focus = Some(current_focus);
        }
        self.active = Some(dialog);
    }

    /// Close the active dialog and return the saved focus token to restore.
    pub fn close(&mut self) -> Option<u8> {
        self.active = None;
        self.saved_focus.take()
    }

    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        if let Some(dialog) = &mut self.active {
            dialog.handle_input(event, tx)
        } else {
            EventState::NotConsumed
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        if let Some(dialog) = &mut self.active {
            dialog.render(f, area, theme, true);
        }
    }

    /// Try to handle a dialog-related `AppEvent`.
    /// Returns `true` if the event was consumed, `false` otherwise.
    pub fn handle_app_message(
        &mut self,
        msg: &AppEvent,
        vault: &Arc<NoteVault>,
        tx: &AppTx,
        current_focus: u8,
    ) -> bool {
        match msg {
            AppEvent::ShowFileOpsMenu(path) => {
                self.open(
                    ActiveDialog::Menu(FileOpsMenuDialog::new(path.clone())),
                    current_focus,
                );
                true
            }
            AppEvent::ShowDeleteDialog(path) => {
                self.open(
                    ActiveDialog::Delete(DeleteConfirmDialog::new(path.clone(), vault.clone())),
                    current_focus,
                );
                true
            }
            AppEvent::ShowRenameDialog(path) => {
                self.open(
                    ActiveDialog::Rename(RenameDialog::new(path.clone(), vault.clone())),
                    current_focus,
                );
                true
            }
            AppEvent::ShowMoveDialog(path) => {
                self.open(
                    ActiveDialog::Move(MoveDialog::new(path.clone(), vault.clone(), tx)),
                    current_focus,
                );
                true
            }
            AppEvent::RenameValidation { available } => {
                if let Some(ActiveDialog::Rename(d)) = &mut self.active {
                    d.validation_state = if *available {
                        ValidationState::Available
                    } else {
                        ValidationState::Taken
                    };
                    d.validation_task = None;
                }
                true
            }
            AppEvent::MoveDirectoriesLoaded(paths) => {
                if let Some(ActiveDialog::Move(d)) = &mut self.active {
                    d.all_dirs = paths.clone();
                    d.filtered = None;
                    d.load_task = None;
                    if d.list_state.selected().is_none() && !d.results().is_empty() {
                        d.list_state.select(Some(0));
                    }
                    d.spawn_validation(tx);
                }
                true
            }
            AppEvent::MoveFilterResults(paths) => {
                if let Some(ActiveDialog::Move(d)) = &mut self.active {
                    d.filter_task = None;
                    d.filtered = Some(paths.clone());
                    if !d.results().is_empty() {
                        d.list_state.select(Some(0));
                    } else {
                        d.list_state.select(None);
                    }
                    d.spawn_validation(tx);
                }
                true
            }
            AppEvent::MoveDestValidation { available } => {
                if let Some(ActiveDialog::Move(d)) = &mut self.active {
                    d.dest_validation = if *available {
                        ValidationState::Available
                    } else {
                        ValidationState::Taken
                    };
                    d.validation_task = None;
                }
                true
            }
            AppEvent::DialogError(msg) => {
                if let Some(dialog) = &mut self.active {
                    dialog.set_error(msg.clone());
                }
                true
            }
            AppEvent::CloseDialog => {
                self.close();
                true
            }
            _ => false,
        }
    }

    /// Convenience: open the help dialog.
    pub fn open_help(&mut self, key_bindings: &KeyBindings, current_focus: u8) {
        self.open(
            ActiveDialog::Help(HelpDialog::new(key_bindings)),
            current_focus,
        );
    }

    pub fn open_quick_note(&mut self, vault: Arc<NoteVault>, current_focus: u8) {
        self.open(
            ActiveDialog::QuickNote(QuickNoteModal::new(vault)),
            current_focus,
        );
    }

    pub fn open_workspace_switcher(&mut self, settings: &AppSettings, current_focus: u8) {
        self.open(
            ActiveDialog::WorkspaceSwitcher(WorkspaceSwitcherModal::new(settings)),
            current_focus,
        );
    }

    /// Convenience: open the create-note dialog.
    pub fn open_create_note(
        &mut self,
        path: VaultPath,
        vault: Arc<NoteVault>,
        current_focus: u8,
    ) {
        self.open(
            ActiveDialog::CreateNote(CreateNoteDialog::new(path, vault)),
            current_focus,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_not_open() {
        let dm = DialogManager::new();
        assert!(!dm.is_open());
    }

    #[test]
    fn open_then_close_returns_focus() {
        let mut dm = DialogManager::new();
        let kb = KeyBindings::empty();
        dm.open_help(&kb, 42);
        assert!(dm.is_open());
        let restored = dm.close();
        assert_eq!(restored, Some(42));
        assert!(!dm.is_open());
    }

    #[tokio::test]
    async fn chained_dialogs_preserve_original_focus() {
        let mut dm = DialogManager::new();
        let path = VaultPath::new("test");
        dm.open(ActiveDialog::Menu(FileOpsMenuDialog::new(path.clone())), 1);
        // Chained dialog (e.g. from menu → delete) should not overwrite saved focus
        let dir = std::env::temp_dir().join("kimun_dm_test");
        std::fs::create_dir_all(&dir).ok();
        let vault = Arc::new(NoteVault::new(&dir).await.unwrap());
        dm.open(
            ActiveDialog::Delete(DeleteConfirmDialog::new(path, vault)),
            99, // this focus should be ignored
        );
        let restored = dm.close();
        assert_eq!(restored, Some(1)); // original focus preserved
    }

    #[test]
    fn close_when_empty_returns_none() {
        let mut dm = DialogManager::new();
        assert_eq!(dm.close(), None);
    }
}
