# Agent connector implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An outside agent answers questions about a vault with date-aware word search, semantic search, and a short method, in any agent app.

**Architecture:** Core gets a note-date model and a dated search. The server's `/api/embeddings` takes the filters that the answer loop already has. The MCP module gets richer `search_notes`, `vault_info` and `semantic_search`, and runs the sync loop. One source file feeds the `kimun-ask` skill and the server instructions.

**Tech Stack:** Rust (core, server, tui crates), rmcp, sqlx, tokio, Python 3 for the CI check and the fixture script.

**Spec:** `specs/agent-connector.md`

## Global Constraints

- Follow `CLAUDE.md`: path and date logic over vault content live in core. Use `VaultPath`. Never hardcode `.md` or `/`.
- The MCP code stays in `tui/src/cli/commands/mcp/`. Shared code (`RagSync`, `server_client`) is called, not copied.
- The server never reads the vault (`adr/0018`). Filters use the pushed fields.
- Qdrant refuses filtered searches. The tool returns that error.
- No LLM key and no external service are required.
- Date inputs are `YYYY-MM-DD`, both ends inclusive. `journal` is the journal date key.
- The eight MCP prompts do not change.
- User docs go in `docs/content/using-kimun/`. The question set stays outside `docs/`.
- The owner makes the commits. The plan has no commit steps. Each task ends when its tests pass.
- Test commands: `cargo test -p kimun_core`, `cargo test -p kimun_server`, `cargo test -p kimun-notes --lib`. Before the last task, run the seven CI gates from `.github/workflows/check.yml`.

## Review Focus

- An older server ignores unknown request fields. A date filter then returns unfiltered results with no error. The client must refuse to send filters unless the server says it supports them.
- A `from` after `to`, or a date that is not `YYYY-MM-DD`, returns an error that names the format. It never returns an empty list.
- A `date_key` that no note has returns an error that lists the real keys. It never returns an empty list.
- An empty query with no date range returns an error. It never lists the whole vault.
- Multibyte text (accents, CJK, emoji) at the snippet cap is cut on a character boundary and never panics.
- A write through an MCP tool succeeds when the server is down, and the sync catches up later.

---

### Task 1: Baseline question set and fixture vault

**Files:**
- Create: `specs/agent-connector-questions.md`
- Create: `tui/tests/agent_questions/build_vault.py`

**Interfaces:**
- Produces: `python3 tui/tests/agent_questions/build_vault.py <dir>` writes a sample vault into `<dir>`, with dates relative to the run day. The questions file is the scoring sheet.

- [ ] **Step 1: Write `build_vault.py`.** Standard library only. It writes about 40 notes: daily journal entries for the last 45 days under `journal/YYYY-MM-DD.md`, project notes with `due` and `created` date properties, notes linked by wikilinks, and notes with tags. Dates are `today - offset`. Run it with `python3 -I`.
- [ ] **Step 2: Write `agent-connector-questions.md`.** Fifteen questions. Each has: the question, the notes a good answer must read (by path), and for time questions the range as an offset ("the seven days before today"). Five more entries, one per retrieval prompt (`research_topic`, `research_note`, `find_connections`, `link_suggestions`, `brainstorm`). Include a scoring table: right notes read, range stated, tool calls, tokens.
- [ ] **Step 3: Make sure that the script works.** Run `python3 -I tui/tests/agent_questions/build_vault.py /tmp/agent-vault` then `ls /tmp/agent-vault/journal | wc -l`. Expected: 45.
- [ ] **Step 4: Hand the baseline to the owner.** The owner runs the set on a build of the current commit and fills the baseline column. Work on later tasks does not wait for it.

### Task 2: Core note dates

**Files:**
- Create: `core/src/vault_dates.rs`
- Modify: `core/src/lib.rs` (add `mod vault_dates;` and re-export `NoteDate`)
- Test: in-file `#[cfg(test)]` in `vault_dates.rs`

**Interfaces:**
- Produces:
  - `pub struct NoteDate { pub source: String, pub day: chrono::NaiveDate }`
  - `NoteVault::note_dates(&self, path: &VaultPath) -> Result<Vec<NoteDate>, VaultError>`. It returns the journal date (source `"journal"`) if the note sits under `journal_path()` and its file name is a date. It also returns each date-typed property. The source is the lowercase key. A date-time uses the local day as written.
  - `NoteVault::date_keys(&self) -> Result<Vec<String>, VaultError>`: `"journal"` when any note has a journal date, plus each property key whose dominant kind is date or date-time (`index::dominant_property_kind`). Sorted.
- Consumes: `crate::dates::parse_iso_date`, `NoteVault::get_properties`, `NoteVault::property_keys`, `journal_path()`.

- [ ] **Step 1: Write failing tests.** `note_dates_reads_journal_name_and_date_properties`, `note_dates_empty_for_plain_note`, `date_keys_lists_journal_and_date_properties_only`, `date_keys_omits_journal_when_no_journal_notes`. Use a temp vault with `journal/2026-11-20.md` and a note with `due: 2026-11-25`.
- [ ] **Step 2: Run** `cargo test -p kimun_core vault_dates`. Expected: FAIL, items not defined.
- [ ] **Step 3: Implement** the three items above in `vault_dates.rs`. Use `VaultPath` slices for the journal check. Never match on a string prefix.
- [ ] **Step 4: Run** `cargo test -p kimun_core vault_dates`. Expected: PASS.
- [ ] **Step 5: Refactor the push.** In `tui/src/server_client/sync.rs` (the `dates` block near line 202) call `vault.note_dates` for the property part, so one function owns the rule. The `journal` source stays server-derived there, so keep that pushed list to properties only. Run `cargo test -p kimun-notes --lib server_client`. Expected: PASS.

### Task 3: Core dated search

**Files:**
- Modify: `core/src/vault_search.rs`
- Modify: `core/src/lib.rs` (re-export `DateFilter`, `DateSource`)
- Test: `core/src/tests/vault.rs`

**Interfaces:**
- Consumes: Task 2 `note_dates`, `NoteVault::search_notes`, `NoteVault::get_notes`.
- Produces:
  - `pub enum DateSource { Journal, Property(String) }`
  - `pub struct DateFilter { pub source: DateSource, pub from: NaiveDate, pub to: NaiveDate }`
  - `NoteVault::search_notes_dated(&self, query: &str, filter: &DateFilter) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError>`
  - Behavior: an empty `query` lists the notes in range, newest note date first. A non-empty `query` keeps relevance order. Notes without the chosen date never match. `from > to` returns a `VaultError` user error that names both dates.

- [ ] **Step 1: Write failing tests.** `dated_search_journal_range_is_inclusive`, `dated_search_property_range_excludes_notes_without_it`, `dated_search_empty_query_orders_newest_first`, `dated_search_with_query_keeps_relevance_order`, `dated_search_rejects_from_after_to`, `dated_search_unknown_property_key_errors_with_keys` (the error lists `date_keys()`).
- [ ] **Step 2: Run** `cargo test -p kimun_core dated_search`. Expected: FAIL.
- [ ] **Step 3: Implement.** Journal: `get_notes(journal_path(), true)`, keep names whose date is in range, then intersect with `search_notes(query)` when `query` is not empty. Property: build `query + " %key>=from %key<=to"` (quote a key with spaces) and sort by the date when `query` is empty (`-^%key`). Reuse the existing query syntax.
- [ ] **Step 4: Run** `cargo test -p kimun_core dated_search`. Expected: PASS.

### Task 4: Server filters on /api/embeddings

**Files:**
- Modify: `server/src/handlers.rs` (`QueryRequest`, `get_embeddings_handler`, `Health`)
- Modify: `server/src/tools.rs` (move the filter parsing out of `parse_call` into a shared function)
- Modify: `adr/0065-the-answer-is-a-bounded-tool-loop-over-pushed-data.md` (add an amendment line)
- Modify: server docs under `docs/content/using-kimun/server.md`
- Test: `server/src/handlers.rs` tests, `server/src/tools.rs` tests

**Interfaces:**
- Produces:
  - `pub fn parse_filter(args: &serde_json::Value) -> Result<QueryFilter, String>` in `tools.rs`. `parse_call` and the handler both use it. The argument names stay `date_from`, `date_to`, `date_key`, `folder`, `tags`.
  - `QueryRequest` gains `#[serde(default)] pub date_from: Option<String>`, `date_to`, `date_key`, `folder`, `tags: Vec<String>`.
  - The health response gains `search_filters: bool` (true on this server version).
- Consumes: `KimunRag::retrieve_filtered`, `QueryFilter`.

- [ ] **Step 1: Write failing tests.** `embeddings_without_filters_behaves_as_before`, `embeddings_filters_by_date_range_and_key`, `embeddings_filters_by_folder_and_tags`, `embeddings_rejects_from_after_to_with_error`, `embeddings_on_qdrant_refuses_filters_with_clear_error` and `health_reports_search_filters`.
- [ ] **Step 2: Run** `cargo test -p kimun_server embeddings`. Expected: FAIL.
- [ ] **Step 3: Extract `parse_filter`** from `parse_call`. Run `cargo test -p kimun_server tools`. Expected: PASS, no behavior change.
- [ ] **Step 4: Implement the handler change.** An empty filter calls `search` as today. A set filter calls `retrieve_filtered` through the same path the loop uses. Map the Qdrant refusal to the existing error.
- [ ] **Step 5: Run** `cargo test -p kimun_server`. Expected: PASS.
- [ ] **Step 6: Docs.** Add the fields and the Qdrant note to the server page. Add one amendment line to `adr/0065`.

### Task 5: Client search with filters

**Files:**
- Modify: `tui/src/server_client/dto.rs` (`QueryRequest`, `Health`)
- Modify: `tui/src/server_client/mod.rs`
- Test: `tui/src/server_client/` tests

**Interfaces:**
- Consumes: Task 4 request fields and `search_filters`.
- Produces:
  - `pub struct SearchFilter { pub date_from: Option<NaiveDate>, pub date_to: Option<NaiveDate>, pub date_key: Option<String>, pub folder: Option<String>, pub tags: Vec<String> }` with `is_empty()`.
  - `RagClient::search_filtered(&self, query: &str, filter: &SearchFilter, context_size: Option<ContextSize>) -> Result<Vec<ChunkResult>, RagError>`. If the filter is not empty and the last health probe shows no `search_filters`, return `RagError` with the message "This server is too old to filter a search. Update the server." Do not send the request.

- [ ] **Step 1: Write failing tests** with the existing stub transport: `search_filtered_sends_fields`, `search_filtered_refuses_on_old_server`, `search_with_empty_filter_sends_no_filter_fields`.
- [ ] **Step 2: Run** `cargo test -p kimun-notes --lib server_client`. Expected: FAIL.
- [ ] **Step 3: Implement** the two items. Keep `search` as a thin call to `search_filtered` with an empty filter. Do not add `crate::` imports to `server_client` beyond `server_client` itself (module boundary gate).
- [ ] **Step 4: Run** the same command. Expected: PASS.

### Task 6: Sync loop in the MCP process

**Files:**
- Modify: `tui/src/rag/sync.rs` (split the loop from `AppTx`)
- Modify: `tui/src/rag/mod.rs`
- Create: `tui/src/cli/commands/mcp/sync.rs`
- Modify: `tui/src/cli/commands/mcp/mod.rs` (`run`)
- Test: `tui/src/cli/commands/mcp/sync.rs`

**Interfaces:**
- Produces:
  - `pub fn spawn_rag_sync_with(vault: Arc<NoteVault>, settings: &SharedSettings, report: Arc<dyn Fn(RagStatus) + Send + Sync>) -> Option<JoinHandle<()>>` in `sync.rs`. `spawn_rag_sync` calls it with a closure that sends `AppMessage::RagStatus` on `tx`. The server-update message stays in `spawn_rag_sync` only, so keep that path unchanged.
  - `pub async fn flush_once(vault: &Arc<NoteVault>, settings: &SharedSettings)` in `mcp/sync.rs`: build a `RagSync`, call `drain()` once, time out after 5 seconds, log errors.
- Consumes: `RagSync`, `server_config`, `load_and_resolve_workspace` settings.

- [ ] **Step 1: Write failing tests** in `mcp/sync.rs` with a fake server transport: `write_reaches_server_within_one_interval`, `write_succeeds_when_server_down`, `repeated_push_of_same_note_changes_nothing`, `flush_once_times_out`.
- [ ] **Step 2: Run** `cargo test -p kimun-notes --lib mcp::sync`. Expected: FAIL.
- [ ] **Step 3: Split `spawn_rag_sync`.** Move the loop body into `spawn_rag_sync_with`. The app caller keeps its behavior. Run `cargo test -p kimun-notes --lib rag`. Expected: PASS before any MCP change.
- [ ] **Step 4: Wire `run`.** Build a `SharedSettings` from the loaded `AppSettings`, share the `Arc<NoteVault>` with the handler, start the sync with a logging closure, and call `flush_once` after `service.waiting()` returns. `KimunHandler::new(vault: NoteVault)` keeps its signature. Add `KimunHandler::from_shared(vault: Arc<NoteVault>) -> Self`.
- [ ] **Step 5: Run** the tests from Step 2. Expected: PASS.

### Task 7: search_notes rework

**Files:**
- Modify: `tui/src/cli/commands/mcp/mod.rs` (`SearchNotesParams`, `search_notes`)
- Create: `tui/src/cli/commands/mcp/results.rs`
- Test: `mod.rs` tests, `results.rs` tests

**Interfaces:**
- Consumes: Task 3 `search_notes_dated`, Task 2 `note_dates`, `get_backlinks`, `get_note_chunks`.
- Produces:
  - `SearchNotesParams { query: Option<String>, from: Option<String>, to: Option<String>, date_key: Option<String>, limit: Option<u32> }`.
  - `results::render_hit(hit: &NoteHit) -> String`. First line: `path — title`. Following lines, indented two spaces: `dates: journal 2026-11-20, due 2026-11-25`, `tags: a, b`, `links: 3 out, 2 in`, `snippet: ...`.
  - `results::cap_chars(text: &str, max: usize) -> String`: cut on a character boundary, add `…`.
  - Defaults: 20 hits, snippet 240 characters. The tool description keeps the query syntax text.

- [ ] **Step 1: Write failing tests.** Name them `render_hit_first_line_keeps_path_dash_title`, `cap_chars_cuts_on_char_boundary` (accents, CJK, emoji), `search_with_range_and_no_query_lists_newest_first`, `search_range_without_date_key_errors_listing_keys`, `search_with_neither_query_nor_range_errors`, `search_invalid_date_names_the_format` and `search_caps_hits_at_limit`.
- [ ] **Step 2: Run** `cargo test -p kimun-notes --lib mcp`. Expected: FAIL.
- [ ] **Step 3: Implement** the params, the renderer and the handler. `date_key` is required when `from` or `to` is set. The snippet is the start of the note's first chunk after the title. Outlink and backlink counts come from the existing link calls.
- [ ] **Step 4: Run** the same command. Expected: PASS, and the existing search tests still pass.

### Task 8: vault_info

**Files:**
- Modify: `tui/src/cli/commands/mcp/mod.rs`
- Create: `tui/src/cli/commands/mcp/info.rs`
- Test: `info.rs`

**Interfaces:**
- Produces: tool `vault_info` (no input). Output text: `today: YYYY-MM-DD` (local date), `date keys: journal, due, created` (each exactly as the search tools accept it), `semantic search: <state>` where state is one of `none`, `unreachable`, `unconfigured`, `semantic-only`, `full`. `info::semantic_state(settings, vault) -> SemanticState` with `Display`.
- Consumes: Task 2 `date_keys`, the server client capability probe, `server_config`.

- [ ] **Step 1: Write failing tests** with the stub transport: one test for each of the five states, and `vault_info_lists_date_keys_exactly_as_searchable`.
- [ ] **Step 2: Run** `cargo test -p kimun-notes --lib mcp::info`. Expected: FAIL.
- [ ] **Step 3: Implement.** The handler gets the settings through `KimunHandler::with_settings(self, SharedSettings) -> Self`, set in `run`. Without settings the state is `none`. If a date property is also named `journal`, list it as `property:journal` and make both tools accept that form.
- [ ] **Step 4: Run** the same command. Expected: PASS.

### Task 9: semantic_search

**Files:**
- Modify: `tui/src/cli/commands/mcp/mod.rs`
- Create: `tui/src/cli/commands/mcp/semantic.rs`
- Test: `semantic.rs`

**Interfaces:**
- Consumes: Task 5 `search_filtered`, Task 7 `results::cap_chars`, Task 8 `semantic_state`.
- Produces: tool `semantic_search` with `query: Option<String>`, `from`, `to`, `date_key`, `folder`, `tags: Vec<String>`, `limit`. A hit renders as `path — title`, then indented `date:`, `score:`, `rank:`, `chunk:` (capped at 400 characters). Errors, each with its fix in the text: no server configured; server unreachable; server unconfigured (no embedder); token missing or rejected; filtered search on Qdrant; server too old for filters. An empty query needs a date range, as in Task 7.

- [ ] **Step 1: Write failing tests** with the stub transport: `semantic_hit_renders_fields`, one test for each error above, `semantic_range_without_date_key_errors_listing_keys`.
- [ ] **Step 2: Run** `cargo test -p kimun-notes --lib mcp::semantic`. Expected: FAIL.
- [ ] **Step 3: Implement.** Reuse `rag_client(settings, vault)` for the client. Map `RagError` variants to the messages. The tool works on a semantic-only server.
- [ ] **Step 4: Run** the same command. Expected: PASS.

### Task 10: Guidance, skill and instructions

**Files:**
- Create: `skills/kimun-ask/guidance.md` (the single source)
- Create: `skills/kimun-ask/SKILL.md`
- Create: `.github/scripts/check-agent-guidance.py`
- Modify: `tui/src/cli/commands/mcp/mod.rs` (`get_info` instructions), `.github/workflows/check.yml`
- Modify: `docs/content/using-kimun/ai-mcp-server.md`, `docs/content/using-kimun/ai-skills.md`
- Test: `.github/scripts/check-agent-guidance.py` self-test, a Rust test on the instructions

**Interfaces:**
- Produces: `guidance.md` holds two marked blocks, `<!-- short:start -->` to `<!-- short:end -->` (about ten lines) and the full method with the five recipes. `SKILL.md` embeds the full text. `get_info` builds its instructions with `include_str!` of the short block plus the existing index and version notes. The CI script exits 1 when the skill or the instructions text differs from `guidance.md`.

- [ ] **Step 1: Write the failing check.** A Rust test `instructions_contain_short_guidance` and a script run that fails because the files do not exist.
- [ ] **Step 2: Write `guidance.md`.** The five steps from the spec method. Five recipes: `research_topic`, `research_note`, `find_connections`, `link_suggestions` (meaning search minus the note's links and backlinks), `brainstorm`. A CLI path section that says the CLI has no semantic search and uses word search, properties and links.
- [ ] **Step 3: Write `SKILL.md`** with name `kimun-ask` and a description that says it works with the MCP tools or the CLI and that a Kimün server adds `semantic_search`. Do not change `skills/kimun-cli`.
- [ ] **Step 4: Wire `get_info`** and the CI step in `check.yml`.
- [ ] **Step 5: Docs.** Describe the new tools and the skill in plain English. Note that semantic search needs a Kimün server with an embedder and no LLM key.
- [ ] **Step 6: Run** `python3 .github/scripts/check-agent-guidance.py` and `cargo test -p kimun-notes --lib mcp`. Expected: both pass.

### Task 11: Final gates and comparison run

**Files:** none.

- [ ] **Step 1: Touch sources, then run the seven CI gates** from the memory note `ci-gates-before-push` (fmt, module boundary grep, `check-host-fs.py`, clippy with `-D warnings`, bench profile check, docs with `RUSTDOCFLAGS="-D warnings"`, workspace tests). Expected: all green.
- [ ] **Step 2: Hand the final run to the owner.** The owner runs the question set on the same agent apps as the baseline and fills the second column. Report the comparison. The five retrieval prompts stay until that comparison is read.

## Self-review notes

- Spec coverage: search tools (Tasks 3, 7, 9), `vault_info` (8), server filters (4, 5), sync (6), guidance and skill (10), question set (1, 11), tests (in each task), docs (4, 10), prompts untouched (constraint).
- One deviation: the spec says a snippet "around the match". The word search returns no match position, so the plan uses the start of the first chunk. A later change can improve it.
- The capability flag `search_filters` and the client refusal in Task 5 are additions to the spec. They follow from the first Review Focus line.
