//! Sort field / order, the state the sort dialog edits, and the property
//! values a listing orders by ([`PropertySort`]).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use kimun_core::nfs::VaultPath;
use kimun_core::note::property_search_key;
use kimun_core::{NoteVault, PropertySortValue};

use crate::logic::message::{AppMessage, AppTx};
use crate::settings::{SortFieldSetting, SortOrderSetting};

/// Each note's value for a property sort, as fetched from the index by
/// `NoteVault::property_sort_values`. Notes without the key are absent.
pub type PropertyValues = Arc<HashMap<VaultPath, PropertySortValue>>;

#[derive(Clone, PartialEq, Debug)]
pub enum SortField {
    Name,
    Title,
    /// Sort by this property key (search form). Query-backed lists put it
    /// in the query; listings order by values fetched from the index (see
    /// `entry_order`).
    Property(String),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SortOrder {
    Ascending,
    Descending,
}

impl From<SortFieldSetting> for SortField {
    fn from(s: SortFieldSetting) -> Self {
        match s {
            SortFieldSetting::Name => Self::Name,
            SortFieldSetting::Title => Self::Title,
        }
    }
}

impl From<SortOrderSetting> for SortOrder {
    fn from(s: SortOrderSetting) -> Self {
        match s {
            SortOrderSetting::Ascending => Self::Ascending,
            SortOrderSetting::Descending => Self::Descending,
        }
    }
}

impl From<SortField> for SortFieldSetting {
    fn from(s: SortField) -> Self {
        match s {
            SortField::Name | SortField::Property(_) => Self::Name,
            SortField::Title => Self::Title,
        }
    }
}

impl From<SortOrder> for SortOrderSetting {
    fn from(s: SortOrder) -> Self {
        match s {
            SortOrder::Ascending => Self::Ascending,
            SortOrder::Descending => Self::Descending,
        }
    }
}

impl SortField {
    pub fn label(&self) -> String {
        match self {
            Self::Name => "N".to_string(),
            Self::Title => "T".to_string(),
            Self::Property(key) => key.clone(),
        }
    }

    /// Next field in the dialog's cycle. `allow_property` comes from the
    /// list's `SortableList::allows_property`.
    pub fn cycle(&self, allow_property: bool) -> Self {
        match self {
            Self::Name => Self::Title,
            Self::Title if allow_property => Self::Property(String::new()),
            Self::Title | Self::Property(_) => Self::Name,
        }
    }
}

impl SortOrder {
    pub fn label(self) -> char {
        match self {
            Self::Ascending => '↑',
            Self::Descending => '↓',
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

/// What the sort dialog shows and changes for one list.
#[derive(Debug, Clone, PartialEq)]
pub struct SortState {
    pub field: SortField,
    pub order: SortOrder,
    /// `Some` only for lists that can group directories (the sidebar).
    pub group_dirs: Option<bool>,
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
            tx.send(AppMessage::Redraw).ok();
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
    tx.send(AppMessage::FlashMessage(format!(
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

#[cfg(test)]
mod tests {
    use super::*;

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
            Ok(AppMessage::FlashMessage(msg)) => {
                assert_eq!(msg, "couldn't load property values: db gone")
            }
            other => panic!("expected a flash, got {other:?}"),
        }
    }
}
