use std::{path::Path, sync::Arc};

use core_notes::{nfs::NotePath, NoteVault};

pub struct EditorData {
    pub note: Arc<NoteVault>,
    pub text: String,
    pub note_path: Option<NotePath>,
}

impl EditorData {
    pub fn new(workspace_path: &Path) -> anyhow::Result<Self> {
        // let file_selector = FilteredList::new(vec![]);
        let note = Arc::new(NoteVault::new(workspace_path)?);
        Ok(Self {
            note,
            text: String::new(),
            note_path: None,
        })
    }
}
