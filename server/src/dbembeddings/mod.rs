use crate::document::FlattenedChunk;
use async_trait::async_trait;
use chrono::NaiveDate;
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

/// An inclusive range of note dates, optionally restricted to one date source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateFilter {
    /// Matched ignoring case; `None` matches a date of any source.
    pub source: Option<String>,
    pub from: NaiveDate,
    pub to: NaiveDate,
}

/// Restricts a [`VectorStore::query`] to chunks whose note matches every set
/// field. The default filter is empty and matches everything.
#[derive(Clone, Debug, Default)]
pub struct QueryFilter {
    pub dates: Option<DateFilter>,
    /// Vault folder; a note matches when its path, without a leading `/`,
    /// starts with `folder` plus `/`, ignoring case.
    pub folder: Option<String>,
    /// Every listed tag must be on the note (tags are stored lowercase).
    pub tags: Vec<String>,
}

impl QueryFilter {
    /// True when no field is set, so the filter matches every chunk.
    pub fn is_empty(&self) -> bool {
        self.dates.is_none() && self.folder.is_none() && self.tags.is_empty()
    }

    pub fn matches(&self, chunk: &FlattenedChunk) -> bool {
        if let Some(range) = &self.dates {
            let hit = chunk.meta.dates.iter().any(|d| {
                range
                    .source
                    .as_deref()
                    .is_none_or(|s| d.source.eq_ignore_ascii_case(s))
                    && d.day >= range.from
                    && d.day <= range.to
            });
            if !hit {
                return false;
            }
        }
        if let Some(folder) = &self.folder {
            // Pushed paths are canonical and absolute (`/work/a.md`); the
            // leading `/` is not part of the folder.
            let prefix = format!("{}/", folder.trim_matches('/').to_lowercase());
            let path = chunk.doc_path.to_lowercase();
            if !path.trim_start_matches('/').starts_with(&prefix) {
                return false;
            }
        }
        self.tags.iter().all(|wanted| {
            chunk
                .meta
                .tags
                .iter()
                .any(|t| t.eq_ignore_ascii_case(wanted))
        })
    }
}

/// A chunk together with its embedding — the row a [`VectorStore`] stores.
/// Produced by the pipeline (which owns splitting and embedding); the store
/// persists text and vector together so they can never drift apart.
#[derive(Clone)]
pub struct EmbeddedChunk {
    pub chunk: FlattenedChunk,
    pub vector: Vec<f32>,
}

/// Pure vector storage, scoped per **collection** — one collection per vault,
/// keyed by the vault's id. Adapters store, delete, and search rows;
/// they never embed, split, or rank — that is pipeline policy above this seam.
///
/// Shared contract (pinned by the conformance suite, run against every
/// adapter): collections are created lazily on first `store`; every read or
/// delete against a collection that does not exist yet returns empty / no-op,
/// never an error (reconciliation starts by reading hashes of a possibly
/// never-pushed vault); `query` returns similarity scores (higher = better),
/// best-first.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Appends rows. Replacing a note's chunks is the pipeline's job (it
    /// deletes the note's paths first), so a store never sees two generations
    /// of the same note.
    async fn store(&self, collection: &str, rows: &[EmbeddedChunk]) -> anyhow::Result<()>;

    /// Removes every chunk of the given note paths.
    async fn delete(&self, collection: &str, paths: &[String]) -> anyhow::Result<()>;

    /// Every stored chunk of the given note paths, with its vector. Lets the
    /// pipeline reuse embeddings for sub-chunks whose text is unchanged when a
    /// note is re-indexed (a one-line edit re-splits the whole note, but most
    /// sub-chunks come out identical). Vectors may be returned normalized;
    /// missing collection → empty, never an error.
    async fn chunks_with_vectors(
        &self,
        collection: &str,
        paths: &[String],
    ) -> anyhow::Result<Vec<EmbeddedChunk>>;

    /// The `limit` best-matching chunks for `vector`, best-first, scored as
    /// similarities (higher = better). Only chunks matching `filter` compete
    /// for the `limit` slots: the filter applies before the cut, not after.
    async fn query(
        &self,
        collection: &str,
        vector: Vec<f32>,
        limit: usize,
        filter: &QueryFilter,
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
    /// `None` on a store that has never had one written.
    async fn read_fingerprint(&self) -> anyhow::Result<Option<String>>;

    /// Records the embedder fingerprint. Overwrites any previous value.
    async fn write_fingerprint(&self, fingerprint: &str) -> anyhow::Result<()>;

    /// Drops every vault collection (all stored vectors). Startup uses this
    /// when the configured embedder's fingerprint no longer matches the stored
    /// one — the old vectors are unusable by definition, and the now-empty
    /// server makes every client's next reconciliation re-push everything.
    /// The fingerprint slot itself is metadata and survives.
    async fn drop_all_collections(&self) -> anyhow::Result<()>;

    /// `{note path → link targets as written}`, one entry per note (not per
    /// chunk), links in stored order without exact duplicates. Missing
    /// collection → empty.
    async fn note_links(&self, collection: &str) -> anyhow::Result<HashMap<String, Vec<String>>>;

    /// The distinct date sources present in the collection (`journal` included
    /// when any note has a journal date), sorted. Missing collection → empty.
    async fn date_sources(&self, collection: &str) -> anyhow::Result<Vec<String>>;

    /// Records the **vault label** for a collection. Overwrites any previous
    /// label. The label is display only, and a vault with no chunks yet can
    /// have one. It is metadata, so `drop_all_collections` keeps it.
    async fn set_vault_label(&self, collection: &str, label: &str) -> anyhow::Result<()>;

    /// `{vault id → vault label}` for every stored label.
    async fn vault_labels(&self) -> anyhow::Result<HashMap<String, String>>;
}

/// Folds `(path, meta)` pairs (one per chunk) into the per-note link map
/// behind [`VectorStore::note_links`]. Chunks of a note share its metadata, so
/// the first chunk seen for a path wins.
pub(crate) fn aggregate_links(
    metas: impl IntoIterator<Item = (String, crate::document::NoteMeta)>,
) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for (path, meta) in metas {
        out.entry(path).or_insert_with(|| {
            let mut seen = std::collections::HashSet::new();
            meta.links
                .into_iter()
                .filter(|l| seen.insert(l.clone()))
                .collect()
        });
    }
    out
}

/// Distinct, sorted date sources across `metas`, behind
/// [`VectorStore::date_sources`].
pub(crate) fn aggregate_date_sources(
    metas: impl IntoIterator<Item = crate::document::NoteMeta>,
) -> Vec<String> {
    metas
        .into_iter()
        .flat_map(|m| m.dates.into_iter().map(|d| d.source))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
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
                meta: Default::default(),
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
            .query(
                c,
                vector_for("the quick brown fox"),
                10,
                &QueryFilter::default(),
            )
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
        let results = store
            .query(c, vector_for("text"), 3, &QueryFilter::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 3);
    }

    pub(crate) async fn missing_collection_is_empty_not_error(store: &dyn VectorStore, c: &str) {
        assert!(store.indexed_notes(c).await.unwrap().is_empty());
        assert!(
            store
                .query(c, vector_for("q"), 10, &QueryFilter::default())
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
            .query(
                c,
                vector_for("alpha section one"),
                10,
                &QueryFilter::default(),
            )
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

    pub(crate) async fn chunks_with_vectors_returns_the_notes_rows(
        store: &dyn VectorStore,
        c: &str,
    ) {
        // Missing collection → empty, never an error.
        assert!(
            store
                .chunks_with_vectors(c, &["a.md".to_string()])
                .await
                .unwrap()
                .is_empty()
        );

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

        let rows = store
            .chunks_with_vectors(c, &["a.md".to_string()])
            .await
            .unwrap();
        assert_eq!(rows.len(), 2, "only the requested note's chunks");
        assert!(rows.iter().all(|r| r.chunk.doc_path == "a.md"));
        for r in &rows {
            assert_eq!(r.vector.len(), DIM);
            // The vector must still point the same way as the original (stores
            // may normalize at rest): cosine of stored vs. original ≈ 1.
            let original = vector_for(&r.chunk.text);
            let dot: f32 = r.vector.iter().zip(&original).map(|(a, b)| a * b).sum();
            let n1: f32 = r.vector.iter().map(|x| x * x).sum::<f32>().sqrt();
            let n2: f32 = original.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!(
                (dot / (n1 * n2) - 1.0).abs() < 1e-5,
                "vector for {:?} does not match what was stored",
                r.chunk.text
            );
        }
    }

    /// Each note's chunks come back in the order they were stored. Only the
    /// SQLite store keeps this promise; on Qdrant the order is not
    /// guaranteed.
    pub(crate) async fn chunks_with_vectors_keeps_document_order(store: &dyn VectorStore, c: &str) {
        let a: Vec<String> = ["e intro", "c body", "a middle", "d detail", "b end"]
            .map(String::from)
            .to_vec();
        let b: Vec<String> = ["z one", "y two", "x three"].map(String::from).to_vec();
        // The notes interleave: b's rows sit between a's in storage order.
        let mut rows = vec![row("b.md", "h2", &b[0])];
        rows.extend(a[..3].iter().map(|t| row("a.md", "h1", t)));
        rows.extend(b[1..].iter().map(|t| row("b.md", "h2", t)));
        store.store(c, &rows).await.unwrap();
        let rest: Vec<EmbeddedChunk> = a[3..].iter().map(|t| row("a.md", "h1", t)).collect();
        store.store(c, &rest).await.unwrap();

        let got = store
            .chunks_with_vectors(c, &["b.md".to_string(), "a.md".to_string()])
            .await
            .unwrap();
        let texts = |path: &str| -> Vec<String> {
            got.iter()
                .filter(|r| r.chunk.doc_path == path)
                .map(|r| r.chunk.text.clone())
                .collect()
        };
        assert_eq!(texts("a.md"), a);
        assert_eq!(texts("b.md"), b);
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
        r.chunk.meta.dates.push(crate::document::NoteDate {
            source: crate::document::JOURNAL_SOURCE.to_string(),
            day: chrono::NaiveDate::from_ymd_opt(2025, 1, 15).unwrap(),
        });
        store.store(c, &[r]).await.unwrap();

        let results = store
            .query(c, vector_for("journal body"), 1, &QueryFilter::default())
            .await
            .unwrap();
        let (_, chunk) = &results[0];
        assert_eq!(chunk.doc_path, "2025-01-15.md");
        assert_eq!(chunk.doc_hash, "h9");
        assert_eq!(chunk.title, "Morning");
        assert_eq!(chunk.text, "journal body");
        assert_eq!(chunk.get_date_string().as_deref(), Some("2025-01-15"));
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn nd(source: &str, day: NaiveDate) -> crate::document::NoteDate {
        crate::document::NoteDate {
            source: source.to_string(),
            day,
        }
    }

    fn row_with(
        path: &str,
        text: &str,
        dates: Vec<crate::document::NoteDate>,
        tags: &[&str],
    ) -> EmbeddedChunk {
        let mut r = row(path, "h", text);
        r.chunk.meta.dates = dates;
        r.chunk.meta.tags = tags.iter().map(|t| t.to_string()).collect();
        r
    }

    fn paths(results: &[(f64, FlattenedChunk)]) -> Vec<String> {
        let mut p: Vec<String> = results.iter().map(|(_, c)| c.doc_path.clone()).collect();
        p.sort();
        p
    }

    pub(crate) async fn query_filters_by_date_range_and_source(store: &dyn VectorStore, c: &str) {
        store
            .store(
                c,
                &[
                    row_with("a.md", "same", vec![nd("journal", day(2026, 11, 18))], &[]),
                    row_with("b.md", "same", vec![nd("journal", day(2026, 11, 25))], &[]),
                    row_with("c.md", "same", vec![nd("due", day(2026, 11, 19))], &[]),
                ],
            )
            .await
            .unwrap();
        let mut filter = QueryFilter {
            dates: Some(DateFilter {
                source: Some("Journal".to_string()),
                from: day(2026, 11, 17),
                to: day(2026, 11, 23),
            }),
            ..Default::default()
        };
        let r = store
            .query(c, vector_for("same"), 10, &filter)
            .await
            .unwrap();
        assert_eq!(vec!["a.md".to_string()], paths(&r));

        filter.dates.as_mut().unwrap().source = None;
        let r = store
            .query(c, vector_for("same"), 10, &filter)
            .await
            .unwrap();
        assert_eq!(vec!["a.md".to_string(), "c.md".to_string()], paths(&r));
    }

    pub(crate) async fn filter_applies_before_the_limit(store: &dyn VectorStore, c: &str) {
        let mut rows: Vec<EmbeddedChunk> = (0..5)
            .map(|i| row_with(&format!("n{i}.md"), "same", vec![], &[]))
            .collect();
        // The match is the worst-scoring row, so a cut-then-filter drops it.
        let mut hit = row_with("hit.md", "same", vec![], &["wanted"]);
        hit.vector = vector_for("quite different text entirely");
        rows.push(hit);
        store.store(c, &rows).await.unwrap();
        let filter = QueryFilter {
            tags: vec!["wanted".to_string()],
            ..Default::default()
        };
        let r = store
            .query(c, vector_for("same"), 2, &filter)
            .await
            .unwrap();
        assert_eq!(vec!["hit.md".to_string()], paths(&r));
    }

    pub(crate) async fn query_filters_by_folder_and_tags(store: &dyn VectorStore, c: &str) {
        store
            .store(
                c,
                &[
                    row_with("work/a.md", "same", vec![], &["x", "y"]),
                    row_with("work/sub/b.md", "same", vec![], &["x"]),
                    row_with("workshop/c.md", "same", vec![], &["x", "y"]),
                    row_with("home/d.md", "same", vec![], &["x", "y"]),
                ],
            )
            .await
            .unwrap();
        let q = |folder: Option<&str>, tags: &[&str]| QueryFilter {
            folder: folder.map(str::to_string),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            ..Default::default()
        };
        let r = store
            .query(c, vector_for("same"), 10, &q(Some("Work"), &[]))
            .await
            .unwrap();
        assert_eq!(
            vec!["work/a.md".to_string(), "work/sub/b.md".to_string()],
            paths(&r)
        );
        let r = store
            .query(c, vector_for("same"), 10, &q(Some("work"), &["x", "y"]))
            .await
            .unwrap();
        assert_eq!(vec!["work/a.md".to_string()], paths(&r));
        // Real clients push canonical absolute paths (`/work/e.md`).
        store
            .store(
                c,
                &[
                    row_with("/work/e.md", "same", vec![], &["z"]),
                    row_with("/other/work/f.md", "same", vec![], &["z"]),
                ],
            )
            .await
            .unwrap();
        let r = store
            .query(c, vector_for("same"), 10, &q(Some("work"), &["z"]))
            .await
            .unwrap();
        assert_eq!(vec!["/work/e.md".to_string()], paths(&r));
        let r = store
            .query(c, vector_for("same"), 10, &q(None, &["y"]))
            .await
            .unwrap();
        assert_eq!(3, r.len());
    }

    pub(crate) async fn store_round_trips_metadata(store: &dyn VectorStore, c: &str) {
        let mut r = row_with(
            "a.md",
            "body",
            vec![
                nd("journal", day(2026, 11, 18)),
                nd("due", day(2026, 12, 1)),
            ],
            &["work"],
        );
        r.chunk.meta.links = vec!["Project X".to_string()];
        let expected = r.chunk.meta.clone();
        store.store(c, &[r]).await.unwrap();
        let rows = store
            .chunks_with_vectors(c, &["a.md".to_string()])
            .await
            .unwrap();
        assert_eq!(expected, rows[0].chunk.meta);
        let q = store
            .query(c, vector_for("body"), 1, &QueryFilter::default())
            .await
            .unwrap();
        assert_eq!(expected, q[0].1.meta);
    }

    pub(crate) async fn note_links_returns_each_note_once(store: &dyn VectorStore, c: &str) {
        let mut a1 = row_with("a.md", "one", vec![], &[]);
        a1.chunk.meta.links = vec!["Zed".into(), "Alpha".into(), "Zed".into()];
        let mut a2 = row_with("a.md", "two", vec![], &[]);
        a2.chunk.meta.links = a1.chunk.meta.links.clone();
        let mut b = row_with("b.md", "three", vec![], &[]);
        b.chunk.meta.links = vec!["a".into()];
        let none = row_with("c.md", "four", vec![], &[]);
        store.store(c, &[a1, a2, b, none]).await.unwrap();

        let links = store.note_links(c).await.unwrap();
        assert_eq!(3, links.len(), "one entry per note: {links:?}");
        assert_eq!(vec!["Zed".to_string(), "Alpha".to_string()], links["a.md"]);
        assert_eq!(vec!["a".to_string()], links["b.md"]);
        assert!(links["c.md"].is_empty());
    }

    pub(crate) async fn date_sources_lists_distinct_sorted_sources_including_journal(
        store: &dyn VectorStore,
        c: &str,
    ) {
        store
            .store(
                c,
                &[
                    row_with("a.md", "x", vec![nd("journal", day(2026, 11, 18))], &[]),
                    row_with("b.md", "y", vec![nd("journal", day(2026, 11, 19))], &[]),
                    row_with(
                        "c.md",
                        "z",
                        vec![nd("due", day(2026, 11, 20)), nd("created", day(2026, 1, 1))],
                        &[],
                    ),
                    row_with("d.md", "w", vec![], &[]),
                ],
            )
            .await
            .unwrap();
        assert_eq!(
            vec!["created".to_string(), "due".into(), "journal".into()],
            store.date_sources(c).await.unwrap()
        );
    }

    pub(crate) async fn missing_collection_returns_empty_for_both_reads(
        store: &dyn VectorStore,
        c: &str,
    ) {
        assert!(store.note_links(c).await.unwrap().is_empty());
        assert!(store.date_sources(c).await.unwrap().is_empty());
    }

    #[test]
    fn matches_applies_each_rule() {
        let mut chunk = row_with(
            "Work/Sub/a.md",
            "t",
            vec![nd("Due", day(2026, 11, 20))],
            &["x", "y"],
        )
        .chunk;
        assert!(QueryFilter::default().matches(&chunk));
        assert!(QueryFilter::default().is_empty());

        let dates = |source: Option<&str>, from, to| QueryFilter {
            dates: Some(DateFilter {
                source: source.map(str::to_string),
                from,
                to,
            }),
            ..Default::default()
        };
        // Inclusive bounds, case-insensitive source, any source on None.
        assert!(dates(Some("due"), day(2026, 11, 20), day(2026, 11, 20)).matches(&chunk));
        assert!(!dates(Some("due"), day(2026, 11, 21), day(2026, 11, 30)).matches(&chunk));
        assert!(!dates(Some("journal"), day(2026, 11, 1), day(2026, 11, 30)).matches(&chunk));
        assert!(dates(None, day(2026, 11, 1), day(2026, 11, 30)).matches(&chunk));
        assert!(!dates(None, day(2026, 11, 1), day(2026, 11, 30)).is_empty());

        let folder = |f: &str| QueryFilter {
            folder: Some(f.to_string()),
            ..Default::default()
        };
        assert!(folder("work").matches(&chunk));
        assert!(folder("WORK/sub").matches(&chunk));
        assert!(!folder("wor").matches(&chunk));
        assert!(!folder("sub").matches(&chunk));
        // A pushed path is canonical and absolute: the leading `/` is not
        // part of the folder.
        for (path, hit) in [
            ("/work/a.md", true),
            ("work/a.md", true),
            ("/WORK/a.md", true),
            ("/other/work/a.md", false),
            ("/workshop/a.md", false),
        ] {
            let c = row_with(path, "t", vec![], &[]).chunk;
            assert_eq!(folder("work").matches(&c), hit, "{path}");
        }

        let tags = |t: &[&str]| QueryFilter {
            tags: t.iter().map(|t| t.to_string()).collect(),
            ..Default::default()
        };
        assert!(tags(&["x", "y"]).matches(&chunk));
        assert!(!tags(&["x", "z"]).matches(&chunk));

        chunk.meta.dates.clear();
        assert!(!dates(None, day(2026, 1, 1), day(2026, 12, 31)).matches(&chunk));
    }
}
