# Kimün RAG Server

**Experimental.** An optional companion server that gives a Kimün vault semantic
search and question-answering. Kimün works fully without it; when it is reachable,
those extra capabilities light up.

## Model

The server is **push-only** (adr/0018): it never reads your notes from disk.
Kimün pushes note content to it and the server only stores embeddings and answers
queries. One server can host **many vaults** at once — each vault is a
**collection**, keyed by the vault's id (a stable UUID Kimün keeps under
`.kimun/` in the vault). Every `/api` request carries a `vault_id`.

```
Kimün  ──push docs / delete / query──▶  RAG server ──▶  vector DB (sqlite-vec | Qdrant)
                                                    └──▶  LLM (Claude | OpenAI | Gemini | Mistral)
```

## Quick start

```bash
mkdir -p ~/.config/kimun
cp config.example.toml ~/.config/kimun/rag.conf
# edit ~/.config/kimun/rag.conf (see Configuration)
cargo run --release --bin rag-server
```

Override host/port/config on the CLI:

```bash
cargo run --bin rag-server -- --config /path/to/rag.conf --host 0.0.0.0 --port 7573
```

First run downloads the embedding model (and the reranker, if enabled) — a few
hundred MB — unless you point `[embedder]` at an external service.

## Configuration

See `config.example.toml` for the annotated template. Sections:

- **`[server]`** — `host`, `port` (default `127.0.0.1:7573`), `max_concurrent_jobs`.
- **`[auth]`** — optional `token`. When set, every `/api` request must send
  `Authorization: Bearer <token>`; `/health` stays open. Required in practice
  once you bind beyond `127.0.0.1`.
- **`[vector_db]`** — `type = "sqlite"` (`db_path`) or `type = "qdrant"`
  (`url`, `collection` — used as a name **prefix**, so each vault's Qdrant
  collection is `<prefix>-<vault-id>`).
- **`[embedder]`** — how text becomes vectors. All collections share one
  embedder (the same model must embed documents and queries):
  - `type = "fastembed"` — local, no network. Optional `model` (e.g.
    `BGESmallENV15`, or a model code like `Xenova/bge-small-en-v1.5`);
    default is BGE-Large (1024 dims).
  - `type = "ollama"` — `url`, `model`, optional `doc_prefix`/`query_prefix`.
  - `type = "openai"` — any OpenAI-compatible `/embeddings` endpoint: `url`,
    `model`, optional `api_key`, `doc_prefix`/`query_prefix`.

  The vector width is detected automatically. **Changing the embedder or model
  invalidates all stored vectors** and forces a re-index — the store is
  recreated on the next start (sqlite) / the Qdrant collection must be dropped.
- **`[llm]`** — `provider` (`claude` | `openai` | `gemini` | `mistral`),
  `model`, optional `api_key`. The key is server-owned; if omitted it falls back
  to the provider's env var (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
  `GEMINI_API_KEY`, `MISTRAL_API_KEY`). Kimün never sends a key.
- **`[reranker]`** — `enabled` (default true), `top_k` (default result count,
  overridable per request via `context_size`).

## API

All `/api` routes require the bearer token when one is configured. `context_size`
is optional (`"small"` = 10, `"medium"` = 20, `"large"` = 40 results); omit it to
use the configured `reranker.top_k`.

### `GET /health`

Capability probe. Returns JSON:

```json
{ "status": "ok", "reranker": true, "llm_provider": "claude", "auth_required": true }
```

### `POST /api/index/docs`

Push a vault's documents. Runs in the background; returns a `job_id`.

```json
{
  "vault_id": "…",
  "docs": [
    { "path": "notes/example.md", "hash": "abc123",
      "sections": [ { "title": "Introduction", "text": "…" } ] }
  ]
}
```

Only documents whose `hash` differs from the server's are re-embedded; unchanged
ones are skipped.

### `POST /api/index/delete`

Remove notes from a vault's collection.

```json
{ "vault_id": "…", "paths": ["notes/example.md"] }
```

### `GET /api/collections/{vault_id}/hashes`

The `{ note-path: content-hash }` map the server holds for a vault. The client
diffs it against its own authoritative set to reconcile — pushing/deleting only
the differences.

### `POST /api/embeddings`

Semantic search (no LLM). Returns the matching chunks.

```json
{ "vault_id": "…", "query": "What are RAG systems?", "context_size": "medium" }
```

### `POST /api/answer`

LLM answer over the retrieved context, using the server-configured LLM. Runs in
the background; returns a `job_id`. Poll `/api/job/{job_id}` for the result
(`answer` + `sources`).

```json
{ "vault_id": "…", "query": "Explain RAG systems", "context_size": "large" }
```

### `GET /api/job/{job_id}`

Job status: `queued` | `processing` | `completed` (with `result`) | `failed`
(with `error`).

## Vector databases

- **SQLite (`sqlite-vec`)** — local, file-based, zero setup. Collections are
  isolated by a partition key. Good for small-to-medium vaults.
- **Qdrant** — a standalone server; one collection per vault. Better at scale.

## Development

```bash
cargo build --release --bin rag-server
cargo test
cargo clippy
cargo fmt
```

The vector-store SQL and the external embedders are unit-tested with a fake
embedder, so most tests run without downloading a model.

## License

See the LICENSE file in the repository root.
