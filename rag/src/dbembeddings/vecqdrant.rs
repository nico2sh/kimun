use std::collections::HashMap;
use log::debug;
use qdrant_client::{
    Qdrant,
    qdrant::{
        CreateCollectionBuilder, Distance, PointStruct, VectorParamsBuilder,
        ScoredPoint, SearchPointsBuilder, UpsertPointsBuilder,
    },
};

use crate::document::KimunChunk;

use super::{
    Embeddings, IndexedNote,
    embedder::{Embedder, fastembedder::FastEmbedder},
};

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

            debug!("Created Qdrant collection: {}", self.collection);
        }

        Ok(())
    }

    fn chunk_to_point(chunk: &KimunChunk, embedding: Vec<f32>, id: u64) -> PointStruct {
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

        PointStruct::new(id, embedding, payload)
    }

    fn point_to_chunk(point: &ScoredPoint) -> anyhow::Result<KimunChunk> {
        let payload = &point.payload;

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
        let date_str = payload
            .get("date")
            .and_then(|v| v.as_str());
        let text = payload
            .get("text")
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

        Ok(KimunChunk {
            content: text,
            metadata: crate::document::KimunMetadata {
                source_path: path,
                title,
                date,
            },
        })
    }
}

#[async_trait::async_trait]
impl Embeddings for VecQdrant {
    fn init(&mut self) -> anyhow::Result<()> {
        // Qdrant initialization is async, so we can't do it here
        // Collection will be created on first use
        Ok(())
    }

    async fn store_embeddings(&self, content: &[KimunChunk]) -> anyhow::Result<()> {
        self.ensure_collection().await?;

        // Generate embeddings
        let embeddings = self.embedder.generate_embeddings(content).await?;

        // Create points in batches of 100
        for (batch_idx, batch) in content.chunks(100).enumerate() {
            let start_id = (batch_idx * 100) as u64;
            let points: Vec<PointStruct> = batch
                .iter()
                .zip(embeddings.iter().skip(batch_idx * 100))
                .enumerate()
                .map(|(i, (chunk, embedding))| {
                    VecQdrant::chunk_to_point(chunk, embedding.clone(), start_id + i as u64)
                })
                .collect();

            // Upsert points using the builder API
            self.client
                .upsert_points(UpsertPointsBuilder::new(&self.collection, points))
                .await?;
        }

        debug!("Stored {} embeddings in Qdrant", content.len());
        Ok(())
    }

    async fn query_embedding(&self, query: &str) -> anyhow::Result<Vec<(f64, KimunChunk)>> {
        self.ensure_collection().await?;

        // Generate query embedding
        let query_vec = self.embedder.prompt_embedding(query).await?;

        // Search for similar vectors using the builder API
        let search_result = self
            .client
            .search_points(
                SearchPointsBuilder::new(&self.collection, query_vec, 128)
                    .with_payload(true)
            )
            .await?;

        // Convert results
        let results: Vec<(f64, KimunChunk)> = search_result
            .result
            .iter()
            .map(|point| {
                let chunk = VecQdrant::point_to_chunk(point).unwrap();
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

    fn get_indexed_notes(&self) -> anyhow::Result<HashMap<String, IndexedNote>> {
        // Qdrant doesn't have built-in index tracking
        // We would need to store this in a separate collection or use payload
        // For now, return empty - this means Qdrant won't support incremental indexing
        // TODO: Implement index tracking in Qdrant using a separate collection or metadata
        Ok(HashMap::new())
    }

    fn mark_as_indexed(&self, _path: &str, _content_hash: &str) -> anyhow::Result<()> {
        // TODO: Implement index tracking
        Ok(())
    }

    fn remove_indexed_note(&self, path: &str) -> anyhow::Result<()> {
        // Delete all points with this path
        // This is synchronous but calls async method - we'll need to handle this
        // For now, return Ok as deletion can happen on next indexing
        // TODO: Implement proper deletion
        debug!("Note removal not yet implemented for Qdrant: {}", path);
        Ok(())
    }
}
