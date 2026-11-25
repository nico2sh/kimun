# Agentic Ask Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the server answer step a bounded tool loop that finds notes by date, tags and links, and records a trail.

**Architecture:** Kimun pushes note dates, tags and links with each note. The vector store filters on them. A new `agent` module runs the loop with four tools over the store. The LLM client gains native tool calling on three wires. A new SSE endpoint streams the trail. The one-shot `/api/answer` job stays and returns the trail.

**Tech Stack:** Rust, axum, sqlx (SQLite store), qdrant-client, reqwest, chrono, serde_json, async-trait.

**Spec:** `specs/agentic-ask.md`. Decision record: `adr/0065-the-answer-is-a-bounded-tool-loop-over-pushed-data.md`.

**Scope:** This plan covers spec delivery steps 1 to 4 and the client transport. The TUI and GUI display of the trail (spec step 5) is a follow-on plan. It needs the stream format that Task 9 fixes.

## Global Constraints

- The server never reads the vault (adr/0018). It only uses pushed fields.
- Every filesystem call in core stays in `nfs` or `system`. This plan adds none.
- Never hardcode `.md` or `/` in core. The server has no `kimun_core` dependency, so it keeps its existing wire-path rule (canonical vault-relative paths with `/` and `.md`).
- The step cap is 5 tool rounds (`MAX_TOOL_ROUNDS`).
- The first `search` runs without the model asking, with the conditioned query from `build_retrieval_query`.
- Every tool is locked to the asking collection.
- Disclosure label: `Not from your notes:`. The no-match notice is a fixed string.
- A request without `today` falls back to the server clock.
- If an endpoint rejects tools, or the model sends malformed calls twice in a row, answer with the existing single pass.
- A note date is the journal filename date (source `journal`) or a date-typed property (source is the lowercased key).
- No migration, no schema version (few servers exist). Old data re-pushes once.
- Do not commit. Report each task green and stop. Run the targeted tests after each task.
- Documents use plain English. Code stays untouched.

## Review Focus

Each line below has its test in the task that owns the code.

1. A date range with `from` after `to` returns a tool error to the model and does not crash the loop (Task 5).
2. `read_note` for a path that does not exist, or that belongs to another vault, returns an error text and no data (Task 5).
3. A property date written as a date-time with an offset still produces a note date (Task 2).
4. The model calls an unknown tool, or sends arguments that do not parse. The loop returns an error result to the model and counts one malformed call (Task 7).
5. A huge note passes through `read_note` and the result is cut with a visible marker (Task 5).

---

## File Structure

- `server/src/document.rs`: add `NoteDate` and `NoteMeta`. Replace `FlattenedChunk.date` with `FlattenedChunk.meta`.
- `server/src/dbembeddings/mod.rs`: add `QueryFilter` and `DateFilter`. Change the `VectorStore` trait. Extend the conformance suite.
- `server/src/dbembeddings/vecsqlite.rs` and `vecqdrant.rs`: store the metadata, apply the filter, implement the two new reads.
- `server/src/tools.rs` (new): tool specs, argument parsing, execution, size cap, link resolution, trail text.
- `server/src/agent.rs` (new): the loop, the prompt, the fallback, the notices.
- `server/src/llmclients/mod.rs`: add `chat` with tools to the three wires.
- `server/src/lib.rs`: `answer` calls the loop. Add the filtered retrieve.
- `server/src/handlers.rs` and `startup.rs`: request and response fields, the SSE route.
- `tui/src/server_client/{dto.rs,sync.rs,mod.rs}`: push the metadata, send `today`, read `trail`, read the stream.

---

### Task 1: Note metadata in the server documents

**Files:**
- Modify: `server/src/document.rs`
- Modify: every site that builds or reads `FlattenedChunk.date` (`server/src/lib.rs`, `server/src/handlers.rs`, `server/src/dbembeddings/vecsqlite.rs`, `server/src/dbembeddings/vecqdrant.rs`, test helpers)
- Test: `server/src/document.rs` (existing `#[cfg(test)]` module)

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
  pub struct NoteDate { pub source: String, pub day: chrono::NaiveDate }
  #[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
  pub struct NoteMeta { pub dates: Vec<NoteDate>, pub tags: Vec<String>, pub links: Vec<String> }
  pub const JOURNAL_SOURCE: &str = "journal";
  ```
- `KimunDoc` gains three fields with `#[serde(default)]`: `dates: Vec<NoteDate>`, `tags: Vec<String>`, `links: Vec<String>`. `links` holds link targets as written in the note.
- `FlattenedChunk.date: Option<NaiveDate>` becomes `FlattenedChunk.meta: NoteMeta`.
- `FlattenedChunk::journal_date(&self) -> Option<NaiveDate>` returns the day of the `journal` note date. `get_date_string` keeps its signature and uses it, so `ChunkResult.date` is unchanged on the wire.

- [ ] **Step 1: Write the failing tests** in `document.rs`:
  - `flatten_adds_journal_date_from_path`: a doc at `journal/2026-11-20.md` gives chunks whose `meta.dates` contains `NoteDate { source: "journal", day: 2026-11-20 }`.
  - `flatten_keeps_pushed_dates_and_adds_journal`: a doc with a pushed `due` date and a journal filename gives both dates, journal first.
  - `flatten_lowercases_tags_and_keeps_link_targets_as_written`: tags `["Work"]` become `["work"]`. A link `[[Project X]]` stays `Project X`.
  - `old_client_doc_without_metadata_still_deserializes`: JSON with only `path`, `hash`, `sections` parses with empty vectors.
- [ ] **Step 2: Run** `cargo test -p kimun-server document::` (use the server package name from `server/Cargo.toml`). Expected: FAIL, the types do not exist.
- [ ] **Step 3: Implement** the types, the `KimunDoc` fields, and the `from_chunks_split` change. Copy `meta` onto every chunk of a doc. Update every compile error from the removed `date` field. The SQLite and Qdrant adapters keep writing the journal date to their existing `date` column and payload key until Task 3 replaces them. Use `meta.journal_date()` there.
- [ ] **Step 4: Run** `cargo test -p kimun-server`. Expected: PASS, including the existing store tests.

### Task 2: The client pushes dates, tags and links

**Files:**
- Modify: `tui/src/server_client/dto.rs` (`WireDoc`)
- Modify: `tui/src/server_client/sync.rs` (`build_doc`, around line 184)
- Test: `tui/src/server_client/sync.rs` test module

**Interfaces:**
- Consumes: `NoteVault::get_properties(&VaultPath) -> Result<Vec<PropertyEntry>, VaultError>`, `NoteVault::labels_of` (the note's labels, in `core/src/vault_search.rs`, check the exact public name there), `NoteVault::get_markdown_and_links` (its `links` carry `NoteLink::raw_link`).
- Produces: `WireDoc` gains `dates: Vec<WireDate>`, `tags: Vec<String>`, `links: Vec<String>`. `WireDate { source: String, day: String }` with `day` as `YYYY-MM-DD`.

- [ ] **Step 1: Write the failing tests** with a temp vault, as the existing `build_doc` tests do:
  - `build_doc_pushes_date_properties`: a note with `date: 2026-11-20` and `due: 2026-12-01` yields two `WireDate`s, sources `date` and `due`.
  - `build_doc_pushes_datetime_with_offset_as_its_written_day`: `created: 2026-11-20T23:30:00-05:00` yields day `2026-11-20`.
  - `build_doc_skips_non_date_properties`: text, number, bool and list values produce no date.
  - `build_doc_pushes_labels_and_note_link_targets`: a note with `#work` and `[[Project X]]` pushes tag `work` and link `Project X`. Image and hashtag links are not in `links`.
- [ ] **Step 2: Run** `cargo test -p kimun-notes server_client::sync` (package name from `tui/Cargo.toml`). Expected: FAIL.
- [ ] **Step 3: Implement** `build_doc` to fill the three fields. Keys are lowercased. For a `DateTime`, use the date part as written (no time zone conversion). Keep the content hash unchanged. Properties and links live in the note text. When they change, the full-text hash already changes.
- [ ] **Step 4: Run** the sync tests. Expected: PASS.

### Task 3: The vector store filters by note date, folder and tags

**Files:**
- Modify: `server/src/dbembeddings/mod.rs`, `vecsqlite.rs`, `vecqdrant.rs`
- Test: the conformance suite in `dbembeddings/mod.rs`, run by both adapters (the filter checks run on SQLite only) (Qdrant tests run where the existing ones run)

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct DateFilter { pub source: Option<String>, pub from: NaiveDate, pub to: NaiveDate } // inclusive
  #[derive(Clone, Debug, Default)]
  pub struct QueryFilter { pub dates: Option<DateFilter>, pub folder: Option<String>, pub tags: Vec<String> }
  impl QueryFilter { pub fn matches(&self, chunk: &FlattenedChunk) -> bool }
  ```
- `VectorStore::query` gains a last parameter `filter: &QueryFilter`. All callers pass `&QueryFilter::default()` unless Task 5 says otherwise.
- Matching rules, one per field:
  - Date: any note date in the range matches. Its source must equal `source`, ignoring case. When `source` is `None`, any source matches.
  - Folder: the path must start with `folder` plus `/`, ignoring case.
  - Tags: every listed tag must be present.
  - An empty filter matches everything.

- [ ] **Step 1: Write the failing tests** in the conformance suite, against the trait:
  - `query_filters_by_date_range_and_source`: three notes (journal 11-18, journal 11-25, `due` 11-19). A `journal` filter 11-17 to 11-23 returns only the first. A filter with no source returns the first and the third.
  - `filter_applies_before_the_limit`: store five near-identical vectors. Only one note matches the filter. Query with `limit` 2. The test expects the match, because the filter must not run after the cut.
  - `query_filters_by_folder_and_tags`.
  - `store_round_trips_metadata`: `chunks_with_vectors` returns the same `meta` that `store` received.
  - A unit test for `QueryFilter::matches` for each rule above.
- [ ] **Step 2: Run** `cargo test -p kimun-server dbembeddings`. Expected: FAIL.
- [ ] **Step 3: Implement SQLite.** Replace the `date` column with one `meta` TEXT column that holds `NoteMeta` as JSON. The store is exhaustive-scan (adr/0026), so `query` applies `QueryFilter::matches` while it scores, before it keeps the top `limit`. If the column set differs, recreate the table. There is no migration.
- [ ] **Step 4: Implement Qdrant in the minimal way.** SQLite is the default store, so Qdrant gets no server-side filtering.
  - Store the metadata as one payload field, `meta`, that holds `NoteMeta` as a JSON string. Read it back into `NoteMeta`.
  - If `query` gets a filter that is not empty, return an error. The text is `filtered search is not supported on the qdrant store`.
  - An empty filter works as before.

  Task 5 turns this error into an `error:` result for the model. The model then searches without filters.
- [ ] **Step 5: Run** the server tests. Expected: PASS for SQLite. The filter tests skip the Qdrant adapter. A separate Qdrant test checks the error for a non-empty filter and the metadata round trip. It runs where a Qdrant instance is available, as the existing Qdrant tests do.

### Task 4: Store reads for links and date sources

**Files:**
- Modify: `server/src/dbembeddings/mod.rs` (trait, conformance), `vecsqlite.rs`, `vecqdrant.rs`

**Interfaces:**
- Produces on `VectorStore`:
  ```rust
  async fn note_links(&self, collection: &str) -> anyhow::Result<HashMap<String, Vec<String>>>; // path -> link targets as written
  async fn date_sources(&self, collection: &str) -> anyhow::Result<Vec<String>>; // distinct, sorted
  ```
  A missing collection returns an empty value, never an error (the existing contract).
- Reading a whole note reuses `chunks_with_vectors(collection, &[path])` and ignores the vectors. Add no new method for it.

- [ ] **Step 1: Write the failing conformance tests:** `note_links_returns_each_note_once`, `date_sources_lists_distinct_sorted_sources_including_journal`, `missing_collection_returns_empty_for_both_reads`.
- [ ] **Step 2: Run** them. Expected: FAIL.
- [ ] **Step 3: Implement** both reads in both adapters. In SQLite, scan the `meta` JSON in Rust. In Qdrant, scroll the `meta` payload only.
- [ ] **Step 4: Run** the server tests. Expected: PASS.

### Task 5: The tools

**Files:**
- Create: `server/src/tools.rs`
- Modify: `server/src/lib.rs` (add `mod tools;` and `async fn retrieve_filtered`)
- Test: `server/src/tools.rs` test module, with a SQLite store in a temp directory (copy the helper style used in `lib.rs` tests, for example `doc(...)` near line 1875)

**Interfaces:**
- Consumes: Tasks 3 and 4. `KimunRag::retrieve_filtered(&self, collection: &CollectionKey, query: &str, filter: &QueryFilter) -> Result<Vec<ScoredChunk>, RagError>` is the existing `retrieve` with a filter. It embeds the query and calls `store.query(..., CANDIDATE_POOL, filter)`.
- Produces:
  ```rust
  pub struct ToolSpec { pub name: &'static str, pub description: &'static str, pub schema: serde_json::Value }
  pub enum ToolCall {
      Search { query: String, filter: QueryFilter },
      ListDateKeys,
      ReadNote { path: String, section: Option<String> },
      RelatedNotes { path: String },
  }
  pub struct ToolOutput { pub text: String, pub chunks: Vec<ScoredChunk>, pub trail: String }
  pub const TOOL_RESULT_CAP: usize = 12_000; // characters
  pub fn tool_specs() -> Vec<ToolSpec>;
  pub fn parse_call(name: &str, args: &serde_json::Value) -> Result<ToolCall, String>;
  pub struct ToolRunner<'a> { /* &KimunRag, &CollectionKey */ }
  impl ToolRunner<'_> { pub async fn run(&self, call: &ToolCall) -> ToolOutput }
  ```
- Tool names: `search`, `list_date_keys`, `read_note`, `related_notes`. `search` arguments: `query`, `date_from`, `date_to` (both `YYYY-MM-DD`, together), `date_key` (a property key or `journal`), `folder`, `tags`.

- [ ] **Step 1: Write the failing tests:**
  - `parse_call_reads_search_filters`: valid arguments give the right `QueryFilter`. Unknown tool name, bad date text and a missing `query` each return `Err(message)`.
  - `search_with_reversed_range_returns_an_error_text` (Review Focus 1): `date_from` after `date_to` gives a `ToolOutput` whose text starts with `error:` and whose `chunks` is empty.
  - `search_filters_by_journal_range`: three journal notes, one inside the range, only that note's chunks come back, and `trail` reads like `Searching journal entries from 2026-11-17 to 2026-11-23`.
  - `store_error_on_a_filter_becomes_an_error_text`: a store that refuses a non-empty filter gives a `ToolOutput` that starts with `error:` and names the refusal. Use a small fake store. The loop does not stop.
  - `list_date_keys_lists_sources`.
  - `read_note_returns_whole_note_and_one_section`.
  - `read_note_unknown_path_is_an_error_text` (Review Focus 2): a path that is not in the collection, and a path that exists only in another collection, both give `error:` and no note text.
  - `read_note_cuts_at_the_cap_with_a_marker` (Review Focus 5): a note over `TOOL_RESULT_CAP` ends with `[truncated]`.
  - `related_notes_resolves_links_and_builds_backlinks`: a wikilink by name matches the note whose path ends with that name, case-insensitive. A link with `/`, `./` or `../` matches the exact path. A name that matches two notes is listed under both. Backlinks come from scanning `note_links`. A link to no note is listed as missing.
- [ ] **Step 2: Run** `cargo test -p kimun-server tools::`. Expected: FAIL.
- [ ] **Step 3: Implement** `tools.rs`. Keep the link matching small, following adr/0051 (name matches the end of a path, path matches exactly). Cut every `text` at `TOOL_RESULT_CAP`. When it cuts, append `[truncated]`. `chunks` holds what the model can cite (search results, and the chunks of a note that `read_note` returns). Each `trail` text is one plain line.
- [ ] **Step 4: Run** the server tests. Expected: PASS.

### Task 6: Tool calling in the LLM client

**Files:**
- Modify: `server/src/llmclients/mod.rs`
- Test: same file, with a local HTTP stub or request-body unit tests, in the style of the existing wire tests

**Interfaces:**
- Consumes: `ToolSpec` from Task 5.
- Produces:
  ```rust
  pub struct ModelToolCall { pub id: String, pub name: String, pub args: serde_json::Value }
  pub enum ChatMessage {
      User(String),
      Assistant { text: Option<String>, calls: Vec<ModelToolCall> },
      ToolResult { call_id: String, text: String },
  }
  pub enum ModelReply { Text(String), Calls { text: Option<String>, calls: Vec<ModelToolCall> } }
  #[derive(Debug)] pub struct ToolsUnsupported; // implements std::error::Error
  // on trait LLMClient:
  async fn chat(&self, system: &str, messages: &[ChatMessage], tools: &[ToolSpec]) -> anyhow::Result<ModelReply>;
  ```
  The default `chat` body returns `Err(ToolsUnsupported.into())`, so a fake client that implements only `ask` falls back.

- [ ] **Step 1: Write the failing tests**, one set per wire:
  - The request body for `OpenAiCompat` has a `tools` array of `{type:"function", function:{name, description, parameters}}`, and tool results go as `role:"tool"` messages with `tool_call_id`.
  - The `Anthropic` body has `tools` with `input_schema`, tool calls come back as `tool_use` blocks, and results go as `tool_result` blocks in a `user` message. `max_tokens` stays 4096.
  - The `Gemini` body has `tools[].functionDeclarations`. Calls come back as `functionCall` parts. Results go as `functionResponse` parts. Gemini has no call ids, so synthesize `call_{n}`.
  - Parsing: a reply with calls gives `ModelReply::Calls`. A text-only reply gives `Text`.
  - `http_400_about_tools_maps_to_tools_unsupported`: a 400 or 404 response whose body mentions `tools` or `function` becomes `ToolsUnsupported`. Other errors stay ordinary errors.
- [ ] **Step 2: Run** `cargo test -p kimun-server llmclients`. Expected: FAIL.
- [ ] **Step 3: Implement** `chat` in `ChatClient` for the three wires. Reuse the existing HTTP, error-shaping and usage-logging code. Keep `ask` as it is.
- [ ] **Step 4: Run** the tests. Expected: PASS.

### Task 7: The loop

**Files:**
- Create: `server/src/agent.rs`
- Modify: `server/src/lib.rs` (`answer`, `Answer`)
- Test: `server/src/agent.rs` test module, with a scripted fake `LLMClient`

**Interfaces:**
- Consumes: `LLMClient::chat`, `ToolRunner`, `build_retrieval_query`, `KimunRag::rank`, `cut_len`.
- Produces:
  ```rust
  pub const MAX_TOOL_ROUNDS: usize = 5;
  pub const MODEL_ONLY_LABEL: &str = "Not from your notes:";
  pub const NO_MATCH_NOTICE: &str = "No notes matched this question. The answer below comes from the model, not from your notes.";
  pub struct TrailStep { pub text: String }
  pub struct AgentOutcome { pub text: String, pub sources: Vec<(usize, ScoredChunk)>, pub trail: Vec<TrailStep> }
  pub async fn run(rag: &KimunRag, llm: &dyn LLMClient, collection: &CollectionKey, question: &str,
                   history: &[(String, String)], today: chrono::NaiveDate, top_k: usize,
                   on_step: &(dyn Fn(&TrailStep) + Send + Sync)) -> Result<AgentOutcome, RagError>;
  ```
- `KimunRag::answer` gains `today: Option<NaiveDate>` (None uses `chrono::Local::now().date_naive()`) and `on_step`, calls `agent::run`, and returns `Answer { text, sources, trail }` (add the `trail: Vec<TrailStep>` field).

Behavior to build, in order:
1. Run the first search with the conditioned query, rank it, apply the context cut. Record a trail step.
2. Send the model four things:
   - The system prompt. It holds today's date, the tool rules, the citation rule and the `MODEL_ONLY_LABEL` rule.
   - The history.
   - The question.
   - The first results, as a context block numbered `[n]`.
3. While the reply is `Calls` and rounds are below `MAX_TOOL_ROUNDS`: run each call through `ToolRunner`, record its trail step, send the results back. At the cap, send one last request with no tools so the model must answer.
4. Merge the gathered chunks. Drop duplicates that share path, title and text. Number the chunks once, from 1. The model cites by these numbers, so keep a running numbering. Never renumber an earlier chunk.
5. If the final merged set is empty, prefix the text with `NO_MATCH_NOTICE`.
6. Malformed call: an unknown tool or bad arguments returns `error: ...` to the model and counts one. A second malformed call in a row stops the loop and runs the single-pass fallback.
7. If `chat` returns `ToolsUnsupported` on the first round, run the single-pass fallback.
8. Fallback: call the existing `llm.ask` path on the first results. Add a trail step `Tool loop skipped: single-pass answer`.

- [ ] **Step 1: Write the failing tests** with a `FakeLlm` that returns a scripted list of `ModelReply` values and records the messages it received:
  - `answers_in_one_round_when_the_model_asks_for_nothing`: the first search ran, no tool calls, the trail has one step.
  - `runs_a_tool_then_answers`: the scripted model calls `search` with a journal range, then answers. The trail has the automatic step and the tool step. Sources contain the chunks from both searches, numbered 1..n with no gaps and no repeats.
  - `stops_at_the_step_cap`: a model that always calls tools gets exactly `MAX_TOOL_ROUNDS` rounds, then one tool-less request.
  - `bad_call_is_returned_to_the_model_and_counts_once` (Review Focus 4): an unknown tool name gives an `error:` result and the loop goes on.
  - `two_malformed_calls_in_a_row_fall_back_to_single_pass`.
  - `tools_unsupported_falls_back_and_the_trail_says_so`.
  - `empty_results_add_the_no_match_notice`.
  - `prompt_contains_today_and_the_label_rule`: check the `system` text the fake received.
  - `missing_today_uses_the_server_clock`.
- [ ] **Step 2: Run** `cargo test -p kimun-server agent::`. Expected: FAIL.
- [ ] **Step 3: Implement** `agent.rs` and update `KimunRag::answer`. Update the doc comment of `answer` and `build_retrieval_query` to say the loop now exists and that the conditioned query still feeds the first search.
- [ ] **Step 4: Run** `cargo test -p kimun-server`. Expected: PASS, including the existing `answer` tests (adjust their call sites for the new parameters).

### Task 8: The request, the response and the client transport

**Files:**
- Modify: `server/src/handlers.rs` (`AnswerRequest`, `answer_handler`), `tui/src/server_client/dto.rs`, `tui/src/server_client/mod.rs` (`ask`)
- Test: `server/src/handlers.rs` and `tui` DTO tests

**Interfaces:**
- `AnswerRequest` gains `#[serde(default)] pub today: Option<chrono::NaiveDate>`.
- The answer job result gains `trail: Vec<String>` (the step texts).
- Client: `QueryRequest` gains `today: Option<String>` with `skip_serializing_if = "Option::is_none"`. `AnswerResult` gains `#[serde(default)] pub trail: Vec<String>`.
- `RagClient::ask` sends the client's local date (`chrono::Local::now().date_naive()` formatted `YYYY-MM-DD`). Keep its other parameters.

- [ ] **Step 1: Write the failing tests:** the server request parses with and without `today`. The job result JSON contains `trail`. The client serializes `today`, and `AnswerResult` parses a body with and without `trail`.
- [ ] **Step 2: Run** the server and `tui` tests for these modules. Expected: FAIL.
- [ ] **Step 3: Implement** the fields and wiring. The handler passes `request.today` to `rag.answer` with a no-op `on_step`.
- [ ] **Step 4: Run** both packages' tests. Expected: PASS.

### Task 9: The streaming endpoint

**Files:**
- Modify: `server/src/handlers.rs`, `server/src/startup.rs` (route near line 340), `tui/src/server_client/{dto.rs,mod.rs}`
- Test: `server/src/startup.rs` route tests (see how line 748 tests a route), and a client parse test

**Interfaces:**
- Route: `POST /api/answer/stream`, same body as `/api/answer`, same auth and semantic-only rejection (`RagError::SemanticOnly` before the stream starts).
- Response: `text/event-stream`. Events, each with a JSON `data` line:
  - `event: step`, data `{"text": "..."}` for each trail step, in order.
  - `event: answer`, data `{"answer": "...", "sources": [ChunkResult...], "trail": ["..."]}` once, last.
  - `event: error`, data `{"error": "..."}`. If the loop fails, the server sends this event last and closes the stream.
- Client: `RagClient::ask_stream(...)` returns a `Stream` of `AnswerEvent::{Step(String), Answer(AnswerResult), Error(String)}`. Choose the stream type that `tui`'s dependencies already support. Do not add a new HTTP crate.

- [ ] **Step 1: Write the failing tests:**
  - A scripted fake LLM behind the route yields `step`, `step`, then `answer` in that order.
  - A semantic-only server returns the existing error status, not a stream.
  - A loop failure sends one `error` event and closes.
  - The client parser turns a raw event text into `AnswerEvent` values, including events split across chunk boundaries.
- [ ] **Step 2: Run** the tests. Expected: FAIL.
- [ ] **Step 3: Implement** the handler with axum's `Sse`. Bridge `on_step` to a `tokio::sync::mpsc` channel. Implement the client reader.
- [ ] **Step 4: Run** the tests. Expected: PASS.

### Task 10: Documentation and the seven gates

**Files:**
- Modify: `server/README.md`. Update the API section for `today`, `trail` and `/api/answer/stream`.
- Modify: `server/CHANGELOG.md`. Check `release-plz.toml` first. If release-plz writes the changelog, leave the file alone.
- Modify: `server/bruno/`. If the folder has one request per endpoint, add one for the stream.
- Modify: `adr/0065-the-answer-is-a-bounded-tool-loop-over-pushed-data.md` and `specs/agentic-ask.md`. Step 1 says how.

- [ ] **Step 1: Update** the server README and the Bruno requests. In the README, say that older notes carry no dates, tags or links. They stay that way until they change. A wipe and a new push of the collection also fixes them.
- [ ] **Step 2: Touch the sources, then run the seven gates** from `ci-gates-before-push` in order:
  ```bash
  rtk proxy find core server tui -name '*.rs' -exec touch {} +
  cargo fmt --all -- --check
  grep -rn --include='*.rs' -E '\bcrate::' tui/src/ropetext tui/src/server_client | grep -vE '^[^:]+:[0-9]+: *//' | grep -vE '\bcrate::(ropetext|server_client)::'
  python3 .github/scripts/check-host-fs.py
  cargo clippy --workspace --all-targets -- -D warnings
  cargo check --profile bench --workspace --all-targets
  RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
  cargo test --workspace
  ```
  Expected: every command passes, and the `grep` prints nothing. Run `cargo fmt --all -- --check` once more at the end.
- [ ] **Step 4: Report** the results per gate and stop. Do not commit.

---

## Calls made without you

- `TOOL_RESULT_CAP` is 12,000 characters.
- Clients push links as written. When a tool runs, the server resolves them. The alternative is to resolve at push time. That goes stale after a target note is created later.
- Qdrant refuses filtered searches. It stores the metadata only, as a JSON string. SQLite is the default store.
- A second malformed call in a row triggers the fallback.

## Self-review notes

- Spec coverage, by task:
  - Tasks 1 and 2: the pushed fields.
  - Task 3: the store filter.
  - Task 4: links and date keys.
  - Task 5: the four tools, the vault lock and the size cap.
  - Task 6: native tool calling.
  - Task 7: the loop, the first search, the step cap, the fallback, the notices and the date prompt.
  - Task 8: the request date and the trail in the one-shot answer.
  - Task 9: streaming.
  - Task 10: docs and gates.
  - The frontend display is the follow-on plan.
- Type names match across tasks: `NoteDate`, `NoteMeta`, `QueryFilter`, `DateFilter`, `ToolSpec`, `ToolCall`, `ToolOutput`, `ModelReply`, `ChatMessage`, `TrailStep`, `AgentOutcome`.
