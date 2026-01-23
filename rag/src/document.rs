// A document is a chunk of text for Kimun
// Several can be in a single file as the document is basically a section within a file
// split by Markdown titles
//

// use kimun_core::{NoteVault, nfs::VaultPath, note::NoteDetails};

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct KimunChunk {
    pub content: String,
    pub metadata: KimunMetadata,
}

#[derive(Debug, Clone)]
pub struct KimunMetadata {
    pub source_path: String,
    pub title: String,
    pub date: Option<chrono::NaiveDate>,
    pub hash: String,
}

impl KimunMetadata {
    pub fn get_date_string(&self) -> Option<String> {
        self.date.map(|d| d.format("%Y-%m-%d").to_string())
    }
}

pub struct ChunkLoader {
    vault_path: PathBuf,
}

impl ChunkLoader {
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            vault_path: db_path,
        }
    }

    pub fn load_notes(&self) -> anyhow::Result<Vec<KimunChunk>> {
        use rusqlite::Connection;

        let db_path = self.vault_path.join("kimun.sqlite");
        let conn = Connection::open(&db_path)?;
        if !db_path.exists() {
            anyhow::bail!("Vault database not found at {:?}", db_path);
        }
        let mut stmt = conn.prepare(
            "SELECT n.path, n.noteName, n.title, nc.breadCrumb, nc.text, hash
             FROM notes n
             JOIN notesContent nc ON n.path = nc.path",
        )?;
        let notes_iter = stmt.query_map([], |row| {
            let path: String = row.get(0)?;
            let note_name: String = row.get(1)?;
            let note_title: String = row.get(2)?;
            let breadcrumb: String = row.get(3)?;
            let text: String = row.get(4)?;
            let hash: String = row.get(5)?;

            let filename = note_name.strip_suffix(".md").unwrap_or(note_name.as_str());
            let date = chrono::NaiveDate::parse_from_str(filename, "%Y-%m-%d").ok();
            let title = if breadcrumb.is_empty() {
                note_title
            } else {
                breadcrumb
            };

            Ok((path, note_name, title, text, date, hash))
        })?;

        let mut result = Vec::new();
        // let path_chunks = self.vault.get_note_chunks(&VaultPath::new("journal"))?;
        // let path_chunks = self.vault.get_note_chunks(&VaultPath::new("journal"))?;
        for note in notes_iter {
            let (path, _note_name, title, text, date, hash) = note?;

            let metadata = KimunMetadata {
                source_path: path,
                title,
                date,
                hash,
            };
            let document = KimunChunk {
                content: text.clone(),
                metadata,
            };
            // We chunk into manageable sizes
            let doc_chunks = ChunkLoader::chunk_document_adaptive(document, 1024, 2048);
            for doc in doc_chunks {
                result.push(doc);
            }
        }

        Ok(result)
    }

    pub fn chunk_document_adaptive(
        doc: KimunChunk,
        target_size: usize,
        max_size: usize,
    ) -> Vec<KimunChunk> {
        let content = &doc.content;
        let mut chunks = Vec::new();
        let mut start = 0;

        let mid_start_size = (target_size - (max_size - target_size) / 2).max(target_size / 2);

        while start < content.len() {
            // Calculate potential end points
            let target_end = (start + target_size).min(content.len());
            let min_end = (start + mid_start_size).min(content.len());
            let max_end = (start + max_size).min(content.len());

            // Try to find natural boundaries (paragraph break, then sentence, then word)
            let paragraph_break = content[min_end..max_end]
                .find("\n\n")
                .map(|pos| min_end + pos + 2);

            let sentence_break = content[min_end..max_end]
                .find(['.', '!', '?', '\n'])
                .map(|pos| min_end + pos + 1);

            let word_break = content[min_end..max_end]
                .find(' ')
                .map(|pos| min_end + pos + 1);

            // Choose the best breakpoint
            let break_end = paragraph_break
                .or(sentence_break)
                .or(word_break)
                .unwrap_or(max_end);

            let end = if content.len() - target_end < content.len() - break_end {
                content.len()
            } else {
                break_end
            };

            let chunk_content = content[start..end].to_string();
            chunks.push(KimunChunk {
                content: chunk_content.trim().to_string(),
                metadata: doc.metadata.clone(),
            });

            start = end;
        }

        chunks
    }

    pub fn _chunk_document(doc: KimunChunk, chunk_size: usize, overlap: usize) -> Vec<KimunChunk> {
        let content = &doc.content;
        let mut chunks = Vec::new();

        // Simple chunking by characters with overlap
        let mut start = 0;
        while start < content.len() {
            let end = (start + chunk_size).min(content.len());

            // Find a good breaking point (end of sentence or paragraph)
            let mut actual_end = end;
            if end < content.len() {
                // Try to find the end of a sentence
                if let Some(pos) = content[start..end].rfind(['.', '!', '?', '\n']) {
                    actual_end = start + pos + 1;
                }
            }

            let chunk_content = content[start..actual_end].to_string();
            chunks.push(KimunChunk {
                content: chunk_content,
                metadata: doc.metadata.clone(),
            });

            start = if actual_end == end && end < content.len() {
                actual_end - overlap
            } else {
                actual_end
            };
        }

        chunks
    }
}

#[cfg(test)]
mod test {
    use super::{ChunkLoader, KimunChunk};

    #[test]
    fn split_chunks() {
        let doc_content = r#"First paragraph.

Second line"#;

        let chunk = KimunChunk {
            content: doc_content.to_string(),
            metadata: super::KimunMetadata {
                source_path: "path".to_string(),
                title: "".to_string(),
                date: None,
                hash: "".to_string(),
            },
        };

        let chunks = ChunkLoader::chunk_document_adaptive(chunk, 10, 20);
        assert_eq!(2, chunks.len());
        assert_eq!("First paragraph.".to_string(), chunks[0].content);
        assert_eq!("Second line".to_string(), chunks[1].content);
    }
}
