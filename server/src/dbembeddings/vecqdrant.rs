use log::debug;
use qdrant_client::{
    Qdrant,
    qdrant::{
        Condition, CreateCollectionBuilder, CreateFieldIndexCollectionBuilder, DeletePointsBuilder,
        Distance, FieldType, Filter, PayloadIncludeSelector, PointId, PointStruct, ScoredPoint,
        ScrollPointsBuilder, SearchPointsBuilder, UpsertPointsBuilder, Value, VectorParamsBuilder,
        with_payload_selector::SelectorOptions,
    },
};
use std::collections::HashMap;
use uuid::Uuid;

use crate::document::FlattenedChunk;

use super::{CollectionInfo, EmbeddedChunk, IndexedNote, QueryFilter, VectorStore};

pub struct VecQdrant {
    client: Qdrant,
    /// Vector width every collection is created at — the embedder's dimension,
    /// fixed at composition time.
    dim: usize,
    /// Namespace prefix; the effective Qdrant collection for a vault is
    /// `<prefix>-<vault-id>` (or just the vault-id when the prefix is empty).
    /// One vault ↔ one Qdrant collection.
    collection_prefix: String,
}

impl VecQdrant {
    pub async fn new(url: String, collection: String, dim: usize) -> anyhow::Result<Self> {
        let client = Qdrant::from_url(&url).build()?;

        Ok(Self {
            client,
            dim,
            collection_prefix: collection,
        })
    }

    /// The Qdrant collection name for a vault id.
    fn collection_name(&self, collection: &str) -> String {
        if self.collection_prefix.is_empty() {
            collection.to_string()
        } else {
            format!("{}-{}", self.collection_prefix, collection)
        }
    }

    /// The reserved qdrant collection holding the embedder fingerprint — inside
    /// the server's prefix namespace, excluded from collection listings by the
    /// `__` marker.
    fn fingerprint_collection(&self) -> String {
        self.collection_name("__fingerprint")
    }

    async fn collection_exists(&self, name: &str) -> anyhow::Result<bool> {
        let collections = self.client.list_collections().await?;
        Ok(collections.collections.iter().any(|c| c.name == name))
    }

    /// Ensures the vault's collection exists at the store's dimension. If it
    /// already exists at a *different* dimension (embedder/model changed),
    /// fails loudly rather than letting later upserts be rejected — the
    /// operator must drop the collection and re-index (adr: dimension change is
    /// destructive).
    async fn ensure_collection(&self, collection: &str) -> anyhow::Result<()> {
        let name = self.collection_name(collection);
        let dim = self.dim as u64;

        if self.collection_exists(&name).await? {
            if let Some(existing) = self.collection_dimension(&name).await?
                && existing != dim
            {
                anyhow::bail!(
                    "Qdrant collection `{name}` has dimension {existing} but the \
                     embedder produces {dim}. The embedder or model changed; drop \
                     the collection and re-index."
                );
            }
            return Ok(());
        }

        self.client
            .create_collection(
                CreateCollectionBuilder::new(&name)
                    .vectors_config(VectorParamsBuilder::new(dim, Distance::Cosine)),
            )
            .await?;
        self.client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(&name, "path", FieldType::Keyword)
                    .wait(true),
            )
            .await?;
        debug!("Created Qdrant collection: {}", name);
        Ok(())
    }

    /// Reads an existing collection's single-vector dimension, if available.
    async fn collection_dimension(&self, name: &str) -> anyhow::Result<Option<u64>> {
        use qdrant_client::qdrant::vectors_config::Config;
        let info = self.client.collection_info(name).await?;
        let dim = info
            .result
            .and_then(|r| r.config)
            .and_then(|c| c.params)
            .and_then(|p| p.vectors_config)
            .and_then(|vc| vc.config)
            .and_then(|cfg| match cfg {
                Config::Params(vp) => Some(vp.size),
                _ => None,
            });
        Ok(dim)
    }

    fn row_to_point(row: &EmbeddedChunk) -> PointStruct {
        let chunk = &row.chunk;
        let mut payload = HashMap::new();
        payload.insert("path".to_string(), Value::from(chunk.doc_path.clone()));
        payload.insert("title".to_string(), Value::from(chunk.title.clone()));
        // The whole note metadata as one JSON string: Qdrant does not filter
        // on it (see `query`), it only carries it back to the caller.
        payload.insert(
            "meta".to_string(),
            Value::from(serde_json::to_string(&chunk.meta).unwrap_or_default()),
        );
        payload.insert("text".to_string(), Value::from(chunk.text.clone()));
        payload.insert("hash".to_string(), Value::from(chunk.doc_hash.clone()));

        PointStruct::new(Uuid::new_v4().to_string(), row.vector.clone(), payload)
    }

    fn payload_to_chunk(payload: &HashMap<String, Value>) -> FlattenedChunk {
        let path = payload
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let title = payload
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let text = payload
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let hash = payload
            .get("hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();

        // A point that has no readable `meta` still searches, without dates,
        // tags or links.
        let meta = payload
            .get("meta")
            .and_then(|v| v.as_str())
            .and_then(|m| serde_json::from_str(m).ok())
            .unwrap_or_default();

        FlattenedChunk {
            doc_path: path,
            doc_hash: hash,
            title,
            text,
            meta,
        }
    }

    /// `(path, meta)` of every point in a collection, reading only those two
    /// payload fields (no vectors, no text). Missing collection → empty.
    async fn scroll_metas(
        &self,
        collection: &str,
    ) -> anyhow::Result<Vec<(String, crate::document::NoteMeta)>> {
        let name = self.collection_name(collection);
        if !self.collection_exists(&name).await? {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let mut pagination = PaginationStatus::First;
        while pagination.has_more() {
            let spb = ScrollPointsBuilder::new(&name)
                .with_payload(SelectorOptions::Include(PayloadIncludeSelector {
                    fields: vec!["path".to_string(), "meta".to_string()],
                }))
                .with_vectors(false)
                .limit(500);
            let spb = match pagination {
                PaginationStatus::Next(point_id) => spb.offset(point_id),
                _ => spb,
            };
            let scroll_result = self.client.scroll(spb).await?;
            for p in &scroll_result.result {
                let chunk = Self::payload_to_chunk(&p.payload);
                out.push((chunk.doc_path, chunk.meta));
            }
            pagination = scroll_result.next_page_offset.into();
        }
        Ok(out)
    }

    fn point_to_chunk(point: &ScoredPoint) -> FlattenedChunk {
        Self::payload_to_chunk(&point.payload)
    }
}

enum PaginationStatus {
    First,
    Next(PointId),
    None,
}

impl PaginationStatus {
    fn has_more(&self) -> bool {
        !matches!(self, PaginationStatus::None)
    }
}

impl From<Option<PointId>> for PaginationStatus {
    fn from(value: Option<PointId>) -> Self {
        match value {
            Some(point) => Self::Next(point),
            None => Self::None,
        }
    }
}

#[async_trait::async_trait]
impl VectorStore for VecQdrant {
    async fn store(&self, collection: &str, rows: &[EmbeddedChunk]) -> anyhow::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        self.ensure_collection(collection).await?;
        let name = self.collection_name(collection);
        let points = rows.iter().map(Self::row_to_point).collect::<Vec<_>>();
        self.client
            .upsert_points(UpsertPointsBuilder::new(&name, points).wait(true))
            .await?;
        Ok(())
    }

    async fn delete(&self, collection: &str, paths: &[String]) -> anyhow::Result<()> {
        let name = self.collection_name(collection);
        // A vault never indexed has no collection; nothing to delete.
        if paths.is_empty() || !self.collection_exists(&name).await? {
            return Ok(());
        }
        self.client
            .delete_points(
                DeletePointsBuilder::new(&name)
                    .points(Filter::must([Condition::matches("path", paths.to_vec())]))
                    .wait(true),
            )
            .await?;
        Ok(())
    }

    async fn chunks_with_vectors(
        &self,
        collection: &str,
        paths: &[String],
    ) -> anyhow::Result<Vec<EmbeddedChunk>> {
        let name = self.collection_name(collection);
        if paths.is_empty() || !self.collection_exists(&name).await? {
            return Ok(Vec::new());
        }

        let filter = Filter::must([Condition::matches("path", paths.to_vec())]);
        let page_size = 200;
        let mut out = Vec::new();
        let mut pagination = PaginationStatus::First;
        while pagination.has_more() {
            let spb = match pagination {
                PaginationStatus::Next(point_id) => ScrollPointsBuilder::new(&name)
                    .filter(filter.clone())
                    .with_payload(true)
                    .with_vectors(true)
                    .offset(point_id)
                    .limit(page_size),
                _ => ScrollPointsBuilder::new(&name)
                    .filter(filter.clone())
                    .with_payload(true)
                    .with_vectors(true)
                    .limit(page_size),
            };
            let scroll_result = self.client.scroll(spb).await?;
            for p in &scroll_result.result {
                use qdrant_client::qdrant::vector_output::Vector as VectorOut;
                // Collections here hold a single unnamed dense vector per
                // point (see ensure_collection); anything else is skipped.
                let Some(VectorOut::Dense(dense)) = p.vectors.as_ref().and_then(|v| v.get_vector())
                else {
                    continue;
                };
                out.push(EmbeddedChunk {
                    chunk: Self::payload_to_chunk(&p.payload),
                    vector: dense.data,
                });
            }
            pagination = scroll_result.next_page_offset.into();
        }
        Ok(out)
    }

    async fn query(
        &self,
        collection: &str,
        vector: Vec<f32>,
        limit: usize,
        filter: &QueryFilter,
    ) -> anyhow::Result<Vec<(f64, FlattenedChunk)>> {
        // Checked first so it needs no round trip to the server. SQLite is the
        // fully supported store; filtering here would need payload indexes.
        if !filter.is_empty() {
            anyhow::bail!("filtered search is not supported on the qdrant store");
        }
        let name = self.collection_name(collection);
        // Missing collection → no results, never an error (matches the trait
        // contract; a search may race the first push).
        if !self.collection_exists(&name).await? {
            return Ok(Vec::new());
        }

        let search_result = self
            .client
            .search_points(SearchPointsBuilder::new(&name, vector, limit as u64).with_payload(true))
            .await?;

        let results: Vec<(f64, FlattenedChunk)> = search_result
            .result
            .iter()
            .map(|point| {
                let chunk = Self::point_to_chunk(point);
                (point.score as f64, chunk)
            })
            .collect();

        debug!(
            "Query returned {} results (scores: {:.4} to {:.4})",
            results.len(),
            results.first().map(|(d, _)| *d).unwrap_or(0.0),
            results.last().map(|(d, _)| *d).unwrap_or(0.0)
        );

        Ok(results)
    }

    async fn indexed_notes(
        &self,
        collection: &str,
    ) -> anyhow::Result<HashMap<String, IndexedNote>> {
        let name = self.collection_name(collection);
        // A vault never indexed yet has no collection. Reconcile starts by
        // reading the server's hash set, so return an empty map rather than
        // erroring on the missing collection — the client then pushes everything.
        if !self.collection_exists(&name).await? {
            return Ok(HashMap::new());
        }

        let page_size = 200;
        let mut result = HashMap::new();
        let mut pagination = PaginationStatus::First;

        while pagination.has_more() {
            let spb = match pagination {
                PaginationStatus::Next(point_id) => ScrollPointsBuilder::new(&name)
                    .with_payload(true)
                    .with_vectors(false)
                    .offset(point_id)
                    .limit(page_size),
                _ => ScrollPointsBuilder::new(&name)
                    .with_payload(true)
                    .with_vectors(false)
                    .limit(page_size),
            };
            let scroll_result = self.client.scroll(spb).await?;

            scroll_result.result.into_iter().for_each(|p| {
                let chunk = Self::payload_to_chunk(&p.payload);
                let path = chunk.doc_path.clone();
                let note = IndexedNote {
                    path: chunk.doc_path,
                    content_hash: chunk.doc_hash,
                    last_indexed: 0,
                };
                result.insert(path, note);
            });

            pagination = scroll_result.next_page_offset.into();
        }

        debug!("Indexed: {}", result.len());

        Ok(result)
    }

    /// NOTE: counts distinct notes via a full payload scroll per collection
    /// (O(points)), so it's meant for the occasional admin collections page. Use
    /// [`collection_names`](Self::collection_names) for pickers.
    async fn list_collections(&self) -> anyhow::Result<Vec<CollectionInfo>> {
        let mut names = self.collection_names().await?;
        names.sort();
        let mut out = Vec::with_capacity(names.len());
        for vault in names {
            // Count distinct indexed notes (reuses the payload scroll so the
            // number matches what reconciliation sees), not raw chunk points.
            let note_count = self
                .indexed_notes(&vault)
                .await
                .map(|m| m.len())
                .unwrap_or(0);
            out.push(CollectionInfo {
                name: vault,
                note_count,
            });
        }
        Ok(out)
    }

    async fn collection_names(&self) -> anyhow::Result<Vec<String>> {
        // Only collections under our prefix belong to this server. With an empty
        // prefix every Qdrant collection on the instance is treated as a vault.
        let dash_prefix = if self.collection_prefix.is_empty() {
            String::new()
        } else {
            format!("{}-", self.collection_prefix)
        };
        let all = self.client.list_collections().await?;
        let names = all
            .collections
            .into_iter()
            .filter_map(|c| {
                let name = if dash_prefix.is_empty() {
                    Some(c.name)
                } else {
                    c.name.strip_prefix(&dash_prefix).map(str::to_string)
                }?;
                // Reserved metadata collections (fingerprint) are not vaults.
                (!name.starts_with("__")).then_some(name)
            })
            .collect();
        Ok(names)
    }

    async fn read_fingerprint(&self) -> anyhow::Result<Option<String>> {
        let name = self.fingerprint_collection();
        if !self.collection_exists(&name).await? {
            return Ok(None);
        }
        let scroll = self
            .client
            .scroll(
                ScrollPointsBuilder::new(&name)
                    .with_payload(true)
                    .with_vectors(false)
                    .limit(1),
            )
            .await?;
        Ok(scroll
            .result
            .first()
            .and_then(|p| p.payload.get("fingerprint"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()))
    }

    async fn write_fingerprint(&self, fingerprint: &str) -> anyhow::Result<()> {
        let name = self.fingerprint_collection();
        if !self.collection_exists(&name).await? {
            self.client
                .create_collection(
                    CreateCollectionBuilder::new(&name)
                        .vectors_config(VectorParamsBuilder::new(1, Distance::Cosine)),
                )
                .await?;
        }
        let mut payload = HashMap::new();
        payload.insert(
            "fingerprint".to_string(),
            Value::from(fingerprint.to_string()),
        );
        // Fixed point id → an upsert overwrites the previous fingerprint.
        let point = PointStruct::new(
            "00000000-0000-0000-0000-000000000001".to_string(),
            vec![0.0f32],
            payload,
        );
        self.client
            .upsert_points(UpsertPointsBuilder::new(&name, vec![point]).wait(true))
            .await?;
        Ok(())
    }

    async fn drop_all_collections(&self) -> anyhow::Result<()> {
        for vault in self.collection_names().await? {
            let full = self.collection_name(&vault);
            self.client.delete_collection(&full).await?;
        }
        Ok(())
    }

    async fn note_links(&self, collection: &str) -> anyhow::Result<HashMap<String, Vec<String>>> {
        let metas = self.scroll_metas(collection).await?;
        Ok(super::aggregate_links(metas))
    }

    async fn date_sources(&self, collection: &str) -> anyhow::Result<Vec<String>> {
        let metas = self.scroll_metas(collection).await?;
        Ok(super::aggregate_date_sources(
            metas.into_iter().map(|(_, m)| m),
        ))
    }
}

/// Conformance against a live Qdrant. `#[ignore]`d because they need a running
/// server: `QDRANT_URL` (default `http://localhost:6334`), then
/// `cargo test -p kimun_server -- --ignored`. Each run uses a fresh prefix-less
/// namespace under `kimun-conformance-*` collections; they are dropped first so
/// reruns start clean.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dbembeddings::conformance;

    /// One namespace per test (prefix `kimun-conf-<tag>`) so parallel `--ignored`
    /// runs don't race each other; each test drops its own leftovers first.
    async fn store(tag: &str) -> VecQdrant {
        let url =
            std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".to_string());
        let store = VecQdrant::new(url, format!("kimun-conf-{tag}"), conformance::DIM)
            .await
            .expect("qdrant client");
        for name in store.collection_names().await.expect("qdrant reachable") {
            let full = store.collection_name(&name);
            let _ = store.client.delete_collection(&full).await;
        }
        store
    }

    #[tokio::test]
    #[ignore = "needs a live Qdrant (set QDRANT_URL, default localhost:6334)"]
    async fn conformance_store_then_query() {
        let s = store("query").await;
        conformance::store_then_query_finds_the_chunk(&s, "v").await;
    }

    #[tokio::test]
    #[ignore = "needs a live Qdrant (set QDRANT_URL, default localhost:6334)"]
    async fn conformance_query_limit() {
        let s = store("limit").await;
        conformance::query_respects_limit(&s, "v").await;
    }

    #[tokio::test]
    #[ignore = "needs a live Qdrant (set QDRANT_URL, default localhost:6334)"]
    async fn conformance_missing_collection() {
        let s = store("missing").await;
        conformance::missing_collection_is_empty_not_error(&s, "nope").await;
    }

    #[tokio::test]
    #[ignore = "needs a live Qdrant (set QDRANT_URL, default localhost:6334)"]
    async fn conformance_note_links() {
        let s = store("links").await;
        conformance::note_links_returns_each_note_once(&s, "v").await;
    }

    #[tokio::test]
    #[ignore = "needs a live Qdrant (set QDRANT_URL, default localhost:6334)"]
    async fn conformance_date_sources() {
        let s = store("sources").await;
        conformance::date_sources_lists_distinct_sorted_sources_including_journal(&s, "v").await;
    }

    #[tokio::test]
    #[ignore = "needs a live Qdrant (set QDRANT_URL, default localhost:6334)"]
    async fn conformance_missing_collection_meta_reads() {
        let s = store("missingmeta").await;
        conformance::missing_collection_returns_empty_for_both_reads(&s, "nope").await;
    }

    #[tokio::test]
    #[ignore = "needs a live Qdrant (set QDRANT_URL, default localhost:6334)"]
    async fn conformance_delete() {
        let s = store("delete").await;
        conformance::delete_removes_every_chunk_of_the_note(&s, "v").await;
    }

    #[tokio::test]
    #[ignore = "needs a live Qdrant (set QDRANT_URL, default localhost:6334)"]
    async fn conformance_chunks_with_vectors() {
        let s = store("chunkvecs").await;
        conformance::chunks_with_vectors_returns_the_notes_rows(&s, "v").await;
    }

    #[tokio::test]
    #[ignore = "needs a live Qdrant (set QDRANT_URL, default localhost:6334)"]
    async fn conformance_indexed_notes() {
        let s = store("notes").await;
        conformance::indexed_notes_reports_one_hash_per_path(&s, "v").await;
    }

    #[tokio::test]
    #[ignore = "needs a live Qdrant (set QDRANT_URL, default localhost:6334)"]
    async fn conformance_collections() {
        let s = store("collections").await;
        conformance::collections_list_each_vault(&s, "vault_a", "vault_b").await;
    }

    #[tokio::test]
    #[ignore = "needs a live Qdrant (set QDRANT_URL, default localhost:6334)"]
    async fn conformance_round_trip() {
        let s = store("roundtrip").await;
        conformance::stored_chunk_round_trips_its_fields(&s, "v").await;
    }

    #[tokio::test]
    #[ignore = "needs a live Qdrant (set QDRANT_URL, default localhost:6334)"]
    async fn conformance_fingerprint_round_trip() {
        let s = store("fingerprint").await;
        conformance::fingerprint_round_trips_and_starts_absent(&s).await;
    }

    #[tokio::test]
    #[ignore = "needs a live Qdrant (set QDRANT_URL, default localhost:6334)"]
    async fn conformance_drop_all() {
        let s = store("dropall").await;
        conformance::drop_all_removes_every_collection_but_not_the_fingerprint_slot(&s, "va", "vb")
            .await;
    }

    #[tokio::test]
    #[ignore = "needs a live Qdrant (set QDRANT_URL, default localhost:6334)"]
    async fn conformance_fingerprint_not_a_collection() {
        let s = store("fpslot").await;
        conformance::fingerprint_slot_never_appears_as_a_collection(&s).await;
    }

    // These two run in plain `cargo test`: the client connects lazily, and
    // neither touches the server.
    #[tokio::test]
    async fn non_empty_filter_is_refused_before_any_server_call() {
        let s = VecQdrant::new(
            "http://127.0.0.1:1".to_string(),
            "unused".to_string(),
            conformance::DIM,
        )
        .await
        .unwrap();
        let filter = QueryFilter {
            tags: vec!["x".to_string()],
            ..Default::default()
        };
        let err = s
            .query("v", conformance::vector_for("q"), 1, &filter)
            .await
            .expect_err("filter must be refused");
        assert_eq!(
            "filtered search is not supported on the qdrant store",
            err.to_string()
        );
    }

    #[test]
    fn metadata_round_trips_through_the_payload() {
        use crate::document::NoteDate;
        let mut row = conformance::row("a.md", "h", "body");
        row.chunk.meta.dates = vec![NoteDate {
            source: "due".to_string(),
            day: chrono::NaiveDate::from_ymd_opt(2026, 12, 1).unwrap(),
        }];
        row.chunk.meta.tags = vec!["work".to_string()];
        row.chunk.meta.links = vec!["Project X".to_string()];
        let point = VecQdrant::row_to_point(&row);
        let back = VecQdrant::payload_to_chunk(&point.payload);
        assert_eq!(row.chunk.meta, back.meta);
    }
}
