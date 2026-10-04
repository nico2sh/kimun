use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::nfs::{NoteEntryData, VaultPath};
use kimun_core::note::NoteContentData;
use kimun_core::{NoteVault, strip_order_directive};

use super::format_journal_date;
use crate::components::file_list::{FileListEntry, SortField, SortOrder, entry_order};
use crate::components::query_vars::QueryContext;
use crate::components::search_list::{Emit, ResolvingRowSource, RowSource, Unresolvable};
use crate::components::sortable::{directive_of_query, is_blank_property, property_key};
use kimun_core::PropertySortValue;
use std::collections::HashMap;

/// Build the note-browser search source: a `SearchNotesProvider` wrapped so it
/// resolves `{note}` against `current_note` and falls back to the recent-notes
/// (empty-query) view when a note-dependent query has no note to resolve
/// against. The single place the browser's resolution policy lives — the app
/// and the tests both construct the source through here.
pub fn resolving_search_source(
    vault: Arc<NoteVault>,
    last_paths: Vec<VaultPath>,
    current_note: Option<VaultPath>,
) -> ResolvingRowSource<FileListEntry> {
    ResolvingRowSource::new(
        Arc::new(SearchNotesProvider::new(vault, last_paths)),
        move || QueryContext::with_note(current_note.clone()),
        Unresolvable::AsEmptyQuery,
    )
}

/// The unwrapped vault-backed search source. Private: build it only through
/// [`resolving_search_source`], so every browser source carries the
/// variable-resolution policy and none can bypass it.
struct SearchNotesProvider {
    vault: Arc<NoteVault>,
    last_paths: Vec<VaultPath>,
}

impl SearchNotesProvider {
    fn new(vault: Arc<NoteVault>, last_paths: Vec<VaultPath>) -> Self {
        Self { vault, last_paths }
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
            is_open: false,
        }
    }
}

#[async_trait]
impl RowSource<FileListEntry> for SearchNotesProvider {
    async fn load(&self, query: &str, emit: Emit<FileListEntry>) {
        // The query arrives already resolved: [`ResolvingRowSource`] substitutes
        // `{note}` and maps the purely-note-dependent-but-no-note case to the
        // empty query ([`Unresolvable::AsEmptyQuery`]), which falls here into
        // the recent-notes branch — a dead-end core search is never run.
        // A sort-only query (an order directive and nothing else — what the
        // sort dialog writes over the recents) keeps the recent notes and
        // reorders them; any real term or filter is a core search.
        let sort_only = !query.trim().is_empty() && strip_order_directive(query).trim().is_empty();
        let mut entries: Vec<FileListEntry> = if query.trim().is_empty() || sort_only {
            // Build a lookup map from all indexed notes so we can resolve each
            // last_path to its full metadata in O(1).
            let all_notes = self.vault.get_all_notes().await.unwrap_or_default();
            // The index returns canonical (vault-absolute) paths,
            // while `last_paths` (from history) may be relative — normalize both
            // sides to the same form before matching.
            let norm = |p: &VaultPath| p.flatten().absolute();
            let mut by_path: std::collections::HashMap<_, _> = all_notes
                .into_iter()
                .map(|(entry, content)| (norm(&entry.path), (entry, content)))
                .collect();

            // last_paths is most-recent-first; iterate as-is.
            self.last_paths
                .iter()
                .filter_map(|path| by_path.remove(&norm(path)))
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
        };
        if sort_only {
            self.sort_recents(query, &mut entries).await;
        }
        emit.replace(entries);
    }
}

impl SearchNotesProvider {
    /// Reorder the recent notes by `query`'s order directive, with the same
    /// comparison the FILES panel uses ([`entry_order`]). A property sort
    /// waits for its indexed values; notes without the key go last, in name
    /// order, in both directions. A blank property key, or values that can't
    /// be read, leave the recency order.
    async fn sort_recents(&self, query: &str, entries: &mut [FileListEntry]) {
        let Some((field, order)) = directive_of_query(query) else {
            return;
        };
        if is_blank_property(&field) {
            return;
        }
        let values = match property_key(&field) {
            Some(key) => Some(self.vault.property_sort_values(key).await),
            None => None,
        };
        order_recents(entries, field, order, values);
    }
}

/// Sort `entries` by `field`; `values` is the property read for a property
/// sort. A failed read is logged and leaves the entries as they are.
fn order_recents<E: std::fmt::Display>(
    entries: &mut [FileListEntry],
    field: SortField,
    order: SortOrder,
    values: Option<Result<HashMap<VaultPath, PropertySortValue>, E>>,
) {
    let values = match values {
        Some(Ok(values)) => Some(Arc::new(values)),
        Some(Err(e)) => {
            tracing::warn!("property sort values for recents: {e}");
            return;
        }
        None => None,
    };
    let cmp = entry_order(field, order, false, values);
    entries.sort_by(|a, b| cmp(a, b));
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
    /// core — the resolution is the wrapper's job, the provider only searches.
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
        let source = resolving_search_source(
            vault.clone(),
            vec![],
            Some(VaultPath::note_path_from("spec")),
        );
        let mut list = SearchList::builder(source, redraw_callback(tx))
            .initial_query("={note}")
            .build();
        list.poll_until_idle().await;
        assert!(
            has_note_named(&list.visible_rows(), "spec"),
            "expected the 'spec' note via resolved {{note}}"
        );

        // Without an open note, "={note}" is purely note-dependent → the
        // wrapper falls back to the (empty) recent-notes view and must NOT
        // match "spec".
        let (tx2, _rx2) = unbounded_channel();
        let source_none = resolving_search_source(vault.clone(), vec![], None);
        let mut list_none = SearchList::builder(source_none, redraw_callback(tx2))
            .initial_query("={note}")
            .build();
        list_none.poll_until_idle().await;
        assert!(
            !has_note_named(&list_none.visible_rows(), "spec"),
            "without an open note, {{note}} resolves to empty and must not match 'spec'"
        );
    }

    /// A note-dependent query with no note to resolve against must fall back
    /// to the recent-notes view (like an empty query) instead of running a
    /// search that core drops — a dead-end empty list.
    #[tokio::test]
    async fn unresolvable_note_query_falls_back_to_recent_notes() {
        let vault = temp_vault("search_provider_unresolvable").await;
        vault.validate_and_init().await.unwrap();
        vault
            .create_note(&VaultPath::note_path_from("spec"), "hello")
            .await
            .unwrap();

        // No note open, bare `<` typed: the sugar can't resolve, so the
        // browser shows the recent notes (here: "spec") rather than nothing.
        let (tx, _rx) = unbounded_channel();
        let source =
            resolving_search_source(vault.clone(), vec![VaultPath::note_path_from("spec")], None);
        let mut list = SearchList::builder(source, redraw_callback(tx))
            .initial_query("<")
            .build();
        list.poll_until_idle().await;
        assert!(
            has_note_named(&list.visible_rows(), "spec"),
            "bare `<` with no open note must fall back to recent notes"
        );
    }

    /// A mixed query — concrete terms plus unresolvable note sugar — must
    /// still run the search (core drops the bare prefix), not silently
    /// discard the user's terms for the recent-notes fallback.
    #[tokio::test]
    async fn mixed_query_with_unresolvable_sugar_still_searches() {
        let vault = temp_vault("search_provider_mixed").await;
        vault.validate_and_init().await.unwrap();
        vault
            .create_note(&VaultPath::note_path_from("gadget"), "widget stuff")
            .await
            .unwrap();
        vault
            .create_note(&VaultPath::note_path_from("other"), "nothing here")
            .await
            .unwrap();

        // No note open, query `widget <`: the `widget` term must still
        // filter; "other" is the most recent note and must NOT appear (that
        // would mean the recent-notes fallback swallowed the query).
        let (tx, _rx) = unbounded_channel();
        let source = resolving_search_source(
            vault.clone(),
            vec![VaultPath::note_path_from("other")],
            None,
        );
        let mut list = SearchList::builder(source, redraw_callback(tx))
            .initial_query("widget <")
            .build();
        list.poll_until_idle().await;
        let rows = list.visible_rows();
        assert!(
            has_note_named(&rows, "gadget"),
            "concrete term `widget` must still match"
        );
        assert!(
            !has_note_named(&rows, "other"),
            "mixed query must not fall back to recent notes"
        );
    }

    // ── Sort-only query: the recent notes, reordered ─────────────────────

    fn note_names(rows: &[&FileListEntry]) -> Vec<String> {
        rows.iter()
            .filter_map(|r| match r {
                FileListEntry::Note { path, .. } => Some(path.get_clean_name()),
                _ => None,
            })
            .collect()
    }

    /// Recents (most recent first: c, a, b) over three notes whose titles
    /// run opposite to their file names, plus an unrelated fourth note that
    /// is not in the recent set.
    async fn recents_vault(
        name: &str,
        bodies: [&str; 3],
    ) -> (std::sync::Arc<NoteVault>, Vec<VaultPath>) {
        let vault = temp_vault(name).await;
        vault.validate_and_init().await.unwrap();
        for (file, body) in ["a", "b", "c"].into_iter().zip(bodies) {
            vault
                .create_note(&VaultPath::note_path_from(file), body)
                .await
                .unwrap();
        }
        vault
            .create_note(&VaultPath::note_path_from("zz_not_recent"), "# Aaa\nx")
            .await
            .unwrap();
        let recents = ["c", "a", "b"]
            .into_iter()
            .map(VaultPath::note_path_from)
            .collect();
        (vault, recents)
    }

    async fn names_for(
        vault: &std::sync::Arc<NoteVault>,
        recents: &[VaultPath],
        query: &str,
    ) -> Vec<String> {
        let (tx, _rx) = unbounded_channel();
        let source = resolving_search_source(vault.clone(), recents.to_vec(), None);
        let mut list = SearchList::builder(source, redraw_callback(tx))
            .initial_query(query)
            .build();
        list.poll_until_idle().await;
        note_names(&list.visible_rows())
    }

    #[tokio::test]
    async fn sort_only_query_orders_the_recent_notes_by_title() {
        // titles: a = Charlie, b = Alpha, c = Bravo
        let (vault, recents) = recents_vault(
            "sp_sort_title",
            ["# Charlie\nx", "# Alpha\nx", "# Bravo\nx"],
        )
        .await;
        assert_eq!(names_for(&vault, &recents, "").await, ["c", "a", "b"]);
        assert_eq!(
            names_for(&vault, &recents, "or:title").await,
            ["b", "c", "a"]
        );
        assert_eq!(
            names_for(&vault, &recents, "-or:title").await,
            ["a", "c", "b"]
        );
    }

    #[tokio::test]
    async fn sort_only_query_orders_the_recent_notes_by_file_name() {
        let (vault, recents) =
            recents_vault("sp_sort_file", ["# Charlie\nx", "# Alpha\nx", "# Bravo\nx"]).await;
        assert_eq!(
            names_for(&vault, &recents, "or:file").await,
            ["a", "b", "c"]
        );
        assert_eq!(
            names_for(&vault, &recents, "-or:file").await,
            ["c", "b", "a"]
        );
    }

    #[tokio::test]
    async fn sort_only_query_orders_the_recent_notes_by_property_missing_last() {
        let (vault, recents) = recents_vault(
            "sp_sort_prop",
            [
                "---\nrank: 10\n---\nx",
                "---\nother: 1\n---\nx",
                "---\nrank: 2\n---\nx",
            ],
        )
        .await;
        // a = 10, b = missing, c = 2
        assert_eq!(
            names_for(&vault, &recents, "or:prop:rank").await,
            ["c", "a", "b"]
        );
        assert_eq!(
            names_for(&vault, &recents, "-or:prop:rank").await,
            ["a", "c", "b"],
            "descending keeps the missing note last"
        );
    }

    #[tokio::test]
    async fn sort_directive_with_a_term_still_runs_the_core_search() {
        let (vault, recents) =
            recents_vault("sp_sort_term", ["# Charlie\nx", "# Alpha\nx", "# Bravo\nx"]).await;
        // `Aaa` only matches the note that is NOT in the recent set.
        let names = names_for(&vault, &recents, "Aaa or:title").await;
        assert_eq!(names, ["zz_not_recent"]);
    }

    #[tokio::test]
    async fn blank_property_key_keeps_recency_order() {
        let (vault, recents) = recents_vault(
            "sp_sort_blank",
            ["# Charlie\nx", "# Alpha\nx", "# Bravo\nx"],
        )
        .await;
        assert_eq!(
            names_for(&vault, &recents, "or:prop:").await,
            ["c", "a", "b"]
        );
    }

    #[test]
    fn a_failed_values_read_keeps_recency_order() {
        let note = |n: &str| FileListEntry::Note {
            path: VaultPath::note_path_from(n),
            title: n.to_string(),
            filename: n.to_string(),
            journal_date: None,
            is_open: false,
        };
        let mut rows = vec![note("c"), note("a"), note("b")];
        order_recents(
            &mut rows,
            SortField::Property("rank".into()),
            SortOrder::Ascending,
            Some(Err::<HashMap<_, _>, _>("boom")),
        );
        let refs: Vec<&FileListEntry> = rows.iter().collect();
        assert_eq!(note_names(&refs), ["c", "a", "b"]);
    }
}
