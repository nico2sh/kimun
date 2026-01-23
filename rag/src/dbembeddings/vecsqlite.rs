use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use chrono::NaiveDate;
use log::debug;
use rusqlite::{ffi::sqlite3_auto_extension, params};
use sqlite_vec::sqlite3_vec_init;
use zerocopy::IntoBytes;

use crate::document::KimunChunk;

use super::{
    Embeddings,
    embedder::{Embedder, fastembedder::FastEmbedder},
};

const DB_FILENAME: &str = "kimun_vec.sqlite";

pub struct VecSQLite {
    embedder: FastEmbedder,
    db_path: PathBuf,
}

impl VecSQLite {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        unsafe {
            sqlite3_auto_extension(Some(std::mem::transmute(sqlite3_vec_init as *const ())));
        }

        let embedder = FastEmbedder::new().unwrap();

        let mut db_path = path.as_ref().to_path_buf();
        db_path.push(DB_FILENAME);
        Self { embedder, db_path }
    }

    fn connection() -> anyhow::Result<rusqlite::Connection> {
        let connection = rusqlite::Connection::open(DB_FILENAME)?;
        Ok(connection)
    }

    fn insert_vec(&self, docs: Vec<(usize, &KimunChunk, &Vec<f32>)>) -> anyhow::Result<()> {
        let mut conn = VecSQLite::connection()?;
        let tx = conn.transaction()?;
        let doc_sql =
            "INSERT INTO docs (rowid, path, title, date, text) VALUES (?1, ?2, ?3, ?4, ?5)";
        let vec_sql = "INSERT INTO vec_items(rowid, embedding) VALUES (?1, ?2)";
        for (id, doc, vec) in docs {
            tx.execute(
                doc_sql,
                params![
                    id,
                    doc.metadata.source_path,
                    doc.metadata.title,
                    doc.metadata
                        .date
                        .map(|date| date.format("%Y-%m-%d").to_string())
                        .unwrap_or_default(),
                    doc.content
                ],
            )?;
            tx.execute(vec_sql, params![id, vec.as_bytes()])?;
        }
        tx.commit()?;
        Ok(())
    }

    fn get_docs(&self, vec: &[f32]) -> anyhow::Result<Vec<(f64, KimunChunk)>> {
        let start = SystemTime::now();
        let conn = VecSQLite::connection()?;
        let mut max_distance = f64::MIN;
        let mut min_distance = f64::MAX;
        let result: Vec<(f64, KimunChunk)> = conn
            .prepare(
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
          AND k = 128
          ORDER BY vec_items.distance
        ",
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
                let kimun_chunk = KimunChunk {
                    content: text,
                    metadata: crate::document::KimunMetadata {
                        source_path: path,
                        title,
                        date: match NaiveDate::parse_from_str(date.as_str(), "%Y-%m-%d") {
                            Ok(d) => Some(d),
                            Err(_) => None,
                        },
                        hash,
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
}

#[async_trait::async_trait]
impl Embeddings for VecSQLite {
    fn init(&self) -> anyhow::Result<()> {
        debug!("Checking the db_path at {}", self.db_path.to_string_lossy());
        let md = std::fs::metadata(&self.db_path)?;
        // We delete the db file
        if md.is_dir() {
            std::fs::remove_dir_all(&self.db_path)?;
        } else {
            std::fs::remove_file(&self.db_path)?;
        }

        debug!("Creating tables");
        let mut conn = VecSQLite::connection()?;
        let tx = conn.transaction()?;
        tx.execute(
            "CREATE VIRTUAL TABLE vec_items USING vec0(embedding float[1024])",
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

    async fn store_embeddings(&self, content: &[KimunChunk]) -> anyhow::Result<()> {
        let embeddings = self.embedder.generate_embeddings(content).await?;
        let embed_chunks = embeddings.chunks(100);
        let mut i = 0;
        for batch in embed_chunks {
            let mut insert_batch = vec![];
            for c in batch {
                insert_batch.push((i, content.get(i).unwrap(), c));
                i += 1;
            }
            self.insert_vec(insert_batch)?;
        }
        Ok(())
    }

    async fn query_embedding(&self, query: &str) -> anyhow::Result<Vec<(f64, KimunChunk)>> {
        let query_embed = self.embedder.prompt_embedding(query).await?;
        self.get_docs(&query_embed)
    }

    fn get_indexed_notes(
        &self,
    ) -> anyhow::Result<std::collections::HashMap<String, crate::dbembeddings::IndexedNote>> {
        use std::collections::HashMap;

        let conn = VecSQLite::connection()?;
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

    fn mark_as_indexed(&self, path: &str, content_hash: &str) -> anyhow::Result<()> {
        let conn = VecSQLite::connection()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;

        conn.execute(
            "INSERT OR REPLACE INTO indexed_notes (path, content_hash, last_indexed) VALUES (?1, ?2, ?3)",
            (path, content_hash, now),
        )?;

        Ok(())
    }

    fn remove_indexed_note(&self, path: &str) -> anyhow::Result<()> {
        let conn = VecSQLite::connection()?;
        conn.execute("DELETE FROM indexed_notes WHERE path = ?1", [path])?;
        Ok(())
    }
}

/// Compute SHA256 hash of content for change detection
pub fn compute_content_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}
