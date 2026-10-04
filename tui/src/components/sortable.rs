//! One contract for every list the sort dialog can sort — the sidebar, the
//! Query panel, the Ctrl+K search browser and the Ctrl+O file finder. The dialog is built from a
//! list's [`SortState`] and [`SortableList::allows_property`], and every
//! selection it emits is applied through [`SortableList::apply_sort`], so the
//! three targets cannot drift apart in how a sort lands.

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
}

/// The property values a listing (sidebar, file finder) orders by: one async
/// fetch per key, cached until the key changes. Results arrive on a channel
/// the owner polls each frame; a result for a key the list no longer sorts
/// by is dropped.
#[derive(Default)]
pub struct PropertySort {
    /// The key the cached / pending values are for.
    key: Option<String>,
    values: Option<PropertyValues>,
    rx: Option<Receiver<(String, PropertyValues)>>,
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
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        self.rx = Some(result_rx);
        let vault = Arc::clone(vault);
        let tx = tx.clone();
        tokio::spawn(async move {
            let values = match vault.property_sort_values(&key).await {
                Ok(values) => values,
                Err(e) => {
                    report_fetch_error(&key, &e, &tx);
                    Default::default()
                }
            };
            result_tx.send((key, Arc::new(values))).ok();
            tx.send(AppEvent::Redraw).ok();
        });
    }

    /// Take a fetched result if one has landed, for the owner to hand to
    /// [`Self::receive`] (through its own re-sorting entry point).
    pub fn poll(&mut self) -> Option<(String, PropertyValues)> {
        let rx = self.rx.as_ref()?;
        match rx.try_recv() {
            Ok(result) => {
                self.rx = None;
                Some(result)
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
    let order_field = match field {
        SortField::Name => OrderField::FileName,
        SortField::Title => OrderField::Title,
        SortField::Property(key) if key.trim().is_empty() => return None,
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
