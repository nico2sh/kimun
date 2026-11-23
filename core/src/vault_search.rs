//! Index-backed lookups on [`NoteVault`]: search, labels and their
//! autocomplete, and the note lists by folder. None of these touch the disk
//! except through the index.

use std::collections::HashMap;

use crate::error::VaultError;
use crate::nfs::{NoteEntryData, VaultPath};
use crate::note::NoteContentData;
use crate::{NoteSuggestion, NoteVault, TagSuggestion};

impl NoteVault {
    /// Searches notes using the vault's query syntax (see [`SearchTerms`](crate::SearchTerms)).
    /// Returns each matching note's entry and content data.
    pub async fn search_notes<S: AsRef<str>>(
        &self,
        search_query: S,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        let search_query = search_query.as_ref();
        let a = self.index.search(search_query).await?;
        Ok(a)
    }

    /// Returns every distinct label persisted in the vault, lowercased.
    pub async fn list_labels(&self) -> Result<Vec<String>, VaultError> {
        Ok(self.index.list_labels().await?)
    }

    /// Returns notes whose name (filename without extension) starts with
    /// `prefix`, case-insensitive, capped at `limit`. Used to feed the
    /// wikilink autocomplete popup — note that the inserted wikilink target
    /// is the `name` field, not the `path`.
    pub async fn suggest_notes_by_prefix(
        &self,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<NoteSuggestion>, VaultError> {
        Ok(self.index.suggest_notes_by_prefix(prefix, limit).await?)
    }

    /// Returns tag labels matching `prefix` (case-insensitive) paired with
    /// usage counts, capped at `limit`. Used to feed the hashtag autocomplete
    /// popup in both the editor and the search box.
    pub async fn suggest_tags_by_prefix(
        &self,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<TagSuggestion>, VaultError> {
        Ok(self.index.suggest_tags_by_prefix(prefix, limit).await?)
    }

    /// Returns every distinct label in the vault paired with the number of
    /// notes carrying it. Labels are returned sorted alphabetically.
    pub async fn label_counts(&self) -> Result<Vec<(String, usize)>, VaultError> {
        let rows = self.index.label_counts().await?;
        Ok(rows.into_iter().map(|(n, c)| (n, c as usize)).collect())
    }

    /// Returns every note path that carries the given label. The label
    /// argument is lowercased before lookup, matching how labels are stored.
    pub async fn notes_with_label<S: AsRef<str>>(
        &self,
        name: S,
    ) -> Result<Vec<VaultPath>, VaultError> {
        Ok(self.index.notes_with_label(name.as_ref()).await?)
    }

    /// Get notes under the given path. When `recursive` is false, only direct
    /// children are returned.
    pub async fn get_notes(
        &self,
        path: &VaultPath,
        recursive: bool,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        let notes = self.index.get_notes(path, recursive).await?;
        Ok(notes)
    }

    /// Counts the notes under each immediate child directory of `parent`,
    /// at any depth. Keys are canonical child directory paths; a child with
    /// no notes is absent, and notes directly in `parent` are not counted.
    pub async fn child_note_counts(
        &self,
        parent: &VaultPath,
    ) -> Result<HashMap<VaultPath, usize>, VaultError> {
        let depth = parent.canonical().get_slices().len();
        let mut counts: HashMap<VaultPath, usize> = HashMap::new();
        for (base, n) in self.index.note_counts_by_base(parent).await? {
            let mut child = VaultPath::new(base).canonical();
            let mut len = child.get_slices().len();
            if len <= depth {
                continue; // a note directly in `parent`
            }
            while len > depth + 1 {
                child = child.get_parent_path().0;
                len -= 1;
            }
            *counts.entry(child).or_insert(0) += n as usize;
        }
        Ok(counts)
    }

    /// Returns every note in the vault with its entry and content data.
    pub async fn get_all_notes(&self) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        let a = self.index.get_all_notes().await?;
        Ok(a)
    }

    /// The note's labels: inline `#hashtags` and frontmatter `tags:` together,
    /// lowercased, sorted, distinct. Index-backed: a missing or not-yet-indexed
    /// note returns an empty list, not `NotFound`.
    pub async fn get_tags(&self, path: &VaultPath) -> Result<Vec<String>, VaultError> {
        Ok(self.index.labels_of(path).await?)
    }
}
