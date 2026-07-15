use crate::document::FlattenedChunk;
use async_trait::async_trait;
use std::{collections::HashMap, fmt::Display};

pub mod embedder;

pub mod vecqdrant;
pub mod vecsqlite;

/// Information about an indexed note
#[derive(Debug, Clone)]
pub struct IndexedNote {
    pub path: String,
    pub content_hash: String,
    pub last_indexed: i64, // Unix timestamp
}

/// One collection's summary for the server admin UI: its name (the vault id)
/// and how many notes it has indexed.
#[derive(Debug, Clone)]
pub struct CollectionInfo {
    pub name: String,
    pub note_count: usize,
}

impl Display for IndexedNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "Path: {}, Hash: {}, Last Indexed: {}",
            self.path, self.content_hash, self.last_indexed
        )
    }
}

/// A chunk together with its embedding — the row a [`VectorStore`] stores.
/// Produced by the pipeline (which owns splitting and embedding); the store
/// persists text and vector together so they can never drift apart.
pub struct EmbeddedChunk {
    pub chunk: FlattenedChunk,
    pub vector: Vec<f32>,
}

/// Pure vector storage, scoped per **collection** — one collection per vault,
/// keyed by the vault's id (adr/0020). Adapters store, delete, and search rows;
/// they never embed, split, or rank — that is pipeline policy above this seam.
///
/// Shared contract (pinned by the conformance suite, run against every
/// adapter): collections are created lazily on first `store`; every read or
/// delete against a collection that does not exist yet returns empty / no-op,
/// never an error (reconciliation starts by reading hashes of a possibly
/// never-pushed vault); `query` returns similarity scores (higher = better),
/// best-first.
#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Appends rows. Replacing a note's chunks is the pipeline's job (it
    /// deletes the note's paths first), so a store never sees two generations
    /// of the same note.
    async fn store(&self, collection: &str, rows: &[EmbeddedChunk]) -> anyhow::Result<()>;

    /// Removes every chunk of the given note paths.
    async fn delete(&self, collection: &str, paths: &[String]) -> anyhow::Result<()>;

    /// The `limit` best-matching chunks for `vector`, best-first, scored as
    /// similarities (higher = better).
    async fn query(
        &self,
        collection: &str,
        vector: Vec<f32>,
        limit: usize,
    ) -> anyhow::Result<Vec<(f64, FlattenedChunk)>>;

    /// The `{note path → IndexedNote}` map for one collection — the authoritative
    /// server-side hash set the client reconciles against.
    async fn indexed_notes(&self, collection: &str)
    -> anyhow::Result<HashMap<String, IndexedNote>>;

    /// Every collection the store holds, with its indexed-note count. Powers the
    /// server admin UI's collections page. May be O(store) per collection on some
    /// backends — use [`collection_names`](Self::collection_names) when only the
    /// names are needed.
    async fn list_collections(&self) -> anyhow::Result<Vec<CollectionInfo>>;

    /// Just the collection names (vault ids), cheaply — no per-collection scan.
    /// For pickers/dropdowns that don't need counts.
    async fn collection_names(&self) -> anyhow::Result<Vec<String>>;

    /// The embedder fingerprint recorded with this store's data, if any —
    /// `None` on a store that has never had one written (adr/0025).
    async fn read_fingerprint(&self) -> anyhow::Result<Option<String>>;

    /// Records the embedder fingerprint. Overwrites any previous value.
    async fn write_fingerprint(&self, fingerprint: &str) -> anyhow::Result<()>;

    /// Drops every vault collection (all stored vectors). Startup uses this
    /// when the configured embedder's fingerprint no longer matches the stored
    /// one — the old vectors are unusable by definition, and the now-empty
    /// server makes every client's next reconciliation re-push everything
    /// (adr/0025). The fingerprint slot itself is metadata and survives.
    async fn drop_all_collections(&self) -> anyhow::Result<()>;
}

/// The conformance suite: the [`VectorStore`] contract as executable checks,
/// written against the trait so every adapter runs the same spec. SQLite runs
/// them in plain `cargo test`; Qdrant runs them `#[ignore]`d against a live
/// server (`QDRANT_URL`, default localhost). A new backend implements the trait
/// and calls these from its test module.
#[cfg(test)]
pub(crate) mod conformance {
    use super::*;

    pub(crate) const DIM: usize = 8;

    /// A deterministic non-zero vector per text, so cosine search is
    /// well-defined without any embedder.
    pub(crate) fn vector_for(text: &str) -> Vec<f32> {
        let mut v = vec![0.0f32; DIM];
        v[0] = 1.0;
        for (i, b) in text.bytes().enumerate() {
            v[1 + (i % (DIM - 1))] += b as f32;
        }
        v
    }

    pub(crate) fn row(path: &str, hash: &str, text: &str) -> EmbeddedChunk {
        EmbeddedChunk {
            chunk: FlattenedChunk {
                doc_path: path.to_string(),
                doc_hash: hash.to_string(),
                title: "T".to_string(),
                text: text.to_string(),
                date: None,
            },
            vector: vector_for(text),
        }
    }

    pub(crate) async fn store_then_query_finds_the_chunk(store: &dyn VectorStore, c: &str) {
        store
            .store(
                c,
                &[
                    row("a.md", "h1", "the quick brown fox"),
                    row("b.md", "h2", "lazy dog sleeps"),
                ],
            )
            .await
            .unwrap();
        let results = store
            .query(c, vector_for("the quick brown fox"), 10)
            .await
            .unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].1.doc_path, "a.md", "best match first");
        assert!(
            results.windows(2).all(|w| w[0].0 >= w[1].0),
            "scores must be best-first"
        );
    }

    pub(crate) async fn query_respects_limit(store: &dyn VectorStore, c: &str) {
        let rows: Vec<EmbeddedChunk> = (0..5)
            .map(|i| row(&format!("n{i}.md"), "h", &format!("text {i}")))
            .collect();
        store.store(c, &rows).await.unwrap();
        let results = store.query(c, vector_for("text"), 3).await.unwrap();
        assert_eq!(results.len(), 3);
    }

    pub(crate) async fn missing_collection_is_empty_not_error(store: &dyn VectorStore, c: &str) {
        assert!(store.indexed_notes(c).await.unwrap().is_empty());
        assert!(
            store
                .query(c, vector_for("q"), 10)
                .await
                .unwrap()
                .is_empty()
        );
        // Deleting from a missing collection is a no-op, not an error.
        store.delete(c, &["x.md".to_string()]).await.unwrap();
    }

    pub(crate) async fn delete_removes_every_chunk_of_the_note(store: &dyn VectorStore, c: &str) {
        store
            .store(
                c,
                &[
                    row("a.md", "h1", "alpha section one"),
                    row("a.md", "h1", "alpha section two"),
                    row("b.md", "h2", "beta"),
                ],
            )
            .await
            .unwrap();
        store.delete(c, &["a.md".to_string()]).await.unwrap();

        let notes = store.indexed_notes(c).await.unwrap();
        assert_eq!(notes.len(), 1);
        assert!(notes.contains_key("b.md"));
        let results = store
            .query(c, vector_for("alpha section one"), 10)
            .await
            .unwrap();
        assert!(
            results.iter().all(|(_, chunk)| chunk.doc_path != "a.md"),
            "no chunk of a deleted note may remain searchable"
        );
    }

    pub(crate) async fn indexed_notes_reports_one_hash_per_path(store: &dyn VectorStore, c: &str) {
        store
            .store(
                c,
                &[
                    row("a.md", "h1", "alpha part one"),
                    row("a.md", "h1", "alpha part two"),
                    row("b.md", "h2", "beta"),
                ],
            )
            .await
            .unwrap();
        let notes = store.indexed_notes(c).await.unwrap();
        assert_eq!(notes.len(), 2, "notes, not chunks");
        assert_eq!(notes.get("a.md").unwrap().content_hash, "h1");
        assert_eq!(notes.get("b.md").unwrap().content_hash, "h2");
    }

    pub(crate) async fn collections_list_each_vault(store: &dyn VectorStore, c1: &str, c2: &str) {
        store.store(c1, &[row("a.md", "h", "x")]).await.unwrap();
        store.store(c2, &[row("b.md", "h", "y")]).await.unwrap();

        let names = store.collection_names().await.unwrap();
        assert!(names.contains(&c1.to_string()));
        assert!(names.contains(&c2.to_string()));

        let infos = store.list_collections().await.unwrap();
        let get = |n: &str| infos.iter().find(|i| i.name == n).map(|i| i.note_count);
        assert_eq!(get(c1), Some(1));
        assert_eq!(get(c2), Some(1));
    }

    pub(crate) async fn fingerprint_round_trips_and_starts_absent(store: &dyn VectorStore) {
        assert_eq!(
            store.read_fingerprint().await.unwrap(),
            None,
            "fresh store has none"
        );
        store
            .write_fingerprint("fastembed:default:1024")
            .await
            .unwrap();
        assert_eq!(
            store.read_fingerprint().await.unwrap().as_deref(),
            Some("fastembed:default:1024")
        );
        // Overwrite wins.
        store.write_fingerprint("ollama:nomic:768").await.unwrap();
        assert_eq!(
            store.read_fingerprint().await.unwrap().as_deref(),
            Some("ollama:nomic:768")
        );
    }

    pub(crate) async fn drop_all_removes_every_collection_but_not_the_fingerprint_slot(
        store: &dyn VectorStore,
        c1: &str,
        c2: &str,
    ) {
        store.write_fingerprint("fp1").await.unwrap();
        store.store(c1, &[row("a.md", "h", "x")]).await.unwrap();
        store.store(c2, &[row("b.md", "h", "y")]).await.unwrap();

        store.drop_all_collections().await.unwrap();

        assert!(
            store.collection_names().await.unwrap().is_empty(),
            "no vault collections left"
        );
        assert!(store.indexed_notes(c1).await.unwrap().is_empty());
        // The fingerprint slot is metadata, not a collection: still writable/readable.
        store.write_fingerprint("fp2").await.unwrap();
        assert_eq!(
            store.read_fingerprint().await.unwrap().as_deref(),
            Some("fp2")
        );
    }

    pub(crate) async fn fingerprint_slot_never_appears_as_a_collection(store: &dyn VectorStore) {
        store.write_fingerprint("fp").await.unwrap();
        let names = store.collection_names().await.unwrap();
        assert!(
            names.is_empty(),
            "metadata must not leak into collections: {names:?}"
        );
        assert!(store.list_collections().await.unwrap().is_empty());
    }

    pub(crate) async fn stored_chunk_round_trips_its_fields(store: &dyn VectorStore, c: &str) {
        let mut r = row("2025-01-15.md", "h9", "journal body");
        r.chunk.title = "Morning".to_string();
        r.chunk.date = chrono::NaiveDate::from_ymd_opt(2025, 1, 15);
        store.store(c, &[r]).await.unwrap();

        let results = store.query(c, vector_for("journal body"), 1).await.unwrap();
        let (_, chunk) = &results[0];
        assert_eq!(chunk.doc_path, "2025-01-15.md");
        assert_eq!(chunk.doc_hash, "h9");
        assert_eq!(chunk.title, "Morning");
        assert_eq!(chunk.text, "journal body");
        assert_eq!(chunk.get_date_string().as_deref(), Some("2025-01-15"));
    }
}
