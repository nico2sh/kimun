use async_trait::async_trait;
use kimun_core::nfs::NoteEntryData;
use kimun_core::note::NoteContentData;

use super::NoteBrowserProvider;
use crate::components::file_list::FileListEntry;

/// A provider pre-populated with a fixed list of notes, used when following a
/// link that resolves to several candidates.  The query is ignored.
pub struct LinkResultsProvider {
    entries: Vec<FileListEntry>,
}

impl LinkResultsProvider {
    pub fn from_results(results: Vec<(NoteEntryData, NoteContentData)>) -> Self {
        let entries = results
            .into_iter()
            .map(|(entry, content)| {
                let filename = entry.path.get_parent_path().1;
                let title = if content.title.trim().is_empty() {
                    "<no title>".to_string()
                } else {
                    content.title
                };
                FileListEntry::Note {
                    path: entry.path,
                    title,
                    filename,
                    journal_date: None,
                }
            })
            .collect();
        Self { entries }
    }
}

#[async_trait]
impl NoteBrowserProvider for LinkResultsProvider {
    async fn load(&self, query: &str) -> Vec<FileListEntry> {
        if query.is_empty() {
            return self.entries.clone();
        }
        let q = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| match e {
                FileListEntry::Note {
                    title,
                    filename,
                    path,
                    ..
                } => {
                    title.to_lowercase().contains(&q)
                        || filename.to_lowercase().contains(&q)
                        || path.to_string().to_lowercase().contains(&q)
                }
                _ => false,
            })
            .cloned()
            .collect()
    }
}
