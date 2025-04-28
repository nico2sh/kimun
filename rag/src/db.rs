use chrono::NaiveDate;
use rusqlite::{ffi::sqlite3_auto_extension, params};
use sqlite_vec::sqlite3_vec_init;
use zerocopy::IntoBytes;

use crate::document::KimunChunk;

const DB_FILENAME: &str = "kimun_vec.sqlite";

pub struct VecDB {}

impl VecDB {
    pub fn new() -> Self {
        unsafe {
            sqlite3_auto_extension(Some(std::mem::transmute(sqlite3_vec_init as *const ())));
        }
        Self {}
    }

    fn connection() -> anyhow::Result<rusqlite::Connection> {
        let connection = rusqlite::Connection::open(DB_FILENAME)?;
        Ok(connection)
    }

    pub fn init(&self) -> anyhow::Result<()> {
        let mut conn = VecDB::connection()?;
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

    pub fn insert_vec(&self, docs: Vec<(usize, &KimunChunk, &Vec<f32>)>) -> anyhow::Result<()> {
        let mut conn = VecDB::connection()?;
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

    pub fn get_docs(&self, vec: &[f32]) -> anyhow::Result<Vec<(f64, KimunChunk)>> {
        let conn = VecDB::connection()?;
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
          AND distance < 0.80
          AND k = 128
          ORDER BY vec_items.distance
        ",
            )?
            .query_map([vec.as_bytes()], |r| {
                let distance: f64 = r.get(0)?;
                let path = r.get(1)?;
                let title = r.get(2)?;
                let date: String = r.get(3)?;
                let text = r.get(4)?;
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

        Ok(result)
    }
}
