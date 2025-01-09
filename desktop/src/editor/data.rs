use std::{path::Path, sync::Arc};

use log::{debug, error};
use notes_core::{nfs::VaultPath, NoteVault};

pub struct EditorData {
    pub vault: Arc<NoteVault>,
    pub text: String,
    pub changed: bool,
    pub note_path: Option<VaultPath>,
}

impl EditorData {
    pub fn new(workspace_path: &Path) -> anyhow::Result<Self> {
        // let file_selector = FilteredList::new(vec![]);
        let vault = Arc::new(NoteVault::new(workspace_path)?);
        Ok(Self {
            vault,
            text: String::new(),
            changed: false,
            note_path: None,
        })
    }

    pub fn load_note(&self, path: &VaultPath) -> anyhow::Result<Self> {
        let vault = (*self.vault).clone();
        let text = vault.load_note(path)?;
        Ok(Self {
            vault: Arc::new(vault),
            text,
            changed: false,
            note_path: Some(path.clone()),
        })
    }

    pub fn save_note(&mut self) {
        debug!("Saving note");
        if let Some(path) = &self.note_path {
            if let Err(e) = self.vault.save_note(path, &self.text) {
                error!("Error saving note: {}", e);
            } else {
                self.changed = false;
            }
        }
    }
}

// We want to save the note if we change the data
impl Drop for EditorData {
    fn drop(&mut self) {
        self.save_note();
    }
}
