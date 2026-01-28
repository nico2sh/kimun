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

/// Splits chunks in a KimunDoc into smaller, more manageable pieces optimized for RAG embeddings.
///
/// This function takes a document that already contains chunks (typically split by markdown sections)
/// and further divides them into smaller pieces suitable for efficient embedding generation.
///
/// # Arguments
///
/// * `doc` - A KimunDoc containing chunks to be split
/// * `target_size` - Preferred size for each chunk in characters (e.g., 512-1024 for most embeddings)
/// * `max_size` - Maximum allowed size before forcing a split (e.g., 2048)
///
/// # Strategy
///
/// The function uses an adaptive splitting strategy that:
/// 1. Prioritizes paragraph breaks (`\n\n`) for semantic coherence
/// 2. Falls back to sentence boundaries (`.`, `!`, `?`, `\n`) if no paragraph breaks found
/// 3. Uses word boundaries (` `) as a last resort
/// 4. Forces a hard split at `max_size` if no natural boundary is found
///
/// # Returns
///
/// A new KimunDoc with the same path and hash, but with smaller, more uniform chunks.
/// Each resulting chunk preserves the title from its parent chunk for context.
///
/// Precision focused:
/// let optimized = split_chunks_for_rag(doc, 512, 1024);
///
/// Balanced focused (sweet spot):
/// let optimized = split_chunks_for_rag(doc, 800, 1536);
///
/// Context heavy:
/// let optimized = split_chunks_for_rag(doc, 1024, 2048);
///
/// # Examples
///
/// ```rust
/// use rag::document::{KimunDoc, Chunk, split_chunks_for_rag};
///
/// let doc = KimunDoc {
///     path: "notes/example.md".to_string(),
///     hash: "abc123".to_string(),
///     chunks: vec![
///         Chunk {
///             title: "Introduction".to_string(),
///             text: "This is a very long text...".to_string(),
///         }
///     ],
/// };
///
/// // Split into chunks targeting 800 chars (~200 tokens), max 1536 chars (~384 tokens)
/// // This is optimal for BGE-Large-EN-V1.5 embeddings (512 token limit)
/// let optimized = split_chunks_for_rag(doc, 800, 1536);
/// ```
pub fn split_chunks_for_rag(doc: KimunDoc, target_size: usize, max_size: usize) -> KimunDoc {
    let mut new_doc = KimunDoc {
        path: doc.path,
        hash: doc.hash,
        chunks: Vec::new(),
    };

    // Calculate the minimum size threshold for looking for break points
    // This ensures we don't create tiny chunks at the beginning
    let min_size = (target_size - (max_size - target_size) / 2).max(target_size / 2);

    for chunk in doc.chunks {
        let content = &chunk.text;
        let title = &chunk.title;

        // Skip empty chunks
        if content.is_empty() {
            continue;
        }

        // If chunk is already smaller than target, keep it as is
        if content.len() <= target_size {
            new_doc.chunks.push(Chunk {
                title: title.clone(),
                text: content.trim().to_string(),
            });
            continue;
        }

        // Split large chunks
        let mut start = 0;
        while start < content.len() {
            // Calculate boundary points for this iteration
            let min_end = (start + min_size).min(content.len());
            let max_end = (start + max_size).min(content.len());

            // If we're near the end and it fits, take everything
            if content.len() - start <= max_size {
                let chunk_text = content[start..].trim();
                if !chunk_text.is_empty() {
                    new_doc.chunks.push(Chunk {
                        text: chunk_text.to_string(),
                        title: title.clone(),
                    });
                }
                break;
            }

            // Search for natural breakpoints in priority order
            // 1. Paragraph break (double newline) - best for semantic coherence
            let paragraph_break = if min_end < max_end {
                content[min_end..max_end]
                    .find("\n\n")
                    .map(|pos| min_end + pos + 2)
            } else {
                None
            };

            // 2. Sentence break - good for maintaining complete thoughts
            let sentence_break = if min_end < max_end {
                content[min_end..max_end]
                    .find(['.', '!', '?', '\n'])
                    .map(|pos| min_end + pos + 1)
            } else {
                None
            };

            // 3. Word break - prevents splitting words
            let word_break = if min_end < max_end {
                content[min_end..max_end]
                    .find(' ')
                    .map(|pos| min_end + pos + 1)
            } else {
                None
            };

            // Choose the best available breakpoint
            let end = paragraph_break
                .or(sentence_break)
                .or(word_break)
                .unwrap_or(max_end);

            let chunk_text = content[start..end].trim();
            if !chunk_text.is_empty() {
                new_doc.chunks.push(Chunk {
                    text: chunk_text.to_string(),
                    title: title.clone(),
                });
            }

            start = end;
        }
    }

    new_doc
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
            // We chunk into manageable sizes optimized for BGE-Large-EN-V1.5 embeddings
            // Target: 800 chars (~200 tokens), Max: 1536 chars (~384 tokens)
            // This balances semantic precision with sufficient context
            let doc_chunks = ChunkLoader::chunk_document_adaptive(doc, 800, 1536);
            result.push(doc_chunks);
        }

        Ok(result)
    }

    /// This function splits even further the chunks to a smaller, more manageable size
    /// for (hopefully) better embeddings
    pub fn chunk_document_adaptive(doc: KimunDoc, target_size: usize, max_size: usize) -> KimunDoc {
        split_chunks_for_rag(doc, target_size, max_size)
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
    fn test_split_chunks_for_rag_multiple_chunks() {
        // Test that all chunks are processed (not overwritten)
        let doc = KimunDoc {
            path: "test.md".to_string(),
            hash: "hash123".to_string(),
            chunks: vec![
                Chunk {
                    title: "Section 1".to_string(),
                    text: "This is the first section with some content. It has multiple sentences.".to_string(),
                },
                Chunk {
                    title: "Section 2".to_string(),
                    text: "This is the second section with different content. It also has multiple sentences.".to_string(),
                },
                Chunk {
                    title: "Section 3".to_string(),
                    text: "Third section here.".to_string(),
                },
            ],
        };

        let result = crate::document::split_chunks_for_rag(doc, 30, 60);

        // Should have chunks from all three original chunks
        assert!(
            result.chunks.len() >= 3,
            "Should preserve all original chunks"
        );

        // Verify titles are preserved
        let titles: Vec<_> = result.chunks.iter().map(|c| c.title.as_str()).collect();
        assert!(titles.contains(&"Section 1"));
        assert!(titles.contains(&"Section 2"));
        assert!(titles.contains(&"Section 3"));
    }

    #[test]
    fn test_split_chunks_for_rag_large_chunk() {
        // Test splitting a large chunk into multiple smaller ones
        let large_text = "Paragraph one with some content here. This is a sentence.\n\n\
                         Paragraph two with more content. Another sentence here.\n\n\
                         Paragraph three continues. Final sentence.";

        let doc = KimunDoc {
            path: "large.md".to_string(),
            hash: "hash456".to_string(),
            chunks: vec![Chunk {
                title: "Large Section".to_string(),
                text: large_text.to_string(),
            }],
        };

        let result = crate::document::split_chunks_for_rag(doc, 50, 100);

        // Should split into multiple chunks
        assert!(result.chunks.len() > 1, "Large chunk should be split");

        // All chunks should have the same title
        for chunk in &result.chunks {
            assert_eq!(chunk.title, "Large Section");
        }

        // All chunks should be within max_size
        for chunk in &result.chunks {
            assert!(
                chunk.text.len() <= 100,
                "Chunk exceeds max size: {}",
                chunk.text.len()
            );
        }
    }

    #[test]
    fn test_split_chunks_for_rag_preserves_small_chunks() {
        // Small chunks should be preserved as-is
        let doc = KimunDoc {
            path: "small.md".to_string(),
            hash: "hash789".to_string(),
            chunks: vec![Chunk {
                title: "Small".to_string(),
                text: "Short text.".to_string(),
            }],
        };

        let result = crate::document::split_chunks_for_rag(doc.clone(), 500, 1000);

        assert_eq!(result.chunks.len(), 1);
        assert_eq!(result.chunks[0].text, "Short text.");
        assert_eq!(result.chunks[0].title, "Small");
    }

    #[test]
    fn test_split_chunks_for_rag_respects_paragraph_breaks() {
        // Should prefer paragraph breaks when splitting
        let text = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";

        let doc = KimunDoc {
            path: "para.md".to_string(),
            hash: "hashABC".to_string(),
            chunks: vec![Chunk {
                title: "Paragraphs".to_string(),
                text: text.to_string(),
            }],
        };

        let result = crate::document::split_chunks_for_rag(doc, 20, 40);

        // Should have split at paragraph boundaries
        assert!(result.chunks.len() >= 2);

        // Chunks should contain complete paragraphs
        for chunk in &result.chunks {
            // Shouldn't split mid-word
            assert!(!chunk.text.starts_with(' '));
        }
    }

    #[test]
    fn test_split_chunks_for_rag_empty_chunks() {
        // Should skip empty chunks
        let doc = KimunDoc {
            path: "empty.md".to_string(),
            hash: "hashDEF".to_string(),
            chunks: vec![
                Chunk {
                    title: "Empty".to_string(),
                    text: "".to_string(),
                },
                Chunk {
                    title: "Not Empty".to_string(),
                    text: "Content here.".to_string(),
                },
            ],
        };

        let result = crate::document::split_chunks_for_rag(doc, 500, 1000);

        assert_eq!(result.chunks.len(), 1);
        assert_eq!(result.chunks[0].title, "Not Empty");
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
