use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use chrono::NaiveDate;
use log::debug;
use rusqlite::{ffi::sqlite3_auto_extension, params};
use sqlite_vec::sqlite3_vec_init;
use zerocopy::IntoBytes;

use crate::document::{FlattenedChunk, KimunDoc};

use super::{Embeddings, embedder::Embedder};

const TOP_RESULTS: u32 = 512;

pub struct VecSQLite {
    embedder: Arc<dyn Embedder>,
    db_path: PathBuf,
}

impl VecSQLite {
    /// Creates a SQLite-backed vector store using `embedder` for both indexing
    /// and querying. The vec table is created at `embedder.dimension()`.
    pub fn new<P: AsRef<Path>>(path: P, embedder: Arc<dyn Embedder>) -> Self {
        unsafe {
            sqlite3_auto_extension(Some(std::mem::transmute(sqlite3_vec_init as *const ())));
        }

        let db_path = path.as_ref().to_path_buf();
        Self { embedder, db_path }
    }

    fn connection(&self) -> anyhow::Result<rusqlite::Connection> {
        let connection = rusqlite::Connection::open(&self.db_path)?;
        Ok(connection)
    }

    fn insert_vec(&self, docs: Vec<(&FlattenedChunk, &Vec<f32>)>) -> anyhow::Result<()> {
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        let doc_sql = "INSERT INTO docs (path, title, date, text) VALUES (?1, ?2, ?3, ?4)";
        let vec_sql = "INSERT INTO vec_items(rowid, embedding) VALUES (?1, ?2)";
        let index_sql = "INSERT OR REPLACE INTO indexed_notes (path, content_hash, last_indexed) VALUES (?1, ?2, ?3)";
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;
        for (doc, vec) in docs {
            tx.execute(
                doc_sql,
                params![
                    doc.doc_path,
                    doc.title,
                    doc.date
                        .map(|date| date.format("%Y-%m-%d").to_string())
                        .unwrap_or_default(),
                    doc.text
                ],
            )?;
            let row_id = tx.last_insert_rowid();
            tx.execute(vec_sql, params![row_id, vec.as_bytes()])?;
            tx.execute(
                index_sql,
                [doc.doc_path.clone(), doc.doc_hash.clone(), now.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn get_docs(&self, vec: &[f32]) -> anyhow::Result<Vec<(f64, FlattenedChunk)>> {
        let start = SystemTime::now();
        let conn = self.connection()?;
        let mut max_distance = f64::MIN;
        let mut min_distance = f64::MAX;
        let result: Vec<(f64, FlattenedChunk)> = conn
            .prepare(
                format!(
                    r"
          SELECT
            distance,
            docs.path,
            docs.title,
            docs.date,
            docs.text,
            indexed_notes.content_hash
          FROM vec_items
          JOIN docs ON vec_items.rowid = docs.rowid
          JOIN indexed_notes ON docs.path = indexed_notes.path
          WHERE vec_items.embedding MATCH ?1
          AND k = {TOP_RESULTS}
          ORDER BY vec_items.distance
        "
                )
                .as_str(),
            )?
            .query_map([vec.as_bytes()], |r| {
                let distance: f64 = r.get(0)?;
                if distance > max_distance {
                    max_distance = distance;
                }
                if distance < min_distance {
                    min_distance = distance;
                }
                let path: String = r.get(1)?;
                let title: String = r.get(2)?;
                let date: String = r.get(3)?;
                let text: String = r.get(4)?;
                let hash: String = r.get(5)?;
                let kimun_chunk = FlattenedChunk {
                    doc_path: path,
                    doc_hash: hash,
                    text,
                    title,
                    date: match NaiveDate::parse_from_str(date.as_str(), "%Y-%m-%d") {
                        Ok(d) => Some(d),
                        Err(_) => None,
                    },
                };
                Ok((distance, kimun_chunk))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let duration = SystemTime::now().duration_since(start).unwrap().as_millis();
        debug!(
            "Retrieved {} chunks in {} milliseconds",
            result.len(),
            duration
        );
        debug!("Min distance: {}", min_distance);
        debug!("Max distance: {}", max_distance);
        Ok(result)
    }

    fn validate_database(&self) -> anyhow::Result<bool> {
        // Try to open the database
        let conn = rusqlite::Connection::open(&self.db_path)?;

        // Check if all required tables exist with correct schema
        let tables_query = "SELECT name FROM sqlite_master WHERE type='table' AND name IN ('docs', 'indexed_notes')";
        let mut stmt = conn.prepare(tables_query)?;
        let table_count: usize = stmt.query_map([], |_| Ok(()))?.count();

        if table_count != 2 {
            return Ok(false);
        }

        // Check if vec_items virtual table exists
        let vec_table_query =
            "SELECT name FROM sqlite_master WHERE type='table' AND name='vec_items'";
        let mut vec_stmt = conn.prepare(vec_table_query)?;
        let vec_exists = vec_stmt.exists([])?;

        if !vec_exists {
            return Ok(false);
        }

        // Verify the stored vector width matches the current embedder. A change
        // (different embedder or model) means every stored vector is in a
        // different space and cannot be reused — treat the store as invalid so
        // it is recreated and re-embedded (via reconciliation).
        let vec_schema: String = conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='vec_items'",
            [],
            |row| row.get(0),
        )?;
        let expected_dim = format!("float[{}]", self.embedder.dimension());
        if !vec_schema.contains(&expected_dim) {
            debug!(
                "Vector width changed (have `{}`, expected `{}`); recreating store",
                vec_schema, expected_dim
            );
            return Ok(false);
        }

        // Verify docs table schema
        let docs_schema_query = "SELECT sql FROM sqlite_master WHERE type='table' AND name='docs'";
        let docs_schema: String = conn.query_row(docs_schema_query, [], |row| row.get(0))?;
        if !docs_schema.contains("rowid")
            || !docs_schema.contains("path")
            || !docs_schema.contains("title")
            || !docs_schema.contains("date")
            || !docs_schema.contains("text")
        {
            return Ok(false);
        }

        // Verify indexed_notes table schema
        let indexed_schema_query =
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='indexed_notes'";
        let indexed_schema: String = conn.query_row(indexed_schema_query, [], |row| row.get(0))?;
        if !indexed_schema.contains("path")
            || !indexed_schema.contains("content_hash")
            || !indexed_schema.contains("last_indexed")
        {
            return Ok(false);
        }

        Ok(true)
    }
}

#[async_trait::async_trait]
impl Embeddings for VecSQLite {
    async fn init(&self) -> anyhow::Result<()> {
        debug!("Checking the db_path at {}", self.db_path.to_string_lossy());

        // Check if file exists
        if self.db_path.exists() {
            // Validate it's a valid SQLite database with expected schema
            match self.validate_database() {
                Ok(true) => {
                    debug!("Database exists and is valid, skipping initialization");
                    return Ok(());
                }
                Ok(false) | Err(_) => {
                    debug!("Database exists but is invalid, recreating");
                    // Remove invalid database file/directory
                    if self.db_path.is_dir() {
                        std::fs::remove_dir_all(&self.db_path)?;
                    } else {
                        std::fs::remove_file(&self.db_path)?;
                    }
                }
            }
        } else {
            debug!("Database does not exist, creating new database");
        }

        // Create new database with schema
        debug!("Creating tables");
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        tx.execute(
            &format!(
                "CREATE VIRTUAL TABLE vec_items USING vec0(embedding float[{}])",
                self.embedder.dimension()
            ),
            [],
        )?;
        tx.execute(
            "CREATE TABLE docs (
            rowid INTEGER PRIMARY KEY,
            path TEXT,
            title TEXT,
            date TEXT,
            text TEXT
        )",
            (), // empty list of parameters.
        )?;

        // Create index tracking table
        tx.execute(
            "CREATE TABLE indexed_notes (
            path TEXT PRIMARY KEY,
            content_hash TEXT NOT NULL,
            last_indexed INTEGER NOT NULL
        )",
            (),
        )?;

        tx.commit()?;

        Ok(())
    }

    async fn store_embeddings(&self, content: &[KimunDoc]) -> anyhow::Result<()> {
        let chunks = FlattenedChunk::from_chunks(content);
        let embeddings = self.embedder.generate_embeddings(&chunks).await?;
        let embed_chunks = embeddings.chunks(100);
        let mut i = 0;
        for batch in embed_chunks {
            let mut insert_batch = vec![];
            for c in batch {
                insert_batch.push((chunks.get(i).unwrap(), c));
                i += 1;
            }
            self.insert_vec(insert_batch)?;
        }
        Ok(())
    }

    async fn delete_embeddings(&self, paths: Vec<&String>) -> anyhow::Result<()> {
        if paths.is_empty() {
            return Ok(());
        }

        let mut conn = self.connection()?;
        let tx = conn.transaction()?;

        // Process each path
        for path in paths {
            // First, get all rowids for docs with this path to delete from vec_items
            let rowids: Vec<i64> = tx
                .prepare("SELECT rowid FROM docs WHERE path = ?1")?
                .query_map([path.as_str()], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?;

            // Delete from vec_items for each rowid
            for rowid in rowids {
                tx.execute("DELETE FROM vec_items WHERE rowid = ?1", [rowid])?;
            }

            // Delete from docs table
            tx.execute("DELETE FROM docs WHERE path = ?1", [path.as_str()])?;

            // Delete from indexed_notes table
            tx.execute("DELETE FROM indexed_notes WHERE path = ?1", [path.as_str()])?;
        }

        tx.commit()?;
        Ok(())
    }

    async fn query_embedding(&self, query: &str) -> anyhow::Result<Vec<(f64, FlattenedChunk)>> {
        let query_embed = self.embedder.prompt_embedding(query).await?;
        self.get_docs(&query_embed)
    }

    async fn get_indexed_notes(
        &self,
    ) -> anyhow::Result<std::collections::HashMap<String, crate::dbembeddings::IndexedNote>> {
        use std::collections::HashMap;

        let conn = self.connection()?;
        let mut stmt =
            conn.prepare("SELECT path, content_hash, last_indexed FROM indexed_notes")?;

        let notes = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    crate::dbembeddings::IndexedNote {
                        path: row.get(0)?,
                        content_hash: row.get(1)?,
                        last_indexed: row.get(2)?,
                    },
                ))
            })?
            .collect::<Result<HashMap<String, crate::dbembeddings::IndexedNote>, _>>()?;

        Ok(notes)
    }

    async fn remove_indexed_note(&self, path: &str) -> anyhow::Result<()> {
        let conn = self.connection()?;
        conn.execute("DELETE FROM indexed_notes WHERE path = ?1", [path])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::FlattenedChunk;
    use async_trait::async_trait;

    /// Embedder that emits fixed-width zero vectors — no model download, so the
    /// SQLite layer is testable in isolation.
    #[derive(Debug)]
    struct FakeEmbedder {
        dim: usize,
    }

    #[async_trait]
    impl Embedder for FakeEmbedder {
        fn dimension(&self) -> usize {
            self.dim
        }
        async fn generate_embeddings(
            &self,
            content: &[FlattenedChunk],
        ) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(content.iter().map(|_| vec![0.0; self.dim]).collect())
        }
        async fn prompt_embedding(&self, _content: &str) -> anyhow::Result<Vec<f32>> {
            Ok(vec![0.0; self.dim])
        }
    }

    #[tokio::test]
    async fn reinit_recreates_when_embedder_dimension_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rag_index.sqlite");

        VecSQLite::new(&path, Arc::new(FakeEmbedder { dim: 4 }))
            .init()
            .await
            .unwrap();

        // Reopening with a different embedder width must recreate the vec table
        // rather than leave a store that would reject the new vectors.
        VecSQLite::new(&path, Arc::new(FakeEmbedder { dim: 8 }))
            .init()
            .await
            .unwrap();

        let conn = rusqlite::Connection::open(&path).unwrap();
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'vec_items'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(sql.contains("float[8]"), "vec table not recreated: {sql}");
    }
}

// Compute SHA256 hash of content for change detection
// pub fn compute_content_hash(content: &str) -> String {
//     use sha2::{Digest, Sha256};
//     let mut hasher = Sha256::new();
//     hasher.update(content.as_bytes());
//     format!("{:x}", hasher.finalize())
// }
