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

The server is the `kimun_rag` crate (a member of the Kimün workspace). Run it
from the repo root with `-p kimun_rag`, or from this `rag/` directory:

```bash
mkdir -p ~/.config/kimun
cp rag/config.example.toml ~/.config/kimun/rag.conf
# edit ~/.config/kimun/rag.conf (see Configuration)
cargo run --release -p kimun_rag --bin rag-server
```

Override host/port/config on the CLI:

```bash
cargo run -p kimun_rag --bin rag-server -- --config /path/to/rag.conf --host 0.0.0.0 --port 7573
```

Open `http://127.0.0.1:7573/` for the [web UI](#web-ui); the API lives under
`/api` (see [API](#api)).

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
ones are skipped. The `hash` covers note **content**, not the chunking — so
changing the chunking/embedding logic won't re-embed existing notes on its own;
drop the store (or bump the embedder, which recreates it) to force a re-index.

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

## Web UI

The server also renders a small admin UI at its root (`http://<host>:<port>/`) —
server-rendered pages, no build step, no external assets. It offers:

- a **dashboard** of the running configuration (bind address, vector DB,
  embedder, LLM, reranker, auth);
- a **config** page to edit the LLM (provider/model/key), reranker, auth token,
  and bind address — changes are **written to the config file and applied on the
  next restart** (the embedder, vector store, and LLM client are built at
  startup, so the live instance is never mutated);
- a **collections** list with per-vault indexed-note counts;
- a **jobs** view (auto-refreshing) for indexing/answer jobs;
- a **test-query** box that runs a semantic search against a collection.

When an `[auth]` token is set, the UI requires signing in with it (the token is
kept in an `HttpOnly` session cookie); with no token it is open, same as the API.
The vector DB and embedder are not editable from the UI — change those in the
config file and restart, since altering them invalidates stored vectors.

## Security

The server is meant to run on a trusted network or behind a gateway you control.
Its threat model is deliberately small; know these boundaries before exposing it:

- **No built-in TLS.** The server speaks plain HTTP, so the bearer token and note
  content cross the wire in the clear. Never bind beyond `127.0.0.1` without a
  TLS-terminating reverse proxy (nginx, Caddy, a tunnel) in front of it. Point
  Kimün at the proxy's `https://` URL.
- **One shared token, one trust domain.** The `[auth]` token authenticates the
  *deployment*, not individual vaults — any holder can push to or query any
  vault on the server (vaults are isolated by id, not by credential). Run one
  server per trust domain; don't share a server across mutually-distrusting users.
- **Fail-open below the loopback bind.** With no token set the API is
  unauthenticated. The server logs a warning when it binds beyond `127.0.0.1`
  without one, but it still serves — set a token yourself; nothing forces it.
- **No rate limiting.** There is no built-in throttle on requests or answer
  jobs; put that in the reverse proxy if you need it.
- **Token stored in plaintext config.** Protect `rag.conf` with filesystem
  permissions (`chmod 600`).

## Vector databases

- **SQLite (`sqlite-vec`)** — local, file-based, zero setup. Collections are
  isolated by a partition key. Good for small-to-medium vaults.
- **Qdrant** — a standalone server; one collection per vault. Better at scale.

## Development

Part of the Kimün Cargo workspace, so the usual workspace commands include it:

```bash
cargo build -p kimun_rag              # or `cargo build --workspace`
cargo test -p kimun_rag               # or `cargo test --workspace`
cargo clippy -p kimun_rag
cargo fmt
```

The vector-store SQL and the external embedders are unit-tested with a fake
embedder, and the web UI is exercised end-to-end with a fake store, so most
tests run without downloading a model.

**SQLite version pin.** This crate uses `rusqlite`, while `core` uses `sqlx`;
both link the native `sqlite3` library, and Cargo permits only one
`libsqlite3-sys` version across the workspace. `rusqlite` is therefore pinned to
the version whose `libsqlite3-sys` matches `sqlx-sqlite`'s. Bump the two in
lockstep (see the comment in `Cargo.toml`).

## License

See the LICENSE file in the repository root.
