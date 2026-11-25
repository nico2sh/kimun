# Kimün RAG Server

**Experimental.** An optional companion server that gives a Kimün vault semantic
search and question-answering. Kimün works fully without it; when it is reachable,
those extra capabilities light up.

## Model

The server is **push-only**: it never reads your notes from disk.
Kimün pushes note content to it and the server only stores embeddings and answers
queries. One server can host **many vaults** at once — each vault is a
**collection**, keyed by the vault's id (a stable UUID Kimün keeps under
`.kimun/` in the vault). Every `/api` request carries a `vault_id`.

```
Kimün  ──push docs / delete / query──▶  RAG server ──▶  vector DB (SQLite | Qdrant)
                                                    └──▶  LLM (Claude | OpenAI | Gemini | Mistral | OpenRouter)
```

## Install

Three ways, by preference:

**Script** (Linux, macOS Apple Silicon) — downloads the latest release binary,
verifies its checksum, installs to `~/.local/bin`. Re-run it to update:

```bash
curl -fsSL https://kimun.2co.dev/install-server.sh | sh
# or additionally set up a login service (systemd user unit / launchd agent):
curl -fsSL https://kimun.2co.dev/install-server.sh | sh -s -- --service
```

Restart the service (e.g. after a config change) with
`systemctl --user restart kimun-server` on Linux, or
`launchctl kickstart -k "gui/$(id -u)/dev.2co.kimun-server"` on macOS.

**Docker** (homelab / NAS / VPS) — multi-arch (amd64, arm64); one volume holds
the config, vector store, and model cache. Update with `docker pull`:

```bash
docker run -d --name kimun-server -p 7573:7573 \
  -v kimun-server-data:/data ghcr.io/nico2sh/kimun-server:latest
```

**Cargo** (from source, any platform with a Rust toolchain — including
Windows and Intel Macs, which have no prebuilt binary):

```bash
cargo install --git https://github.com/nico2sh/kimun kimun_server
```

This installs the `kimun-server` binary into `~/.cargo/bin`. From a source
checkout, `cargo install --path server` does the same.

Windows binaries ship as zips on the
[releases page](https://github.com/nico2sh/kimun/releases) (`kimun_server-v*`
tags). There are no Intel Mac binaries — the ONNX runtime dropped
x86_64 macOS support.

## Quick start

Start with working local defaults (embedded SQLite + the local fastembed
embedder, semantic-only) — no config file needed:

```bash
kimun-server --default-config
```

Or run from a config file (see [Configuration](#configuration)):

```bash
mkdir -p ~/.config/kimun
cp server/config.example.toml ~/.config/kimun/server.toml
# edit ~/.config/kimun/server.toml, then:
kimun-server
```

Override host/port/config on the CLI:

```bash
kimun-server --config /path/to/server.toml --host 0.0.0.0 --port 7573
```

(Working in the repo instead? Substitute
`cargo run --release -p kimun_server --bin kimun-server --` for `kimun-server`.)

Open `http://127.0.0.1:7573/` for the [web UI](#web-ui); the API lives under
`/api` (see [API](#api)).

First run downloads the embedding model (and the reranker, if enabled) — a few
hundred MB. Pointing `[embedder]` at an external service skips the embedding
model, but the reranker model still downloads unless `[reranker]` is disabled.

To connect Kimün, set the server address in Preferences (Server section) or in
`config.toml` — `kimun_server_url = "http://localhost:7573"` under `[global]`,
plus `kimun_server_token` when the server has an `[auth]` token. See the
[user documentation](https://nico2sh.github.io/kimun/using-kimun/server/).

## Configuration

See `config.example.toml` for the annotated template. Sections:

- **`[server]`** — `host`, `port` (default `127.0.0.1:7573`), `max_concurrent_jobs`.
- **`[auth]`** — optional `token`. When set, every `/api` request must send
  `Authorization: Bearer <token>`; `/health` stays open. Required in practice
  once you bind beyond `127.0.0.1`.
- **`[vector_db]`** — pick a backend:
  - `type = "sqlite"` (`path`) — **embedded SQLite**: a local directory holding
    a single database file, one collection per vault, no server. Zero setup;
    the default. (`type = "lance"` is accepted as a legacy alias from before
    the LanceDB backend was replaced.)
  - `type = "qdrant"` (`url`, `collection` — used as a name **prefix**, so each
    vault's Qdrant collection is `<prefix>-<vault-id>`) — a standalone server,
    better at scale.
- **`[embedder]`** — how text becomes vectors. All collections share one
  embedder (the same model must embed documents and queries):
  - `type = "fastembed"` — local, no network. Optional `model` (e.g.
    `BGESmallENV15`, or a model code like `Xenova/bge-small-en-v1.5`);
    default is BGE-Large (1024 dims).
  - `type = "ollama"` — `url`, `model`, optional `doc_prefix`/`query_prefix`.
  - `type = "openai"` — any OpenAI-compatible `/embeddings` endpoint: `url`,
    `model`, optional `api_key`, `doc_prefix`/`query_prefix`. This also covers
    cloud providers whose embeddings API is OpenAI-compatible — Mistral
    (`url = "https://api.mistral.ai/v1"`, `model = "mistral-embed"`) and
    Google Gemini via its OpenAI-compatibility layer
    (`url = "https://generativelanguage.googleapis.com/v1beta/openai"`,
    `model = "gemini-embedding-001"`). See `config.example.toml`.
  - `type = "openrouter"` — OpenRouter's `/embeddings` endpoint, with a fixed
    URL: `model` (required, for example `openai/text-embedding-3-small`),
    optional `api_key` (falls back to `OPENROUTER_API_KEY`),
    `doc_prefix`/`query_prefix`.

  The vector width is detected automatically. **Changing the embedder or model
  invalidates all stored vectors** and forces a re-index — drop the store (the
  SQLite directory or the Qdrant collection), then re-push.
- **`[llm]`** — `provider` (`claude` | `openai` | `gemini` | `mistral` | `openrouter`),
  `model`, optional `api_key`. The key is server-owned; if omitted it falls back
  to the provider's env var (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
  `GEMINI_API_KEY`, `MISTRAL_API_KEY`, `OPENROUTER_API_KEY`). Kimün never sends
  a key. The `openrouter` provider also takes an optional `reasoning`
  (`low` | `medium` | `high`), sent as `reasoning.effort`; leave it out and the
  model keeps its default. A model call that takes longer than 10 minutes
  fails.
- **`[reranker]`** — `enabled` (default true), the **context cut** — the one
  sizing rule for query results on both surfaces and both reranker paths:
  `context_cut` picks the strategy, each with its own knobs.
  `fixed` returns exactly `top_k` results (search notes / answer chunks;
  per-request `context_size` applies here only). `score-range` (default)
  keeps chunks at or above `score_range_cutoff` (default 0.4) of the
  normalized score range, measured between the pool's 5th/95th score
  percentiles so outliers can't stretch it. `largest-drop` cuts at the
  biggest relative gap between distinct notes' best scores at note positions
  `drop_window_min..=drop_window_max` (defaults 3–30), keeping the
  gap-closing note and every chunk above it. And a backend:
  - `type = "fastembed"` (default) — local cross-encoder, model downloaded
    from Hugging Face on first start regardless of the `[embedder]` choice.
  - `type = "http"` — any Cohere/Jina-compatible rerank endpoint (`url`,
    optional `model`, optional `api_key` sent as a bearer token; `/rerank` is
    appended to `url`): [Cohere](https://docs.cohere.com/reference/rerank),
    [Jina AI](https://jina.ai/reranker/),
    [Voyage AI](https://docs.voyageai.com/reference/reranker-api), or
    self-hosted [vLLM](https://docs.vllm.ai) / Infinity. OpenAI, Mistral,
    Gemini, and Anthropic offer no rerank API — Anthropic points to Voyage
    for embeddings and reranking. See `config.example.toml` for each.

  Reranker failure is non-fatal, at startup and at query time alike: an
  initialization failure (blocked model download, unreachable endpoint) logs
  a warning and the server runs without reranking (`/health` reports
  `"reranker": false` with the reason under `"reranker_error"`), and a rerank
  call failing mid-query degrades that request to plain vector ranking
  instead of failing it. Unlike the embedder, switching rerankers never
  invalidates stored vectors.

## API

All `/api` routes require the bearer token when one is configured. `context_size`
is optional (`"small"` = 10, `"medium"` = 20, `"large"` = 40 results); omit it to
use the configured `reranker.top_k`. It only applies under the `fixed` context
cut — the adaptive cuts size results from the pool's score shape.

### `GET /health`

Capability probe. Returns JSON:

```json
{ "status": "ok", "reranker": true, "reranker_error": null, "llm_provider": "claude", "auth_required": true }
```

`reranker` reports the *active* reranker: `false` with a non-null
`reranker_error` means one is configured but failed to initialize.

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

Each document also carries note dates, tags and outgoing links. The server
takes the folder from the path. The assistant uses them to filter by date,
folder or tag. These do not change the
`hash`. The SQLite store drops and recreates its `chunks` table on the first
start after an upgrade, because the columns differ. Then every client pushes all
notes again once, which re-embeds the whole vault. Qdrant stores the metadata
too, but it refuses filtered searches. On Qdrant the assistant cannot filter by
date, folder or tags. On Qdrant the order of a note's sections in
`read_note` is not guaranteed. Older Qdrant points keep no metadata until their note
changes. They also lose their journal date until their note is pushed again.

### `POST /api/index/delete`

Remove notes from a vault's collection.

```json
{ "vault_id": "…", "paths": ["notes/example.md"] }
```

### `GET /api/collections/{vault_id}/hashes`

The `{ note-path: content-hash }` map the server holds for a vault. The client
diffs it against its own authoritative set to reconcile — pushing/deleting only
the differences.

### `PUT /api/collections/{vault_id}/label`

Set the vault label for a vault. The web UI shows the label next to the
vault id, so you can see which collection is which. The label is display only.
The server still identifies a vault by its id.

```json
{ "label": "kimun-notes" }
```

On success the server answers `204 No Content`. A second call overwrites the
label. The server answers `400` in these cases:

- The vault id is not valid.
- The label is empty after trimming, or longer than 100 characters.
- The label contains a control character, for example a newline or a tab.

When an `[auth]` token is set, the route needs the bearer token. Kimün sends
the name of the workspace from its configuration. It never sends a disk path.
Qdrant does not store labels: on Qdrant the call succeeds and the web UI shows
no label.

### `POST /api/embeddings`

Semantic search (no LLM). Returns the matching chunks plus `query_time_ms`,
the wall-clock duration of the search pipeline.

```json
{ "vault_id": "…", "query": "What are RAG systems?", "context_size": "medium" }
```

### `POST /api/answer`

LLM answer over the retrieved context, using the server-configured LLM. Runs in
the background; returns a `job_id`. Poll `/api/job/{job_id}` for the result
(`answer` + `sources`).

```json
{ "vault_id": "…", "query": "Explain RAG systems", "context_size": "large",
  "today": "2026-11-25" }
```

`today` is optional (`YYYY-MM-DD`). It is the client's local date, used for
questions like "last week". If the value is malformed, the server rejects the
request as a bad request (HTTP 422). If the value is well formed but more than
400 days from the server clock, or missing, the server uses its own clock.

The completed job result holds `answer`, `sources` and `trail`. `trail` is a
list of strings, one per step the assistant took (for example a search or a
note it read).

**How answers work.** The first search always runs. After that, the model can
call tools for up to 5 rounds. There are four tools: `search` (notes by
meaning, with optional date, folder and tag filters), `list_date_keys` (the
date properties the notes have), `read_note` (a whole note or one section) and
`related_notes` (the links to and from a note). If the LLM provider rejects
tool calls, the server falls back to a single pass over the first search
results. If a later round fails, the server answers in a single pass from the
notes gathered so far. If that single pass fails too, the server tries once more
with the first search results only. The model can run at most 4 tool calls in
one turn. The server does not run the extra calls and tells the model so. The assistant is told to start each passage from the
model's own knowledge with "Not from your notes:". If no note matched, the
answer starts with a notice that says so. If the answer cites none of your
notes, it starts with a second notice that says so.

One answer can make several calls to the model. It can therefore take longer
than 10 minutes in total. If the app stops waiting, the server cancels a
streamed answer. A polled answer keeps running until it finishes.

### `POST /api/answer/stream`

Same request body as `/api/answer`, but the answer streams back as server-sent
events instead of a job. No job is created. The events are:

- `step`: `{"text": "…"}`, one per step, as the assistant works;
- `answer`: `{"answer": "…", "sources": […], "trail": […]}`, the last event
  on success;
- `error`: `{"error": "…"}`, the last event on failure.

The server rejects a bad request (an invalid `vault_id`, or no LLM
configured) with a normal HTTP error before the stream starts. After that,
failures arrive as an `error` event.

### `GET /api/job/{job_id}`

Job status: `queued` | `processing` | `completed` (with `result`) | `failed`
(with `error`).

While an indexing job embeds notes, the response also has a `progress` object:
`{"done": 100, "total": 250}`. Both numbers count chunks, because the embedder
works on chunks and the server only knows their number after it splits the
notes. The field is absent for other jobs and before embedding starts. A note
is stored only after all of its chunks are embedded, so a vault appears on the
collections page when its job completes. The Jobs page of the web UI shows the
same numbers in its Detail column.

## Web UI

The server also renders a small admin UI at its root (`http://<host>:<port>/`) —
server-rendered pages, no build step, no external assets. It offers:

- a **dashboard** of the running configuration (bind address, vector DB,
  embedder, LLM, reranker, auth);
- a **config** page to edit the LLM (provider/model/key), reranker, auth token,
  and bind address — changes are **written to the config file and applied on the
  next restart** (the embedder, vector store, and LLM client are built at
  startup, so the live instance is never mutated). A **Restart server** button
  applies them in place: the process drains in-flight requests, re-reads the
  config file, and rebinds — no service manager needed, and a
  bind-address change moves the server to the new address;
- a **collections** list with per-vault indexed-note counts. The list shows
  the vault label of each vault, or a dash when the vault has no label;
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
- **Token stored in plaintext config.** Protect `server.toml` with filesystem
  permissions (`chmod 600`).

## Vector databases

- **SQLite** — embedded, file-based, no server: the store is a single database
  file (`embeddings.db`) in a local directory, one collection per vault. Zero
  setup; the default. Exhaustive (exact) KNN — embeddings are L2-normalized at
  write time and every query scans the collection computing dot products, which
  is exact by definition and comfortably fast for small-to-medium vaults (tens
  of thousands of chunks scan in milliseconds). No ANN index to build or tune.
- **Qdrant** — a standalone server; one collection per vault. Better at scale.

## Development

Part of the Kimün Cargo workspace, so the usual workspace commands include it:

```bash
cargo build -p kimun_server              # or `cargo build --workspace`
cargo test -p kimun_server               # or `cargo test --workspace`
cargo clippy -p kimun_server
cargo fmt
```

There is no system prerequisite for the store: SQLite is compiled in (bundled)
via the same `sqlx` the core index uses. On **Windows** use the default
`x86_64-pc-windows-msvc` toolchain with the Visual Studio C++ build tools
installed — parts of the dependency tree compile C.

The vector-store adapters and the external embedders are unit-tested with a fake
embedder (the SQLite adapter runs against a real on-disk store in a temp dir),
and the web UI is exercised end-to-end with a fake store, so most tests run
without downloading a model or reaching a live Qdrant.

## License

See the LICENSE file in the repository root.
