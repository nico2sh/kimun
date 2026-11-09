//! Saved searches, toolkit-free: the Saved Searches modal's rows
//! ([`SearchItem`], the virtual "Backlinks (current note)" entry, 1–9
//! quick-select numbering), their ranking ([`rank_to_indices`]) and the
//! load-once source that also performs deletes ([`SavedSearchSource`]); and
//! what saving under a name would do ([`SaveHint`], [`save_hint`]). The
//! saved-search breadcrumb is [`crate::logic::query::SavedSearchBreadcrumb`].
//! See CONTEXT.md (Saved search).

use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::{NoteVault, SavedSearch};

use crate::logic::list::{Emit, ListRow, RowSource, YankTarget};

/// One row in the modal. `index` is the 1–9 quick-select number (only the
/// first nine USER searches get one). The virtual backlinks entry is pinned
/// at the top (supplied as the engine's `leading_row`) and is never numbered
/// or deletable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchItem {
    pub index: Option<u8>,
    pub name: String,
    pub query: String,
    pub is_virtual: bool,
}

impl SearchItem {
    /// A normal (user) saved-search item with a quick-select index.
    pub fn saved(index: u8, name: &str, query: &str) -> Self {
        Self {
            index: Some(index),
            name: name.to_string(),
            query: query.to_string(),
            is_virtual: false,
        }
    }
}

impl ListRow for SearchItem {
    /// The virtual backlinks row is filter-exempt: returning `None` makes the
    /// engine keep it present regardless of the query (it is also prepended by
    /// the engine when the rank closure drops it).
    fn match_text(&self) -> Option<&str> {
        if self.is_virtual {
            None
        } else {
            Some(&self.name)
        }
    }

    /// The stored query, not the display name — the query is the reusable
    /// thing (paste it into another search, or into a note).
    fn yank_target(&self) -> Option<YankTarget> {
        Some(YankTarget::new(self.query.clone(), "query"))
    }
}

pub const VIRTUAL_BACKLINKS_NAME: &str = "Backlinks (current note)";
pub const VIRTUAL_BACKLINKS_QUERY: &str = "<{note}";

pub struct SavedSearchesModel;

impl SavedSearchesModel {
    /// Build the USER rows from the vault's saved searches: the first nine get
    /// quick-select indices 1..=9, the rest are unnumbered. The pinned virtual
    /// backlinks row is NOT included here — the engine supplies it via
    /// [`RowSource::leading_row`].
    pub fn user_items(user: Vec<SavedSearch>) -> Vec<SearchItem> {
        user.into_iter()
            .enumerate()
            .map(|(i, s)| SearchItem {
                index: if i < 9 { Some((i + 1) as u8) } else { None },
                name: s.name,
                query: s.query,
                is_virtual: false,
            })
            .collect()
    }
}

/// Rank the rows named by `base` (indices into `rows`, USER rows only, already
/// in list order), returning DISPLAY INDICES into `rows`. An exact leading-index
/// match (filter parses to a u8 equal to a row's `index`) ranks that row first;
/// otherwise a case-insensitive name substring match. Stable order preserves
/// `base` order within a rank. Empty filter → `base` unchanged. The engine
/// re-adds any filter-exempt rows (the virtual backlinks row) that this closure
/// omits, so it may ignore the virtual row.
pub fn rank_to_indices(rows: &[SearchItem], base: &[usize], filter: &str) -> Vec<usize> {
    let f = filter.trim();
    if f.is_empty() {
        return base.to_vec();
    }
    let as_index: Option<u8> = f.parse().ok();
    let needle = f.to_lowercase();
    let mut ranked: Vec<(usize, u8)> = Vec::new(); // (index, rank: 0 = best)
    for &i in base {
        let it = &rows[i];
        let exact_index = as_index.is_some() && it.index == as_index;
        let name_match = it.name.to_lowercase().contains(&needle);
        if exact_index {
            ranked.push((i, 0));
        } else if name_match {
            ranked.push((i, 1));
        }
    }
    // stable sort by rank keeps `base` relative order within a rank
    ranked.sort_by_key(|(_, r)| *r);
    ranked.into_iter().map(|(i, _)| i).collect()
}

/// Loads the vault's saved searches once (`reload_on_query == false`); the
/// local [`Filter::Rank`](crate::logic::list::Filter::Rank) narrows the set per keystroke. The virtual backlinks
/// row is supplied by [`leading_row`](RowSource::leading_row), not the load.
///
/// Deletes are routed THROUGH the load (via `pending_delete`) so the delete and
/// the subsequent list-read happen in one ordered async step — avoiding the
/// race where a separately-spawned delete and a `reload()` interleave and the
/// reload reads pre-delete state.
pub struct SavedSearchSource {
    vault: Arc<NoteVault>,
    pending_delete: Arc<std::sync::Mutex<Option<String>>>,
}

impl SavedSearchSource {
    /// `pending_delete` is the shared slot a UI fills with a saved-search name
    /// before reloading its list: the next load deletes it, then lists.
    pub fn new(
        vault: Arc<NoteVault>,
        pending_delete: Arc<std::sync::Mutex<Option<String>>>,
    ) -> Self {
        Self {
            vault,
            pending_delete,
        }
    }
}

#[async_trait]
impl RowSource<SearchItem> for SavedSearchSource {
    async fn load(&self, _query: &str, emit: Emit<SearchItem>) {
        // Drain any pending delete BEFORE listing, so the list read below is
        // ordered strictly after the delete completes.
        let to_delete = self.pending_delete.lock().unwrap().take();
        if let Some(name) = to_delete {
            self.vault.delete_saved_search(&name).await.ok();
        }
        let user = self.vault.list_saved_searches().await.unwrap_or_default();
        emit.replace(SavedSearchesModel::user_items(user));
    }

    fn leading_row(&self, _query: &str) -> Option<SearchItem> {
        Some(SearchItem {
            index: None,
            name: VIRTUAL_BACKLINKS_NAME.to_string(),
            query: VIRTUAL_BACKLINKS_QUERY.to_string(),
            is_virtual: true,
        })
    }

    fn reload_on_query(&self) -> bool {
        false
    }
}

/// What submitting the save-search dialog will do with the current name
/// field — drives the live hint line so an overwrite is never silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveHint {
    /// The name matches the saved search the query came from (the breadcrumb
    /// provenance): submitting updates it in place.
    Update(String),
    /// The name matches a different existing saved search: submitting
    /// replaces that search's query. Rendered as a warning.
    Overwrite(String),
    /// A fresh name: submitting creates a new saved search.
    SaveNew,
    /// The name field is empty: submitting saves a new search named after the
    /// query itself (the query-as-name fallback).
    SaveNewAsQuery(String),
    /// The existing names have not loaded yet, so save-new vs overwrite is
    /// unknown (the provenance Update case never waits — it is checked
    /// synchronously). Submitting still saves.
    Pending,
}

/// The name a submit would save under: the typed name, or the trimmed
/// query when the field is empty (the query-as-name fallback).
pub fn effective_save_name<'a>(typed: &'a str, query: &'a str) -> &'a str {
    let typed = typed.trim();
    if typed.is_empty() {
        query.trim()
    } else {
        typed
    }
}

/// What submitting `typed` (the raw name field) for `query` would do.
/// `provenance` is the saved search the query came from (the breadcrumb),
/// `existing` the vault's saved-search names (`None` until loaded). Name
/// matching delegates to core's `saved_search_name_matches` — the same rule
/// `save_search` applies on write, so the hint can never disagree with the
/// actual save outcome.
pub fn save_hint(
    typed: &str,
    query: &str,
    provenance: Option<&str>,
    existing: Option<&[String]>,
) -> SaveHint {
    let matches = kimun_core::saved_search_name_matches;
    let effective = effective_save_name(typed, query);
    if let Some(p) = provenance
        && matches(p, effective)
    {
        return SaveHint::Update(p.to_string());
    }
    let Some(existing) = existing else {
        return SaveHint::Pending;
    };
    if let Some(name) = existing.iter().find(|n| matches(n, effective)) {
        return SaveHint::Overwrite(name.clone());
    }
    if typed.trim().is_empty() {
        SaveHint::SaveNewAsQuery(query.to_string())
    } else {
        SaveHint::SaveNew
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_items_skip_virtual_and_number_first_nine() {
        let user: Vec<SavedSearch> = (0..11)
            .map(|i| SavedSearch {
                name: format!("s{i}"),
                query: format!("#{i}"),
            })
            .collect();
        let items = SavedSearchesModel::user_items(user);
        // No virtual row here — it is supplied by leading_row.
        assert!(items.iter().all(|i| !i.is_virtual));
        assert_eq!(items[0].index, Some(1));
        assert_eq!(items[8].index, Some(9));
        assert_eq!(items[9].index, None); // 10th user search unnumbered
    }

    /// Every index into `items`, in order — the `base` a list with no
    /// `order_by` hands the ranker.
    fn all(items: &[SearchItem]) -> Vec<usize> {
        (0..items.len()).collect()
    }

    #[test]
    fn rank_exact_index_first() {
        let items = vec![
            SearchItem::saved(1, "todo", "#todo"),
            SearchItem::saved(2, "backlinks-ish", "<{note}"),
            SearchItem::saved(3, "two-things", "#a"),
        ];
        let idx = rank_to_indices(&items, &all(&items), "2");
        assert_eq!(items[idx[0]].name, "backlinks-ish"); // index 2 wins
        let idx = rank_to_indices(&items, &all(&items), "tod");
        assert_eq!(items[idx[0]].name, "todo");
    }

    #[test]
    fn rank_empty_filter_returns_all_in_order() {
        let items = vec![
            SearchItem::saved(1, "a", "#a"),
            SearchItem::saved(2, "b", "#b"),
        ];
        let idx = rank_to_indices(&items, &all(&items), "");
        assert_eq!(idx, vec![0, 1]);
    }

    /// The ranker never re-derives the candidate set: it walks `base`, so a
    /// list order (or a narrower candidate set) reaches the ranked output.
    #[test]
    fn rank_honors_the_base_order_within_a_rank() {
        let items = vec![
            SearchItem::saved(1, "todo-a", "#a"),
            SearchItem::saved(2, "todo-b", "#b"),
            SearchItem::saved(3, "todo-c", "#c"),
        ];
        // Same rank for all three (name substring), reversed base order.
        let idx = rank_to_indices(&items, &[2, 1, 0], "todo");
        assert_eq!(idx, vec![2, 1, 0]);
        // A base that omits a row omits it from the result.
        let idx = rank_to_indices(&items, &[0, 2], "todo");
        assert_eq!(idx, vec![0, 2]);
        // An empty filter passes `base` straight through.
        assert_eq!(rank_to_indices(&items, &[1, 0], ""), vec![1, 0]);
    }

    #[test]
    fn rank_name_substring_only_matches() {
        let items = vec![
            SearchItem::saved(1, "todo", "#todo"),
            SearchItem::saved(2, "ideas", "#ideas"),
        ];
        let idx = rank_to_indices(&items, &all(&items), "ide");
        assert_eq!(idx.len(), 1);
        assert_eq!(items[idx[0]].name, "ideas");
    }
}
