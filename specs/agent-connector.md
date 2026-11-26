# Agent connector: better tools and guidance for outside agents

Status: design, approved by the owner in conversation on 2026-11-26. Not built.

## Intent

An outside agent must answer questions about the notes in a vault as well as the Ask workspace does. The agent runs in an agent app that Kimün does not control, for example Claude Code, Codex, OpenCode or Hermes. The agent's own model does the thinking.

The owner wants the MCP server and the skills to give that model what Ask gives its model. Ask turns a time reference such as "last week" into a date range. Ask also reads a first result and then looks for linked notes. See `adr/0065`.

The term for the MCP server and the skills together is **Agent connector** (see `CONTEXT.md`).

## Findings that shaped the design

- The MCP `search_notes` tool takes one query string. It returns only lines of the form `path — title`. It returns no dates, no tags and no snippet. The CLI `kimun search --format json` returns much more.
- The query syntax can filter on a date property, for example `%due<2026-11-20`. It has no filter on the journal date, which comes from the note file name. Core derives that date in `core/src/vault_journal.rs`.
- The server instructions do not mention the query syntax or dates. The agent does not know today's date or the date keys of the vault.
- `/api/embeddings` accepts only a query and a context size. The date, folder and tag filters of `adr/0065` exist only inside the answer loop.
- A Kimün server with an embedder and no `[llm]` section runs in the semantic-only state. It answers `/api/embeddings` and rejects `/api/answer`. A local fastembed model needs no network and no key. So semantic search needs no external service.
- Core reports every write to an index observer. Only `RagSync` registers one, and only the terminal app and the desktop app start it. The `kimun mcp` command does not start it. A note that an agent writes through MCP reaches the server only after an app runs a sync.
- The MCP module is `tui/src/cli/commands/mcp/`. It holds eight prompts in `prompts.rs`.

## Goals

- An agent answers a time-bound question ("notes from last week") with a range that it chose and stated.
- When a Kimün server is available, an agent finds notes by meaning. This works with a local embedder and no LLM key.
- An agent decides from a result list which notes to read, without reading every hit.
- An agent can follow links from a note to related notes in a loop.
- The index on the server stays current while an agent writes through MCP.
- The method works in every agent app, with or without the skill installed.
- The result is measurable with a fixed question set.

## Non-goals

- A server-side loop for agents. An `ask_notes` tool that calls `/api/answer` is a later, optional addition. It needs an LLM key.
- Parsing of relative date phrases in the server, the CLI or the MCP server. The agent resolves them.
- Changes to the eight MCP prompts. A later step decides whether to remove or upgrade the five retrieval prompts. It uses the question set to decide.
- Semantic search in the CLI.
- A new write path or new write tools.

## Anti-goals

- A regression in what the MCP server does today.
- Any requirement for an LLM key or an external network service.
- A tool description so long that an agent app pays a large cost on every session.
- Two copies of the guidance text that drift apart.

## Constraints

- Follow `CLAUDE.md`. All path logic and all date logic over vault content live in core. Use `VaultPath`. Do not hardcode the `.md` extension or the `/` separator.
- The MCP code stays in its own module under `tui/src/cli/commands/mcp/`. New tools, the result shape, the instruction text and the sync start-up live there. They call shared code, such as `RagSync` in `tui/src/rag/`. They do not copy it.
- The server never reads the vault (`adr/0018`). Filters work on the fields that the client pushes.
- Few servers are deployed. A re-push of all notes is acceptable. No migration is needed.
- SQLite is the default vector store. Qdrant refuses filtered searches. The tool returns that error.
- User documentation goes in `docs/content/using-kimun/`. This spec and the question set stay outside `docs/`.

## Approaches considered

1. The agent's own model does the thinking. We improve its tools and guidance. Chosen.
2. One `ask_notes` tool calls `/api/answer`. The server runs the loop. This gives the best answers and is the simplest for the agent. It needs a server with an LLM and a key, so it fails for local-only users. It also pays for two models. Rejected for now.
3. Approach 1 for everyone, plus `ask_notes` for servers that can answer. Deferred. The design keeps this a one-tool addition.

The owner chose approach 1.

## Design

### Decisions the owner made

- The agent resolves relative dates. The tools take explicit dates.
- There are two search tools, `search_notes` (words) and `semantic_search` (meaning). The tool descriptions and the server instructions each say which tool fits which question.
- `/api/embeddings` gets optional filters.
- The MCP process runs the sync loop.
- `search_notes` returns a richer result. The output of the existing tool changes.
- A new skill, `kimun-ask`, carries the full method. The server instructions carry a short version. Both come from one source.
- The question set is part of this work.
- The eight prompts stay as they are.

### search_notes

- New optional inputs: `from`, `to` (dates, `YYYY-MM-DD`, both inclusive), `date_key` and `limit`.
- `date_key` names a note date source: the journal date, or a date property key. If `from` or `to` is present, `date_key` is required. The error lists the available keys. This follows the **Note date** term: each date keeps its source.
- A note without the chosen date never matches a date filter.
- The word search needs a journal-date filter. Core does not have one today (see Findings). Core must add it. The builder chooses the form, for example a filter argument on the search call or a query operator. If it is a query operator, the terminal and desktop apps and the docs gain it too.
- Each hit has: `path`, `title`, `dates` (a map from source to date), `tags`, `snippet` (capped), `outlinks` (a count) and `backlinks` (a count). `path` comes first. The CLI JSON shape is the reference for names.
- The result list has a cap. The default is 20 hits. The snippet length has a cap. A broad query must not flood the context of the agent.
- The result format is compact text that a model reads well. The builder chooses JSON lines or JSON.

### semantic_search

- Inputs: `query`, and the optional `from`, `to`, `date_key`, `folder`, `tags` and `limit`.
- Each hit has: `path`, `title`, `date`, `chunk` (the matching text, capped), `score` and `rank`.
- It uses the existing server client (`tui/src/server_client`). It reads the server URL and token from the app configuration.
- It returns a clear error for each of these states: no server configured, server unreachable, server unconfigured (no embedder), no token or a rejected token, and a filtered search on Qdrant. Each message names the fix.
- It needs no LLM. It works on a semantic-only server.

### vault_info

- A new small tool. The name is the author's call.
- It returns three things. The first is today's date (local). The second is the date keys of the vault: the journal date and each date property key. The third is the semantic search state: none, unreachable, unconfigured, semantic-only or full.
- Its purpose is to give the agent the facts that it needs to resolve "last week" and to choose a tool. The server instructions tell the agent to call it first for a time question.

### Server: /api/embeddings filters

- New optional request fields: `from`, `to`, `date_key`, `folder` and `tags`.
- The answer loop already implements these filters (`adr/0065`). Expose the same code. Do not write a second version.
- A request without the fields behaves as today.
- If the agent needs to see that filters exist, update the `Health` capability data. Update the server documentation, and document the Qdrant refusal.
- Amend `adr/0065` or add a short ADR to record that the search endpoint now takes the filters.

### Sync in the MCP process

- While `kimun mcp` runs and a server is configured, the process runs the same sync as the apps: it reconciles at start and drains the observer on the usual interval.
- `spawn_rag_sync` today needs the app message channel (`AppTx`) to report status. Split the sync loop from the status reporting. The MCP process reports status to the log only.
- A write through any MCP write tool must reach the server within one interval. If the server is down, the write must not wait for it and must not fail.
- Two processes, an app and an MCP process, can run the sync for one vault at the same time. The push is keyed by hash. A test must show that a second push of the same note changes nothing.
- On exit, the process makes a best effort to drain once.

### Guidance and the kimun-ask skill

- One source file holds the method. The skill file and the server instructions both come from it. When they differ, a CI script fails. It works like `.github/scripts/check-host-fs.py`. The builder chooses the file location and the build step.
- The method, in short:
  1. For a time reference, call `vault_info`, choose a range, and state the range in the answer.
  2. For an open question, try `semantic_search` first. If the tool reports no server, use `search_notes`. For exact words, labels or properties, call `search_notes`.
  3. Decide from the result list which notes to read. Read only those, and prefer `get_chunks` for one section of a long note.
  4. When a note points to another note, follow it with `get_outlinks` and `get_backlinks`.
  5. Cite the note paths. Say which parts do not come from the notes.
- The server instructions carry about ten lines of this method, plus the existing index and version notes.
- The skill is named `kimun-ask`. Its description says that it works with the MCP tools or with the CLI, and that a Kimün server adds `semantic_search`.
- The skill carries a recipe for each of the five retrieval prompts: `research_topic`, `research_note`, `find_connections`, `link_suggestions` and `brainstorm`. A recipe holds the task instruction and the tool sequence. For example, "related but not yet linked" is a meaning search, minus the notes that the note already links to or is linked from.
- The skill also covers the CLI path. The CLI has no semantic search in this round, so the CLI path uses word search, properties and links only. The skill says so.
- The existing `kimun-cli` skill stays. Update the user documentation in `docs/content/using-kimun/ai-mcp-server.md` and `ai-skills.md`.

### Question set

- A file of about fifteen questions, kept in the repository outside `docs/`, with a small sample vault as a fixture.
- Each question lists the notes that a good answer must read. Time questions also list the range that a good answer must state.
- Five more entries map to the five retrieval prompts. Each one is run once with the old prompt and once with the new tools and the skill.
- Scoring has three parts: did the agent read the right notes, did it state the range, and how many tool calls and tokens did it use.
- The owner runs the set by hand on two or three agent apps. The first run is a baseline before any change.

### Tests (no model needed)

- Date filters for both tools: range, inclusive ends, the journal date, a date property, and a note without the date.
- The result shape, the snippet cap and the hit cap.
- `vault_info` output in each server state.
- `semantic_search` errors in each state, with a fake server.
- `/api/embeddings` filters on SQLite, and the refusal on Qdrant.
- The sync in the MCP process: a write through a tool reaches a fake server, a down server does not fail the write, and a repeated push changes nothing.
- The CI check that the skill and the instructions match.

## Calls made by the author

The owner did not decide these. When the owner reads this doc, the owner must check them.

- Empty query. If `from` or `to` is present, `query` is optional in `search_notes`. Then the call lists the notes in the range. `semantic_search` refuses a range with no query. It points the agent to `search_notes`. The server cannot list a range (ruling R11 during the build). The hits come in note date order, newest first. With a query, the hits come in relevance order. Core needs a date-only listing path for this.
- Result shape. The first line of each `search_notes` hit keeps the form `path — title`. The other fields follow on indented lines. A script that reads the old lines still works.
- Baseline run. The baseline is not a gate on the build. The owner runs it from a build of the commit before any change. The build work continues in parallel.
- Question set fixtures. A script builds the sample vault with dates relative to the day of the run. Each question states its expected range as an offset, for example "the seven days before today". The author drafts the fifteen questions and the owner reviews them.
- Collection identity. The collection comes from the Vault ID, which is stored in the vault. The MCP process must use the same configuration loader as the apps, so the server URL and token match. No other work is needed.
- Journal key. The date key `journal` means the journal date, as in `adr/0065`. `vault_info` shows the exact string to pass for each key. If a date property is also named `journal`, the builder must make the two unambiguous, and `vault_info` must show how.

## Left to the builder

- The names of fields and of `vault_info`.
- The result text format.
- The form of the journal-date filter in core.
- The location of the guidance source file and the build step.
- The order of work. A sensible order is: baseline run of the question set, core date filter, server filters, `search_notes`, `vault_info`, `semantic_search`, sync, guidance and skill, documentation, final run.

## Follow-ups

- Decide whether to remove or upgrade the five retrieval prompts, using the question set.
- For servers that can answer, add `ask_notes`.
- Semantic search in the CLI.
