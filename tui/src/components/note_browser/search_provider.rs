use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::nfs::{NoteEntryData, VaultPath};
use kimun_core::note::NoteContentData;

use crate::components::file_list::FileListEntry;
use super::{NoteBrowserProvider, format_journal_date};

pub struct SearchNotesProvider {
    vault: Arc<NoteVault>,
    last_paths: Vec<VaultPath>,
}

impl SearchNotesProvider {
    pub fn new(vault: Arc<NoteVault>, last_paths: Vec<VaultPath>) -> Self {
        Self { vault, last_paths }
    }

    fn to_entry(&self, entry: NoteEntryData, content: NoteContentData) -> FileListEntry {
        let filename = entry.path.get_parent_path().1;
        let title = if content.title.trim().is_empty() {
            "<no title>".to_string()
        } else {
            content.title
        };
        let journal_date = self.vault.journal_date(&entry.path).map(format_journal_date);
        FileListEntry::Note {
            path: entry.path,
            title,
            filename,
            journal_date,
        }
    }
}

#[async_trait]
impl NoteBrowserProvider for SearchNotesProvider {
    async fn load(&self, query: &str) -> Vec<FileListEntry> {
        if query.is_empty() {
            // Build a lookup map from all indexed notes so we can resolve each
            // last_path to its full metadata in O(1).
            let all_notes = self.vault.get_all_notes().await.unwrap_or_default();
            let mut by_path: std::collections::HashMap<_, _> = all_notes
                .into_iter()
                .map(|(entry, content)| (entry.path.clone(), (entry, content)))
                .collect();

            // last_paths is most-recent-last; iterate in reverse for most-recent-first.
            self.last_paths
                .iter()
                .rev()
                .filter_map(|path| by_path.remove(path))
                .map(|(entry, content)| self.to_entry(entry, content))
                .collect()
        } else {
            self.vault
                .search_notes(query)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|(entry, content)| self.to_entry(entry, content))
                .collect()
        }
    }
}
