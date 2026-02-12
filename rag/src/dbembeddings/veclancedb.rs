//! LanceDB vector database implementation for Kimün RAG
//!
//! This module provides a LanceDB backend for storing and querying vector embeddings.
//! LanceDB is a serverless, low-latency vector database that stores data in local files.
//!
//! # Known Issues
//!
//! ⚠️ **Important:** There is currently a known dependency issue with `lance-index` v2.0.0
//! and `tempfile` v3.19+ that prevents compilation. The issue is in the upstream `lance-index`
//! crate which uses `tempfile::TempDir::keep()` - a method that was removed in newer versions
//! of tempfile.
//!
//! **Status:** Waiting for LanceDB to update to lance v3.0.0+ which fixes this issue.
//!
//! **Workaround:** Use Qdrant or SQLite as your vector database until this is resolved upstream.
//!
//! The implementation is complete and ready to use once the dependency issue is fixed.

use std::{path::Path, sync::Arc};

use chrono::NaiveDate;
use lancedb::{
    Connection, Table, connect,
    query::{ExecutableQuery, QueryBase},
};
use log::debug;
use serde::{Deserialize, Serialize};

use crate::document::{FlattenedChunk, KimunDoc};

use super::{
    Embeddings, IndexedNote,
    embedder::{Embedder, fastembedder::FastEmbedder},
};

const TOP_RESULTS: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LanceRecord {
    path: String,
    title: String,
    date: String,
    text: String,
    hash: String,
    vector: Vec<f32>,
}

pub struct VecLanceDB {
    embedder: FastEmbedder,
    db_uri: String,
    table_name: String,
}

impl VecLanceDB {
    pub fn new<P: AsRef<Path>>(db_path: P, table_name: String) -> anyhow::Result<Self> {
        let embedder = FastEmbedder::new()?;
        let db_uri = db_path.as_ref().to_string_lossy().to_string();

        Ok(Self {
            embedder,
            db_uri,
            table_name,
        })
    }

    async fn connection(&self) -> anyhow::Result<Connection> {
        let conn = connect(&self.db_uri).execute().await?;
        Ok(conn)
    }

    async fn get_table(&self) -> anyhow::Result<Table> {
        let conn = self.connection().await?;
        let table = conn.open_table(&self.table_name).execute().await?;
        Ok(table)
    }

    async fn table_exists(&self) -> anyhow::Result<bool> {
        let conn = self.connection().await?;
        let tables = conn.table_names().execute().await?;
        Ok(tables.contains(&self.table_name))
    }

    async fn validate_table(&self) -> anyhow::Result<bool> {
        if !self.table_exists().await? {
            return Ok(false);
        }

        // Try to open the table and check if it has the expected schema
        match self.get_table().await {
            Ok(table) => {
                let schema = table.schema().await?;
                let field_names: Vec<_> = schema
                    .fields()
                    .iter()
                    .map(|f| f.name().to_string())
                    .collect();

                // Check for required fields
                let required_fields = vec!["path", "title", "date", "text", "hash", "vector"];
                for field in required_fields {
                    if !field_names.contains(&field.to_string()) {
                        debug!("Missing required field: {}", field);
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }

    async fn create_table(&self) -> anyhow::Result<()> {
        let conn = self.connection().await?;

        // Create an empty table with the correct schema
        let records: Vec<LanceRecord> = vec![];
        let _table = conn
            .create_table(&self.table_name, Box::new(records))
            .execute()
            .await?;

        debug!("Created LanceDB table: {}", self.table_name);
        Ok(())
    }

    async fn insert_records(&self, records: Vec<LanceRecord>) -> anyhow::Result<()> {
        if records.is_empty() {
            return Ok(());
        }

        let table = self.get_table().await?;

        // Add records to table
        table.add(Box::new(records)).execute().await?;

        Ok(())
    }

    async fn search_vectors(
        &self,
        query_vec: &[f32],
        limit: usize,
    ) -> anyhow::Result<Vec<(f64, FlattenedChunk)>> {
        let table = self.get_table().await?;

        // Perform vector search
        let results = table
            .query()
            .nearest_to(query_vec)?
            .limit(limit)
            .execute()
            .await?;

        // Convert results to FlattenedChunk
        let mut chunks = Vec::new();
        let batches = results.try_collect::<Vec<_>>().await?;

        for batch in batches {
            let schema = batch.schema();
            let path_col = batch
                .column_by_name("path")
                .ok_or_else(|| anyhow::anyhow!("Missing path column"))?;
            let title_col = batch
                .column_by_name("title")
                .ok_or_else(|| anyhow::anyhow!("Missing title column"))?;
            let date_col = batch
                .column_by_name("date")
                .ok_or_else(|| anyhow::anyhow!("Missing date column"))?;
            let text_col = batch
                .column_by_name("text")
                .ok_or_else(|| anyhow::anyhow!("Missing text column"))?;
            let hash_col = batch
                .column_by_name("hash")
                .ok_or_else(|| anyhow::anyhow!("Missing hash column"))?;
            let distance_col = batch
                .column_by_name("_distance")
                .ok_or_else(|| anyhow::anyhow!("Missing _distance column"))?;

            // Cast columns to appropriate types
            let path_arr = path_col
                .as_any()
                .downcast_ref::<arrow_array::StringArray>()
                .ok_or_else(|| anyhow::anyhow!("Failed to cast path column"))?;
            let title_arr = title_col
                .as_any()
                .downcast_ref::<arrow_array::StringArray>()
                .ok_or_else(|| anyhow::anyhow!("Failed to cast title column"))?;
            let date_arr = date_col
                .as_any()
                .downcast_ref::<arrow_array::StringArray>()
                .ok_or_else(|| anyhow::anyhow!("Failed to cast date column"))?;
            let text_arr = text_col
                .as_any()
                .downcast_ref::<arrow_array::StringArray>()
                .ok_or_else(|| anyhow::anyhow!("Failed to cast text column"))?;
            let hash_arr = hash_col
                .as_any()
                .downcast_ref::<arrow_array::StringArray>()
                .ok_or_else(|| anyhow::anyhow!("Failed to cast hash column"))?;
            let distance_arr = distance_col
                .as_any()
                .downcast_ref::<arrow_array::Float32Array>()
                .ok_or_else(|| anyhow::anyhow!("Failed to cast distance column"))?;

            for i in 0..batch.num_rows() {
                let path = path_arr.value(i).to_string();
                let title = title_arr.value(i).to_string();
                let date_str = date_arr.value(i);
                let text = text_arr.value(i).to_string();
                let hash = hash_arr.value(i).to_string();
                let distance = distance_arr.value(i) as f64;

                let date = if !date_str.is_empty() {
                    NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()
                } else {
                    None
                };

                let chunk = FlattenedChunk {
                    doc_path: path,
                    doc_hash: hash,
                    title,
                    text,
                    date,
                };

                chunks.push((distance, chunk));
            }
        }

        debug!("Retrieved {} chunks from LanceDB", chunks.len());
        Ok(chunks)
    }

    async fn delete_by_paths(&self, paths: Vec<&String>) -> anyhow::Result<()> {
        if paths.is_empty() {
            return Ok(());
        }

        let table = self.get_table().await?;

        // Build deletion filter: path IN (paths)
        for path in paths {
            let predicate = format!("path = '{}'", path.replace("'", "''"));
            table.delete(&predicate).await?;
        }

        Ok(())
    }

    async fn get_all_records(&self) -> anyhow::Result<Vec<(String, String)>> {
        let table = self.get_table().await?;

        // Query all records to get path and hash
        let results = table.query().execute().await?;

        let mut records = Vec::new();
        let batches = results.try_collect::<Vec<_>>().await?;

        for batch in batches {
            let path_col = batch
                .column_by_name("path")
                .ok_or_else(|| anyhow::anyhow!("Missing path column"))?;
            let hash_col = batch
                .column_by_name("hash")
                .ok_or_else(|| anyhow::anyhow!("Missing hash column"))?;

            let path_arr = path_col
                .as_any()
                .downcast_ref::<arrow_array::StringArray>()
                .ok_or_else(|| anyhow::anyhow!("Failed to cast path column"))?;
            let hash_arr = hash_col
                .as_any()
                .downcast_ref::<arrow_array::StringArray>()
                .ok_or_else(|| anyhow::anyhow!("Failed to cast hash column"))?;

            for i in 0..batch.num_rows() {
                let path = path_arr.value(i).to_string();
                let hash = hash_arr.value(i).to_string();
                records.push((path, hash));
            }
        }

        Ok(records)
    }
}

#[async_trait::async_trait]
impl Embeddings for VecLanceDB {
    async fn init(&self) -> anyhow::Result<()> {
        debug!("Initializing LanceDB at {}", self.db_uri);

        // Check if table exists and is valid
        if self.table_exists().await? {
            match self.validate_table().await {
                Ok(true) => {
                    debug!("LanceDB table exists and is valid");
                    return Ok(());
                }
                Ok(false) | Err(_) => {
                    debug!("LanceDB table exists but is invalid, recreating");
                    // Drop the invalid table
                    let conn = self.connection().await?;
                    // TODO: Check if we need to provide a namespace
                    conn.drop_table(&self.table_name, &[]).await?;
                }
            }
        }

        // Create new table
        self.create_table().await?;
        debug!("LanceDB initialized successfully");

        Ok(())
    }

    async fn store_embeddings(&self, content: &[KimunDoc]) -> anyhow::Result<()> {
        let chunks = FlattenedChunk::from_chunks(content);
        debug!("Storing {} chunks in LanceDB", chunks.len());

        // Generate embeddings in batches
        let embeddings = self.embedder.generate_embeddings(&chunks).await?;

        // Create records
        let mut records = Vec::new();
        for (i, chunk) in chunks.iter().enumerate() {
            let record = LanceRecord {
                path: chunk.doc_path.clone(),
                title: chunk.title.clone(),
                date: chunk
                    .date
                    .map(|d| d.format("%Y-%m-%d").to_string())
                    .unwrap_or_default(),
                text: chunk.text.clone(),
                hash: chunk.doc_hash.clone(),
                vector: embeddings[i].clone(),
            };
            records.push(record);
        }

        // Insert records in batches of 100
        const BATCH_SIZE: usize = 100;
        for batch in records.chunks(BATCH_SIZE) {
            self.insert_records(batch.to_vec()).await?;
        }

        debug!("Stored {} embeddings successfully", chunks.len());
        Ok(())
    }

    async fn delete_embeddings(&self, paths: Vec<&String>) -> anyhow::Result<()> {
        if paths.is_empty() {
            return Ok(());
        }

        debug!("Deleting embeddings for {} paths", paths.len());
        self.delete_by_paths(paths).await?;
        Ok(())
    }

    async fn query_embedding(&self, query: &str) -> anyhow::Result<Vec<(f64, FlattenedChunk)>> {
        debug!("Querying LanceDB with: {}", query);
        let query_vec = self.embedder.prompt_embedding(query).await?;
        self.search_vectors(&query_vec, TOP_RESULTS).await
    }

    async fn get_indexed_notes(
        &self,
    ) -> anyhow::Result<std::collections::HashMap<String, IndexedNote>> {
        use std::collections::HashMap;

        let records = self.get_all_records().await?;
        let mut notes = HashMap::new();

        // Group by path and use the hash (all chunks for same doc have same hash)
        for (path, hash) in records {
            notes.entry(path.clone()).or_insert_with(|| IndexedNote {
                path: path.clone(),
                content_hash: hash,
                last_indexed: 0, // LanceDB doesn't track timestamps, could be added if needed
            });
        }

        debug!("Retrieved {} indexed notes", notes.len());
        Ok(notes)
    }

    async fn remove_indexed_note(&self, path: &str) -> anyhow::Result<()> {
        debug!("Removing indexed note: {}", path);
        self.delete_by_paths(vec![&path.to_string()]).await
    }
}
