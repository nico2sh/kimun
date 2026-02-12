# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Kimün is a notes application focused on simplicity and powerful searchability. The codebase consists of three main components:

1. **Core Library** ([core/](core/)) - Shared business logic for note management, file system operations, and database indexing
2. **Desktop App** ([desktop/](desktop/)) - Tauri-based Dioxus UI application for note-taking
3. **RAG Server** ([rag/](rag/)) - Standalone HTTP server for semantic search and AI-powered Q&A

## Building and Running

### Desktop Application

```bash
# Development mode
cd desktop
dx serve

# Production build
cd desktop
dx build --release

# Production bundle for distribution
cd desktop
dx bundle --release
```

### RAG Server

```bash
# Run with default config (~/.config/kimun/rag.conf)
cd rag
cargo run --bin rag-server

# Run with custom config and port
cargo run --bin rag-server -- --config /path/to/config.toml --port 3000

# Build release binary
cargo build --release --bin rag-server
```

### Core Library

```bash
cd core
cargo test
cargo build --release
```

## Architecture

### Core Library (`kimun_core`)

The core library is the foundation shared by both the desktop app and RAG server.

**Key modules:**
- `NoteVault` - Main entry point managing the vault lifecycle, initialization, and indexing
- `VaultDB` - SQLite-based database storing note metadata and content in `kimun.sqlite`
  - `notes` table: path, title, date
  - `notesContent` table: path, content
- `nfs` module - File system abstraction layer with `VaultPath` and `VaultEntry` types for path management
   - `VaultPath` defines the location in disk for any note file or directory
   - `VaultEntry` is the type of entry you could find in a `VaultPath`, either a note, a directory or an attachment
- `note` module - Note parsing, content chunking, and metadata extraction

**Database Schema:**
The vault DB (`kimun.sqlite`) must exist at the workspace root with:
- `notes(path TEXT PRIMARY KEY, title TEXT, date TEXT)`
- `notesContent(path TEXT PRIMARY KEY, content TEXT)`

### Desktop Application

Built with Dioxus (React-like framework for Rust) in a component-based architecture.

**Application State Management:**
- `AppState` - Global app state (current path, preview mode, browser visibility)
- `EditorState` - Editor-specific state
- `PubSub<GlobalEvent>` - Event bus for cross-component communication
- `FocusManager` - UI Focus state tracking

**Key Components:**
- Search box with fuzzy matching using `nucleo` crate
- Note browser with file tree navigation
- Preview pane with Markdown rendering
- Editor with syntax highlighting via `syntect`

**Search Syntax:**
- Free text: case-insensitive with `*` wildcards
- File filtering: `@filename` or `at:filename`
- Section filtering: `>section` or `in:section`
- Directory filtering: `/directory` 
- Combined: `@tasks >personal kimun`

### RAG Server

Async HTTP server (Axum) providing semantic search and LLM-powered answers over notes.

**Architecture Layers:**

1. **Embeddings Layer**
   - FastEmbed with BGE-Large-EN-V15 (1024 dimensions)
   - Vector storage: Qdrant (`vecqdrant.rs`, recommended) or SQLite (`vecsqlite.rs`, deprecated)
   - LanceDB (`veclancedb.rs`) implementation exists but currently disabled due to dependency issue (waiting for lance v3.0.0+)
   - Content hash-based incremental indexing to avoid reprocessing unchanged notes

2. **LLM Clients** 
   - Unified `LLMClient` trait for multiple providers
   - Supported: Claude (`claude.rs`), OpenAI (`openai.rs`), Gemini (`gemini.rs`), Mistral (`mistral.rs`)
   - API keys from environment variables or `X-API-Key` header

3. **Reranker**
   - BGE Reranker Base cross-encoder
   - Optional post-processing to improve result relevance
   - Dynamic context window sizing (10/20/40 results based on request parameter)
   - ~15-30% quality improvement with ~100ms latency

4. **HTTP Handlers** 
   - Async job queue for long-running operations
   - Job status tracking via UUID
   - RESTful JSON API

**Configuration:**
RAG server requires `~/.config/kimun/rag.conf` (copy from [rag/config.example.toml](rag/config.example.toml))

**Note:** The global `reranker.top_k` config setting can be overridden per-request using the `context_size` parameter in `/api/answer` calls.

**API Endpoints:**
- `POST /api/index/all` - Index all notes from vault
- `POST /api/index/single` - Index single note with chunks
- `POST /api/index/docs` - Index a list of documents with chunks (accepts `Vec<KimunDoc>`)
- `POST /api/embeddings` - Semantic search (no LLM)
- `POST /api/answer` - LLM-powered answer with sources
  - Supports dynamic LLM selection via `llm_provider` and `llm_model` in body
  - Override API key via `X-API-Key` header
  - Control context window size via `context_size` parameter (`"small"`, `"medium"`, `"large"`)
- `GET /api/job/{job_id}` - Check async job status

**Context Window Sizes:**
The `/api/answer` endpoint supports configurable context window sizes:

| Size | Documents | Use Case | Performance |
|------|-----------|----------|-------------|
| `"small"` | 10 | Quick, focused answers | Fastest response |
| `"medium"` | 20 | Balanced (default) | Standard response |
| `"large"` | 40 | Comprehensive analysis | Slower, more thorough |

**Request Parameters for `/api/answer`:**
```json
{
  "query": "string (required)",
  "llm_provider": "string (optional) - claude, openai, gemini, mistral",
  "llm_model": "string (optional) - provider-specific model name",
  "context_size": "string (optional) - small, medium (default), large"
}
```

**Data Flow:**

Indexing:
```
Vault DB → ChunkLoader → FastEmbed → Vector DB
                                   ↓
                         Content Hash → Indexed Notes Table
```

Querying:
```
Query + context_size → FastEmbed → Vector DB (similarity search)
                    ↓
                Top 128 results → Reranker (optional) → Top N results (10/20/40)
                                 ↓
                            LLM Client → Answer with sources
```

**Note:** The final context window size is determined by the `context_size` parameter:
- `"small"` → 10 results
- `"medium"` → 20 results (default)
- `"large"` → 40 results

## Context Window Sizing

The RAG server supports dynamic context window sizing on a per-request basis through the `context_size` parameter in the `/api/answer` endpoint. This allows you to balance between response speed and comprehensiveness based on your specific needs.

### Choosing Context Size

**Small (`"small"` - 10 documents):**
- **Best for:** Quick questions, simple lookups, when you need fast responses
- **Performance:** Fastest response time, lowest token usage
- **Use cases:** "What's the definition of X?", "When did Y happen?", quick factual queries

**Medium (`"medium"` - 20 documents, default):**
- **Best for:** Most general-purpose queries, balanced performance
- **Performance:** Standard response time, balanced quality vs. speed
- **Use cases:** General questions, explanations, moderate complexity analysis
- **Note:** This is the default if no `context_size` is specified (backward compatibility)

**Large (`"large"` - 40 documents):**
- **Best for:** Complex analysis, comprehensive summaries, when you need thorough coverage
- **Performance:** Slower response, higher quality and completeness
- **Use cases:** "Summarize everything about topic X", complex research queries, detailed analysis

### Implementation Details

- The `context_size` parameter is applied after vector similarity search and deduplication
- When reranking is enabled, the BGE reranker processes all results and returns the top N based on your chosen size
- When reranking is disabled, the top N results from vector search are used directly
- The feature is backward compatible - omitting `context_size` defaults to "medium" (20 documents)
- Context size affects both the information sent to the LLM and the sources returned in the response

### Examples by Use Case

**Quick Fact Lookup (Small):**
```bash
curl -X POST http://localhost:7573/api/answer \
  -H "Content-Type: application/json" \
  -d '{"query": "What is the password for the staging environment?", "context_size": "small"}'
```

**General Question (Medium):**
```bash
curl -X POST http://localhost:7573/api/answer \
  -H "Content-Type: application/json" \
  -d '{"query": "How do I set up the development environment?", "context_size": "medium"}'
```

**Comprehensive Analysis (Large):**
```bash
curl -X POST http://localhost:7573/api/answer \
  -H "Content-Type: application/json" \
  -d '{"query": "Summarize all the architecture decisions and trade-offs", "context_size": "large"}'
```

## Development Workflows

### Running Tests

```bash
# Core library tests
cd core && cargo test

# RAG server tests
cd rag && cargo test

# Desktop tests
cd desktop && cargo test
```

### Testing RAG Server Locally

```bash
# 1. Start Qdrant (recommended vector database)
# Using Docker:
docker run -p 6333:6333 -p 6334:6334 -v $(pwd)/qdrant_storage:/qdrant/storage qdrant/qdrant
# Or install and run Qdrant locally: https://qdrant.tech/documentation/quick-start/

# 2. Set up config
mkdir -p ~/.config/kimun
cp rag/config.example.toml ~/.config/kimun/rag.conf
# Edit rag.conf:
#   - Set vault.path to your notes directory
#   - Ensure vector_db type is set to "qdrant"

# 3. Export API key for your chosen LLM
export ANTHROPIC_API_KEY=sk-ant-...  # for Claude
export OPENAI_API_KEY=sk-...         # for OpenAI
export GEMINI_API_KEY=...            # for Gemini
export MISTRAL_API_KEY=...           # for Mistral

# 4. Run server
cd rag && cargo run --bin rag-server

# 5. Index notes
curl -X POST http://localhost:7573/api/index/all

# 6. Query (basic)
curl -X POST http://localhost:7573/api/answer \
  -H "Content-Type: application/json" \
  -d '{"query": "What are my notes about?"}'

# Query with different context sizes
curl -X POST http://localhost:7573/api/answer \
  -H "Content-Type: application/json" \
  -d '{"query": "What are my productivity tips?", "context_size": "small"}'

curl -X POST http://localhost:7573/api/answer \
  -H "Content-Type: application/json" \
  -d '{"query": "What are my productivity tips?", "context_size": "medium"}'

curl -X POST http://localhost:7573/api/answer \
  -H "Content-Type: application/json" \
  -d '{"query": "What are my productivity tips?", "context_size": "large"}'

# Query with specific LLM and context size
curl -X POST http://localhost:7573/api/answer \
  -H "Content-Type: application/json" \
  -d '{"query": "Summarize my meeting notes", "llm_provider": "claude", "llm_model": "claude-3-5-sonnet-20241022", "context_size": "large"}'
```

### Adding a New LLM Provider to RAG

1. Create `rag/src/llmclients/yourprovider.rs` implementing the `LLMClient` trait
2. Add to `rag/src/llmclients/mod.rs` exports
3. Update `rag/src/config.rs` enum `LLMProvider`
4. Wire up in `rag/src/bin/rag-server.rs` `create_rag_from_config()`
5. Update `rag/config.example.toml` with example configuration

### Adding a New Vector Database to RAG

1. Create `rag/src/dbembeddings/vecyourdb.rs` implementing the `Embeddings` trait
2. Implement all trait methods including incremental indexing support
3. Update `rag/src/dbembeddings/mod.rs`
4. Add config type to `rag/src/config.rs` `VectorDBConfig` enum
5. Wire up in `rag/src/bin/rag-server.rs` `create_rag_from_config()`

## Project Conventions

- Rust edition 2021 (core/desktop) and 2024 (rag)
- Use `VaultPath` type (not raw `PathBuf`) for all vault-relative paths
- Database operations in core use rusqlite with bundled SQLite
- Desktop uses Dioxus signals for reactive state
- RAG uses Tokio async runtime throughout
- Error handling: `anyhow::Result` in RAG, custom error types (`VaultError`, `DBError`, `FSError`) in core
- Logging: `log` crate in core, `tracing` in RAG server
