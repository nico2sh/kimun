# Chunk Splitting for RAG - Usage Examples

The `split_chunks_for_rag` function takes a `KimunDoc` (which already contains chunks from markdown sections) and splits them into smaller, more manageable pieces optimized for embedding generation in RAG systems.

## Basic Usage

```rust
use kimun_rag::{KimunDoc, Chunk, split_chunks_for_rag};

// Create a document with large chunks
let doc = KimunDoc {
    path: "notes/ai-research.md".to_string(),
    hash: "abc123def456".to_string(),
    chunks: vec![
        Chunk {
            title: "Introduction > Background".to_string(),
            text: "A very long piece of text that spans multiple paragraphs...\n\n\
                   This could be several thousand characters...\n\n\
                   And might be too large for efficient embedding...".to_string(),
        }
    ],
};

// Split into smaller chunks optimized for embeddings
// Target: 800 chars (~200 tokens, ideal for BGE-Large-EN-V1.5)
// Max: 1536 chars (~384 tokens, well under the 512 token limit)
let optimized_doc = split_chunks_for_rag(doc, 800, 1536);

// Now optimized_doc.chunks contains smaller, more uniform pieces
// Each preserves the original title for context
```

## Recommended Sizes for Different Embedding Models

### BGE-Large-EN-V1.5 (default in Kimun) - RECOMMENDED
```rust
let optimized = split_chunks_for_rag(doc, 800, 1536);  // Default in ChunkLoader
```
- Token limit: 512 tokens
- **Target: 800 chars ≈ 200 tokens** - Optimal balance of context and precision
- **Max: 1536 chars ≈ 384 tokens** - Safely under model limit
- **Why**: Research shows 256-384 tokens is the sweet spot for RAG retrieval quality

### Precision-focused (when you need very specific retrieval)
```rust
let optimized = split_chunks_for_rag(doc, 512, 1024);
```
- Target: 512 chars ≈ 128 tokens
- Max: 1024 chars ≈ 256 tokens
- **Use when**: Answering very specific questions that need pinpoint accuracy

### OpenAI text-embedding-3-small/large
```rust
let optimized = split_chunks_for_rag(doc, 1024, 2048);
```
- Token limit: 8191 tokens (very high)
- Recommended: Keep chunks smaller for better semantic coherence

### Cohere embed-multilingual-v3
```rust
let optimized = split_chunks_for_rag(doc, 800, 1536);
```
- Token limit: 512 tokens
- Same as BGE recommendation

## Splitting Strategy

The function uses an intelligent splitting strategy with priorities:

1. **Paragraph breaks** (`\n\n`) - Highest priority for semantic coherence
2. **Sentence boundaries** (`.`, `!`, `?`, `\n`) - Maintains complete thoughts
3. **Word boundaries** (` `) - Prevents splitting words
4. **Hard split** - Forces split at `max_size` if no natural boundary found

### Example: Paragraph-Aware Splitting

```rust
let doc = KimunDoc {
    path: "article.md".to_string(),
    hash: "xyz789".to_string(),
    chunks: vec![
        Chunk {
            title: "Main Content".to_string(),
            text: "First paragraph with several sentences. More content here.\n\n\
                   Second paragraph is completely separate. Different topic.\n\n\
                   Third paragraph continues...".to_string(),
        }
    ],
};

// Will prefer splitting at \n\n boundaries
let result = split_chunks_for_rag(doc, 60, 120);

// Result chunks will contain complete paragraphs when possible
```

## Handling Multiple Chunks

The function preserves all chunks (fixing a bug in the original implementation):

```rust
let doc = KimunDoc {
    path: "notes.md".to_string(),
    hash: "multi123".to_string(),
    chunks: vec![
        Chunk {
            title: "Section 1".to_string(),
            text: "Content for section 1...".to_string(),
        },
        Chunk {
            title: "Section 2".to_string(),
            text: "Content for section 2...".to_string(),
        },
        Chunk {
            title: "Section 3".to_string(),
            text: "Content for section 3...".to_string(),
        },
    ],
};

let result = split_chunks_for_rag(doc, 512, 1024);

// All sections are processed and their chunks are included
// Each maintains its original title for context
```

## Integration with RAG Pipeline

### Before Indexing

```rust
use kimun_rag::{KimunDoc, split_chunks_for_rag};

// Load documents from vault (already chunked by markdown sections)
let docs: Vec<KimunDoc> = load_from_vault()?;

// Further split for optimal embedding size
let optimized_docs: Vec<KimunDoc> = docs
    .into_iter()
    .map(|doc| split_chunks_for_rag(doc, 512, 1024))
    .collect();

// Now index the optimized documents
for doc in optimized_docs {
    embeddings_db.index_document(doc).await?;
}
```

### With the Existing ChunkLoader

The `ChunkLoader::load_notes()` method already calls this function internally with optimal defaults:

```rust
use kimun_rag::document::ChunkLoader;

let loader = ChunkLoader::new(vault_path);
let docs = loader.load_notes()?;  // Already split with target=800, max=1536 (optimal!)
```

To customize the chunking size for special use cases:

```rust
let raw_docs = load_raw_from_db()?;
let custom_docs: Vec<KimunDoc> = raw_docs
    .into_iter()
    .map(|doc| split_chunks_for_rag(doc, 512, 1024))  // Smaller for precision
    .collect();
```

## Performance Considerations

- **Small chunks** (256-512 chars): Better semantic focus, but more API calls for embedding
- **Medium chunks** (512-1024 chars): Good balance for most use cases
- **Large chunks** (1024-2048 chars): Fewer API calls, but may lose semantic precision

Choose based on your:
- Embedding model token limits
- Cost considerations (API calls)
- Semantic precision requirements
- Query specificity

## Common Patterns

### Balanced RAG (default - RECOMMENDED)
```rust
let doc = split_chunks_for_rag(doc, 800, 1536);
```
Best overall choice for most use cases.

### Precision-focused RAG (specific queries)
```rust
let doc = split_chunks_for_rag(doc, 512, 1024);
```
When you need pinpoint accuracy for very specific questions.

### Context-heavy RAG (broad questions)
```rust
let doc = split_chunks_for_rag(doc, 1024, 2048);
```
When questions need more surrounding context, but approaching token limits.
