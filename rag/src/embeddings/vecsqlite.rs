use std::{path::PathBuf, time::SystemTime};

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
    pub fn new() -> Self {
        unsafe {
            sqlite3_auto_extension(Some(std::mem::transmute(sqlite3_vec_init as *const ())));
        }

        let embedder = FastEmbedder::new().unwrap();

        let mut db_path = std::env::current_dir().unwrap();
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
            docs.text
          FROM vec_items
          JOIN docs ON vec_items.rowid = docs.rowid
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
                let kimun_chunk = KimunChunk {
                    content: text,
                    metadata: crate::document::KimunMetadata {
                        source_path: path,
                        title,
                        date: match NaiveDate::parse_from_str(date.as_str(), "%Y-%m-%d") {
                            Ok(d) => Some(d),
                            Err(_) => None,
                        },
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
        debug!("Max distance: {}", max_distance);
        debug!("Min distance: {}", min_distance);
        Ok(result)
    }
}

impl Embeddings for VecSQLite {
    fn init(&mut self) -> anyhow::Result<()> {
        let md = std::fs::metadata(&self.db_path)?;
        // We delete the db file
        if md.is_dir() {
            std::fs::remove_dir_all(&self.db_path)?;
        } else {
            std::fs::remove_file(&self.db_path)?;
        }

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

    async fn query_embedding<S: AsRef<str>>(
        &self,
        query: S,
    ) -> anyhow::Result<Vec<(f64, KimunChunk)>> {
        let query_embed = self.embedder.prompt_embedding(query).await?;
        self.get_docs(&query_embed)
    }
}
