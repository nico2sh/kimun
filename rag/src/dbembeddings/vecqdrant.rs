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

use crate::document::KimunChunk;

use super::{
    Embeddings, IndexedNote,
    embedder::{Embedder, fastembedder::FastEmbedder},
};

const TOP_RESULTS: u64 = 512;

pub struct VecQdrant {
    embedder: FastEmbedder,
    client: Qdrant,
    collection: String,
}

impl VecQdrant {
    pub async fn new(url: String, collection: String) -> anyhow::Result<Self> {
        let client = Qdrant::from_url(&url).build()?;
        let embedder = FastEmbedder::new()?;

        Ok(Self {
            embedder,
            client,
            collection,
        })
    }

    async fn ensure_collection(&self) -> anyhow::Result<()> {
        // Check if collection exists
        let collections = self.client.list_collections().await?;
        let exists = collections
            .collections
            .iter()
            .any(|c| c.name == self.collection);

        if !exists {
            // Create collection with cosine distance and 1024 dimensions
            self.client
                .create_collection(
                    CreateCollectionBuilder::new(&self.collection)
                        .vectors_config(VectorParamsBuilder::new(1024, Distance::Cosine)),
                )
                .await?;

            self.client
                .create_field_index(
                    CreateFieldIndexCollectionBuilder::new(
                        self.collection.as_str(),
                        "path",
                        FieldType::Keyword,
                    )
                    .wait(true),
                )
                .await?;

            debug!("Created Qdrant collection: {}", self.collection);
        }

        Ok(())
    }

    fn chunk_to_point(chunk: &KimunChunk, embedding: Vec<f32>) -> PointStruct {
        use qdrant_client::qdrant::Value;
        use std::collections::HashMap;

        let mut payload = HashMap::new();
        payload.insert(
            "path".to_string(),
            Value::from(chunk.metadata.source_path.clone()),
        );
        payload.insert(
            "title".to_string(),
            Value::from(chunk.metadata.title.clone()),
        );
        payload.insert(
            "date".to_string(),
            Value::from(chunk.metadata.get_date_string().unwrap_or_default()),
        );
        payload.insert("text".to_string(), Value::from(chunk.content.clone()));
        payload.insert("hash".to_string(), Value::from(chunk.metadata.hash.clone()));

        PointStruct::new(Uuid::new_v4().to_string(), embedding, payload)
    }

    fn payload_to_chunk(payload: &HashMap<String, Value>) -> KimunChunk {
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

        KimunChunk {
            content: text,
            metadata: crate::document::KimunMetadata {
                source_path: path,
                title,
                date,
                hash,
            },
        }
    }

    fn point_to_chunk(point: &ScoredPoint) -> KimunChunk {
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
        // Qdrant initialization is async, so we can't do it here
        // Collection will be created on first use
        self.ensure_collection().await?;
        Ok(())
    }

    async fn store_embeddings(&self, content: &[KimunChunk]) -> anyhow::Result<()> {
        // Generate embeddings
        let embeddings = self.embedder.generate_embeddings(content).await?;

        // Create points in batches of 100
        for (batch_idx, batch) in content.chunks(100).enumerate() {
            let points: Vec<PointStruct> = batch
                .iter()
                .zip(embeddings.iter().skip(batch_idx * 100))
                .map(|(chunk, embedding)| VecQdrant::chunk_to_point(chunk, embedding.clone()))
                .collect();

            // Upsert points using the builder API
            self.client
                .upsert_points(UpsertPointsBuilder::new(&self.collection, points))
                .await?;
        }

        Ok(())
    }

    async fn delete_embeddings(&self, paths: Vec<&String>) -> anyhow::Result<()> {
        // For each path in paths, delete all points where payload.path matches
        self.client
            .delete_points(
                DeletePointsBuilder::new(&self.collection)
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

    async fn query_embedding(&self, query: &str) -> anyhow::Result<Vec<(f64, KimunChunk)>> {
        // Generate query embedding
        let query_vec = self.embedder.prompt_embedding(query).await?;

        // Search for similar vectors using the builder API
        let search_result = self
            .client
            .search_points(
                SearchPointsBuilder::new(&self.collection, query_vec, TOP_RESULTS)
                    .with_payload(true),
            )
            .await?;

        // Convert results
        let results: Vec<(f64, KimunChunk)> = search_result
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

    async fn get_indexed_notes(&self) -> anyhow::Result<HashMap<String, IndexedNote>> {
        let page_size = 200;
        let mut result = HashMap::new();
        let mut pagination = PaginationStatus::First;

        while pagination.has_more() {
            let spb = match pagination {
                PaginationStatus::Next(point_id) => ScrollPointsBuilder::new(&self.collection)
                    .with_payload(true)
                    .with_vectors(false)
                    .offset(point_id)
                    .limit(page_size),
                _ => ScrollPointsBuilder::new(&self.collection)
                    .with_payload(true)
                    .with_vectors(false)
                    .limit(page_size),
            };
            let scroll_result = self.client.scroll(spb).await?;

            scroll_result.result.into_iter().for_each(|p| {
                let chunk = VecQdrant::payload_to_chunk(&p.payload);
                let path = chunk.metadata.source_path.clone();
                let note = IndexedNote {
                    path: chunk.metadata.source_path,
                    content_hash: chunk.metadata.hash,
                    last_indexed: 0,
                };
                result.insert(path, note);
            });

            pagination = scroll_result.next_page_offset.into();
        }

        debug!("Indexed: {}", result.len());

        Ok(result)
    }

    async fn mark_as_indexed(&self, path: &str, content_hash: &str) -> anyhow::Result<()> {
        self.client
            .set_payload(
                SetPayloadPointsBuilder::new(
                    &self.collection,
                    Payload::try_from(json!({
                        "hash": content_hash.to_string(),
                    }))
                    .unwrap(),
                )
                .points_selector(Filter::must([Condition::matches("path", path.to_string())]))
                .wait(true),
            )
            .await?;
        Ok(())
    }

    async fn remove_indexed_note(&self, path: &str) -> anyhow::Result<()> {
        // Delete all points with this path
        self.client
            .set_payload(
                SetPayloadPointsBuilder::new(
                    &self.collection,
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
}
