//! Embedded, file-backed vector store built on [LanceDB](https://lancedb.com).
//!
//! Unlike Qdrant this needs no server: the whole store is a directory of Lance
//! datasets on local disk. One **table per vault** (keyed by the vault id, adr/
//! 0020) holds every chunk's embedding plus its metadata columns. Good for a
//! single-machine deployment where running a separate Qdrant is overkill.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use futures::TryStreamExt;
use lancedb::arrow::arrow_array::{
    Array, FixedSizeListArray, Float32Array, RecordBatch, StringArray, types::Float32Type,
};
use lancedb::arrow::arrow_schema::{DataType, Field, Schema};
use lancedb::query::{ExecutableQuery, QueryBase, Select};
use lancedb::{Connection, DistanceType, Table, connect};
use log::debug;

use super::{CollectionInfo, Embeddings, IndexedNote, embedder::Embedder};
use crate::document::{FlattenedChunk, KimunDoc};

/// Candidate pool size pulled from the vector search before reranking — matches
/// the Qdrant backend so both return the same breadth of context.
const TOP_RESULTS: usize = 80;

pub struct VecLance {
    embedder: Arc<dyn Embedder>,
    connection: Connection,
}

impl VecLance {
    /// Opens (creating the directory if needed) the Lance database rooted at
    /// `path`. Each vault becomes a table inside it.
    pub async fn new(path: impl AsRef<Path>, embedder: Arc<dyn Embedder>) -> anyhow::Result<Self> {
        let uri = path.as_ref().to_string_lossy().to_string();
        let connection = connect(&uri).execute().await?;
        Ok(Self {
            embedder,
            connection,
        })
    }

    /// The Arrow schema for a vault table at the embedder's vector width. Column
    /// order here must match [`build_batch`](Self::build_batch).
    fn schema(dim: usize) -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("path", DataType::Utf8, false),
            Field::new("hash", DataType::Utf8, false),
            Field::new("title", DataType::Utf8, true),
            Field::new("date", DataType::Utf8, true),
            Field::new("text", DataType::Utf8, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    dim as i32,
                ),
                false,
            ),
        ]))
    }

    async fn table_exists(&self, collection: &str) -> anyhow::Result<bool> {
        let names = self.connection.table_names().execute().await?;
        Ok(names.iter().any(|n| n == collection))
    }

    /// Opens the vault's table, creating it empty (at the embedder's dimension)
    /// if absent. Fails loudly if an existing table's vector width differs from
    /// the embedder's — the model changed and the operator must drop the table
    /// and re-index (adr: dimension change is destructive). The returned bool is
    /// `true` when the table was just created, so the caller can skip the
    /// upsert-delete against a table that is known to be empty.
    async fn open_or_create(&self, collection: &str) -> anyhow::Result<(Table, bool)> {
        let dim = self.embedder.dimension();
        if self.table_exists(collection).await? {
            let table = self.connection.open_table(collection).execute().await?;
            if let Some(existing) = Self::vector_dim(table.schema().await?.as_ref())
                && existing != dim
            {
                anyhow::bail!(
                    "LanceDB table `{collection}` has dimension {existing} but the \
                     embedder produces {dim}. The embedder or model changed; drop \
                     the table and re-index."
                );
            }
            Ok((table, false))
        } else {
            let table = self
                .connection
                .create_empty_table(collection, Self::schema(dim))
                .execute()
                .await?;
            debug!("Created LanceDB table: {collection}");
            Ok((table, true))
        }
    }

    /// The declared width of the `vector` column in an existing table's schema.
    fn vector_dim(schema: &Schema) -> Option<usize> {
        match schema.field_with_name("vector").ok()?.data_type() {
            DataType::FixedSizeList(_, n) => Some(*n as usize),
            _ => None,
        }
    }

    fn build_batch(
        chunks: &[FlattenedChunk],
        embeddings: Vec<Vec<f32>>,
        dim: usize,
    ) -> anyhow::Result<RecordBatch> {
        let paths = StringArray::from_iter_values(chunks.iter().map(|c| c.doc_path.as_str()));
        let hashes = StringArray::from_iter_values(chunks.iter().map(|c| c.doc_hash.as_str()));
        let titles = StringArray::from_iter_values(chunks.iter().map(|c| c.title.as_str()));
        let dates = StringArray::from_iter_values(
            chunks
                .iter()
                .map(|c| c.get_date_string().unwrap_or_default()),
        );
        let texts = StringArray::from_iter_values(chunks.iter().map(|c| c.text.as_str()));
        let vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
            embeddings
                .into_iter()
                .map(|v| Some(v.into_iter().map(Some).collect::<Vec<_>>())),
            dim as i32,
        );

        RecordBatch::try_new(
            Self::schema(dim),
            vec![
                Arc::new(paths),
                Arc::new(hashes),
                Arc::new(titles),
                Arc::new(dates),
                Arc::new(texts),
                Arc::new(vectors),
            ],
        )
        .map_err(Into::into)
    }

    /// A `path IN ('a', 'b')` SQL predicate with single quotes escaped, for
    /// Lance's SQL-string filters.
    fn path_in_predicate(paths: &[String]) -> String {
        let list = paths
            .iter()
            .map(|p| format!("'{}'", p.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ");
        format!("path IN ({list})")
    }

    /// Pulls the string column `name` out of a batch (empty-string fallback if a
    /// row is null or the column is missing/of another type).
    fn strings(batch: &RecordBatch, name: &str) -> Vec<String> {
        match batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
        {
            Some(arr) => (0..arr.len())
                .map(|i| {
                    if arr.is_null(i) {
                        String::new()
                    } else {
                        arr.value(i).to_string()
                    }
                })
                .collect(),
            None => vec![String::new(); batch.num_rows()],
        }
    }

    fn chunk_from_parts(
        path: String,
        hash: String,
        title: String,
        date: String,
        text: String,
    ) -> FlattenedChunk {
        let date = if date.is_empty() {
            None
        } else {
            chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok()
        };
        FlattenedChunk {
            doc_path: path,
            doc_hash: hash,
            title,
            text,
            date,
        }
    }
}

#[async_trait::async_trait]
impl Embeddings for VecLance {
    async fn init(&self) -> anyhow::Result<()> {
        // Tables are per-vault and created lazily on first store; nothing to do
        // at server start.
        Ok(())
    }

    async fn store_embeddings(&self, collection: &str, content: &[KimunDoc]) -> anyhow::Result<()> {
        let (table, just_created) = self.open_or_create(collection).await?;
        let dim = self.embedder.dimension();

        // Upsert by path: a re-pushed doc replaces its old chunks instead of
        // accumulating stale ones. All of a path's chunks arrive together in
        // `content`, and index writes are serialized upstream, so deleting the
        // incoming paths before inserting is safe. (This is stricter than the
        // Qdrant backend, which appends — but it keeps `/hashes` deterministic.)
        // Skip it entirely for a table we just created: it is empty, and the
        // initial full-vault index would otherwise build a `path IN (…)` list
        // over every note to delete nothing.
        if !just_created {
            let mut paths: Vec<String> = content.iter().map(|d| d.path.clone()).collect();
            paths.sort();
            paths.dedup();
            if !paths.is_empty() {
                table
                    .delete(Self::path_in_predicate(&paths).as_str())
                    .await?;
            }
        }

        // Sub-split sections to the embedding window, then embed and store each
        // sub-chunk 1:1 — so a row's stored text is exactly the text that
        // produced its vector.
        let chunks = FlattenedChunk::from_chunks_split(content, 800, 1536);
        debug!("{} docs split to {} chunks", content.len(), chunks.len());
        const BATCH_SIZE: usize = 100;

        for batch in chunks.chunks(BATCH_SIZE) {
            let embeddings = self.embedder.generate_embeddings(batch).await?;
            if embeddings.len() != batch.len() {
                anyhow::bail!(
                    "embedder returned {} vectors for {} chunks",
                    embeddings.len(),
                    batch.len()
                );
            }
            let rb = Self::build_batch(batch, embeddings, dim)?;
            table.add(rb).execute().await?;
        }

        Ok(())
    }

    async fn delete_embeddings(&self, collection: &str, paths: Vec<&String>) -> anyhow::Result<()> {
        // A vault never indexed has no table; nothing to delete.
        if !self.table_exists(collection).await? {
            return Ok(());
        }
        let owned: Vec<String> = paths.into_iter().map(|p| p.to_owned()).collect();
        if owned.is_empty() {
            return Ok(());
        }
        let table = self.connection.open_table(collection).execute().await?;
        table
            .delete(Self::path_in_predicate(&owned).as_str())
            .await?;
        Ok(())
    }

    async fn query_embedding(
        &self,
        collection: &str,
        query: &str,
    ) -> anyhow::Result<Vec<(f64, FlattenedChunk)>> {
        if !self.table_exists(collection).await? {
            return Ok(Vec::new());
        }
        let table = self.connection.open_table(collection).execute().await?;
        let query_vec = self.embedder.prompt_embedding(query).await?;

        let batches: Vec<RecordBatch> = table
            .query()
            .limit(TOP_RESULTS)
            .nearest_to(query_vec)?
            .distance_type(DistanceType::Cosine)
            .execute()
            .await?
            .try_collect()
            .await?;

        let mut results: Vec<(f64, FlattenedChunk)> = Vec::new();
        for batch in &batches {
            let paths = Self::strings(batch, "path");
            let hashes = Self::strings(batch, "hash");
            let titles = Self::strings(batch, "title");
            let dates = Self::strings(batch, "date");
            let texts = Self::strings(batch, "text");
            let distances = batch
                .column_by_name("_distance")
                .and_then(|c| c.as_any().downcast_ref::<Float32Array>());

            for i in 0..batch.num_rows() {
                // Report a cosine *similarity* (higher = better), matching the
                // Qdrant backend's score semantics: similarity = 1 - distance.
                let score = distances.map(|d| 1.0 - d.value(i) as f64).unwrap_or(0.0);
                let chunk = Self::chunk_from_parts(
                    paths[i].clone(),
                    hashes[i].clone(),
                    titles[i].clone(),
                    dates[i].clone(),
                    texts[i].clone(),
                );
                results.push((score, chunk));
            }
        }

        debug!(
            "Query returned {} results (scores: {:.4} to {:.4})",
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
        // No table yet → empty set, so the client pushes everything (never error
        // on a missing collection mid-reconcile).
        if !self.table_exists(collection).await? {
            return Ok(HashMap::new());
        }
        let table = self.connection.open_table(collection).execute().await?;

        let batches: Vec<RecordBatch> = table
            .query()
            .select(Select::columns(&["path", "hash"]))
            .execute()
            .await?
            .try_collect()
            .await?;

        let mut result = HashMap::new();
        for batch in &batches {
            let paths = Self::strings(batch, "path");
            let hashes = Self::strings(batch, "hash");
            for (path, hash) in paths.into_iter().zip(hashes) {
                // Upsert-by-path keeps one hash per path, so this is deterministic.
                result.insert(
                    path.clone(),
                    IndexedNote {
                        path,
                        content_hash: hash,
                        last_indexed: 0,
                    },
                );
            }
        }

        debug!("Indexed: {}", result.len());
        Ok(result)
    }

    async fn remove_indexed_note(&self, collection: &str, path: &str) -> anyhow::Result<()> {
        // Lower-level hook (unused by the server): blank the hash so a reconcile
        // treats the note as changed, mirroring the Qdrant backend. Chunks stay.
        if !self.table_exists(collection).await? {
            return Ok(());
        }
        let table = self.connection.open_table(collection).execute().await?;
        table
            .update()
            .only_if(Self::path_in_predicate(&[path.to_string()]))
            .column("hash", "''")
            .execute()
            .await?;
        Ok(())
    }

    async fn list_collections(&self) -> anyhow::Result<Vec<CollectionInfo>> {
        let mut names = self.collection_names().await?;
        names.sort();
        let mut out = Vec::with_capacity(names.len());
        for vault in names {
            let note_count = self
                .get_indexed_notes(&vault)
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
        // Every table in the Lance directory is a vault collection.
        Ok(self.connection.table_names().execute().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::KimunSection;

    /// Deterministic fake: embeds text to a small non-zero vector so cosine
    /// search is well-defined without downloading a model.
    struct FakeEmbedder {
        dim: usize,
    }

    #[async_trait::async_trait]
    impl Embedder for FakeEmbedder {
        async fn generate_embeddings(
            &self,
            content: &[FlattenedChunk],
        ) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(content.iter().map(|c| self.embed(&c.text)).collect())
        }
        async fn prompt_embedding(&self, content: &str) -> anyhow::Result<Vec<f32>> {
            Ok(self.embed(content))
        }
        fn dimension(&self) -> usize {
            self.dim
        }
    }

    impl FakeEmbedder {
        fn embed(&self, text: &str) -> Vec<f32> {
            let mut v = vec![0.0f32; self.dim];
            v[0] = 1.0; // keep the vector non-zero (cosine needs a non-zero norm)
            for (i, b) in text.bytes().enumerate() {
                v[1 + (i % (self.dim - 1))] += b as f32;
            }
            v
        }
    }

    fn doc(path: &str, hash: &str, text: &str) -> KimunDoc {
        KimunDoc {
            path: path.to_string(),
            hash: hash.to_string(),
            sections: vec![KimunSection {
                title: "T".to_string(),
                text: text.to_string(),
            }],
        }
    }

    async fn store() -> (tempfile::TempDir, VecLance) {
        let dir = tempfile::tempdir().unwrap();
        let store = VecLance::new(dir.path(), Arc::new(FakeEmbedder { dim: 8 }))
            .await
            .unwrap();
        (dir, store)
    }

    #[tokio::test]
    async fn store_then_query_returns_a_stored_chunk() {
        let (_dir, store) = store().await;
        store
            .store_embeddings(
                "vault1",
                &[
                    doc("a.md", "h1", "the quick brown fox"),
                    doc("b.md", "h2", "lazy dog sleeps"),
                ],
            )
            .await
            .unwrap();

        let results = store
            .query_embedding("vault1", "quick brown fox")
            .await
            .unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().any(|(_, c)| c.doc_path == "a.md"));
    }

    #[tokio::test]
    async fn get_indexed_notes_reports_paths_and_hashes() {
        let (_dir, store) = store().await;
        store
            .store_embeddings(
                "v",
                &[doc("a.md", "h1", "alpha"), doc("b.md", "h2", "beta")],
            )
            .await
            .unwrap();

        let notes = store.get_indexed_notes("v").await.unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes.get("a.md").unwrap().content_hash, "h1");
        assert_eq!(notes.get("b.md").unwrap().content_hash, "h2");
    }

    #[tokio::test]
    async fn missing_collection_yields_empty_not_error() {
        let (_dir, store) = store().await;
        assert!(store.get_indexed_notes("nope").await.unwrap().is_empty());
        assert!(store.query_embedding("nope", "q").await.unwrap().is_empty());
        // Deleting from a missing collection is a no-op, not an error.
        store
            .delete_embeddings("nope", vec![&"x.md".to_string()])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn restore_replaces_stale_chunks_for_a_changed_doc() {
        let (_dir, store) = store().await;
        store
            .store_embeddings("v", &[doc("a.md", "h1", "first version")])
            .await
            .unwrap();
        // Re-push the same path with a new hash/content.
        store
            .store_embeddings("v", &[doc("a.md", "h2", "second version")])
            .await
            .unwrap();

        let notes = store.get_indexed_notes("v").await.unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(
            notes.get("a.md").unwrap().content_hash,
            "h2",
            "new hash replaces old"
        );
    }

    #[tokio::test]
    async fn delete_removes_the_note() {
        let (_dir, store) = store().await;
        store
            .store_embeddings(
                "v",
                &[doc("a.md", "h1", "alpha"), doc("b.md", "h2", "beta")],
            )
            .await
            .unwrap();
        store
            .delete_embeddings("v", vec![&"a.md".to_string()])
            .await
            .unwrap();

        let notes = store.get_indexed_notes("v").await.unwrap();
        assert_eq!(notes.len(), 1);
        assert!(notes.contains_key("b.md"));
        assert!(!notes.contains_key("a.md"));
    }

    #[tokio::test]
    async fn collection_names_lists_each_vault_table() {
        let (_dir, store) = store().await;
        store
            .store_embeddings("vault_a", &[doc("a.md", "h", "x")])
            .await
            .unwrap();
        store
            .store_embeddings("vault_b", &[doc("b.md", "h", "y")])
            .await
            .unwrap();

        let mut names = store.collection_names().await.unwrap();
        names.sort();
        assert_eq!(names, vec!["vault_a".to_string(), "vault_b".to_string()]);

        let infos = store.list_collections().await.unwrap();
        assert_eq!(infos.len(), 2);
        assert!(infos.iter().all(|i| i.note_count == 1));
    }
}
