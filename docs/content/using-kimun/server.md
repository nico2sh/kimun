+++
title = "Semantic Search & Ask (Server)"
weight = 23
+++

# Kimün Server: Semantic Search and Ask

> Experimental: The server and its TUI integration are in active development. The configuration and the behavior can change between releases.

The built-in search of Kimün does full-text and fuzzy matching. The optional Kimün server adds semantic search, which finds notes by meaning instead of by exact words. If you configure an LLM on the server, it also adds Ask. With Ask, you put a question in plain language and get an answer from your own notes.

Kimün works fully without the server. When you configure a server and Kimün can reach it, these features turn on automatically.

## What you get

- Semantic search: a `SEM` view in the drawer (the activity rail) finds notes by meaning. A search for "how do I deploy" also finds the note that says "release procedure". The list has a preview. It opens at the matched section, highlighted. Press Alt+Right to open it and Alt+Left to close it. Enter still opens the note.
- Ask: a conversation with your notes. Switch to the ASK entry in the activity rail (or press `F6`) and type a question. The server retrieves the most relevant note chunks. Its configured LLM answers from them and cites the source notes inline as `[1]`, `[2]`, and so on. You can ask follow-up questions in the same conversation. Kimün keeps the history and sends it with each question. Ask needs an LLM that you configured on the server. Without an LLM, the server only does semantic search, and the ASK entry is not in the rail.
- Automatic sync: the TUI pushes note content to the server in the background every few seconds. This keeps the server in step with the vault. The footer shows the connection state: `rag: online`, `rag: syncing`, `rag: offline`, or `rag: not configured`.
- Multi-vault: one server hosts many vaults at once. Each vault has its own isolated collection.
- Web admin UI: the server has a small dashboard at its root URL. It shows the running configuration, the collections of each vault, the indexing and answer jobs, a test-query box, and a configuration editor.

The server never reads your notes from disk. Kimün pushes note content to it. The server stores only embeddings and answers queries. The embedder, the vector store, and optionally the LLM can all run locally, so your notes can stay on your machine.

## Asking questions

### Starting a conversation

- Switch to ASK in the activity rail, or press `F6`, to go to the question composer.
- Type a question and press `Enter`. The answer appears below the question, with `[1]`, `[2]`, … markers that point at the notes it used.
- Type a follow-up in the same box. Kimün sends the earlier turns with it, so the answer can refer to them.
- When the conversation has focus, `j` and `k` move between turns. `i` or `/` moves back to the composer.

### Sources and the reader

- The Sources panel of the drawer lists the notes behind the selected answer. It ranks them by relevance.
- To open the reader, press `Enter`, `l` or `Right` on a source. The reader shows the text of the note with the retrieved section highlighted.
- In the reader, `j` and `k` scroll. `h`, `Left` or `Esc` goes back to the source list. Alt+Right and Alt+Left also grow and shrink the preview, in the filter box too. `o` opens the note in the editor, and this also works from the list.
- When you click a `[n]` citation in the answer, Kimün selects that source in the Sources panel.

### Acting on an answer

When a turn is selected in the conversation, these keys work:

- `y`: copy the answer to the clipboard, without citation markers.
- `e`: save the answer as a new note. Its citations become `[[wikilinks]]` to the source notes.
- `r`: regenerate the answer from the same sources.

### When the server goes offline

If the connection drops during a conversation, the composer shows that asking is not available. The conversation stays as it is. You can still browse the turns, read their sources, copy an answer, or save an answer as a note. When the server is reachable again, you can ask new questions and regenerate answers.

## Installing the server

There are three ways to install the server. They are in order of preference.

### Script (Linux, macOS Apple Silicon)

The script downloads the latest release binary, makes sure that the checksum is correct, and installs the binary to `~/.local/bin`. To update, run the same command again. The script reads the installed version and downloads only when a newer release exists:

```sh
curl -fsSL https://kimun.2co.dev/install-server.sh | sh
```

To also run the server on login, add `--service`. The script creates a systemd user unit on Linux and a launchd agent on macOS. It also restarts the server automatically after updates:

```sh
curl -fsSL https://kimun.2co.dev/install-server.sh | sh -s -- --service
```

To manage the service, use `systemctl` on Linux or `launchctl` on macOS. For example, restart the service after you edit the configuration:

```sh
# Linux
systemctl --user restart kimun-server
systemctl --user status kimun-server

# macOS
launchctl kickstart -k "gui/$(id -u)/dev.2co.kimun-server"
```

To remove the server that the script installed, run it with `--uninstall`. The script stops and removes the user service if you set one up, and then it removes the binary. It does not remove your configuration (`~/.config/kimun/server.toml`) or your data. The data are the vector store and the embedding model under `~/.local/share/kimun`. Delete them yourself if you do not need them. You cannot combine `--uninstall` with `--service`.

```sh
curl -fsSL https://kimun.2co.dev/install-server.sh | sh -s -- --uninstall
```

### Docker (homelab, NAS, VPS)

The multi-arch images (amd64, arm64) are at `ghcr.io/nico2sh/kimun-server`. A single `/data` volume holds the configuration file, the vector store, and the embedding model cache. To update, run `docker pull`, or use a tool like Watchtower:

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

The first start creates `/data/server.toml` with working defaults (embedded SQLite and a local embedder). To apply changes, edit the file, or use the Config page of the web UI. Then restart the container. Inside the container, the server binds `0.0.0.0`. Before you publish the port beyond your machine, set an `[auth]` token. The server speaks plain HTTP, so also put a reverse proxy that terminates TLS in front of it.

### Cargo (from source)

This method works anywhere that has a [Rust toolchain](https://rustup.rs). This includes Windows and Intel Macs, which have no prebuilt binary:

```sh
cargo install --git https://github.com/nico2sh/kimun kimun_server
```

The command builds the `kimun-server` binary and installs it into `~/.cargo/bin`. Windows zips are also on the [releases page](https://github.com/nico2sh/kimun/releases) (`kimun_server-v*` tags).

### Update notices

Once a day, the server looks on GitHub for a newer release. When one exists, the dashboard of the web UI shows it with the upgrade command for your install (`docker pull` for the image, or the install script again otherwise). The status bar of Kimün adds `server x.y.z available` next to the server status. The server does not download or install anything automatically. To turn the check off, set this in the server configuration:

```toml
[server]
update_check = false
```

Servers from 0.4.3 and earlier are older than the update notices. They cannot report their version, so Kimün always shows `server update available` for them. The setting above does not apply to those servers. To clear the notice, upgrade the server.

## Running the server

To start with local defaults (an embedded SQLite vector store and a local embedding model), run this command. No configuration file is necessary:

```sh
kimun-server --default-config
```

The first run downloads the embedding model (a few hundred MB). When the server is running, open `http://127.0.0.1:7573/` for the web UI.

The configuration file is `~/.config/kimun/server.toml`. If the file does not exist, `--default-config` creates it. To choose the items below, edit the file, or use the Config page of the web UI:

- Embedder: local [fastembed](https://github.com/Anush008/fastembed-rs) models (no network), or an external Ollama or OpenAI-compatible embeddings endpoint. The OpenAI-compatible option also covers cloud providers. Examples are [Mistral](https://docs.mistral.ai/api/endpoint/embeddings), whose native API is OpenAI-compatible, and [Google Gemini](https://ai.google.dev/gemini-api/docs/embeddings), through its OpenAI-compatibility endpoint:

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
- Vector store: embedded SQLite (no setup, the default) or a standalone [Qdrant](https://qdrant.tech) server.
- Reranker: it improves the order of the results, and it is on by default. It can be a local cross-encoder model or an external rerank API. The server downloads a local model on first start, and the embedder choice does not affect it. The external options are [Cohere](https://docs.cohere.com/reference/rerank), [Jina AI](https://jina.ai/reranker/), [Voyage AI](https://docs.voyageai.com/reference/reranker-api), or a self-hosted vLLM or Infinity server. OpenAI, Mistral, Gemini, and Anthropic offer no rerank API. If the reranker fails to start, for example because a proxy blocks the model download, the server keeps running without it.
- LLM for Ask: Claude, OpenAI, Gemini, Mistral, or any local OpenAI-compatible endpoint (Ollama, llama.cpp, …). Leave it unset for a server that does only semantic search.
- Auth: an optional bearer token. Set a token whenever the server binds beyond `127.0.0.1`.

The web UI writes configuration edits to the file, and the server applies them on the next restart. The Restart server button on the Config page restarts the server in place. It finishes the current requests, reloads the file, and starts again. Connected Kimün clients reconnect on their own. For the full configuration and API reference, see the [server README](https://github.com/nico2sh/kimun/tree/main/server).

```sh
# common flags
kimun-server --config /path/to/server.toml   # explicit configuration file
kimun-server --host 0.0.0.0 --port 7573      # override the bind address
```

## Connecting Kimün to the server

You can set the server address in Preferences or in the configuration file.

In Preferences, open Preferences (`Ctrl+,`) and pick the Server section. Enter the server address (the URL with the port). When the field is empty, the placeholder shows the default local address, `http://localhost:7573`. To keep the feature off, leave the field empty.

In the configuration file, set the URL in the `[global]` section of the `config.toml` of Kimün:

```toml
[global]
kimun_server_url = "http://localhost:7573"
# only if the server has an [auth] token configured:
kimun_server_token = "your-token"
```

The server connection is global. Every workspace syncs to the same server, and each workspace has its own collection.

After you set the address, Kimün connects to the server and starts to sync. The footer shows the connection status. The `SEM` drawer view appears. The `ASK` rail entry appears when the server has an LLM configured.

## A note on security

The server speaks plain HTTP, and it is for a trusted network. If you expose it beyond `127.0.0.1`, set an `[auth]` token and put a reverse proxy that terminates TLS in front of it. Then point `kimun_server_url` at the `https://` address of the proxy. The details are in the
[server README](https://github.com/nico2sh/kimun/tree/main/server#security).
