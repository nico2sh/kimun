// A document is a chunk of text for Kimun
// Several can be in a single file as the document is basically a section within a file
// split by Markdown titles

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// This is the representation of a doc, which contains different chunks
/// usually split by markdown section titles
/// The breadcrumb (the hierarchical sequence of titles and subtitles) is used
/// as the title of each chunk
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KimunDoc {
    pub path: String,
    pub hash: String,
    pub chunks: Vec<Chunk>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chunk {
    pub title: String,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct FlattenedChunk {
    pub doc_path: String,
    pub doc_hash: String,
    pub title: String,
    pub text: String,
    pub date: Option<chrono::NaiveDate>,
}

impl FlattenedChunk {
    pub fn from_chunks(chunks: &[KimunDoc]) -> Vec<FlattenedChunk> {
        let mut result = vec![];
        for chunkp in chunks {
            // Get the last segment of the path (separated by '/')
            let filename = chunkp.path.rsplit('/').next().unwrap_or("");

            // Remove .md extension
            let filename_without_ext = filename.strip_suffix(".md").unwrap_or(filename);

            // Try to parse as date in format %Y-%m-%d
            let date = chrono::NaiveDate::parse_from_str(filename_without_ext, "%Y-%m-%d").ok();
            for chunk in &chunkp.chunks {
                result.push(FlattenedChunk {
                    doc_path: chunkp.path.to_owned(),
                    doc_hash: chunkp.hash.to_owned(),
                    title: chunk.title.to_owned(),
                    text: chunk.text.to_owned(),
                    date,
                });
            }
        }

        result
    }

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

    pub fn load_notes(&self) -> anyhow::Result<Vec<KimunDoc>> {
        use rusqlite::Connection;

        let db_path = self.vault_path.join("kimun.sqlite");
        let conn = Connection::open(&db_path)?;
        if !db_path.exists() {
            anyhow::bail!("Vault database not found at {:?}", db_path);
        }
        let mut stmt = conn.prepare(
            "SELECT n.path, n.noteName, n.title, nc.breadCrumb, nc.text, hash
             FROM notes n
             JOIN notesContent nc ON n.path = nc.path ORDER BY n.path",
        )?;
        let mut rows = stmt.query([])?;

        let mut chunks: Vec<KimunDoc> = vec![];
        while let Some(row) = rows.next()? {
            let path: String = row.get(0)?;
            let breadcrumb: String = row.get(1)?;
            let text: String = row.get(2)?;
            let hash: String = row.get(3)?;
            if let Some(ch) = chunks.last_mut() {
                if ch.path != path {
                    let new_chunk = KimunDoc {
                        path: path.clone(),
                        hash,
                        chunks: vec![Chunk {
                            title: breadcrumb,
                            text,
                        }],
                    };
                    chunks.push(new_chunk);
                } else {
                    ch.chunks.push(Chunk {
                        title: breadcrumb,
                        text,
                    });
                }
            } else {
                let new_chunk = KimunDoc {
                    path: path.clone(),
                    hash,
                    chunks: vec![Chunk {
                        title: breadcrumb,
                        text,
                    }],
                };
                chunks.push(new_chunk);
            }
        }

        let mut result = Vec::new();
        for doc in chunks {
            // We chunk into manageable sizes
            let doc_chunks = ChunkLoader::chunk_document_adaptive(doc, 1024, 2048);
            result.push(doc_chunks);
        }

        Ok(result)
    }

    /// This function splits even further the chunks to a smaller, more manageable size
    /// for (hopefully) better embeddings
    pub fn chunk_document_adaptive(doc: KimunDoc, target_size: usize, max_size: usize) -> KimunDoc {
        let mut new_doc = KimunDoc {
            path: doc.path,
            hash: doc.hash,
            chunks: vec![],
        };
        for chunk in doc.chunks {
            let content = &chunk.text;
            let title = &chunk.title;

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
                chunks.push(Chunk {
                    text: chunk_content.trim().to_string(),
                    title: title.clone(),
                });

                start = end;
            }
            new_doc.chunks = chunks;
        }

        new_doc
    }

    // pub fn _chunk_document(
    //     doc: FlattenedChunk,
    //     chunk_size: usize,
    //     overlap: usize,
    // ) -> Vec<FlattenedChunk> {
    //     let content = &doc.text;
    //     let mut chunks = Vec::new();

    //     // Simple chunking by characters with overlap
    //     let mut start = 0;
    //     while start < content.len() {
    //         let end = (start + chunk_size).min(content.len());

    //         // Find a good breaking point (end of sentence or paragraph)
    //         let mut actual_end = end;
    //         if end < content.len() {
    //             // Try to find the end of a sentence
    //             if let Some(pos) = content[start..end].rfind(['.', '!', '?', '\n']) {
    //                 actual_end = start + pos + 1;
    //             }
    //         }

    //         let chunk_content = content[start..actual_end].to_string();
    //         chunks.push(FlattenedChunk {
    //             doc_path: doc.doc_path.clone(),
    //             doc_hash: doc.doc_hash.clone(),
    //             text: chunk_content,

    //             metadata: doc.metadata.clone(),
    //         });

    //         start = if actual_end == end && end < content.len() {
    //             actual_end - overlap
    //         } else {
    //             actual_end
    //         };
    //     }

    //     chunks
    // }
}

#[cfg(test)]
mod test {
    use crate::document::{Chunk, KimunDoc};

    use super::ChunkLoader;

    #[test]
    fn split_chunks() {
        let doc_content = r#"First paragraph.

Second line"#;

        let chunk = KimunDoc {
            path: "path".to_string(),

            hash: "".to_string(),
            chunks: vec![Chunk {
                title: "".to_string(),
                text: doc_content.to_string(),
            }],
        };

        let chunks = ChunkLoader::chunk_document_adaptive(chunk, 10, 20);
        assert_eq!(2, chunks.chunks.len());
        assert_eq!("First paragraph.".to_string(), chunks.chunks[0].text);
        assert_eq!("Second line".to_string(), chunks.chunks[1].text);
    }

    #[test]
    fn test_kimundoc_serialize_basic() {
        let doc = KimunDoc {
            path: "test/path.md".to_string(),
            hash: "abc123".to_string(),
            chunks: vec![
                Chunk {
                    title: "Introduction".to_string(),
                    text: "This is the intro text.".to_string(),
                },
                Chunk {
                    title: "Main Content".to_string(),
                    text: "This is the main content.".to_string(),
                },
            ],
        };

        let json = serde_json::to_string(&doc).expect("Failed to serialize");

        assert!(json.contains("test/path.md"));
        assert!(json.contains("abc123"));
        assert!(json.contains("Introduction"));
        assert!(json.contains("This is the intro text."));
        assert!(json.contains("Main Content"));
        assert!(json.contains("This is the main content."));
    }

    #[test]
    fn test_kimundoc_deserialize_basic() {
        let json = r#"{
            "path": "notes/2024-01-15.md",
            "hash": "def456",
            "chunks": [
                {
                    "title": "Morning Notes",
                    "text": "Started the day early."
                },
                {
                    "title": "Evening Notes",
                    "text": "Productive day overall."
                }
            ]
        }"#;

        let doc: KimunDoc = serde_json::from_str(json).expect("Failed to deserialize");

        assert_eq!(doc.path, "notes/2024-01-15.md");
        assert_eq!(doc.hash, "def456");
        assert_eq!(doc.chunks.len(), 2);
        assert_eq!(doc.chunks[0].title, "Morning Notes");
        assert_eq!(doc.chunks[0].text, "Started the day early.");
        assert_eq!(doc.chunks[1].title, "Evening Notes");
        assert_eq!(doc.chunks[1].text, "Productive day overall.");
    }

    #[test]
    fn test_kimundoc_roundtrip() {
        let original = KimunDoc {
            path: "projects/kimun.md".to_string(),
            hash: "hash789".to_string(),
            chunks: vec![
                Chunk {
                    title: "Overview".to_string(),
                    text: "Kimun is a notes app.".to_string(),
                },
                Chunk {
                    title: "Features".to_string(),
                    text: "Search, organize, and sync.".to_string(),
                },
            ],
        };

        let json = serde_json::to_string(&original).expect("Failed to serialize");
        let deserialized: KimunDoc = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(original.path, deserialized.path);
        assert_eq!(original.hash, deserialized.hash);
        assert_eq!(original.chunks.len(), deserialized.chunks.len());

        for (orig, deser) in original.chunks.iter().zip(deserialized.chunks.iter()) {
            assert_eq!(orig.title, deser.title);
            assert_eq!(orig.text, deser.text);
        }
    }

    #[test]
    fn test_kimundoc_empty_chunks() {
        let doc = KimunDoc {
            path: "empty.md".to_string(),
            hash: "empty_hash".to_string(),
            chunks: vec![],
        };

        let json = serde_json::to_string(&doc).expect("Failed to serialize");
        let deserialized: KimunDoc = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.path, "empty.md");
        assert_eq!(deserialized.hash, "empty_hash");
        assert_eq!(deserialized.chunks.len(), 0);
    }

    #[test]
    fn test_kimundoc_special_characters() {
        let doc = KimunDoc {
            path: "special/chars.md".to_string(),
            hash: "special123".to_string(),
            chunks: vec![
                Chunk {
                    title: "Special \"Quotes\" & Symbols".to_string(),
                    text: "Text with\nnewlines\tand\ttabs".to_string(),
                },
                Chunk {
                    title: "Unicode 🎉 Emoji".to_string(),
                    text: "こんにちは世界 and Ñoño".to_string(),
                },
            ],
        };

        let json = serde_json::to_string(&doc).expect("Failed to serialize");
        let deserialized: KimunDoc = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.chunks[0].title, "Special \"Quotes\" & Symbols");
        assert_eq!(
            deserialized.chunks[0].text,
            "Text with\nnewlines\tand\ttabs"
        );
        assert_eq!(deserialized.chunks[1].title, "Unicode 🎉 Emoji");
        assert_eq!(deserialized.chunks[1].text, "こんにちは世界 and Ñoño");
    }

    #[test]
    fn test_kimundoc_large_text() {
        let large_text = "a".repeat(10000);
        let doc = KimunDoc {
            path: "large.md".to_string(),
            hash: "large_hash".to_string(),
            chunks: vec![Chunk {
                title: "Large Chunk".to_string(),
                text: large_text.clone(),
            }],
        };

        let json = serde_json::to_string(&doc).expect("Failed to serialize");
        let deserialized: KimunDoc = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.chunks[0].text.len(), 10000);
        assert_eq!(deserialized.chunks[0].text, large_text);
    }

    #[test]
    fn test_kimundoc_pretty_print() {
        let doc = KimunDoc {
            path: "pretty.md".to_string(),
            hash: "pretty_hash".to_string(),
            chunks: vec![Chunk {
                title: "Title".to_string(),
                text: "Text".to_string(),
            }],
        };

        let json_pretty = serde_json::to_string_pretty(&doc).expect("Failed to serialize");
        assert!(json_pretty.contains("\n"));

        let deserialized: KimunDoc =
            serde_json::from_str(&json_pretty).expect("Failed to deserialize");
        assert_eq!(deserialized.path, "pretty.md");
    }

    #[test]
    fn test_chunk_serialize_deserialize() {
        let chunk = Chunk {
            title: "Test Chunk".to_string(),
            text: "This is test content.".to_string(),
        };

        let json = serde_json::to_string(&chunk).expect("Failed to serialize chunk");
        let deserialized: Chunk = serde_json::from_str(&json).expect("Failed to deserialize chunk");

        assert_eq!(chunk.title, deserialized.title);
        assert_eq!(chunk.text, deserialized.text);
    }

    #[test]
    fn test_kimundoc_empty_strings() {
        let doc = KimunDoc {
            path: "".to_string(),
            hash: "".to_string(),
            chunks: vec![Chunk {
                title: "".to_string(),
                text: "".to_string(),
            }],
        };

        let json = serde_json::to_string(&doc).expect("Failed to serialize");
        let deserialized: KimunDoc = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.path, "");
        assert_eq!(deserialized.hash, "");
        assert_eq!(deserialized.chunks[0].title, "");
        assert_eq!(deserialized.chunks[0].text, "");
    }

    #[test]
    fn test_kimundoc_markdown_content() {
        let markdown_text = r#"# Heading 1
## Heading 2
- List item 1
- List item 2

```rust
fn main() {
    println!("Hello");
}
```

[Link](https://example.com)"#;

        let doc = KimunDoc {
            path: "markdown.md".to_string(),
            hash: "md_hash".to_string(),
            chunks: vec![Chunk {
                title: "Markdown Section".to_string(),
                text: markdown_text.to_string(),
            }],
        };

        let json = serde_json::to_string(&doc).expect("Failed to serialize");
        let deserialized: KimunDoc = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.chunks[0].text, markdown_text);
    }
}
