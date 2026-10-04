//! One contract for every list the sort dialog can sort — the sidebar, the
//! Query panel, the Ctrl+K search browser and the Ctrl+O file finder. The dialog is built from a
//! list's [`SortState`] and [`SortableList::allows_property`], and every
//! selection it emits is applied through [`SortableList::apply_sort`], so the
//! targets cannot drift apart in how a sort lands.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use kimun_core::note::property_search_key;
use kimun_core::{NoteVault, OrderBy, OrderField, SearchTerms, with_order_directive};

use crate::components::events::{AppEvent, AppTx};
use crate::components::file_list::{PropertyValues, SortField, SortOrder};

/// What the sort dialog shows and changes for one list.
#[derive(Debug, Clone, PartialEq)]
pub struct SortState {
    pub field: SortField,
    pub order: SortOrder,
    /// `Some` only for lists that can group directories (the sidebar).
    pub group_dirs: Option<bool>,
}

/// A list the sort dialog can sort.
pub trait SortableList {
    fn sort_state(&self) -> SortState;
    /// Apply a selection from the sort dialog. Property sorts are only
    /// offered where [`Self::allows_property`] says so.
    fn apply_sort(&mut self, state: &SortState, tx: &AppTx);
    /// Whether the dialog offers Property. Query-backed lists put the key in
    /// the query; listings fetch values with [`PropertySort`].
    fn allows_property(&self) -> bool;
    /// The rows are in the list's natural order (recency, match rank), not
    /// any sort the dialog offers — it then shows "Unsorted" until a pick.
    /// [`Self::sort_state`] still names the field / order the first toggle
    /// starts from. Only the file finder and the Ctrl+K recents say so.
    fn is_unsorted(&self) -> bool {
        false
    }
}

/// One values fetch as the task delivers it. `failed`: the read errored and
/// `values` is the empty stand-in (every row sorts as missing).
struct FetchResult {
    key: String,
    values: PropertyValues,
    failed: bool,
}

/// The property values a listing (sidebar, file finder) orders by: one async
/// fetch per key, cached only while the list keeps sorting by that key (an
/// Order / Group toggle reuses them; leaving the property sort or a failed
/// read does not count as a cache). Results arrive on a channel
/// the owner polls each frame; a result for a key the list no longer sorts
/// by is dropped.
#[derive(Default)]
pub struct PropertySort {
    /// The key the cached / pending values are for.
    key: Option<String>,
    values: Option<PropertyValues>,
    rx: Option<Receiver<FetchResult>>,
    /// The last read for `key` failed: `values` is the empty stand-in, so
    /// the next [`Self::ensure`] retries.
    failed: bool,
    /// Fetches started, for tests that pin when a refetch happens.
    #[cfg(test)]
    pub(crate) fetches: usize,
}

impl PropertySort {
    /// The cached values, if they are for `key` (compared in core's search
    /// form, so `Rank` and `rank` share them).
    pub fn values_for(&self, key: &str) -> Option<PropertyValues> {
        let key = property_search_key(key)?;
        (self.key.as_deref() == Some(key.as_str()))
            .then(|| self.values.clone())
            .flatten()
    }

    /// The cached values a listing sorted by `field` orders with: `None` for
    /// Name / Title, or while a property sort's values are still awaited.
    pub fn values_for_field(&self, field: &SortField) -> Option<PropertyValues> {
        property_key(field).and_then(|k| self.values_for(k))
    }

    /// A property sort whose values have not landed yet — the listing keeps
    /// its current order until they do.
    pub fn is_awaiting(&self, field: &SortField) -> bool {
        property_key(field).is_some_and(|k| self.values_for(k).is_none())
    }

    /// Fetch `key`'s values in the background (a redraw follows the result).
    /// Values cached for another key are dropped; for the same key they stay
    /// in use until the fresh ones land. A blank key fetches nothing.
    pub fn fetch(&mut self, vault: &Arc<NoteVault>, key: &str, tx: &AppTx) {
        let Some(key) = property_search_key(key) else {
            return;
        };
        if self.key.as_deref() != Some(key.as_str()) {
            self.values = None;
            self.key = Some(key.clone());
        }
        self.failed = false;
        #[cfg(test)]
        {
            self.fetches += 1;
        }
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        self.rx = Some(result_rx);
        let vault = Arc::clone(vault);
        let tx = tx.clone();
        tokio::spawn(async move {
            let (values, failed) = match vault.property_sort_values(&key).await {
                Ok(values) => (values, false),
                Err(e) => {
                    report_fetch_error(&key, &e, &tx);
                    (Default::default(), true)
                }
            };
            result_tx
                .send(FetchResult {
                    key,
                    values: Arc::new(values),
                    failed,
                })
                .ok();
            tx.send(AppEvent::Redraw).ok();
        });
    }

    /// Make `key`'s values available: fetch them unless a read for that key
    /// is in flight or succeeded while the list kept sorting by it (so
    /// Order / Group toggles reuse what was read). A list that reloads
    /// calls [`Self::fetch`] instead, so edits since the last read show up.
    pub fn ensure(&mut self, vault: &Arc<NoteVault>, key: &str, tx: &AppTx) {
        let Some(folded) = property_search_key(key) else {
            return;
        };
        let same_key = self.key.as_deref() == Some(folded.as_str());
        if same_key && (self.rx.is_some() || (self.values.is_some() && !self.failed)) {
            return;
        }
        self.fetch(vault, key, tx);
    }

    /// Follow a sort the list just applied: a property sort makes its values
    /// available ([`Self::ensure`]); Name / Title forget the cache, so coming
    /// back to a property later reads fresh values.
    pub fn sync(&mut self, vault: &Arc<NoteVault>, field: &SortField, tx: &AppTx) {
        match property_key(field) {
            Some(key) => self.ensure(vault, key, tx),
            None => self.clear(),
        }
    }

    /// Drop the cached values and any fetch in flight.
    pub fn clear(&mut self) {
        self.key = None;
        self.values = None;
        self.rx = None;
        self.failed = false;
    }

    /// Take a fetched result if one has landed, for the owner to hand to
    /// [`Self::receive`] (through its own re-sorting entry point).
    pub fn poll(&mut self) -> Option<(String, PropertyValues)> {
        let rx = self.rx.as_ref()?;
        match rx.try_recv() {
            Ok(FetchResult {
                key,
                values,
                failed,
            }) => {
                self.rx = None;
                self.failed = failed;
                Some((key, values))
            }
            Err(TryRecvError::Disconnected) => {
                self.rx = None;
                None
            }
            Err(TryRecvError::Empty) => None,
        }
    }

    /// Accept `values` if they are for the key the list sorts by.
    pub fn receive(&mut self, key: &str, values: PropertyValues) -> bool {
        let key = property_search_key(key);
        if key.is_none() || self.key != key {
            return false;
        }
        self.values = Some(values);
        true
    }

    /// `true` while a fetch is in flight.
    #[cfg(test)]
    pub fn is_pending(&self) -> bool {
        self.rx.is_some()
    }
}

/// A failed values fetch: log it and tell the user. Every row then sorts as
/// missing (the fetch delivers an empty map).
fn report_fetch_error(key: &str, e: &dyn std::fmt::Display, tx: &AppTx) {
    tracing::warn!("property sort values for {key}: {e}");
    tx.send(AppEvent::FlashMessage(format!(
        "couldn't load property values: {e}"
    )))
    .ok();
}

/// The key of a property sort, `None` for Name / Title.
pub fn property_key(field: &SortField) -> Option<&str> {
    match field {
        SortField::Property(key) => Some(key.as_str()),
        _ => None,
    }
}

/// A property sort with no key yet — not a usable order.
pub fn is_blank_property(field: &SortField) -> bool {
    matches!(field, SortField::Property(key) if key.trim().is_empty())
}

/// The sort a query string asks for, read from its first order directive.
/// `(Name, Ascending)` when the query has none. Shared by every query-backed
/// list so the dialog opens on what the query actually says.
pub fn order_of_query(query: &str) -> (SortField, SortOrder) {
    directive_of_query(query).unwrap_or((SortField::Name, SortOrder::Ascending))
}

/// The sort a query's first usable order directive asks for; `None` when it
/// has none (a bare `or:prop:` with no key is not usable).
pub fn directive_of_query(query: &str) -> Option<(SortField, SortOrder)> {
    let st = SearchTerms::from_query_string(query);
    let (field, asc) = match st.order_by.first()? {
        OrderBy::Title { asc } => (SortField::Title, *asc),
        OrderBy::FileName { asc } => (SortField::Name, *asc),
        OrderBy::Property { key, asc } => (SortField::Property(key.clone()), *asc),
    };
    let order = if asc {
        SortOrder::Ascending
    } else {
        SortOrder::Descending
    };
    Some((field, order))
}

/// `query` with its order directive replaced by `field` / `order` — the query
/// string is the single source of truth for a query-backed list's sort.
/// `None` for a property sort with no key yet (not a usable order).
pub fn query_with_sort(query: &str, field: &SortField, order: SortOrder) -> Option<String> {
    if is_blank_property(field) {
        return None;
    }
    let order_field = match field {
        SortField::Name => OrderField::FileName,
        SortField::Title => OrderField::Title,
        SortField::Property(key) => OrderField::Property(key.clone()),
    };
    let asc = matches!(order, SortOrder::Ascending);
    Some(with_order_directive(query, order_field, asc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_of_query_reads_the_directive() {
        assert_eq!(
            order_of_query("widget -or:title"),
            (SortField::Title, SortOrder::Descending)
        );
        assert_eq!(
            order_of_query("#work or:prop:due"),
            (SortField::Property("due".into()), SortOrder::Ascending)
        );
        assert_eq!(
            order_of_query("widget"),
            (SortField::Name, SortOrder::Ascending)
        );
    }

    #[test]
    fn query_with_sort_round_trips_and_skips_empty_keys() {
        let q = query_with_sort("x", &SortField::Name, SortOrder::Descending).unwrap();
        assert_eq!(order_of_query(&q), (SortField::Name, SortOrder::Descending));
        assert_eq!(
            query_with_sort("x", &SortField::Property(" ".into()), SortOrder::Ascending),
            None
        );
    }

    #[test]
    fn property_sort_cache_folds_the_key_like_core() {
        use kimun_core::PropertySortValue::Number;
        let mut sort = PropertySort {
            key: Some("rank".into()),
            ..Default::default()
        };
        let values: PropertyValues = Arc::new(std::collections::HashMap::from([(
            kimun_core::nfs::VaultPath::note_path_from("/a"),
            Number(1.0),
        )]));
        assert!(sort.receive(" Rank ", values));
        assert!(sort.values_for("RANK").is_some());
        assert!(sort.values_for("rank").is_some());
        assert!(sort.values_for("other").is_none());
        assert!(sort.values_for(" ").is_none());
        assert!(!sort.receive("other", Arc::default()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fetch_reuses_the_cache_for_another_casing() {
        use kimun_core::PropertySortValue::Number;
        let vault = crate::test_support::temp_vault("prop-sort-casing").await;
        vault.validate_and_init().await.unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut sort = PropertySort::default();
        sort.fetch(&vault, "Rank", &tx);
        let values: PropertyValues = Arc::new(std::collections::HashMap::from([(
            kimun_core::nfs::VaultPath::note_path_from("/a"),
            Number(1.0),
        )]));
        assert!(sort.receive("rank", values));
        sort.fetch(&vault, "RANK", &tx);
        assert!(
            sort.values_for("rank").is_some(),
            "a refetch under another casing keeps the cached values"
        );
    }

    /// `ensure` fetches only for a new key: the same key (in any casing)
    /// reuses the cache or the fetch already in flight.
    #[tokio::test(flavor = "multi_thread")]
    async fn ensure_fetches_only_when_the_key_changes() {
        let vault = crate::test_support::temp_vault("prop-sort-ensure").await;
        vault.validate_and_init().await.unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut sort = PropertySort::default();
        sort.ensure(&vault, "rank", &tx);
        assert_eq!(sort.fetches, 1);
        sort.ensure(&vault, "Rank", &tx);
        assert_eq!(sort.fetches, 1, "same key while in flight: no refetch");
        assert!(sort.receive("rank", Arc::default()));
        sort.rx = None;
        sort.ensure(&vault, " RANK ", &tx);
        assert_eq!(sort.fetches, 1, "same key with cached values: no refetch");
        sort.ensure(&vault, "due", &tx);
        assert_eq!(sort.fetches, 2, "another key fetches");
        sort.fetch(&vault, "due", &tx);
        assert_eq!(sort.fetches, 3, "fetch always refetches (list reloads)");
    }

    /// A failed fetch still orders (every row missing) but is not a cache:
    /// applying the same key again retries.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_fetch_is_retried_on_the_same_key() {
        let vault = crate::test_support::temp_vault("prop-sort-retry").await;
        vault.validate_and_init().await.unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut sort = PropertySort::default();
        sort.ensure(&vault, "rank", &tx);
        // Stand in for the task's failed read.
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        sort.rx = Some(result_rx);
        result_tx
            .send(FetchResult {
                key: "rank".into(),
                values: Arc::default(),
                failed: true,
            })
            .unwrap();
        let (key, values) = sort.poll().expect("a result");
        assert!(sort.receive(&key, values));
        assert!(sort.values_for("rank").is_some(), "rows still order");
        sort.ensure(&vault, "rank", &tx);
        assert_eq!(sort.fetches, 2, "a failed fetch is retried");
    }

    /// Switching to Name / Title forgets the values: coming back fetches.
    #[tokio::test(flavor = "multi_thread")]
    async fn sync_clears_the_cache_for_a_non_property_sort() {
        let vault = crate::test_support::temp_vault("prop-sort-sync").await;
        vault.validate_and_init().await.unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut sort = PropertySort::default();
        let rank = SortField::Property("rank".into());
        sort.sync(&vault, &rank, &tx);
        assert!(sort.receive("rank", Arc::default()));
        sort.rx = None;
        sort.sync(&vault, &rank, &tx);
        assert_eq!(sort.fetches, 1, "same key: reused");
        sort.sync(&vault, &SortField::Name, &tx);
        assert!(sort.values_for("rank").is_none());
        sort.sync(&vault, &rank, &tx);
        assert_eq!(sort.fetches, 2, "back from Name: refetched");
    }

    #[test]
    fn a_fetch_error_is_flashed() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        report_fetch_error("rank", &"db gone", &tx);
        match rx.try_recv() {
            Ok(AppEvent::FlashMessage(msg)) => {
                assert_eq!(msg, "couldn't load property values: db gone")
            }
            other => panic!("expected a flash, got {other:?}"),
        }
    }
}
