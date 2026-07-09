# RAG Integration — Build Plan

Sequenced plan to take the `rag/` server from experimental/pull to the
push-only, multi-vault, Kimün-owned architecture decided in adr/0018–0020 and
CONTEXT.md (RAG section).

Decisions this plan implements:
- Push-only sync; server never reads the vault (adr/0018).
- Core network-free; **index observer** seam + `kimun_rag_client` crate;
  **reconciliation** (hash-diff) is the backbone, no durable outbox (adr/0019).
- Multi-vault server, one **collection** per **Vault ID** in `.kimun/` (adr/0020).
- Server-owned LLM config, bearer-token auth, per-note hash (v1).
- TUI: additive surfaces only.
- Web UI: axum server-rendered (askama/maud) + htmx, assets embedded, no node.

Dependency graph:
```
P1 Core seam ─┐
              ├─► P3 RAG client ─► P4 TUI surfaces
P2 Server ────┤
              └─► P5 Web UI
```
P1 and P2 are independent (parallelizable). P3 needs both. P4 needs P3. P5 needs P2.

---

## Phase 1 — Core: index observer seam + Vault ID

No network in core. Everything here is offline-testable.

**1.1 Index observer seam**
- Define an `IndexObserver` trait: `on_change(path, content_hash, kind)` where
  `kind = Upsert | Delete`. Thin event, no chunk text (adr/0019).
- Register zero-or-one observer on `NoteVault` (or `NoteIndex`). Default none →
  zero cost when RAG unused.
- Emit at the choke points already carrying the hash:
  - `NoteIndex::save_note` — `core/src/index/mod.rs:1411`
  - `NoteIndex::delete_notes` — `core/src/index/mod.rs:1396`/`:253`
  - `NoteIndex::apply(IndexDiff)` — `core/src/index/mod.rs:218` (bulk sync: emit
    per to_add/to_modify/to_delete entry)
- Call synchronously; observer must be cheap (dirty-set insert). Core does not
  await network.
- Tests: each mutation path (create/save/append/replace/delete/rename/bulk sync)
  produces the expected events with correct hash + kind.

**1.2 Vault ID**
- Generate a UUID once, persist under `.kimun/` (new file, e.g.
  `.kimun/vault-id`). Travels with the vault; excluded from index (dotdir
  already excluded).
- Core API: `NoteVault::vault_id() -> VaultId` (read-or-create on open).
- Tests: id stable across reopen; new id on fresh vault; survives rename/move.

**Ships:** core emits change events + exposes a stable vault id. No behaviour
change for existing users (no observer registered).

---

## Phase 2 — Server: push-only + multi-vault + auth

Independent of P1. Reshapes the existing `rag/` crate.

**2.1 Strip the pull model** (adr/0018)
- Delete `ChunkLoader` (`rag/src/document.rs:222`), `index_all_impl` +
  `index_all_handler`, `index_single_*`, `store_single_note_impl`,
  `store_embeddings*` on `KimunRag` that read `kimun.sqlite`.
- Remove `vault.path` from `RagConfig` (`rag/src/config.rs:23`).
- This deletes, rather than fixes, the known bugs: `store_single_note_impl`
  empty-doc return (handlers.rs:593), `ChunkLoader` column mismatch
  (document.rs:242), `store_embeddings_incremental` re-store-all (lib.rs:195).

**2.2 Collection dimension** (adr/0020)
- Every request carries a Vault ID. Add to `KimunDoc` wire type / request bodies.
- SQLite backend (`dbembeddings/vecsqlite.rs`): add a collection column to the
  vectors + indexed-notes tables; scope all queries by it.
- Qdrant backend (`dbembeddings/vecqdrant.rs`): collection-per-Vault-ID.
- `get_indexed_notes` becomes per-collection.
- New endpoint `GET /api/collections/{vault_id}/hashes` → `{path: hash}` for
  reconciliation. Auto-create collection on first push.
- Keep `/api/index/docs` (push), add explicit delete path
  (`POST /api/index/delete` with paths), `/api/embeddings`, `/api/answer`,
  `/health`, job status.

**2.3 Config + LLM ownership**
- Fix `RerankerConfig` drift — add `top_k` (config.rs:70 vs README/example).
- LLM provider/model/**keys** move into server persisted config (editable by P5
  web UI). Drop the per-request `X-API-Key` + provider override in
  `answer_handler` (handlers.rs:301) — `/answer` takes query + vault-id only.
- `/health` returns capability info (LLM configured? reranker on?) so the client
  can gate features.

**2.4 Auth**
- Bearer-token middleware (tower layer). Required when bind ≠ 127.0.0.1.
- Token stored in server config, shown in web UI.

**Ships:** a push-only multi-vault server with auth, reachable by curl/Bruno
before any Kimün wiring exists.

---

## Phase 3 — `kimun_rag_client` crate

Needs P1 (seam + vault id) and P2 (endpoints). New workspace crate depending on
`kimun_core` for types (`ContentChunk`, `VaultPath`, `VaultId`).

- HTTP client (reqwest) + config (server URL, token).
- `probe()` → hits `/health`; drives an online/offline + capability state.
- Implements `IndexObserver` → folds events into an in-memory dirty-set.
- **Drain**: for dirty upserts, pull sections via
  `NoteVault::get_note_chunks` (core/src/lib.rs:470), assemble `KimunDoc`
  {path, hash, vault-id, sections (breadcrumb→title, text)}, `POST /index/docs`;
  for deletes, call the delete endpoint. On failure, stay dirty.
- **Reconciliation** (adr/0019): fetch `/collections/{id}/hashes`, diff against
  `NoteIndex` authoritative `{path→hash}`, push/delete deltas. Run on connect +
  on an interval. First run = reconcile vs empty collection = full index.
- Query helpers: `semantic_search(query)` → `/embeddings`; `ask(query)` →
  `/answer` (job poll).
- Tests: against a mock server (dirty-set drain, reconcile diff math, offline →
  reconnect self-heal).

**Ships:** a headless client library that keeps a vault in sync and can query.
Exercisable via a tiny CLI harness (replace the stubbed `rag/src/bin/rag.rs`).

---

## Phase 4 — TUI surfaces (additive)

Needs P3. All gated on server reachability — invisible when standalone.

**4.1 Wiring**
- App owns a `RagClient`; register it as the vault's index observer; spawn a
  background drain/reconcile task on a tokio handle.
- Config: `rag_server_url` + token in `GlobalConfig`
  (tui/src/settings/workspace_config.rs:41). Edit in preferences + an onboarding
  step.

**4.2 Semantic search** — server-backed **Row source** feeding a `SearchList`
(reuse the seam; see CONTEXT "Row source"). A distinct mode/surface, not blended
into the FTS query language (adr-scope: additive). Hidden when offline.

**4.3 RAG answer** — a new **Overlay** (like the Saved Searches modal): prompt →
`ask()` → streamed/awaited answer with cited source chunks (path + breadcrumb),
each openable in the editor.

**4.4 Status** — a small indicator (connected / syncing / offline) so the user
knows when capabilities are live.

**Ships:** semantic search + Q&A in the TUI when a server is configured and up.

---

## Phase 5 — Server web UI

Needs P2. axum server-rendered + htmx, assets embedded (rust-embed/include_dir).

- Pages: vector-DB config; LLM config (provider/model/keys); collections list
  (human label = vault name metadata, doc counts); job status (htmx polling);
  optional test-query box.
- Protected by the P2 bearer token or a first-run admin password.
- No node build step; one self-contained binary.

**Ships:** point-and-click server configuration; closes the "web UI" goal.

---

## Cross-cutting / deferred

- **Per-chunk hash** skip — payload is forward-shaped for it; not v1 (adr/0019).
- **Per-collection LLM override** — global for v1 (adr/0020).
- **Embedding model config** — hardcoded BGE-Large for v1; changing it later
  invalidates all vectors (dim change) → forces a full reconcile; document as a
  destructive action when it becomes configurable.
- **Vault-copy shares collection** — regenerate Vault ID to fork (adr/0020).
- **TLS guidance** for remote (non-localhost) servers.
