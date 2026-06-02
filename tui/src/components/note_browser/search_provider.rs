use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::nfs::{NoteEntryData, VaultPath};
use kimun_core::note::NoteContentData;

use super::format_journal_date;
use crate::components::file_list::FileListEntry;
use crate::components::query_vars::resolve_query;
use crate::components::search_list::{Emit, RowSource};

pub struct SearchNotesProvider {
    vault: Arc<NoteVault>,
    last_paths: Vec<VaultPath>,
    /// The note open when the browser was launched, used to resolve query
    /// variables like `{note}` before the query reaches core. `None` when no
    /// note is open (e.g. launched from the root browse view).
    current_note: Option<VaultPath>,
}

impl SearchNotesProvider {
    pub fn new(
        vault: Arc<NoteVault>,
        last_paths: Vec<VaultPath>,
        current_note: Option<VaultPath>,
    ) -> Self {
        Self {
            vault,
            last_paths,
            current_note,
        }
    }

    fn to_entry(&self, entry: NoteEntryData, content: NoteContentData) -> FileListEntry {
        let filename = entry.path.get_parent_path().1;
        let title = if content.title.trim().is_empty() {
            "<no title>".to_string()
        } else {
            content.title
        };
        let journal_date = self
            .vault
            .journal_date(&entry.path)
            .map(format_journal_date);
        FileListEntry::Note {
            path: entry.path,
            title,
            filename,
            journal_date,
        }
    }
}

#[async_trait]
impl RowSource<FileListEntry> for SearchNotesProvider {
    async fn load(&self, query: &str, emit: Emit<FileListEntry>) {
        let entries: Vec<FileListEntry> = if query.is_empty() {
            // Build a lookup map from all indexed notes so we can resolve each
            // last_path to its full metadata in O(1).
            let all_notes = self.vault.get_all_notes().await.unwrap_or_default();
            let mut by_path: std::collections::HashMap<_, _> = all_notes
                .into_iter()
                .map(|(entry, content)| (entry.path.clone(), (entry, content)))
                .collect();

            // last_paths is most-recent-first; iterate as-is.
            self.last_paths
                .iter()
                .filter_map(|path| by_path.remove(path))
                .map(|(entry, content)| self.to_entry(entry, content))
                .collect()
        } else {
            // Resolve query variables ({note}, …) against the open note before
            // handing a plain query to core — the same presentation-layer step
            // the Query panel does. Without this, `{note}` reaches core
            // literally and matches nothing.
            let resolved = resolve_query(query, self.current_note.as_ref());
            self.vault
                .search_notes(&resolved)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|(entry, content)| self.to_entry(entry, content))
                .collect()
        };
        emit.replace(entries);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::events::redraw_callback;
    use crate::components::search_list::SearchList;
    use crate::test_support::temp_vault;
    use tokio::sync::mpsc::unbounded_channel;

    fn has_note_named(rows: &[&FileListEntry], name: &str) -> bool {
        rows.iter().any(|r| match r {
            FileListEntry::Note { path, .. } => path.get_clean_name() == name,
            _ => false,
        })
    }

    /// `{note}` must be resolved against the open note before the query reaches
    /// core — the same presentation-layer step the Query panel performs.
    #[tokio::test]
    async fn resolves_note_variable_before_search() {
        let vault = temp_vault("search_provider_note_var").await;
        // Build the DB schema/index (temp_vault only opens the vault).
        vault.validate_and_init().await.unwrap();
        vault
            .create_note(&VaultPath::note_path_from("spec"), "hello")
            .await
            .unwrap();

        // With the open note = "spec", "={note}" resolves to "=spec" and the
        // name filter matches the note.
        let (tx, _rx) = unbounded_channel();
        let provider = SearchNotesProvider::new(
            vault.clone(),
            vec![],
            Some(VaultPath::note_path_from("spec")),
        );
        let mut list = SearchList::builder(provider, redraw_callback(tx))
            .initial_query("={note}")
            .build();
        list.poll_until_idle().await;
        assert!(
            has_note_named(&list.visible_rows(), "spec"),
            "expected the 'spec' note via resolved {{note}}"
        );

        // Without an open note, "={note}" resolves to bare "=" and must NOT
        // match "spec" — proving the literal `{note}` was substituted away.
        let (tx2, _rx2) = unbounded_channel();
        let provider_none = SearchNotesProvider::new(vault.clone(), vec![], None);
        let mut list_none = SearchList::builder(provider_none, redraw_callback(tx2))
            .initial_query("={note}")
            .build();
        list_none.poll_until_idle().await;
        assert!(
            !has_note_named(&list_none.visible_rows(), "spec"),
            "without an open note, {{note}} resolves to empty and must not match 'spec'"
        );
    }
}
