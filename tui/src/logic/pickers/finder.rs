//! The file finder's rows (TUI Ctrl+O, leader `f f`): every note in the vault,
//! most recently modified first for a blank query, nucleo fuzzy ranked over
//! `"<filename> <title>"` otherwise, with a `Create: <query>` leading row.

use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::nfs::{NoteEntryData, VaultPath};
use kimun_core::note::NoteContentData;
use nucleo::Matcher;
use nucleo::pattern::{CaseMatching, Normalization, Pattern};

use crate::logic::files::{FileListEntry, format_journal_date};
use crate::logic::list::{Emit, RowSource};

/// One indexed note as [`NoteVault::get_all_notes`] returns it.
pub type NoteRecord = (NoteEntryData, NoteContentData);

/// The picker row for an indexed note: filename, display title (`<no title>`
/// for a blank one), and the journal date when the note is a journal entry.
pub fn note_row(
    vault: &NoteVault,
    entry: &NoteEntryData,
    content: &NoteContentData,
) -> FileListEntry {
    FileListEntry::Note {
        path: entry.path.clone(),
        title: FileListEntry::display_title(content.title.clone()),
        filename: entry.path.get_parent_path().1,
        journal_date: vault.journal_date(&entry.path).map(format_journal_date),
        is_open: false,
    }
}

/// Indices into `notes`, most recently modified first (`modified_secs`
/// descending; equal times keep `notes` order). The blank-query order.
pub fn recency_order(notes: &[NoteRecord]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..notes.len()).collect();
    idx.sort_by_key(|&i| std::cmp::Reverse(notes[i].0.modified_secs));
    idx
}

// Adapts (index, haystack) for nucleo's `match_list`.
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

fn candidates(notes: &[NoteRecord]) -> Vec<MatchEntry> {
    notes
        .iter()
        .enumerate()
        .map(|(idx, (entry, content))| MatchEntry {
            idx,
            text: format!("{} {}", entry.path.get_parent_path().1, content.title),
        })
        .collect()
}

fn rank(candidates: Vec<MatchEntry>, query: &str) -> Vec<usize> {
    let mut matcher = Matcher::new(nucleo::Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    pattern
        .match_list(candidates, &mut matcher)
        .into_iter()
        .map(|(e, _score)| e.idx)
        .collect()
}

/// Indices into `notes` matching `query`, best first: nucleo over
/// `"<filename> <title>"`, ignore case, smart normalisation. **Blocking** (CPU
/// bound over the whole vault) — run it off the UI thread.
pub fn fuzzy_match(notes: &[NoteRecord], query: &str) -> Vec<usize> {
    rank(candidates(notes), query)
}

/// The finder's display indices for `query`: [`recency_order`] when blank,
/// else [`fuzzy_match`] (blocking).
pub fn finder_order(notes: &[NoteRecord], query: &str) -> Vec<usize> {
    if query.is_empty() {
        recency_order(notes)
    } else {
        fuzzy_match(notes, query)
    }
}

/// The finder's rows for `query` (without the `Create:` row, see
/// [`create_row`]). Blocking for a non-blank query, like [`fuzzy_match`].
pub fn finder_rows(vault: &NoteVault, notes: &[NoteRecord], query: &str) -> Vec<FileListEntry> {
    finder_order(notes, query)
        .into_iter()
        .map(|i| note_row(vault, &notes[i].0, &notes[i].1))
        .collect()
}

/// The `Create: <query>` row: a note named `query` resolved against
/// `current_dir` (the note extension added, `.`/`..` flattened). Present for
/// every non-blank query — even when a note of that name already exists, in
/// which case creating it just opens the existing note (`load_or_create_note`).
pub fn create_row(current_dir: &VaultPath, query: &str) -> Option<FileListEntry> {
    if query.is_empty() {
        return None;
    }
    let resolved = current_dir
        .append(&VaultPath::note_path_from(query))
        .flatten();
    Some(FileListEntry::CreateNote {
        filename: resolved.to_string(),
        path: resolved,
    })
}

/// The finder as a [`RowSource`]: loads the vault's notes once (cached for the
/// provider's life), then re-ranks on every query; [`create_row`] is the
/// leading row. Drive it with a [`ListCore`](crate::logic::list::ListCore).
pub struct FileFinderProvider {
    vault: Arc<NoteVault>,
    current_dir: VaultPath,
    notes_cache: Arc<tokio::sync::OnceCell<Vec<NoteRecord>>>,
}

impl FileFinderProvider {
    /// A finder over `vault`; `Create:` rows resolve against `current_dir`.
    pub fn new(vault: Arc<NoteVault>, current_dir: VaultPath) -> Self {
        Self {
            vault,
            current_dir,
            notes_cache: Arc::new(tokio::sync::OnceCell::new()),
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

        let order = if query.is_empty() {
            recency_order(notes)
        } else {
            let candidates = candidates(notes);
            let query = query.to_string();
            tokio::task::spawn_blocking(move || rank(candidates, &query))
                .await
                .unwrap_or_default()
        };
        let rows = order
            .into_iter()
            .map(|i| note_row(&self.vault, &notes[i].0, &notes[i].1))
            .collect();
        // The "Create: <query>" affordance is the engine's leading row (see
        // `leading_row`), so it is not part of the loaded set.
        emit.replace(rows);
    }

    fn leading_row(&self, query: &str) -> Option<FileListEntry> {
        create_row(&self.current_dir, query)
    }
}
