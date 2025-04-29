// A document is a chunk of text for Kimun
// Several can be in a single file as the document is basically a section within a file
// split by Markdown titles
//

use kimun_core::{NoteVault, nfs::VaultPath, note::NoteDetails};

#[derive(Debug, Clone)]
pub struct KimunChunk {
    pub content: String,
    pub metadata: KimunMetadata,
}

impl KimunChunk {
    pub fn to_embed_payload(&self) -> String {
        format!("passage: {}\n{}", self.metadata.title, self.content)
    }
}

#[derive(Debug, Clone)]
pub struct KimunMetadata {
    pub source_path: String,
    pub title: String,
    pub date: Option<chrono::NaiveDate>,
}

impl KimunMetadata {
    pub fn get_date_string(&self) -> Option<String> {
        self.date.map(|d| d.format("%Y-%m-%d").to_string())
    }
}

pub struct ChunkLoader {
    vault: NoteVault,
}

impl ChunkLoader {
    pub fn new(vault: NoteVault) -> Self {
        Self { vault }
    }

    pub fn load_notes(&self) -> anyhow::Result<Vec<KimunChunk>> {
        let mut result = Vec::new();
        // let path_chunks = self.vault.get_note_chunks(&VaultPath::new("journal"))?;
        let path_chunks = self.vault.get_note_chunks(&VaultPath::new("journal"))?;
        for (path, chunks) in path_chunks.iter() {
            let (_parent, file) = path.get_parent_path();
            let filename = file.strip_suffix(".md").unwrap_or(file.as_str());

            let date = match chrono::NaiveDate::parse_from_str(filename, "%Y-%m-%d") {
                Ok(d) => Some(d),
                Err(_) => None,
            };
            for chunk in chunks {
                let title = if chunk.breadcrumb.is_empty() {
                    NoteDetails::get_title_from_text(&chunk.text)
                } else {
                    chunk.breadcrumb.join(", ")
                };
                let metadata = KimunMetadata {
                    source_path: path.to_string(),
                    title,
                    date,
                };
                let document = KimunChunk {
                    content: chunk.text.clone(),
                    metadata,
                };

                // We chunk into manageable sizes
                let doc_chunks = ChunkLoader::chunk_document_adaptive(document, 1024, 2048);
                for doc in doc_chunks {
                    result.push(doc);
                }
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

    pub fn chunk_document(doc: KimunChunk, chunk_size: usize, overlap: usize) -> Vec<KimunChunk> {
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
            },
        };

        let chunks = ChunkLoader::chunk_document_adaptive(chunk, 10, 20);
        assert_eq!(2, chunks.len());
        assert_eq!("First paragraph.".to_string(), chunks[0].content);
        assert_eq!("Second line".to_string(), chunks[1].content);
    }
}
