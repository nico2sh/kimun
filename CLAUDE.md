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
   - Vector storage: SQLite (`vecsqlite.rs`) or Qdrant (`vecqdrant.rs`)
   - Content hash-based incremental indexing to avoid reprocessing unchanged notes

2. **LLM Clients** 
   - Unified `LLMClient` trait for multiple providers
   - Supported: Claude (`claude.rs`), OpenAI (`openai.rs`), Gemini (`gemini.rs`), Mistral (`mistral.rs`)
   - API keys from environment variables or `X-API-Key` header

3. **Reranker** 
   - BGE Reranker Base cross-encoder
   - Optional post-processing to improve top-k results
   - ~15-30% quality improvement with ~100ms latency

4. **HTTP Handlers** 
   - Async job queue for long-running operations
   - Job status tracking via UUID
   - RESTful JSON API

**Configuration:**
RAG server requires `~/.config/kimun/rag.conf` (copy from [rag/config.example.toml](rag/config.example.toml))

**API Endpoints:**
- `POST /api/index/all` - Index all notes from vault
- `POST /api/index/single` - Index single note with chunks
- `POST /api/embeddings` - Semantic search (no LLM)
- `POST /api/answer` - LLM-powered answer with sources
  - Supports dynamic LLM selection via `llm_provider` and `llm_model` in body
  - Override API key via `X-API-Key` header
- `GET /api/job/{job_id}` - Check async job status

**Data Flow:**

Indexing:
```
Vault DB → ChunkLoader → FastEmbed → Vector DB
                                   ↓
                         Content Hash → Indexed Notes Table
```

Querying:
```
Query → FastEmbed → Vector DB (similarity search)
      ↓
Top 128 results → Reranker (optional) → Top 20 results
      ↓
LLM Client → Answer with sources
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
# 1. Set up config
mkdir -p ~/.config/kimun
cp rag/config.example.toml ~/.config/kimun/rag.conf
# Edit rag.conf: set vault.path to your notes directory

# 2. Export API key for your chosen LLM
export ANTHROPIC_API_KEY=sk-ant-...  # for Claude
export OPENAI_API_KEY=sk-...         # for OpenAI
export GEMINI_API_KEY=...            # for Gemini
export MISTRAL_API_KEY=...           # for Mistral

# 3. Run server
cd rag && cargo run --bin rag-server

# 4. Index notes
curl -X POST http://localhost:7573/api/index/all

# 5. Query
curl -X POST http://localhost:7573/api/answer \
  -H "Content-Type: application/json" \
  -d '{"query": "What are my notes about?"}'
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
