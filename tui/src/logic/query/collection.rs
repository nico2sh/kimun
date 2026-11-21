//! The loader behind a facet collection: every note a query matches, bounded
//! and read in full, ready for its facets to be cut into pieces. Unlike
//! [`load_query_page`](super::load_query_page) it keeps whole texts and cuts
//! no context paragraph.

use futures::StreamExt as _;
use kimun_core::NoteVault;
use kimun_core::error::VaultError;
use kimun_core::nfs::VaultPath;

use super::find::order_hits;
use super::vars::{QueryContext, resolve_for_search};

/// The most notes a collection loads; the rest are counted, not read.
pub const COLLECTION_NOTE_CAP: usize = 500;

/// How many notes are read at once.
const READS_AT_ONCE: usize = 8;

/// One note of a collection, with its whole text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedNote {
    pub path: VaultPath,
    pub title: String,
    pub text: String,
}

/// What a collection query loaded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CollectionLoad {
    /// The notes read, in query order, at most [`COLLECTION_NOTE_CAP`].
    pub notes: Vec<LoadedNote>,
    /// How many notes the query matched, before the cap.
    pub total: usize,
    /// Notes within the cap that could not be read (skipped from `notes`).
    pub unreadable: usize,
}

/// Resolve `template` against `ctx`; `None` when it needs a note and none is open.
pub fn resolve_collection_query(template: &str, ctx: &QueryContext) -> Option<String> {
    resolve_for_search(template, ctx)
}

/// Search `resolved`, order (the `or:` directive, else lowercase file name),
/// keep at most [`COLLECTION_NOTE_CAP`], read the texts 8 at a time. `total`
/// is the hits before the cap.
pub async fn load_collection(
    vault: &NoteVault,
    resolved: &str,
) -> Result<CollectionLoad, VaultError> {
    let mut hits = vault.search_notes(resolved).await?;
    order_hits(resolved, &mut hits);
    let total = hits.len();
    hits.truncate(COLLECTION_NOTE_CAP);
    let read: Vec<Option<LoadedNote>> = futures::stream::iter(hits)
        .map(|(entry, content)| async move {
            let text = vault.get_note_text(&entry.path).await.ok()?;
            Some(LoadedNote {
                path: entry.path,
                title: content.title,
                text,
            })
        })
        .buffered(READS_AT_ONCE)
        .collect()
        .await;
    let unreadable = read.iter().filter(|n| n.is_none()).count();
    Ok(CollectionLoad {
        notes: read.into_iter().flatten().collect(),
        total,
        unreadable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str) -> VaultPath {
        VaultPath::note_path_from(name)
    }

    async fn vault_with(prefix: &str, notes: &[(&str, &str)]) -> std::sync::Arc<NoteVault> {
        let vault = crate::test_support::temp_vault(prefix).await;
        vault.validate_and_init().await.unwrap();
        for (name, body) in notes {
            vault.create_note(&p(name), *body).await.unwrap();
        }
        vault
    }

    fn names(load: &CollectionLoad) -> Vec<String> {
        load.notes
            .iter()
            .map(|n| n.path.get_parent_path().1.to_lowercase())
            .collect()
    }

    #[test]
    fn resolve_returns_none_without_a_note_for_a_note_query() {
        let none = QueryContext::default();
        assert_eq!(resolve_collection_query("<{note}", &none), None);
        assert_eq!(resolve_collection_query("<", &none), None);
        assert_eq!(
            resolve_collection_query("#todo", &none).as_deref(),
            Some("#todo")
        );
    }

    #[test]
    fn resolve_replaces_note_with_the_quoted_clean_name() {
        let ctx = QueryContext::with_note(Some(p("/my plan")));
        assert_eq!(
            resolve_collection_query("<{note}", &ctx).as_deref(),
            Some("<\"my plan\"")
        );
        let ctx = QueryContext::with_note(Some(p("plan")));
        assert_eq!(
            resolve_collection_query("<", &ctx).as_deref(),
            Some("<plan")
        );
    }

    #[tokio::test]
    async fn load_orders_by_the_query_directive() {
        let vault = vault_with(
            "col-order",
            &[("alpha", "#s a"), ("bravo", "#s b"), ("echo", "#s e")],
        )
        .await;
        let load = load_collection(&vault, "#s -or:title").await.unwrap();
        assert_eq!(load.total, 3);
        assert!(names(&load)[0].starts_with("echo"));
        let load = load_collection(&vault, "#s or:title").await.unwrap();
        assert!(names(&load)[0].starts_with("alpha"));
    }

    #[tokio::test]
    async fn load_falls_back_to_lowercase_file_name() {
        let vault = vault_with(
            "col-name",
            &[
                ("delta", "#s"),
                ("Alpha", "#s"),
                ("charlie", "#s"),
                ("bravo", "#s"),
            ],
        )
        .await;
        let load = load_collection(&vault, "#s").await.unwrap();
        let got = names(&load);
        let mut sorted = got.clone();
        sorted.sort();
        assert_eq!(got, sorted);
        assert!(got[0].starts_with("alpha") && got[3].starts_with("delta"));
        assert!(load.notes.iter().all(|n| n.text.contains("#s")));
        assert_eq!(load.unreadable, 0);
    }

    #[tokio::test]
    async fn load_caps_at_the_note_cap_and_reports_the_total() {
        let vault = crate::test_support::temp_vault("col-cap").await;
        vault.validate_and_init().await.unwrap();
        let over = COLLECTION_NOTE_CAP + 3;
        for i in 0..over {
            vault
                .create_note(&p(&format!("n{i:04}")), "#s x")
                .await
                .unwrap();
        }
        let load = load_collection(&vault, "#s").await.unwrap();
        assert_eq!(load.total, over);
        assert_eq!(load.notes.len(), COLLECTION_NOTE_CAP);
        assert_eq!(load.unreadable, 0);
        assert!(names(&load)[0].starts_with("n0000"));
    }

    #[tokio::test]
    async fn load_counts_an_unreadable_note_and_skips_it() {
        let vault = vault_with("col-gone", &[("a", "#s"), ("b", "#s"), ("c", "#s")]).await;
        // Removed behind the index's back: indexed, but unreadable.
        std::fs::remove_file(vault.path_to_pathbuf(&p("b"))).unwrap();
        let load = load_collection(&vault, "#s").await.unwrap();
        assert_eq!(load.total, 3);
        assert_eq!(load.unreadable, 1);
        assert_eq!(names(&load).len(), 2);
        assert!(names(&load).iter().all(|n| !n.starts_with('b')));
    }

    /// Whether at most eight reads run at once is not observable through
    /// `NoteVault` (it exposes no read hook), so this pins the constant; the
    /// bound itself is `buffered(READS_AT_ONCE)` in `load_collection`.
    #[test]
    fn load_reads_at_most_eight_at_once() {
        assert_eq!(READS_AT_ONCE, 8);
    }

    #[tokio::test]
    async fn a_query_with_no_hits_loads_nothing() {
        let vault = vault_with("col-none", &[("a", "alpha")]).await;
        let load = load_collection(&vault, "#nothing").await.unwrap();
        assert_eq!(load, CollectionLoad::default());
    }
}
