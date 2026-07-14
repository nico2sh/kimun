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

use super::{CollectionInfo, EmbeddedChunk, IndexedNote, VectorStore};
use crate::document::FlattenedChunk;

pub struct VecLance {
    /// Vector width every table is created at — the embedder's dimension,
    /// fixed at composition time.
    dim: usize,
    connection: Connection,
}

impl VecLance {
    /// Opens (creating the directory if needed) the Lance database rooted at
    /// `path`. Each vault becomes a table inside it, at vector width `dim`.
    pub async fn new(path: impl AsRef<Path>, dim: usize) -> anyhow::Result<Self> {
        let uri = path.as_ref().to_string_lossy().to_string();
        let connection = connect(&uri).execute().await?;
        Ok(Self { dim, connection })
    }

    /// The Arrow schema for a vault table at the store's vector width. Column
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

    /// Opens the vault's table, creating it empty (at the store's dimension)
    /// if absent. Fails loudly if an existing table's vector width differs —
    /// the embedder/model changed and the operator must drop the table and
    /// re-index (adr: dimension change is destructive).
    async fn open_or_create(&self, collection: &str) -> anyhow::Result<Table> {
        if self.table_exists(collection).await? {
            let table = self.connection.open_table(collection).execute().await?;
            if let Some(existing) = Self::vector_dim(table.schema().await?.as_ref())
                && existing != self.dim
            {
                anyhow::bail!(
                    "LanceDB table `{collection}` has dimension {existing} but the \
                     embedder produces {}. The embedder or model changed; drop \
                     the table and re-index.",
                    self.dim
                );
            }
            Ok(table)
        } else {
            let table = self
                .connection
                .create_empty_table(collection, Self::schema(self.dim))
                .execute()
                .await?;
            debug!("Created LanceDB table: {collection}");
            Ok(table)
        }
    }

    /// The declared width of the `vector` column in an existing table's schema.
    fn vector_dim(schema: &Schema) -> Option<usize> {
        match schema.field_with_name("vector").ok()?.data_type() {
            DataType::FixedSizeList(_, n) => Some(*n as usize),
            _ => None,
        }
    }

    fn build_batch(rows: &[EmbeddedChunk], dim: usize) -> anyhow::Result<RecordBatch> {
        let paths =
            StringArray::from_iter_values(rows.iter().map(|r| r.chunk.doc_path.as_str()));
        let hashes =
            StringArray::from_iter_values(rows.iter().map(|r| r.chunk.doc_hash.as_str()));
        let titles = StringArray::from_iter_values(rows.iter().map(|r| r.chunk.title.as_str()));
        let dates = StringArray::from_iter_values(
            rows.iter()
                .map(|r| r.chunk.get_date_string().unwrap_or_default()),
        );
        let texts = StringArray::from_iter_values(rows.iter().map(|r| r.chunk.text.as_str()));
        let vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
            rows.iter()
                .map(|r| Some(r.vector.iter().copied().map(Some).collect::<Vec<_>>())),
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
impl VectorStore for VecLance {
    async fn store(&self, collection: &str, rows: &[EmbeddedChunk]) -> anyhow::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let table = self.open_or_create(collection).await?;
        let rb = Self::build_batch(rows, self.dim)?;
        table.add(rb).execute().await?;
        Ok(())
    }

    async fn delete(&self, collection: &str, paths: &[String]) -> anyhow::Result<()> {
        // A vault never indexed has no table; nothing to delete.
        if paths.is_empty() || !self.table_exists(collection).await? {
            return Ok(());
        }
        let table = self.connection.open_table(collection).execute().await?;
        table
            .delete(Self::path_in_predicate(paths).as_str())
            .await?;
        Ok(())
    }

    async fn query(
        &self,
        collection: &str,
        vector: Vec<f32>,
        limit: usize,
    ) -> anyhow::Result<Vec<(f64, FlattenedChunk)>> {
        if !self.table_exists(collection).await? {
            return Ok(Vec::new());
        }
        let table = self.connection.open_table(collection).execute().await?;

        let batches: Vec<RecordBatch> = table
            .query()
            .limit(limit)
            .nearest_to(vector)?
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

    async fn indexed_notes(
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
                // The pipeline deletes a note's paths before re-storing, so all
                // of a path's rows carry one hash and this is deterministic.
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

    async fn list_collections(&self) -> anyhow::Result<Vec<CollectionInfo>> {
        let mut names = self.collection_names().await?;
        names.sort();
        let mut out = Vec::with_capacity(names.len());
        for vault in names {
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
        // Every table in the Lance directory is a vault collection.
        Ok(self.connection.table_names().execute().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dbembeddings::conformance;

    async fn store() -> (tempfile::TempDir, VecLance) {
        let dir = tempfile::tempdir().unwrap();
        let store = VecLance::new(dir.path(), conformance::DIM).await.unwrap();
        (dir, store)
    }

    #[tokio::test]
    async fn conformance_store_then_query() {
        let (_dir, s) = store().await;
        conformance::store_then_query_finds_the_chunk(&s, "v").await;
    }

    #[tokio::test]
    async fn conformance_query_limit() {
        let (_dir, s) = store().await;
        conformance::query_respects_limit(&s, "v").await;
    }

    #[tokio::test]
    async fn conformance_missing_collection() {
        let (_dir, s) = store().await;
        conformance::missing_collection_is_empty_not_error(&s, "nope").await;
    }

    #[tokio::test]
    async fn conformance_delete() {
        let (_dir, s) = store().await;
        conformance::delete_removes_every_chunk_of_the_note(&s, "v").await;
    }

    #[tokio::test]
    async fn conformance_indexed_notes() {
        let (_dir, s) = store().await;
        conformance::indexed_notes_reports_one_hash_per_path(&s, "v").await;
    }

    #[tokio::test]
    async fn conformance_collections() {
        let (_dir, s) = store().await;
        conformance::collections_list_each_vault(&s, "vault_a", "vault_b").await;
    }

    #[tokio::test]
    async fn conformance_round_trip() {
        let (_dir, s) = store().await;
        conformance::stored_chunk_round_trips_its_fields(&s, "v").await;
    }

    #[tokio::test]
    async fn dimension_mismatch_fails_loudly() {
        let dir = tempfile::tempdir().unwrap();
        let s8 = VecLance::new(dir.path(), 8).await.unwrap();
        s8.store("v", &[conformance::row("a.md", "h", "x")])
            .await
            .unwrap();

        // Reopen the same directory at another width: writes must be refused.
        let s16 = VecLance::new(dir.path(), 16).await.unwrap();
        let row = EmbeddedChunk {
            chunk: conformance::row("b.md", "h", "y").chunk,
            vector: vec![0.5; 16],
        };
        let err = s16.store("v", &[row]).await.expect_err("dim change must fail");
        assert!(err.to_string().contains("dimension"));
    }
}
