# Kimun RAG Server

A high-performance Retrieval-Augmented Generation (RAG) server for querying your personal knowledge base with semantic search and AI-powered answers.

## Features

- **Multiple Vector Databases**: Support for SQLite (with sqlite-vec) and Qdrant
- **Multiple LLM Providers**: Gemini, Claude, OpenAI, and Mistral
- **Cross-Encoder Reranking**: Improved search relevance using BGE Reranker
- **Incremental Indexing**: Content hash-based change detection to avoid reindexing
- **Async Job Queue**: Background processing for indexing and query operations
- **RESTful API**: Clean HTTP endpoints for all operations

## Examples

### Using different LLMs for different queries

```bash
# Use Claude for complex reasoning (with custom API key)
curl -X POST http://localhost:8080/api/answer \
  -H "Content-Type: application/json" \
  -H "X-API-Key: sk-ant-your-key-here" \
  -d '{
    "query": "Compare and contrast the architectural patterns in my notes",
    "llm_provider": "claude",
    "llm_model": "claude-3-5-sonnet-20241022"
  }'

# Use GPT-4o-mini for quick, simple questions
curl -X POST http://localhost:8080/api/answer \
  -H "Content-Type: application/json" \
  -H "X-API-Key: sk-your-openai-key" \
  -d '{
    "query": "What is the main topic of note X?",
    "llm_provider": "openai",
    "llm_model": "gpt-4o-mini"
  }'

# Use Gemini for cost-effective queries
curl -X POST http://localhost:8080/api/answer \
  -H "Content-Type: application/json" \
  -H "X-API-Key: your-gemini-key" \
  -d '{
    "query": "List all the projects mentioned in my notes",
    "llm_provider": "gemini",
    "llm_model": "gemini-2.5-flash"
  }'
```

## Quick Start

### 1. Configuration

Copy the example configuration:

```bash
mkdir -p ~/.config/kimun
cp config.example.toml ~/.config/kimun/rag.conf
```

Edit `~/.config/kimun/rag.conf`:

```toml
[server]
host = "127.0.0.1"
port = 8080

[vault]
path = "/path/to/your/notes"

[vector_db]
type = "sqlite"
db_path = "./rag_index.sqlite"

[llm]
provider = "claude"
model = "claude-3-5-sonnet-20241022"

[reranker]
enabled = true
top_k = 20
```

### 2. Set API Keys

Export the appropriate API key for your chosen LLM provider:

```bash
# For Claude
export ANTHROPIC_API_KEY=sk-ant-...

# For OpenAI
export OPENAI_API_KEY=sk-...

# For Gemini
export GEMINI_API_KEY=...

# For Mistral
export MISTRAL_API_KEY=...
```

### 3. Run the Server

```bash
cargo run --bin rag-server
```

Or with custom configuration:

```bash
cargo run --bin rag-server -- --config /path/to/config.toml --port 3000
```

## API Endpoints

### Health Check

```bash
GET /health
```

Returns: `OK`

### Index All Notes

Index all notes from your vault database:

```bash
POST /api/index/all
```

Response:
```json
{
  "job_id": "uuid",
  "message": "Indexing job started"
}
```

### Index Single Note

Replace all chunks for a specific note:

```bash
POST /api/index/single
Content-Type: application/json

{
  "path": "/path/to/note.md",
  "chunks": [
    {
      "content": "Note content here",
      "title": "Note Title",
      "date": "2024-01-22"  // Optional, YYYY-MM-DD format
    }
  ]
}
```

Response:
```json
{
  "job_id": "uuid",
  "message": "Successfully indexed path /path/to/note.md"
}
```

### Get Embeddings

Query for similar chunks (no LLM, just semantic search):

```bash
POST /api/embeddings
Content-Type: application/json

{
  "query": "What are RAG systems?"
}
```

Response:
```json
{
  "chunks": [
    {
      "path": "/notes/ai.md",
      "title": "AI Concepts",
      "date": "2024-01-15",
      "content": "RAG systems combine retrieval and generation...",
      "similarity_score": 0.89
    }
  ]
}
```

### Answer with LLM

Get an AI-generated answer based on your notes. You can optionally specify which LLM provider to use for this specific request:

```bash
POST /api/answer
Content-Type: application/json

{
  "query": "Explain RAG systems"
}
```

Or with dynamic LLM selection (provider and model in body, API key in header):

```bash
POST /api/answer
Content-Type: application/json
X-API-Key: sk-ant-your-api-key-here

{
  "query": "Explain RAG systems",
  "llm_provider": "claude",
  "llm_model": "claude-3-5-sonnet-20241022"
}
```

**Request Parameters:**
- **Body:**
  - `query` (required): The question to answer
  - `llm_provider` (optional): Which LLM to use - `"claude"`, `"openai"`, `"gemini"`, or `"mistral"`
  - `llm_model` (optional): Specific model name
- **Headers:**
  - `X-API-Key` (optional): Override the default API key for this request

**Supported LLM providers:**
- `"claude"` - Models: `claude-3-5-sonnet-20241022`, `claude-3-opus-20240229`, etc.
- `"openai"` - Models: `gpt-4o-mini`, `gpt-4o`, `gpt-4-turbo`, etc.
- `"gemini"` - Models: `gemini-2.5-flash`, `gemini-1.5-pro`, etc.
- `"mistral"` - Uses `mistral-large-latest`

Response:
```json
{
  "job_id": "uuid",
  "message": "Query job started"
}
```

**Use cases for dynamic LLM selection:**
- Test different models without restarting the server
- Use cheaper models for simple queries, powerful models for complex ones
- Multi-tenant scenarios with per-user API keys (via `X-API-Key` header)
- A/B testing different LLM providers

**Security note:** API keys are passed via the `X-API-Key` header (not in the request body) to:
- Prevent keys from appearing in logs that might record request bodies
- Follow HTTP best practices for authentication credentials
- Enable easier filtering in proxies and middleware

### Check Job Status

Check the status of an async job:

```bash
GET /api/job/{job_id}
```

Response:
```json
{
  "job_id": "uuid",
  "status": "completed",  // or "queued", "processing", "failed"
  "result": {
    // Job-specific result data
  },
  "error": null
}
```

For indexing jobs:
```json
{
  "result": {
    "indexed": 10,
    "skipped": 5,
    "updated": 0,
    "errors": 0
  }
}
```

For answer jobs:
```json
{
  "result": {
    "answer": "RAG systems are...",
    "sources": [...]
  }
}
```

## Configuration Options

### Vector Database

#### SQLite (Local, file-based)
```toml
[vector_db]
type = "sqlite"
db_path = "./rag_index.sqlite"
```

#### Qdrant (Standalone server)
```toml
[vector_db]
type = "qdrant"
url = "http://localhost:6333"
collection = "kimun_embeddings"
```

To run Qdrant with Docker:
```bash
docker-compose up -d
```

### LLM Providers

#### Claude (Recommended for quality)
```toml
[llm]
provider = "claude"
model = "claude-3-5-sonnet-20241022"
```

#### OpenAI (Cost-effective)
```toml
[llm]
provider = "openai"
model = "gpt-4o-mini"
```

#### Gemini (Most cost-effective)
```toml
[llm]
provider = "gemini"
model = "gemini-2.5-flash-preview-04-17"
```

#### Mistral
```toml
[llm]
provider = "mistral"
model = "mistral-large-latest"
```

### Reranking

Cross-encoder reranking improves search quality by 15-30% but adds ~100ms latency:

```toml
[reranker]
enabled = true  # Set to false to disable
top_k = 20      # Number of results after reranking
```

## Architecture

### Components

1. **Embeddings Layer** (`dbembeddings/`)
   - FastEmbed with BGE-Large-EN-V15 model (1024 dimensions)
   - SQLite or Qdrant for vector storage
   - Content hash-based incremental indexing

2. **LLM Clients** (`llmclients/`)
   - Unified interface for multiple providers
   - Consistent prompt formatting
   - Environment-based API key management

3. **Reranker** (`reranker.rs`)
   - BGE Reranker Base cross-encoder model
   - Improves initial vector search results
   - Configurable top-k filtering

4. **HTTP Handlers** (`handlers.rs`)
   - Axum-based async HTTP server
   - Job tracking for async operations
   - JSON request/response format

### Data Flow

1. **Indexing**:
   ```
   Vault DB → ChunkLoader → FastEmbed → Vector DB
                                      ↓
                              Content Hash → Indexed Notes Table
   ```

2. **Querying**:
   ```
   Query → FastEmbed → Vector DB (similarity search)
         ↓
   Top 128 results → Reranker (optional) → Top 20 results
         ↓
   LLM Client → Answer with sources
   ```

## Performance

- **Embedding generation**: ~50-100ms per chunk (BGE-Large)
- **Vector search**: <10ms (SQLite), <5ms (Qdrant)
- **Reranking**: ~100ms for 128→20 results
- **LLM response**: 500-2000ms (provider-dependent)

### Optimization Tips

1. **Use Qdrant for >10k notes** - Better performance at scale
2. **Enable reranking** - Significantly improves answer quality
3. **Adjust top_k** - Lower values = faster, higher = more context
4. **SQLite for development** - Simpler setup, good for <10k notes

## Development

### Building

```bash
cargo build --release --bin rag-server
```

### Testing

```bash
# Run unit tests
cargo test

# Run integration tests
cargo test --test '*'
```

### Adding a New LLM Provider

1. Create `src/llmclients/yourprovider.rs`
2. Implement the `LLMClient` trait
3. Add to `src/llmclients/mod.rs`
4. Update `config.rs` with new provider enum
5. Wire up in `rag-server.rs`

### Adding a New Vector Database

1. Create `src/dbembeddings/vecyourdb.rs`
2. Implement the `Embeddings` trait
3. Handle index tracking methods
4. Update config and server initialization

## Troubleshooting

### "Vault database not found"

Ensure your vault path points to a directory containing `kimun.sqlite` with tables:
- `notes` (columns: path, title, date)
- `notesContent` (columns: path, content)

### "Failed to download model"

First run downloads embedding models (~500MB) and reranker model (~300MB). Ensure:
- Stable internet connection
- ~1GB free disk space
- Write permissions in cache directory

### "API key not found"

Export the appropriate environment variable before starting the server:
```bash
export ANTHROPIC_API_KEY=your-key-here
```

### High memory usage

The embedding models load into memory (~1.5GB total). This is normal. To reduce:
- Disable reranking (`enabled = false`)
- Use smaller batch sizes for indexing

## License

See LICENSE file in the repository root.

## Contributing

Contributions welcome! Please:
1. Follow existing code style
2. Add tests for new features
3. Update documentation
4. Run `cargo fmt` and `cargo clippy`
