use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;
use kimun_core::NoteVault;
use kimun_core::nfs::NoteEntryData;
use kimun_core::note::NoteContentData;

use crate::components::file_list::FileListEntry;
use super::NoteBrowserProvider;

pub struct SearchNotesProvider {
    vault: Arc<NoteVault>,
}

impl SearchNotesProvider {
    pub fn new(vault: Arc<NoteVault>) -> Self {
        Self { vault }
    }

    fn into_entry(&self, entry: NoteEntryData, content: NoteContentData) -> FileListEntry {
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
            let mut notes = self.vault.get_all_notes().await.unwrap_or_default();
            notes.sort_by(|(a, _), (b, _)| b.modified_secs.cmp(&a.modified_secs));
            notes.truncate(20);
            notes
                .into_iter()
                .map(|(entry, content)| self.into_entry(entry, content))
                .collect()
        } else {
            self.vault
                .search_notes(query)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|(entry, content)| self.into_entry(entry, content))
                .collect()
        }
    }
}

fn format_journal_date(date: NaiveDate) -> String {
    date.format("%A, %B %-d, %Y").to_string()
}
