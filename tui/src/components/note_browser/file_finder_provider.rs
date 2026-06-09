use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::nfs::{NoteEntryData, VaultPath};
use kimun_core::note::NoteContentData;
use nucleo::Matcher;
use nucleo::pattern::{CaseMatching, Normalization, Pattern};

use super::format_journal_date;
use crate::components::file_list::FileListEntry;
use crate::components::search_list::{Emit, RowSource};

// ---------------------------------------------------------------------------
// MatchEntry — adapts (index, haystack_str) for nucleo match_list
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct MatchEntry {
    idx: usize,
    text: String,
}

impl AsRef<str> for MatchEntry {
    fn as_ref(&self) -> &str {
        &self.text
    }
}

// ---------------------------------------------------------------------------
// FileFinderProvider
// ---------------------------------------------------------------------------

pub struct FileFinderProvider {
    vault: Arc<NoteVault>,
    current_dir: VaultPath,
    notes_cache: Arc<tokio::sync::OnceCell<Vec<(NoteEntryData, NoteContentData)>>>,
}

impl FileFinderProvider {
    pub fn new(vault: Arc<NoteVault>, current_dir: VaultPath) -> Self {
        Self {
            vault,
            current_dir,
            notes_cache: Arc::new(tokio::sync::OnceCell::new()),
        }
    }

    fn to_entry(&self, entry: &NoteEntryData, content: &NoteContentData) -> FileListEntry {
        let filename = entry.path.get_parent_path().1;
        let title = if content.title.trim().is_empty() {
            "<no title>".to_string()
        } else {
            content.title.clone()
        };
        let journal_date = self
            .vault
            .journal_date(&entry.path)
            .map(format_journal_date);
        FileListEntry::Note {
            path: entry.path.clone(),
            title,
            filename,
            journal_date,
            is_open: false,
        }
    }
}

#[async_trait]
impl RowSource<FileListEntry> for FileFinderProvider {
    async fn load(&self, query: &str, emit: Emit<FileListEntry>) {
        let vault = Arc::clone(&self.vault);
        let notes = self
            .notes_cache
            .get_or_init(|| async move { vault.get_all_notes().await.unwrap_or_default() })
            .await;

        if query.is_empty() {
            let mut sorted = notes.clone();
            sorted.sort_by_key(|(entry, _)| std::cmp::Reverse(entry.modified_secs));
            let entries: Vec<FileListEntry> = sorted
                .iter()
                .map(|(entry, content)| self.to_entry(entry, content))
                .collect();
            emit.replace(entries);
            return;
        }

        // Non-empty query: nucleo fuzzy filter
        let candidates: Vec<MatchEntry> = notes
            .iter()
            .enumerate()
            .map(|(i, (entry, content))| {
                let filename = entry.path.get_parent_path().1;
                let text = format!("{} {}", filename, content.title);
                MatchEntry { idx: i, text }
            })
            .collect();

        let query_str = query.to_string();
        let matched = tokio::task::spawn_blocking(move || {
            let mut matcher = Matcher::new(nucleo::Config::DEFAULT);
            let pattern = Pattern::parse(&query_str, CaseMatching::Ignore, Normalization::Smart);
            pattern.match_list(candidates, &mut matcher)
        })
        .await
        .unwrap_or_default();

        let result: Vec<FileListEntry> = matched
            .into_iter()
            .map(|(e, _score)| self.to_entry(&notes[e.idx].0, &notes[e.idx].1))
            .collect();

        // The "Create: <query>" affordance is supplied as a leading row by the
        // engine (see `leading_row`), so it is not part of the loaded set.
        emit.replace(result);
    }

    fn leading_row(&self, query: &str) -> Option<FileListEntry> {
        if query.is_empty() {
            return None;
        }
        // Build a note with `query` resolved against the current directory.
        let resolved = self
            .current_dir
            .append(&VaultPath::note_path_from(query))
            .flatten();
        Some(FileListEntry::CreateNote {
            filename: resolved.to_string(),
            path: resolved,
        })
    }
}
