//! Embedded, file-backed vector store built on SQLite (via `sqlx`, the same
//! driver kimun_core's index uses).
//!
//! Unlike Qdrant this needs no server: the whole store is a single
//! `embeddings.db` file inside a local directory. Rows live in one `chunks`
//! table, scoped by a `collections` table — one **collection per vault**
//! (keyed by the vault id, adr/0020) — holding every chunk's embedding blob
//! plus its metadata columns.
//!
//! There is deliberately no ANN index. Vaults hold thousands to tens of
//! thousands of chunks; an exhaustive scan at that scale is a few dozen
//! million multiply-adds — well under a millisecond of CPU — so `query`
//! scans every row and computes the score in Rust. Embeddings are
//! L2-normalized at write time, which reduces cosine similarity to a plain
//! dot product at query time and gives exact (not approximate) top-k.
//!
//! The embedder fingerprint (adr/0025) lives in a `meta` key/value table in
//! the same file, so it survives `drop_all_collections` like VecQdrant's
//! metadata collection does.

use std::collections::HashMap;
use std::path::Path;

use log::debug;
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};

use super::{CollectionInfo, EmbeddedChunk, IndexedNote, VectorStore};
use crate::document::FlattenedChunk;

/// The database file inside the store directory.
const DB_FILE: &str = "embeddings.db";
const FINGERPRINT_KEY: &str = "embedder.fingerprint";

pub struct VecSqlite {
    /// Vector width every collection is created at — the embedder's dimension,
    /// fixed at composition time.
    dim: usize,
    pool: SqlitePool,
}

impl VecSqlite {
    /// Opens (creating the directory and database if needed) the SQLite store
    /// rooted at `path`. Each vault becomes a collection inside it, at vector
    /// width `dim`.
    pub async fn new(path: impl AsRef<Path>, dim: usize) -> anyhow::Result<Self> {
        std::fs::create_dir_all(path.as_ref())?;
        let options = SqliteConnectOptions::new()
            .filename(path.as_ref().join(DB_FILE))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        // A single connection: SQLite has one writer anyway, and every query
        // here is sub-millisecond, so pooling buys nothing but lock traffic.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        sqlx::raw_sql(
            "CREATE TABLE IF NOT EXISTS collections (
                 id        INTEGER PRIMARY KEY,
                 name      TEXT NOT NULL UNIQUE,
                 dimension INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS chunks (
                 id            INTEGER PRIMARY KEY,
                 collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
                 path          TEXT NOT NULL,
                 hash          TEXT NOT NULL,
                 title         TEXT,
                 date          TEXT,
                 text          TEXT NOT NULL,
                 embedding     BLOB NOT NULL
             );
             CREATE INDEX IF NOT EXISTS chunks_by_path ON chunks(collection_id, path);
             CREATE TABLE IF NOT EXISTS meta (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )
        .execute(&pool)
        .await?;
        Ok(Self { dim, pool })
    }

    /// The collection's `(id, dimension)`, or `None` if it was never created.
    async fn collection<'e, E>(executor: E, name: &str) -> anyhow::Result<Option<(i64, usize)>>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let row = sqlx::query("SELECT id, dimension FROM collections WHERE name = ?")
            .bind(name)
            .fetch_optional(executor)
            .await?;
        Ok(row.map(|r| (r.get::<i64, _>(0), r.get::<i64, _>(1) as usize)))
    }

    /// The embedding as little-endian `f32` bytes, L2-normalized so cosine
    /// similarity at query time is a plain dot product. A zero vector (norm 0)
    /// is stored as-is; it scores 0 against everything.
    fn to_blob(vector: &[f32]) -> Vec<u8> {
        let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        let scale = if norm > 0.0 { 1.0 / norm } else { 1.0 };
        vector
            .iter()
            .flat_map(|x| (x * scale).to_le_bytes())
            .collect()
    }

    /// The dot product of the (normalized) stored blob against the
    /// (normalized) query — their cosine similarity, higher = better.
    fn score(blob: &[u8], query: &[f32]) -> f32 {
        blob.chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .zip(query)
            .map(|(a, b)| a * b)
            .sum()
    }

    /// A `?, ?, …` placeholder list of length `n`.
    fn placeholders(n: usize) -> String {
        vec!["?"; n].join(", ")
    }

    fn chunk_from_parts(
        path: String,
        hash: String,
        title: Option<String>,
        date: Option<String>,
        text: String,
    ) -> FlattenedChunk {
        FlattenedChunk {
            doc_path: path,
            doc_hash: hash,
            title: title.unwrap_or_default(),
            text,
            date: date.and_then(|d| chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok()),
        }
    }
}

#[async_trait::async_trait]
impl VectorStore for VecSqlite {
    async fn store(&self, collection: &str, rows: &[EmbeddedChunk]) -> anyhow::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        // Open the collection, creating it (at the store's dimension) if
        // absent. Fails loudly if an existing collection's vector width
        // differs — the embedder/model changed and the operator must drop the
        // collection and re-index (adr: dimension change is destructive).
        let cid = match Self::collection(&mut *tx, collection).await? {
            Some((_, existing)) if existing != self.dim => {
                anyhow::bail!(
                    "SQLite collection `{collection}` has dimension {existing} but the \
                     embedder produces {}. The embedder or model changed; drop \
                     the collection and re-index.",
                    self.dim
                );
            }
            Some((id, _)) => id,
            None => {
                let result = sqlx::query("INSERT INTO collections (name, dimension) VALUES (?, ?)")
                    .bind(collection)
                    .bind(self.dim as i64)
                    .execute(&mut *tx)
                    .await?;
                debug!("Created SQLite collection: {collection}");
                result.last_insert_rowid()
            }
        };
        for row in rows {
            if row.vector.len() != self.dim {
                anyhow::bail!(
                    "Chunk of `{}` has dimension {} but the store is at {}",
                    row.chunk.doc_path,
                    row.vector.len(),
                    self.dim
                );
            }
            sqlx::query(
                "INSERT INTO chunks (collection_id, path, hash, title, date, text, embedding)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(cid)
            .bind(&row.chunk.doc_path)
            .bind(&row.chunk.doc_hash)
            .bind(&row.chunk.title)
            .bind(row.chunk.get_date_string())
            .bind(&row.chunk.text)
            .bind(Self::to_blob(&row.vector))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn delete(&self, collection: &str, paths: &[String]) -> anyhow::Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        // A vault never indexed has no collection; nothing to delete.
        let Some((cid, _)) = Self::collection(&self.pool, collection).await? else {
            return Ok(());
        };
        let sql = format!(
            "DELETE FROM chunks WHERE collection_id = ? AND path IN ({})",
            Self::placeholders(paths.len())
        );
        let mut query = sqlx::query(&sql).bind(cid);
        for path in paths {
            query = query.bind(path);
        }
        query.execute(&self.pool).await?;
        Ok(())
    }

    async fn query(
        &self,
        collection: &str,
        vector: Vec<f32>,
        limit: usize,
    ) -> anyhow::Result<Vec<(f64, FlattenedChunk)>> {
        let Some((cid, _)) = Self::collection(&self.pool, collection).await? else {
            return Ok(Vec::new());
        };
        if vector.len() != self.dim {
            anyhow::bail!(
                "Query vector has dimension {} but the store is at {}",
                vector.len(),
                self.dim
            );
        }
        // Same trick as at write time: a normalized query makes the score the
        // cosine similarity, matching the Qdrant backend's semantics.
        let query = {
            let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
            let scale = if norm > 0.0 { 1.0 / norm } else { 1.0 };
            vector.iter().map(|x| x * scale).collect::<Vec<_>>()
        };

        // Pass 1: exhaustive scan over the embedding blobs only — top-k ids.
        let mut scored: Vec<(f32, i64)> =
            sqlx::query("SELECT id, embedding FROM chunks WHERE collection_id = ?")
                .bind(cid)
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .map(|r| {
                    let blob: Vec<u8> = r.get(1);
                    (Self::score(&blob, &query), r.get::<i64, _>(0))
                })
                .collect();
        scored.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
        scored.truncate(limit);

        // Pass 2: fetch the winners' metadata, then emit best-first.
        let ids: Vec<i64> = scored.iter().map(|(_, id)| *id).collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT id, path, hash, title, date, text FROM chunks WHERE id IN ({})",
            Self::placeholders(ids.len())
        );
        let mut fetch = sqlx::query(&sql);
        for id in &ids {
            fetch = fetch.bind(id);
        }
        let mut by_id: HashMap<i64, FlattenedChunk> = fetch
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|r| {
                (
                    r.get::<i64, _>(0),
                    Self::chunk_from_parts(r.get(1), r.get(2), r.get(3), r.get(4), r.get(5)),
                )
            })
            .collect();

        let results: Vec<(f64, FlattenedChunk)> = scored
            .into_iter()
            .filter_map(|(score, id)| by_id.remove(&id).map(|c| (score as f64, c)))
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
        // No collection yet → empty set, so the client pushes everything (never
        // error on a missing collection mid-reconcile).
        let Some((cid, _)) = Self::collection(&self.pool, collection).await? else {
            return Ok(HashMap::new());
        };
        // The pipeline deletes a note's paths before re-storing, so all of a
        // path's rows carry one hash and DISTINCT collapses to one row per note.
        let result: HashMap<String, IndexedNote> =
            sqlx::query("SELECT DISTINCT path, hash FROM chunks WHERE collection_id = ?")
                .bind(cid)
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .map(|r| {
                    let path: String = r.get(0);
                    (
                        path.clone(),
                        IndexedNote {
                            path,
                            content_hash: r.get(1),
                            last_indexed: 0,
                        },
                    )
                })
                .collect();

        debug!("Indexed: {}", result.len());
        Ok(result)
    }

    async fn list_collections(&self) -> anyhow::Result<Vec<CollectionInfo>> {
        let out = sqlx::query(
            "SELECT c.name, COUNT(DISTINCT k.path)
             FROM collections c LEFT JOIN chunks k ON k.collection_id = c.id
             GROUP BY c.id ORDER BY c.name",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|r| CollectionInfo {
            name: r.get(0),
            note_count: r.get::<i64, _>(1) as usize,
        })
        .collect();
        Ok(out)
    }

    async fn collection_names(&self) -> anyhow::Result<Vec<String>> {
        let names = sqlx::query("SELECT name FROM collections ORDER BY name")
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|r| r.get(0))
            .collect();
        Ok(names)
    }

    async fn read_fingerprint(&self) -> anyhow::Result<Option<String>> {
        let row = sqlx::query("SELECT value FROM meta WHERE key = ?")
            .bind(FINGERPRINT_KEY)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<String, _>(0).trim().to_string()))
    }

    async fn write_fingerprint(&self, fingerprint: &str) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO meta (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(FINGERPRINT_KEY)
        .bind(fingerprint)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn drop_all_collections(&self) -> anyhow::Result<()> {
        // The `meta` table (fingerprint slot) is metadata and survives.
        sqlx::raw_sql("DELETE FROM chunks; DELETE FROM collections;")
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dbembeddings::conformance;

    async fn store() -> (tempfile::TempDir, VecSqlite) {
        let dir = tempfile::tempdir().unwrap();
        let store = VecSqlite::new(dir.path(), conformance::DIM).await.unwrap();
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
    async fn conformance_fingerprint_round_trip() {
        let (_dir, s) = store().await;
        conformance::fingerprint_round_trips_and_starts_absent(&s).await;
    }

    #[tokio::test]
    async fn conformance_drop_all() {
        let (_dir, s) = store().await;
        conformance::drop_all_removes_every_collection_but_not_the_fingerprint_slot(&s, "va", "vb")
            .await;
    }

    #[tokio::test]
    async fn conformance_fingerprint_not_a_collection() {
        let (_dir, s) = store().await;
        conformance::fingerprint_slot_never_appears_as_a_collection(&s).await;
    }

    #[tokio::test]
    async fn dimension_mismatch_fails_loudly() {
        let dir = tempfile::tempdir().unwrap();
        let s8 = VecSqlite::new(dir.path(), 8).await.unwrap();
        s8.store("v", &[conformance::row("a.md", "h", "x")])
            .await
            .unwrap();

        // Reopen the same directory at another width: writes must be refused.
        let s16 = VecSqlite::new(dir.path(), 16).await.unwrap();
        let row = EmbeddedChunk {
            chunk: conformance::row("b.md", "h", "y").chunk,
            vector: vec![0.5; 16],
        };
        let err = s16
            .store("v", &[row])
            .await
            .expect_err("dim change must fail");
        assert!(err.to_string().contains("dimension"));
    }

    #[tokio::test]
    async fn store_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let s = VecSqlite::new(dir.path(), conformance::DIM).await.unwrap();
            s.store("v", &[conformance::row("a.md", "h1", "persistent chunk")])
                .await
                .unwrap();
            s.write_fingerprint("fp").await.unwrap();
        }
        let s = VecSqlite::new(dir.path(), conformance::DIM).await.unwrap();
        let results = s
            .query("v", conformance::vector_for("persistent chunk"), 1)
            .await
            .unwrap();
        assert_eq!(results[0].1.doc_path, "a.md");
        assert_eq!(s.read_fingerprint().await.unwrap().as_deref(), Some("fp"));
    }
}
