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

    fn insert_vec(
        &self,
        collection: &str,
        docs: Vec<(&FlattenedChunk, &Vec<f32>)>,
    ) -> anyhow::Result<()> {
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        let doc_sql =
            "INSERT INTO docs (collection, path, title, date, text) VALUES (?1, ?2, ?3, ?4, ?5)";
        let vec_sql = "INSERT INTO vec_items(rowid, collection, embedding) VALUES (?1, ?2, ?3)";
        let index_sql = "INSERT OR REPLACE INTO indexed_notes (collection, path, content_hash, last_indexed) VALUES (?1, ?2, ?3, ?4)";
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;
        for (doc, vec) in docs {
            tx.execute(
                doc_sql,
                params![
                    collection,
                    doc.doc_path,
                    doc.title,
                    doc.date
                        .map(|date| date.format("%Y-%m-%d").to_string())
                        .unwrap_or_default(),
                    doc.text
                ],
            )?;
            let row_id = tx.last_insert_rowid();
            tx.execute(vec_sql, params![row_id, collection, vec.as_bytes()])?;
            tx.execute(
                index_sql,
                params![collection, doc.doc_path, doc.doc_hash, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Upserts the `indexed_notes` hash row for every note, independent of how
    /// many chunks it produced — so an empty note is still recorded and the
    /// reconcile hash set converges.
    fn mark_notes_indexed(&self, collection: &str, docs: &[KimunDoc]) -> anyhow::Result<()> {
        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;
        let sql = "INSERT OR REPLACE INTO indexed_notes (collection, path, content_hash, last_indexed) VALUES (?1, ?2, ?3, ?4)";
        for doc in docs {
            tx.execute(sql, params![collection, doc.path, doc.hash, now])?;
        }
        tx.commit()?;
        Ok(())
    }

    fn get_docs(
        &self,
        collection: &str,
        vec: &[f32],
    ) -> anyhow::Result<Vec<(f64, FlattenedChunk)>> {
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
          JOIN indexed_notes ON docs.collection = indexed_notes.collection AND docs.path = indexed_notes.path
          WHERE vec_items.collection = ?2
          AND vec_items.embedding MATCH ?1
          AND k = {TOP_RESULTS}
          ORDER BY vec_items.distance
        "
                )
                .as_str(),
            )?
            .query_map(params![vec.as_bytes(), collection], |r| {
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
                // Return a similarity (higher = better) so the score has the
                // same orientation as the Qdrant backend and downstream
                // dedup/rerank/take all agree. Rows already arrive ordered by
                // ascending distance (best first).
                let similarity = 1.0 / (1.0 + distance);
                Ok((similarity, kimun_chunk))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let duration = SystemTime::now()
            .duration_since(start)
            .unwrap_or_default()
            .as_millis();
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
        // `collection` gates the pre-multi-vault schema: an older store lacks it,
        // so every scoped insert/query would fail at runtime with "no such
        // column" — treat it as invalid and rebuild.
        if !docs_schema.contains("rowid")
            || !docs_schema.contains("collection")
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
        if !indexed_schema.contains("collection")
            || !indexed_schema.contains("path")
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
                "CREATE VIRTUAL TABLE vec_items USING vec0(collection text partition key, embedding float[{}])",
                self.embedder.dimension()
            ),
            [],
        )?;
        tx.execute(
            "CREATE TABLE docs (
            rowid INTEGER PRIMARY KEY,
            collection TEXT NOT NULL,
            path TEXT,
            title TEXT,
            date TEXT,
            text TEXT
        )",
            (), // empty list of parameters.
        )?;
        tx.execute(
            "CREATE INDEX docs_collection_path ON docs (collection, path)",
            (),
        )?;

        // Create index tracking table, keyed per collection so one vault's
        // hashes never collide with another's.
        tx.execute(
            "CREATE TABLE indexed_notes (
            collection TEXT NOT NULL,
            path TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            last_indexed INTEGER NOT NULL,
            PRIMARY KEY (collection, path)
        )",
            (),
        )?;

        tx.commit()?;

        Ok(())
    }

    async fn store_embeddings(&self, collection: &str, content: &[KimunDoc]) -> anyhow::Result<()> {
        // Sub-split sections to the embedding window (same as the qdrant backend)
        // so long sections aren't truncated by the model.
        let chunks = FlattenedChunk::from_chunks_split(content, 800, 1536);
        // Record every pushed note's hash even when it produced no chunks (empty
        // note), so the reconcile `/hashes` set includes it and the client
        // stops re-pushing it every pass.
        self.mark_notes_indexed(collection, content)?;
        if chunks.is_empty() {
            return Ok(());
        }
        let embeddings = self.embedder.generate_embeddings(&chunks).await?;
        if embeddings.len() != chunks.len() {
            anyhow::bail!(
                "embedder returned {} vectors for {} chunks; refusing to index a \
                 misaligned batch",
                embeddings.len(),
                chunks.len()
            );
        }
        let embed_chunks = embeddings.chunks(100);
        let mut i = 0;
        for batch in embed_chunks {
            let mut insert_batch = vec![];
            for c in batch {
                insert_batch.push((chunks.get(i).unwrap(), c));
                i += 1;
            }
            self.insert_vec(collection, insert_batch)?;
        }
        Ok(())
    }

    async fn delete_embeddings(&self, collection: &str, paths: Vec<&String>) -> anyhow::Result<()> {
        if paths.is_empty() {
            return Ok(());
        }

        let mut conn = self.connection()?;
        let tx = conn.transaction()?;

        // Process each path, scoped to the collection
        for path in paths {
            // First, get all rowids for docs with this path to delete from vec_items
            let rowids: Vec<i64> = tx
                .prepare("SELECT rowid FROM docs WHERE collection = ?1 AND path = ?2")?
                .query_map(params![collection, path.as_str()], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?;

            // Delete from vec_items for each rowid
            for rowid in rowids {
                tx.execute("DELETE FROM vec_items WHERE rowid = ?1", [rowid])?;
            }

            // Delete from docs table
            tx.execute(
                "DELETE FROM docs WHERE collection = ?1 AND path = ?2",
                params![collection, path.as_str()],
            )?;

            // Delete from indexed_notes table
            tx.execute(
                "DELETE FROM indexed_notes WHERE collection = ?1 AND path = ?2",
                params![collection, path.as_str()],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    async fn query_embedding(
        &self,
        collection: &str,
        query: &str,
    ) -> anyhow::Result<Vec<(f64, FlattenedChunk)>> {
        let query_embed = self.embedder.prompt_embedding(query).await?;
        self.get_docs(collection, &query_embed)
    }

    async fn get_indexed_notes(
        &self,
        collection: &str,
    ) -> anyhow::Result<std::collections::HashMap<String, crate::dbembeddings::IndexedNote>> {
        use std::collections::HashMap;

        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT path, content_hash, last_indexed FROM indexed_notes WHERE collection = ?1",
        )?;

        let notes = stmt
            .query_map(params![collection], |row| {
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

    async fn remove_indexed_note(&self, collection: &str, path: &str) -> anyhow::Result<()> {
        let conn = self.connection()?;
        conn.execute(
            "DELETE FROM indexed_notes WHERE collection = ?1 AND path = ?2",
            params![collection, path],
        )?;
        Ok(())
    }

    async fn list_collections(&self) -> anyhow::Result<Vec<crate::dbembeddings::CollectionInfo>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT collection, COUNT(*) FROM indexed_notes GROUP BY collection ORDER BY collection",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(crate::dbembeddings::CollectionInfo {
                    name: row.get::<_, String>(0)?,
                    note_count: row.get::<_, i64>(1)? as usize,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    async fn collection_names(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.connection()?;
        let mut stmt =
            conn.prepare("SELECT DISTINCT collection FROM indexed_notes ORDER BY collection")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
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

    /// Embedder that returns one more vector than it was given — simulates a
    /// misbehaving external endpoint.
    #[derive(Debug)]
    struct MiscountEmbedder;

    #[async_trait]
    impl Embedder for MiscountEmbedder {
        fn dimension(&self) -> usize {
            4
        }
        async fn generate_embeddings(
            &self,
            content: &[FlattenedChunk],
        ) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok((0..content.len() + 1).map(|_| vec![0.0; 4]).collect())
        }
        async fn prompt_embedding(&self, _content: &str) -> anyhow::Result<Vec<f32>> {
            Ok(vec![0.0; 4])
        }
    }

    #[tokio::test]
    async fn store_rejects_misaligned_embedding_count() {
        use crate::document::{KimunDoc, KimunSection};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rag_index.sqlite");
        let store = VecSQLite::new(&path, Arc::new(MiscountEmbedder));
        store.init().await.unwrap();

        let doc = KimunDoc {
            path: "n.md".to_string(),
            hash: "h".to_string(),
            sections: vec![KimunSection {
                title: "t".to_string(),
                text: "x".to_string(),
            }],
        };
        // Embedder returns 2 vectors for 1 chunk → must be rejected, not panic
        // or silently drop.
        assert!(store.store_embeddings("v", &[doc]).await.is_err());
    }

    fn doc(path: &str, hash: &str, text: &str) -> crate::document::KimunDoc {
        use crate::document::{KimunDoc, KimunSection};
        KimunDoc {
            path: path.to_string(),
            hash: hash.to_string(),
            sections: vec![KimunSection {
                title: "t".to_string(),
                text: text.to_string(),
            }],
        }
    }

    #[tokio::test]
    async fn collections_are_isolated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rag_index.sqlite");
        let store = VecSQLite::new(&path, Arc::new(FakeEmbedder { dim: 4 }));
        store.init().await.unwrap();

        store
            .store_embeddings("vaultA", &[doc("a.md", "ha", "alpha")])
            .await
            .unwrap();
        store
            .store_embeddings("vaultB", &[doc("b.md", "hb", "beta")])
            .await
            .unwrap();

        // Indexed-note hash sets are per collection.
        let a = store.get_indexed_notes("vaultA").await.unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a.get("a.md").unwrap().content_hash, "ha");
        let b = store.get_indexed_notes("vaultB").await.unwrap();
        assert_eq!(b.len(), 1);
        assert!(b.contains_key("b.md"));

        // Queries only see their own collection's chunks.
        let res_a = store.query_embedding("vaultA", "q").await.unwrap();
        assert!(!res_a.is_empty());
        assert!(res_a.iter().all(|(_, c)| c.doc_path == "a.md"));

        // Deleting in one collection leaves the other intact.
        store
            .delete_embeddings("vaultA", vec![&"a.md".to_string()])
            .await
            .unwrap();
        assert!(store.get_indexed_notes("vaultA").await.unwrap().is_empty());
        assert_eq!(store.get_indexed_notes("vaultB").await.unwrap().len(), 1);
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
