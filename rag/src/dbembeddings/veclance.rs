use std::path::{Path, PathBuf};

use lancedb::Connection;

use super::{Embeddings, embedder::fastembedder::FastEmbedder};

const DB_FILENAME: &str = "kimun-lance";

pub struct VecLance {
    db_path: PathBuf,
    embedder: FastEmbedder,
}

impl VecLance {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        let mut db_path = path.as_ref().to_path_buf();
        db_path.push(DB_FILENAME);
        let embedder = FastEmbedder::new().unwrap();
        Self { db_path, embedder }
    }

    async fn get_connection(&self) -> anyhow::Result<Connection> {
        let conn = lancedb::connect(self.db_path.to_string_lossy().as_ref())
            .execute()
            .await?;
        Ok(conn)
    }
}

impl Embeddings for VecLance {
    fn init(&mut self) -> anyhow::Result<()> {
        todo!()
    }

    async fn store_embeddings(
        &self,
        content: &[crate::document::KimunChunk],
    ) -> anyhow::Result<()> {
        todo!()
    }

    async fn query_embedding<S: AsRef<str>>(
        &self,
        content: S,
    ) -> anyhow::Result<Vec<(f64, crate::document::KimunChunk)>> {
        todo!()
    }
}
