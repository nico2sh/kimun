+++
title = "Semantic Search & Ask (Server)"
weight = 23
+++

# Semantic Search & Ask — the Kimün Server

> **Experimental.** The server and its TUI integration are under active
> development; configuration and behavior may change between releases.

Kimün's built-in search is fast full-text and fuzzy matching. The optional
**Kimün server** adds a second kind of search on top: **semantic** — finding
notes by meaning rather than exact words — and, when configured with an LLM,
**Ask**: natural-language question-answering grounded in your own notes.

Kimün works fully without the server. When a server is configured and
reachable, the extra capabilities light up automatically.

## What you get

- **Semantic search** — a `SEM` view in the drawer (activity rail) that finds
  notes by meaning: searching "how do I deploy" also surfaces the note that
  says "release procedure".
- **Ask (RAG)** — press `F6` to ask a question in plain language; the server
  retrieves the most relevant note chunks and has its configured LLM answer
  from them, citing the source notes. Only available when the server has an
  LLM configured; with no LLM the server is semantic-search-only and the Ask
  overlay stays hidden.
- **Automatic sync** — the TUI pushes note content to the server in the
  background (every few seconds) and keeps it reconciled with the vault. The
  footer shows the connection state: `rag: online`, `rag: syncing`,
  `rag: offline`, or `rag: not configured`.
- **Multi-vault** — one server hosts many vaults at once; each vault is its
  own isolated collection.
- **Web admin UI** — the server serves a small dashboard at its root URL:
  running configuration, per-vault collections, indexing/answer jobs, a
  test-query box, and a config editor.

The server is **push-only**: it never reads your notes from disk. Kimün sends
note content to it, and the server stores only embeddings (and answers
queries). Everything — embeddings, vector store, optionally the LLM — can run
locally, so your notes never have to leave your machine.

## Installing the server

Three ways, by preference:

### Script (Linux, macOS Apple Silicon)

Downloads the latest release binary, verifies its checksum, and installs it
to `~/.local/bin`. **Re-run the same command to update** — it detects the
installed version and only downloads when a newer release exists:

```sh
curl -fsSL https://kimun.2co.dev/install-server.sh | sh
```

Add `--service` to also run the server on login (a systemd user unit on
Linux, a launchd agent on macOS) and restart it automatically on updates:

```sh
curl -fsSL https://kimun.2co.dev/install-server.sh | sh -s -- --service
```

To manage the service — for example to restart it after editing the config —
use `systemctl` on Linux or `launchctl` on macOS:

```sh
# Linux
systemctl --user restart kimun-server
systemctl --user status kimun-server

# macOS
launchctl kickstart -k "gui/$(id -u)/dev.2co.kimun-server"
```

### Docker (homelab, NAS, VPS)

Multi-arch images (amd64, arm64) at `ghcr.io/nico2sh/kimun-server`. A single
`/data` volume holds the config file, the vector store, and the embedding
model cache. **Update with `docker pull`** (or a tool like Watchtower):

```sh
docker run -d --name kimun-server \
  -p 7573:7573 \
  -v kimun-server-data:/data \
  ghcr.io/nico2sh/kimun-server:latest
```

Or with compose:

```yaml
services:
  kimun-server:
    image: ghcr.io/nico2sh/kimun-server:latest
    ports:
      - "7573:7573"
    volumes:
      - kimun-server-data:/data
    restart: unless-stopped

volumes:
  kimun-server-data:
```

The first start seeds `/data/server.toml` with working defaults (embedded
SQLite + local embedder); edit it — or use the web UI Config page — and
restart the container to apply. Inside the container the server binds
`0.0.0.0`, so set an `[auth]` token before publishing the port beyond your
machine, and put a TLS-terminating reverse proxy in front — the server speaks
plain HTTP.

### Cargo (from source)

Works anywhere with a [Rust toolchain](https://rustup.rs) — including Windows
and Intel Macs, which have no prebuilt binary:

```sh
cargo install --git https://github.com/nico2sh/kimun kimun_server
```

This builds and installs the `kimun-server` binary into `~/.cargo/bin`.
Windows zips are also on the
[releases page](https://github.com/nico2sh/kimun/releases) (`kimun_server-v*`
tags).

## Running the server

Start with working local defaults — embedded SQLite vector store plus a
local embedding model, no config file needed:

```sh
kimun-server --default-config
```

The first run downloads the embedding model (a few hundred MB). Once it is
up, open `http://127.0.0.1:7573/` for the web UI.

The config file lives at `~/.config/kimun/server.toml` (`--default-config`
creates it if missing). Edit it — or use the web UI's Config page — to choose:

- **Embedder** — local [fastembed](https://github.com/Anush008/fastembed-rs)
  models (no network), or an external Ollama / OpenAI-compatible embeddings
  endpoint. The OpenAI-compatible option also covers cloud providers such as
  [Mistral](https://docs.mistral.ai/api/endpoint/embeddings) (their native API
  is OpenAI-compatible) and
  [Google Gemini](https://ai.google.dev/gemini-api/docs/embeddings) (via its
  OpenAI-compatibility endpoint):

  ```toml
  # Mistral
  [embedder]
  type = "openai"
  url = "https://api.mistral.ai/v1"
  model = "mistral-embed"
  api_key = "..."
  ```

  ```toml
  # Google Gemini
  [embedder]
  type = "openai"
  url = "https://generativelanguage.googleapis.com/v1beta/openai"
  model = "gemini-embedding-001"
  api_key = "..."
  ```
- **Vector store** — embedded SQLite (zero setup, the default) or a
  standalone [Qdrant](https://qdrant.tech) server.
- **Reranker** — improves result ordering; on by default. Either a local
  cross-encoder model (downloaded on first start, independent of the embedder
  choice) or an external rerank API — [Cohere](https://docs.cohere.com/reference/rerank),
  [Jina AI](https://jina.ai/reranker/),
  [Voyage AI](https://docs.voyageai.com/reference/reranker-api), or a
  self-hosted vLLM/Infinity server. (OpenAI, Mistral, Gemini, and Anthropic
  offer no rerank API.) If the reranker fails to start — for example the
  model download is blocked by a proxy — the server keeps running without it.
- **LLM for Ask** — Claude, OpenAI, Gemini, Mistral, or any local
  OpenAI-compatible endpoint (Ollama, llama.cpp, …). Leave unset for a
  semantic-search-only server.
- **Auth** — an optional bearer token; required in practice if the server
  binds beyond `127.0.0.1`.

Config edits from the web UI are written to the file and applied on the next
restart — the **Restart server** button on the Config page triggers that
restart in place (the server drains current requests, reloads the file, and
comes right back; connected Kimün clients reconnect on their own). See the
[server README](https://github.com/nico2sh/kimun/tree/main/server)
for the full configuration and API reference.

```sh
# common flags
kimun-server --config /path/to/server.toml   # explicit config file
kimun-server --host 0.0.0.0 --port 7573      # override the bind address
```

## Connecting Kimün to the server

Two ways:

**Preferences** — open Preferences (`Ctrl+,`), pick the **Server** section,
and enter the server address (URL including port). When the field is empty,
the placeholder shows the default local address, `http://localhost:7573`.
Leave it empty to keep the feature off.

**Config file** — set the URL in the `[global]` section of Kimün's
`config.toml`:

```toml
[global]
kimun_server_url = "http://localhost:7573"
# only if the server has an [auth] token configured:
kimun_server_token = "your-token"
```

The server connection is global — every workspace syncs to the same server,
each into its own collection.

Once configured, Kimün probes the server, starts syncing, and the footer
shows the connection status. The `SEM` drawer view appears, and `F6` opens
Ask when the server has an LLM.

## A note on security

The server speaks plain HTTP and is meant for a trusted network. If you
expose it beyond `127.0.0.1`, set an `[auth]` token **and** put a
TLS-terminating reverse proxy in front of it, then point
`kimun_server_url` at the proxy's `https://` address. Details in the
[server README](https://github.com/nico2sh/kimun/tree/main/server#security).
