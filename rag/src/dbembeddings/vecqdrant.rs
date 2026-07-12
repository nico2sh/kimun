use log::debug;
use qdrant_client::{
    Payload, Qdrant,
    qdrant::{
        Condition, CreateCollectionBuilder, CreateFieldIndexCollectionBuilder, DeletePointsBuilder,
        Distance, FieldType, Filter, PointId, PointStruct, ScoredPoint, ScrollPointsBuilder,
        SearchPointsBuilder, SetPayloadPointsBuilder, UpsertPointsBuilder, Value,
        VectorParamsBuilder,
    },
};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

use crate::document::{FlattenedChunk, KimunDoc};

use std::sync::Arc;

use super::{Embeddings, IndexedNote, embedder::Embedder};

const TOP_RESULTS: u64 = 80;

pub struct VecQdrant {
    embedder: Arc<dyn Embedder>,
    client: Qdrant,
    /// Namespace prefix; the effective Qdrant collection for a vault is
    /// `<prefix>-<vault-id>` (or just the vault-id when the prefix is empty).
    /// One vault ↔ one Qdrant collection (adr/0020).
    collection_prefix: String,
}

impl VecQdrant {
    pub async fn new(
        url: String,
        collection: String,
        embedder: Arc<dyn Embedder>,
    ) -> anyhow::Result<Self> {
        let client = Qdrant::from_url(&url).build()?;

        Ok(Self {
            embedder,
            client,
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

    /// Ensures the vault's collection exists at the embedder's dimension. If it
    /// already exists at a *different* dimension (embedder/model changed),
    /// fails loudly rather than letting later upserts be rejected — the
    /// operator must drop the collection and re-index (adr: dimension change is
    /// destructive).
    async fn ensure_collection(&self, collection: &str) -> anyhow::Result<()> {
        let name = self.collection_name(collection);
        let dim = self.embedder.dimension() as u64;

        let collections = self.client.list_collections().await?;
        let exists = collections.collections.iter().any(|c| c.name == name);

        if exists {
            if let Some(existing) = self.collection_dimension(&name).await? {
                if existing != dim {
                    anyhow::bail!(
                        "Qdrant collection `{name}` has dimension {existing} but the \
                         embedder produces {dim}. The embedder or model changed; drop \
                         the collection and re-index."
                    );
                }
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

    fn chunk_to_point(chunk: &FlattenedChunk, embedding: Vec<f32>) -> PointStruct {
        use qdrant_client::qdrant::Value;
        use std::collections::HashMap;

        let mut payload = HashMap::new();
        payload.insert("path".to_string(), Value::from(chunk.doc_path.clone()));
        payload.insert("title".to_string(), Value::from(chunk.title.clone()));
        payload.insert(
            "date".to_string(),
            Value::from(chunk.get_date_string().unwrap_or_default()),
        );
        payload.insert("text".to_string(), Value::from(chunk.text.clone()));
        payload.insert("hash".to_string(), Value::from(chunk.doc_hash.clone()));

        PointStruct::new(Uuid::new_v4().to_string(), embedding, payload)
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
        let date_str = payload.get("date").and_then(|v| v.as_str());
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

        let date = if let Some(ds) = date_str {
            if !ds.is_empty() {
                chrono::NaiveDate::parse_from_str(ds, "%Y-%m-%d").ok()
            } else {
                None
            }
        } else {
            None
        };

        FlattenedChunk {
            doc_path: path,
            doc_hash: hash,
            title,
            text,
            date,
        }
    }

    fn point_to_chunk(point: &ScoredPoint) -> FlattenedChunk {
        let payload = &point.payload;

        VecQdrant::payload_to_chunk(payload)
    }
}

enum PaginationStatus {
    First,
    Next(PointId),
    None,
}

impl PaginationStatus {
    fn has_more(&self) -> bool {
        match self {
            PaginationStatus::None => false,
            _ => true,
        }
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
impl Embeddings for VecQdrant {
    async fn init(&self) -> anyhow::Result<()> {
        // Collections are per-vault and created lazily on first store, so there
        // is nothing to do at server start.
        Ok(())
    }

    async fn store_embeddings(&self, collection: &str, content: &[KimunDoc]) -> anyhow::Result<()> {
        self.ensure_collection(collection).await?;
        let name = self.collection_name(collection);
        const BATCH_SIZE: usize = 100;

        // Sub-split sections to the embedding window, then embed and store each
        // sub-chunk 1:1 — so a point's stored text is exactly the text that
        // produced its vector (mismatched otherwise).
        let chunks = FlattenedChunk::from_chunks_split(content, 800, 1536);
        debug!("{} docs split to {} chunks", content.len(), chunks.len());

        for batch in chunks.chunks(BATCH_SIZE) {
            let embeddings = self.embedder.generate_embeddings(batch).await?;
            if embeddings.len() != batch.len() {
                anyhow::bail!(
                    "embedder returned {} vectors for {} chunks",
                    embeddings.len(),
                    batch.len()
                );
            }
            let points = batch
                .iter()
                .zip(embeddings)
                .map(|(chunk, embedding)| VecQdrant::chunk_to_point(chunk, embedding))
                .collect::<Vec<PointStruct>>();
            self.client
                .upsert_points(UpsertPointsBuilder::new(&name, points))
                .await?;
        }

        Ok(())
    }

    async fn delete_embeddings(&self, collection: &str, paths: Vec<&String>) -> anyhow::Result<()> {
        // For each path in paths, delete all points where payload.path matches
        self.client
            .delete_points(
                DeletePointsBuilder::new(self.collection_name(collection))
                    .points(Filter::must([Condition::matches(
                        "path",
                        paths
                            .into_iter()
                            .map(|p| p.to_owned())
                            .collect::<Vec<String>>(),
                    )]))
                    .wait(true),
            )
            .await?;
        Ok(())
    }

    async fn query_embedding(
        &self,
        collection: &str,
        query: &str,
    ) -> anyhow::Result<Vec<(f64, FlattenedChunk)>> {
        // Generate query embedding
        let query_vec = self.embedder.prompt_embedding(query).await?;

        // Search for similar vectors using the builder API
        let search_result = self
            .client
            .search_points(
                SearchPointsBuilder::new(self.collection_name(collection), query_vec, TOP_RESULTS)
                    .with_payload(true),
            )
            .await?;

        // Convert results
        let results: Vec<(f64, FlattenedChunk)> = search_result
            .result
            .iter()
            .map(|point| {
                let chunk = VecQdrant::point_to_chunk(point);
                (point.score as f64, chunk)
            })
            .collect();

        debug!(
            "Query returned {} results (distances: {:.4} to {:.4})",
            results.len(),
            results.first().map(|(d, _)| *d).unwrap_or(0.0),
            results.last().map(|(d, _)| *d).unwrap_or(0.0)
        );

        Ok(results)
    }

    async fn get_indexed_notes(
        &self,
        collection: &str,
    ) -> anyhow::Result<HashMap<String, IndexedNote>> {
        let name = self.collection_name(collection);
        // A vault never indexed yet has no collection. Reconcile starts by
        // reading the server's hash set, so return an empty map rather than
        // erroring on the missing collection — the client then pushes everything.
        let collections = self.client.list_collections().await?;
        if !collections.collections.iter().any(|c| c.name == name) {
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
                let chunk = VecQdrant::payload_to_chunk(&p.payload);
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

    async fn remove_indexed_note(&self, collection: &str, path: &str) -> anyhow::Result<()> {
        // Delete all points with this path
        self.client
            .set_payload(
                SetPayloadPointsBuilder::new(
                    self.collection_name(collection),
                    Payload::try_from(json!({
                        "hash": "",
                    }))
                    .unwrap(),
                )
                .points_selector(Filter::must([Condition::matches("path", path.to_string())]))
                .wait(true),
            )
            .await?;
        Ok(())
    }

    /// NOTE: counts distinct notes via a full payload scroll per collection
    /// (O(points)), so it's meant for the occasional admin collections page. Use
    /// [`collection_names`](Self::collection_names) for pickers.
    async fn list_collections(&self) -> anyhow::Result<Vec<crate::dbembeddings::CollectionInfo>> {
        let mut names = self.collection_names().await?;
        names.sort();
        let mut out = Vec::with_capacity(names.len());
        for vault in names {
            // Count distinct indexed notes (reuses the payload scroll so the
            // number matches what reconciliation sees), not raw chunk points.
            let note_count = self
                .get_indexed_notes(&vault)
                .await
                .map(|m| m.len())
                .unwrap_or(0);
            out.push(crate::dbembeddings::CollectionInfo {
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
                if dash_prefix.is_empty() {
                    Some(c.name)
                } else {
                    c.name.strip_prefix(&dash_prefix).map(str::to_string)
                }
            })
            .collect();
        Ok(names)
    }
}
