//! Saved searches on [`NoteVault`]: the named queries kept in the vault's
//! `saved_searches` file, and the prefix lookup that feeds query autocomplete.

use crate::error::VaultError;
use crate::nfs::saved_searches;
use crate::{saved_search_name_matches, NoteVault, SavedSearch};

impl NoteVault {
    /// List the vault's saved searches (see `SavedSearch`). Empty if none.
    pub async fn list_saved_searches(&self) -> Result<Vec<SavedSearch>, VaultError> {
        Ok(saved_searches::read_saved_searches(self.workspace_path()).await?)
    }

    /// Saved searches whose name starts with `prefix` (case-insensitive,
    /// ASCII folding to match [`Self::save_search`]/[`Self::delete_saved_search`]), in
    /// stored order, capped at `limit`. Feeds the `?`-prefix autocomplete in
    /// the query input — mirrors [`Self::suggest_notes_by_prefix`] /
    /// [`Self::suggest_tags_by_prefix`] so prefix matching stays in core. Saved
    /// searches are file-backed (not indexed), so this reads the small TOML
    /// file; it is the single place to add caching if it ever gets hot.
    pub async fn suggest_saved_searches_by_prefix(
        &self,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<SavedSearch>, VaultError> {
        let prefix = prefix.to_ascii_lowercase();
        let mut matches = saved_searches::read_saved_searches(self.workspace_path()).await?;
        matches.retain(|s| s.name.to_ascii_lowercase().starts_with(&prefix));
        matches.truncate(limit);
        Ok(matches)
    }

    /// Insert or replace a saved search by name (case-insensitive match,
    /// preserving the existing position on overwrite). Appends if new.
    pub async fn save_search(&self, name: &str, query: &str) -> Result<(), VaultError> {
        let mut all = saved_searches::read_saved_searches(self.workspace_path()).await?;
        let entry = SavedSearch {
            name: name.to_string(),
            query: query.to_string(),
        };
        match all
            .iter_mut()
            .find(|s| saved_search_name_matches(&s.name, name))
        {
            Some(existing) => *existing = entry,
            None => all.push(entry),
        }
        saved_searches::write_saved_searches(self.workspace_path(), &all).await?;
        Ok(())
    }

    /// Delete a saved search by name (case-insensitive). No-op if absent.
    pub async fn delete_saved_search(&self, name: &str) -> Result<(), VaultError> {
        let mut all = saved_searches::read_saved_searches(self.workspace_path()).await?;
        all.retain(|s| !saved_search_name_matches(&s.name, name));
        saved_searches::write_saved_searches(self.workspace_path(), &all).await?;
        Ok(())
    }

    /// Rename a saved search, preserving its position and query. No-op if absent.
    pub async fn rename_saved_search(&self, old: &str, new: &str) -> Result<(), VaultError> {
        let mut all = saved_searches::read_saved_searches(self.workspace_path()).await?;
        if let Some(existing) = all
            .iter_mut()
            .find(|s| saved_search_name_matches(&s.name, old))
        {
            existing.name = new.to_string();
        }
        saved_searches::write_saved_searches(self.workspace_path(), &all).await?;
        Ok(())
    }

    // ── Pinned notes ──────────────────────────────────────────────────────
}
