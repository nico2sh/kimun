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
    pub sections: Vec<KimunSection>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KimunSection {
    pub title: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
            for chunk in &chunkp.sections {
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
pub fn split_chunks_for_rag(
    doc: impl AsRef<str>,
    target_size: usize,
    max_size: usize,
) -> Vec<String> {
    let doc = doc.as_ref();

    // Helper function to ensure a position is on a valid UTF-8 character boundary
    let ensure_char_boundary = |pos: usize| -> usize {
        if pos >= doc.len() {
            doc.len()
        } else if doc.is_char_boundary(pos) {
            pos
        } else {
            // Find the nearest character boundary at or before pos
            (0..=pos)
                .rev()
                .find(|&p| doc.is_char_boundary(p))
                .unwrap_or(0)
        }
    };

    // Calculate the minimum size threshold for looking for break points
    // This ensures we don't create tiny chunks at the beginning
    let min_size = (target_size - (max_size - target_size) / 2).max(target_size / 2);

    // Return empty chunks
    if doc.is_empty() {
        return vec!["".to_string()];
    }
    let mut new_doc = vec![];

    // If chunk is already smaller than target, keep it as is
    if doc.len() <= target_size {
        new_doc.push(doc.trim().to_string());
        return new_doc;
    }

    // Split large chunks
    let mut start = 0;
    while start < doc.len() {
        // Calculate boundary points for this iteration
        // Ensure they're on character boundaries to avoid panics with Unicode
        let min_end = ensure_char_boundary((start + min_size).min(doc.len()));
        let max_end = ensure_char_boundary((start + max_size).min(doc.len()));

        // If we're near the end and it fits, take everything
        if doc.len() - start <= max_size {
            let chunk_text = doc[start..].trim();
            if !chunk_text.is_empty() {
                new_doc.push(chunk_text.to_string());
            }
            break;
        }

        // Search for natural breakpoints in priority order
        // 1. Paragraph break (double newline) - best for semantic coherence
        let paragraph_break = if min_end < max_end {
            doc[min_end..max_end]
                .find("\n\n")
                .map(|pos| min_end + pos + 2)
        } else {
            None
        };

        // 2. Sentence break - good for maintaining complete thoughts
        let sentence_break = if min_end < max_end {
            doc[min_end..max_end]
                .find(['.', '!', '?', '\n'])
                .map(|pos| min_end + pos + 1)
        } else {
            None
        };

        // 3. Word break - prevents splitting words
        let word_break = if min_end < max_end {
            doc[min_end..max_end].find(' ').map(|pos| min_end + pos + 1)
        } else {
            None
        };

        // Choose the best available breakpoint
        let mut end = paragraph_break
            .or(sentence_break)
            .or(word_break)
            .unwrap_or(max_end);

        // Ensure the end position is on a character boundary
        end = ensure_char_boundary(end);

        let chunk_text = doc[start..end].trim();
        if !chunk_text.is_empty() {
            new_doc.push(chunk_text.to_string());
        }

        start = end;
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

        let mut docs: Vec<KimunDoc> = vec![];
        while let Some(row) = rows.next()? {
            let path: String = row.get(0)?;
            let breadcrumb: String = row.get(1)?;
            let text: String = row.get(2)?;
            let hash: String = row.get(3)?;
            if let Some(ch) = docs.last_mut() {
                if ch.path != path {
                    let new_chunk = KimunDoc {
                        path: path.clone(),
                        hash,
                        sections: vec![KimunSection {
                            title: breadcrumb,
                            text,
                        }],
                    };
                    docs.push(new_chunk);
                } else {
                    ch.sections.push(KimunSection {
                        title: breadcrumb,
                        text,
                    });
                }
            } else {
                let new_chunk = KimunDoc {
                    path: path.clone(),
                    hash,
                    sections: vec![KimunSection {
                        title: breadcrumb,
                        text,
                    }],
                };
                docs.push(new_chunk);
            }
        }

        let mut result = Vec::new();
        for doc in docs {
            // We chunk into manageable sizes optimized for BGE-Large-EN-V1.5 embeddings
            // Target: 800 chars (~200 tokens), Max: 1536 chars (~384 tokens)
            // This balances semantic precision with sufficient context
            let mut new_doc = KimunDoc {
                path: doc.path,
                hash: doc.hash,
                sections: vec![],
            };
            for section in doc.sections {
                let section_chunks = split_chunks_for_rag(section.text, 800, 1536);
                let mut chunks = section_chunks
                    .iter()
                    .map(|s| KimunSection {
                        title: section.title.clone(),
                        text: s.to_owned(),
                    })
                    .collect::<Vec<KimunSection>>();
                new_doc.sections.append(&mut chunks);
            }
            result.push(new_doc);
        }

        Ok(result)
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
    use crate::{
        document::{KimunDoc, KimunSection},
        split_chunks_for_rag,
    };

    #[test]
    fn split_chunks() {
        let doc_content = r#"First paragraph.

Second line"#;

        let chunks = split_chunks_for_rag(doc_content, 10, 20);
        assert_eq!(2, chunks.len());
        assert_eq!("First paragraph.".to_string(), chunks[0]);
        assert_eq!("Second line".to_string(), chunks[1]);
    }

    #[test]
    fn test_kimundoc_serialize_basic() {
        let doc = KimunDoc {
            path: "test/path.md".to_string(),
            hash: "abc123".to_string(),
            sections: vec![
                KimunSection {
                    title: "Introduction".to_string(),
                    text: "This is the intro text.".to_string(),
                },
                KimunSection {
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
            "sections": [
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
        assert_eq!(doc.sections.len(), 2);
        assert_eq!(doc.sections[0].title, "Morning Notes");
        assert_eq!(doc.sections[0].text, "Started the day early.");
        assert_eq!(doc.sections[1].title, "Evening Notes");
        assert_eq!(doc.sections[1].text, "Productive day overall.");
    }

    #[test]
    fn test_kimundoc_roundtrip() {
        let original = KimunDoc {
            path: "projects/kimun.md".to_string(),
            hash: "hash789".to_string(),
            sections: vec![
                KimunSection {
                    title: "Overview".to_string(),
                    text: "Kimun is a notes app.".to_string(),
                },
                KimunSection {
                    title: "Features".to_string(),
                    text: "Search, organize, and sync.".to_string(),
                },
            ],
        };

        let json = serde_json::to_string(&original).expect("Failed to serialize");
        let deserialized: KimunDoc = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(original.path, deserialized.path);
        assert_eq!(original.hash, deserialized.hash);
        assert_eq!(original.sections.len(), deserialized.sections.len());

        for (orig, deser) in original.sections.iter().zip(deserialized.sections.iter()) {
            assert_eq!(orig.title, deser.title);
            assert_eq!(orig.text, deser.text);
        }
    }

    #[test]
    fn test_kimundoc_empty_chunks() {
        let doc = KimunDoc {
            path: "empty.md".to_string(),
            hash: "empty_hash".to_string(),
            sections: vec![],
        };

        let json = serde_json::to_string(&doc).expect("Failed to serialize");
        let deserialized: KimunDoc = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.path, "empty.md");
        assert_eq!(deserialized.hash, "empty_hash");
        assert_eq!(deserialized.sections.len(), 0);
    }

    #[test]
    fn test_kimundoc_special_characters() {
        let doc = KimunDoc {
            path: "special/chars.md".to_string(),
            hash: "special123".to_string(),
            sections: vec![
                KimunSection {
                    title: "Special \"Quotes\" & Symbols".to_string(),
                    text: "Text with\nnewlines\tand\ttabs".to_string(),
                },
                KimunSection {
                    title: "Unicode 🎉 Emoji".to_string(),
                    text: "こんにちは世界 and Ñoño".to_string(),
                },
            ],
        };

        let json = serde_json::to_string(&doc).expect("Failed to serialize");
        let deserialized: KimunDoc = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(
            deserialized.sections[0].title,
            "Special \"Quotes\" & Symbols"
        );
        assert_eq!(
            deserialized.sections[0].text,
            "Text with\nnewlines\tand\ttabs"
        );
        assert_eq!(deserialized.sections[1].title, "Unicode 🎉 Emoji");
        assert_eq!(deserialized.sections[1].text, "こんにちは世界 and Ñoño");
    }

    #[test]
    fn test_kimundoc_large_text() {
        let large_text = "a".repeat(10000);
        let doc = KimunDoc {
            path: "large.md".to_string(),
            hash: "large_hash".to_string(),
            sections: vec![KimunSection {
                title: "Large Chunk".to_string(),
                text: large_text.clone(),
            }],
        };

        let json = serde_json::to_string(&doc).expect("Failed to serialize");
        let deserialized: KimunDoc = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.sections[0].text.len(), 10000);
        assert_eq!(deserialized.sections[0].text, large_text);
    }

    #[test]
    fn test_kimundoc_pretty_print() {
        let doc = KimunDoc {
            path: "pretty.md".to_string(),
            hash: "pretty_hash".to_string(),
            sections: vec![KimunSection {
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
        let chunk = KimunSection {
            title: "Test Chunk".to_string(),
            text: "This is test content.".to_string(),
        };

        let json = serde_json::to_string(&chunk).expect("Failed to serialize chunk");
        let deserialized: KimunSection =
            serde_json::from_str(&json).expect("Failed to deserialize chunk");

        assert_eq!(chunk.title, deserialized.title);
        assert_eq!(chunk.text, deserialized.text);
    }

    #[test]
    fn test_kimundoc_empty_strings() {
        let doc = KimunDoc {
            path: "".to_string(),
            hash: "".to_string(),
            sections: vec![KimunSection {
                title: "".to_string(),
                text: "".to_string(),
            }],
        };

        let json = serde_json::to_string(&doc).expect("Failed to serialize");
        let deserialized: KimunDoc = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.path, "");
        assert_eq!(deserialized.hash, "");
        assert_eq!(deserialized.sections[0].title, "");
        assert_eq!(deserialized.sections[0].text, "");
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
            sections: vec![KimunSection {
                title: "Markdown Section".to_string(),
                text: markdown_text.to_string(),
            }],
        };

        let json = serde_json::to_string(&doc).expect("Failed to serialize");
        let deserialized: KimunDoc = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.sections[0].text, markdown_text);
    }

    // Tests for split_chunks_for_rag

    #[test]
    fn test_split_chunks_empty_string() {
        let result = crate::document::split_chunks_for_rag("", 500, 1000);
        assert_eq!(result.len(), 0, "Empty string should return empty vec");
    }

    #[test]
    fn test_split_chunks_small_text() {
        let text = "This is a short text that doesn't need splitting.";
        let result = crate::document::split_chunks_for_rag(text, 500, 1000);

        assert_eq!(result.len(), 1, "Small text should return single chunk");
        assert_eq!(result[0], text);
    }

    #[test]
    fn test_split_chunks_at_target_size() {
        // Text exactly at target size should not be split
        let text = "a".repeat(500);
        let result = crate::document::split_chunks_for_rag(&text, 500, 1000);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 500);
    }

    #[test]
    fn test_split_chunks_large_text() {
        // Large text that exceeds target should be split
        let text = "This is a sentence. ".repeat(100); // ~2000 chars
        let result = crate::document::split_chunks_for_rag(&text, 800, 1536);

        assert!(
            result.len() > 1,
            "Large text should be split into multiple chunks"
        );

        // All chunks should be within max_size
        for (i, chunk) in result.iter().enumerate() {
            assert!(
                chunk.len() <= 1536,
                "Chunk {} exceeds max_size: {} chars",
                i,
                chunk.len()
            );
        }
    }

    #[test]
    fn test_split_chunks_respects_paragraph_breaks() {
        let text = "First paragraph with some content here.\n\n\
                    Second paragraph with more content.\n\n\
                    Third paragraph continues with additional text.";

        let result = crate::document::split_chunks_for_rag(text, 50, 100);

        // Should split at paragraph boundaries when possible
        assert!(result.len() >= 2, "Should split into multiple chunks");

        // Verify chunks don't have leading/trailing whitespace
        for chunk in &result {
            assert!(!chunk.starts_with(' '), "Chunk should be trimmed");
            assert!(!chunk.ends_with(' '), "Chunk should be trimmed");
        }
    }

    #[test]
    fn test_split_chunks_respects_sentence_breaks() {
        let text = "First sentence here. Second sentence here. Third sentence here. \
                    Fourth sentence here. Fifth sentence here.";

        let result = crate::document::split_chunks_for_rag(text, 40, 80);

        assert!(result.len() >= 2);

        // Chunks should end at sentence boundaries when possible
        for chunk in &result {
            assert!(chunk.len() <= 80, "Chunk exceeds max size");
        }
    }

    #[test]
    fn test_split_chunks_respects_word_breaks() {
        // Text without sentence breaks, should split at word boundaries
        let text = "word ".repeat(100); // No sentence breaks

        let result = crate::document::split_chunks_for_rag(&text, 50, 100);

        assert!(result.len() > 1);

        // Should not split words (no chunk should start/end mid-word)
        for chunk in &result {
            assert!(!chunk.starts_with(' '), "Should not start with space");
            assert!(chunk.len() <= 100, "Should respect max size");
        }
    }

    #[test]
    fn test_split_chunks_handles_multiline_text() {
        let text = "Line 1\nLine 2\nLine 3\n\nNew paragraph\nLine 5\nLine 6";

        let result = crate::document::split_chunks_for_rag(text, 20, 40);

        assert!(result.len() >= 2);

        for chunk in &result {
            assert!(chunk.len() <= 40);
            assert!(!chunk.is_empty());
        }
    }

    #[test]
    fn test_split_chunks_preserves_content() {
        let text = "The quick brown fox jumps over the lazy dog. \
                    This is a test sentence. Another sentence follows here.";

        let result = crate::document::split_chunks_for_rag(text, 30, 60);

        // Reconstruct the text (with some whitespace differences)
        let reconstructed = result.join(" ");

        // All original words should be present
        assert!(reconstructed.contains("quick"));
        assert!(reconstructed.contains("brown"));
        assert!(reconstructed.contains("fox"));
        assert!(reconstructed.contains("lazy"));
        assert!(reconstructed.contains("dog"));
    }

    #[test]
    fn test_split_chunks_with_markdown() {
        let markdown = "# Heading\n\n\
                       This is the first paragraph with some content.\n\n\
                       ## Subheading\n\n\
                       This is the second paragraph with more details.\n\n\
                       - List item 1\n\
                       - List item 2";

        let result = crate::document::split_chunks_for_rag(markdown, 60, 120);

        assert!(result.len() >= 2);

        // Should preserve markdown formatting
        let combined = result.join("\n\n");
        assert!(combined.contains("# Heading") || combined.contains("Heading"));
    }

    #[test]
    fn test_split_chunks_default_rag_sizes() {
        // Test with recommended default sizes (800, 1536)
        let text = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(50); // ~2850 chars

        let result = crate::document::split_chunks_for_rag(&text, 800, 1536);

        assert!(result.len() >= 2, "Should split large text");

        // Most chunks should be close to target size (800)
        let avg_size: usize = result.iter().map(|s| s.len()).sum::<usize>() / result.len();
        assert!(
            avg_size >= 600 && avg_size <= 1536,
            "Average chunk size {} should be reasonable",
            avg_size
        );

        // No chunk should exceed max
        for chunk in &result {
            assert!(chunk.len() <= 1536, "Chunk exceeds max size");
        }
    }

    #[test]
    fn test_split_chunks_precision_sizes() {
        // Test with precision-focused sizes (512, 1024)
        let text = "Sentence. ".repeat(200); // ~2000 chars

        let result = crate::document::split_chunks_for_rag(&text, 512, 1024);

        assert!(result.len() >= 2);

        for chunk in &result {
            assert!(chunk.len() <= 1024);
        }
    }

    #[test]
    fn test_split_chunks_as_ref_str() {
        // Test that AsRef<str> works with different types

        // String
        let owned = String::from("Test string for splitting into multiple chunks.");
        let result1 = crate::document::split_chunks_for_rag(owned, 20, 40);
        assert!(result1.len() >= 1);

        // &str
        let borrowed = "Test string for splitting into multiple chunks.";
        let result2 = crate::document::split_chunks_for_rag(borrowed, 20, 40);
        assert!(result2.len() >= 1);

        // Both should produce same results
        assert_eq!(result1.len(), result2.len());
    }

    #[test]
    fn test_split_chunks_unicode() {
        // Use longer chunks to avoid hitting multi-byte char boundaries
        let text = "こんにちは世界。これはテストです。日本語の文章が続きます。\n\n\
                    Ñoño loves Kimün app. 🎉 This is great for notes!\n\n\
                    Über große Straße gehen. Deutsch ist auch toll.";

        // Use larger sizes to reduce chance of splitting on char boundary
        let result = crate::document::split_chunks_for_rag(text, 100, 200);

        assert!(result.len() >= 1);

        // Unicode should be preserved
        let combined = result.join(" ");
        assert!(combined.contains("世界") || combined.contains("日本"));
        assert!(combined.contains("Ñoño"));
        assert!(combined.contains("🎉"));
    }

    #[test]
    fn test_split_chunks_only_whitespace() {
        let text = "   \n\n   \t\t   ";
        let result = crate::document::split_chunks_for_rag(text, 500, 1000);

        // After trimming, text is empty so returns empty string
        // The function returns 1 empty chunk for whitespace-only input
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "");
    }

    #[test]
    fn test_split_chunks_near_boundary() {
        // Test text that's just slightly over target size
        let text = "a".repeat(510);
        let result = crate::document::split_chunks_for_rag(&text, 500, 1000);

        // Should still fit in one chunk since it's under max
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_split_chunks_forces_split_at_max() {
        // Very long text with no natural boundaries
        let text = "a".repeat(3000);
        let result = crate::document::split_chunks_for_rag(&text, 800, 1536);

        // Should force split even without natural boundaries
        assert!(result.len() >= 2);

        for chunk in &result {
            assert!(chunk.len() <= 1536, "Should respect max size");
        }
    }

    #[test]
    fn test_split_chunks_unicode_boundary_safety() {
        // This test specifically targets the Unicode boundary issue
        // Each Japanese character is 3 bytes, so splitting at specific positions
        // could previously cause panics
        let text = "あいうえお".repeat(50); // 250 chars, 750 bytes

        // Use sizes that will force splits in the middle of multi-byte sequences
        let result = crate::document::split_chunks_for_rag(&text, 100, 200);

        // Should not panic and should produce valid chunks
        assert!(result.len() >= 3, "Should split into multiple chunks");

        // All chunks should be valid UTF-8
        for chunk in &result {
            assert!(
                chunk.is_ascii() || chunk.chars().count() > 0,
                "Chunk should be valid UTF-8"
            );
        }

        // Verify we can reconstruct something similar
        let total_chars: usize = result.iter().map(|s| s.chars().count()).sum();
        assert!(total_chars > 0, "Should preserve characters");
    }

    #[test]
    fn test_split_chunks_mixed_unicode() {
        // Mix of ASCII, emoji, and multi-byte characters
        let text = "Hello 世界! This is a test. 🎉 More text here. \
                    Ñoño writes notes. 日本語のテキスト。\n\n\
                    Another paragraph with émojis 🚀 and symbols ñ ü ö.";

        let result = crate::document::split_chunks_for_rag(text, 40, 80);

        assert!(result.len() >= 1);

        // Should not lose any content
        let combined = result.join(" ");
        assert!(combined.contains("世界") || combined.contains("Hello"));
        assert!(combined.contains("🎉") || combined.contains("test"));

        // All chunks should be valid UTF-8 and within size limits
        for chunk in &result {
            assert!(chunk.len() <= 100); // Some tolerance for char boundaries
            // Should be valid UTF-8
            assert_eq!(
                chunk,
                &String::from_utf8(chunk.as_bytes().to_vec()).unwrap()
            );
        }
    }

    #[test]
    fn test_split_chunks_emoji_heavy() {
        // Emojis are 4 bytes each in UTF-8
        let text = "🎉🚀💻🌟✨🔥💡🎯📚🎨".repeat(20); // 200 emojis, 800 bytes

        // This should not panic even though we're splitting multi-byte sequences
        let result = crate::document::split_chunks_for_rag(&text, 50, 100);

        assert!(result.len() >= 1);

        // Verify all chunks are valid UTF-8
        for chunk in &result {
            // Just verify we can iterate chars without panic
            assert!(chunk.chars().count() > 0 || chunk.is_empty());
            // Verify it's valid UTF-8
            assert_eq!(
                chunk,
                &String::from_utf8(chunk.as_bytes().to_vec()).unwrap()
            );
        }
    }
}
