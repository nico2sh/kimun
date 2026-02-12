# Vector Database Options for Kimün RAG

This document describes the available vector database backends for the Kimün RAG server.

## Recommended: Qdrant

**Status:** ✅ Production-ready

Qdrant is a high-performance vector database designed for similarity search. It's the recommended choice for production deployments.

### Setup

Using Docker (easiest):
```bash
docker run -p 6333:6333 -p 6334:6334 \
  -v $(pwd)/qdrant_storage:/qdrant/storage \
  qdrant/qdrant
```

Or install locally:
```bash
# See https://qdrant.tech/documentation/quick-start/
```

### Configuration

In your `~/.config/kimun/rag.conf`:
```toml
[vector_db]
type = "qdrant"
url = "http://localhost:6333"
collection = "kimun_embeddings"
```

### Features
- High performance for large datasets
- Excellent scalability
- Built-in filtering and metadata support
- Can be deployed locally or in the cloud
- Active development and community

## SQLite with sqlite-vec

**Status:** ⚠️ Being deprecated

A simpler, file-based vector database using SQLite with the sqlite-vec extension.

### Configuration

```toml
[vector_db]
type = "sqlite"
db_path = "./rag_index.sqlite"
```

### Deprecation Notice

SQLite vector storage is being deprecated in favor of Qdrant. While it still works, we recommend migrating to Qdrant for:
- Better performance with large datasets
- More advanced filtering capabilities
- Production-grade reliability

## LanceDB

**Status:** 🚧 Temporarily disabled

LanceDB implementation exists but is currently disabled due to a dependency compatibility issue.

### Issue Details

The `lance-index` v2.0.0 crate has a dependency on an older version of the `tempfile` crate that conflicts with other dependencies. Specifically, it uses `tempfile::TempDir::keep()` which was removed in `tempfile` v3.19+.

### Implementation Status

The LanceDB implementation is complete and ready to use at [`src/dbembeddings/veclancedb.rs`](src/dbembeddings/veclancedb.rs). It includes:
- Local file-based storage
- Full `Embeddings` trait implementation
- Incremental indexing support
- Content hash tracking

### Configuration (when enabled)

```toml
[vector_db]
type = "lancedb"
db_path = "./lance_db"
table_name = "kimun_embeddings"
```

### Resolution

Waiting for LanceDB to update to `lance` v3.0.0+ which resolves this dependency issue. Once the upstream dependency is fixed, simply uncomment the `lancedb` dependency in `Cargo.toml` and the module export in `src/dbembeddings/mod.rs`.

### Workaround

In the meantime, use Qdrant (recommended) or SQLite.

## Migration Between Databases

To migrate between vector databases:

1. Export your notes using `/api/index/all`
2. Stop the RAG server
3. Update your `rag.conf` to use the new database type
4. Start the RAG server
5. Re-index your notes using `/api/index/all`

The incremental indexing system will handle content hash tracking automatically for each backend.
