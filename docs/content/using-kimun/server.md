+++
title = "Semantic Search & Ask (Server)"
weight = 23
+++

# Kimün Server: Semantic Search and Ask

> **Experimental.** The server and its TUI integration are under active
> development; configuration and behavior may change between releases.

Kimün's built-in search does full-text and fuzzy matching. The optional
Kimün server adds semantic search, which finds notes by meaning rather than
exact words. If the server has an LLM configured, it also adds Ask: you ask a
question in plain language and get an answer drawn from your own notes.

Kimün works fully without the server. When a server is configured and
reachable, these features turn on automatically.

## What you get

- **Semantic search:** a `SEM` view in the drawer (activity rail) that finds
  notes by meaning. Searching "how do I deploy" also finds the note that
  says "release procedure".
- **Ask:** a conversation with your notes. Switch to the **ASK** entry in the
  activity rail (or press `F6`) and type a question. The server retrieves the
  most relevant note chunks and has its configured LLM answer from them,
  citing the source notes inline as `[1]`, `[2]`, and so on. You can ask
  follow-up questions in the same conversation; Kimün keeps the history and
  sends it with each question. Ask needs an LLM configured on the server.
  Without one, the server only does semantic search and the **ASK** entry is
  hidden from the rail.
- **Automatic sync:** the TUI pushes note content to the server in the
  background every few seconds and keeps it in step with the vault. The
  footer shows the connection state: `rag: online`, `rag: syncing`,
  `rag: offline`, or `rag: not configured`.
- **Multi-vault:** one server hosts many vaults at once, each in its own
  isolated collection.
- **Web admin UI:** the server has a small dashboard at its root URL showing
  the running configuration, per-vault collections, indexing and answer jobs,
  a test-query box, and a config editor.

The server never reads your notes from disk. Kimün pushes note content to it,
and the server stores only embeddings and answers queries. The embedder, the
vector store, and optionally the LLM can all run locally, so your notes don't
have to leave your machine.

## Asking questions

### Starting a conversation

- Switch to **ASK** in the activity rail, or press `F6`, to go to the
  question composer.
- Type a question and press `Enter`. The answer appears below it, with
  `[1]`, `[2]`, … markers pointing at the notes it drew from.
- Type a follow-up in the same box. Earlier turns are sent with it, so the
  answer can refer to them.
- With the conversation focused, `j`/`k` move between turns; `i` or `/` jumps
  back to the composer.

### Sources and the reader

- The drawer's **Sources** panel lists the notes behind the selected answer,
  ranked by relevance.
- Press `Enter` or `l` on a source to open the reader: the note's text with
  the retrieved section highlighted.
- In the reader, `j`/`k` scroll, `h` or `Esc` goes back to the source list,
  and `o` opens the note in the editor (this works from the list too).
- Clicking a `[n]` citation in the answer selects that source in the Sources
  panel.

### Acting on an answer

With a turn selected in the conversation:

- `y`: copy the answer to the clipboard, without citation markers.
- `e`: save the answer as a new note. Its citations become `[[wikilinks]]`
  to the source notes.
- `r`: regenerate the answer from the same sources.

### When the server goes offline

If the connection drops mid-conversation, the composer shows that asking is
unavailable, but the conversation stays as it is. You can still browse turns,
read their sources, copy an answer, or save one as a note. Asking new
questions and regenerating answers work again once the server is reachable.

## Installing the server

There are three ways to install it, in order of preference.

### Script (Linux, macOS Apple Silicon)

Downloads the latest release binary, verifies its checksum, and installs it
to `~/.local/bin`. To update, re-run the same command. It checks the
installed version and only downloads when a newer release exists:

```sh
curl -fsSL https://kimun.2co.dev/install-server.sh | sh
```

Add `--service` to also run the server on login (a systemd user unit on
Linux, a launchd agent on macOS) and restart it automatically on updates:

```sh
curl -fsSL https://kimun.2co.dev/install-server.sh | sh -s -- --service
```

To manage the service (for example, to restart it after editing the config),
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
model cache. To update, run `docker pull` (or use a tool like Watchtower):

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
SQLite and a local embedder). Edit it, or use the web UI's Config page, then
restart the container to apply the changes. Inside the container the server
binds `0.0.0.0`, so set an `[auth]` token before publishing the port beyond
your machine. The server speaks plain HTTP, so also put a TLS-terminating
reverse proxy in front of it.

### Cargo (from source)

Works anywhere with a [Rust toolchain](https://rustup.rs), including Windows
and Intel Macs, which have no prebuilt binary:

```sh
cargo install --git https://github.com/nico2sh/kimun kimun_server
```

This builds and installs the `kimun-server` binary into `~/.cargo/bin`.
Windows zips are also on the
[releases page](https://github.com/nico2sh/kimun/releases) (`kimun_server-v*`
tags).

## Running the server

To start with local defaults (an embedded SQLite vector store and a local
embedding model), run the following. No config file is needed:

```sh
kimun-server --default-config
```

The first run downloads the embedding model (a few hundred MB). Once it is
up, open `http://127.0.0.1:7573/` for the web UI.

The config file lives at `~/.config/kimun/server.toml` (`--default-config`
creates it if missing). Edit it, or use the web UI's Config page, to choose:

- **Embedder:** local [fastembed](https://github.com/Anush008/fastembed-rs)
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
- **Vector store:** embedded SQLite (no setup, the default) or a
  standalone [Qdrant](https://qdrant.tech) server.
- **Reranker:** improves result ordering and is on by default. It can be a
  local cross-encoder model (downloaded on first start, independent of the
  embedder choice) or an external rerank API: [Cohere](https://docs.cohere.com/reference/rerank),
  [Jina AI](https://jina.ai/reranker/),
  [Voyage AI](https://docs.voyageai.com/reference/reranker-api), or a
  self-hosted vLLM/Infinity server. (OpenAI, Mistral, Gemini, and Anthropic
  offer no rerank API.) If the reranker fails to start, for example because
  a proxy blocks the model download, the server keeps running without it.
- **LLM for Ask:** Claude, OpenAI, Gemini, Mistral, or any local
  OpenAI-compatible endpoint (Ollama, llama.cpp, …). Leave unset for a
  semantic-search-only server.
- **Auth:** an optional bearer token. Set one whenever the server binds
  beyond `127.0.0.1`.

Config edits from the web UI are written to the file and applied on the next
restart. The **Restart server** button on the Config page restarts the server
in place: it drains current requests, reloads the file, and starts again.
Connected Kimün clients reconnect on their own. See the
[server README](https://github.com/nico2sh/kimun/tree/main/server)
for the full configuration and API reference.

```sh
# common flags
kimun-server --config /path/to/server.toml   # explicit config file
kimun-server --host 0.0.0.0 --port 7573      # override the bind address
```

## Connecting Kimün to the server

You can set the server address in Preferences or in the config file.

**Preferences:** open Preferences (`Ctrl+,`), pick the **Server** section,
and enter the server address (URL including port). When the field is empty,
the placeholder shows the default local address, `http://localhost:7573`.
Leave it empty to keep the feature off.

**Config file:** set the URL in the `[global]` section of Kimün's
`config.toml`:

```toml
[global]
kimun_server_url = "http://localhost:7573"
# only if the server has an [auth] token configured:
kimun_server_token = "your-token"
```

The server connection is global. Every workspace syncs to the same server,
each into its own collection.

Once the address is set, Kimün connects to the server and starts syncing, and
the footer shows the connection status. The `SEM` drawer view appears, and the `ASK`
rail entry appears once the server has an LLM configured.

## A note on security

The server speaks plain HTTP and is meant for a trusted network. If you
expose it beyond `127.0.0.1`, set an `[auth]` token and put a
TLS-terminating reverse proxy in front of it, then point
`kimun_server_url` at the proxy's `https://` address. Details in the
[server README](https://github.com/nico2sh/kimun/tree/main/server#security).
