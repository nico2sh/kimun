# MCP Prompt Templates Design

**Date:** 2026-04-02
**Status:** Approved

## Overview

Add four MCP prompt templates to the `kimun mcp` server: `daily_review`, `find_connections`, `research_note`, and `brainstorm`. Each prompt fetches vault data at request time and returns it embedded in a `Vec<PromptMessage>` so any MCP-compatible client (Claude Desktop, Claude Code, etc.) receives rich context without needing to call tools first.

## Architecture

`tui/src/cli/commands/mcp.rs` is refactored into a directory:

```
tui/src/cli/commands/mcp/
    mod.rs       ← existing content: KimunHandler struct, 8 tools, ServerHandler, run()
    prompts.rs   ← new: #[prompt_router] impl block with 4 prompt methods + parameter structs
```

### Changes to `mod.rs`

- `KimunHandler` gains a `prompt_router: PromptRouter<KimunHandler>` field.
- `KimunHandler::new()` initialises both `tool_router` and `prompt_router`.
- The `ServerHandler` impl receives both `#[tool_handler]` and `#[prompt_handler]` attributes.
- `ServerCapabilities::builder()` gains `.enable_prompts()` alongside the existing `.enable_tools()` and `.enable_resources()`.

### `prompts.rs`

Contains a single `#[prompt_router]` impl block on `KimunHandler` with the four prompt methods, plus one parameter struct per prompt. Shares `KimunHandler`'s `Arc<NoteVault>` for all vault access.

## Prompts

### `daily_review`

**Description:** Loads today's (or a specified date's) journal entry and asks the LLM to review the day.

**Parameters:**
- `date: Option<String>` — YYYY-MM-DD format; defaults to today.

**Data fetched:** Journal entry text for the given date via `NoteVault::journal_entry()` (today) or `load_or_create_note` path construction (specific date).

**Returned message (user role):**

```
Here is my journal entry for {date}:

---
{journal_content}
---

Please review this journal entry:
1. Summarize what was accomplished
2. Identify any action items or follow-ups
3. Note any recurring themes worth tracking
```

If no journal entry exists for the date, the message says so instead of including empty content.

---

### `find_connections`

**Description:** Loads a note and its backlink paths, asks the LLM to identify non-obvious connections to the rest of the vault.

**Parameters:**
- `path: String` — vault-relative path to the note.

**Data fetched:**
- Full note text via `NoteVault::get_note_text`.
- Backlink paths (not content) via `NoteVault::get_backlinks`.

The LLM can call the `show_note` tool to read any backlinked note it wants to explore further.

**Returned message (user role):**

```
Here is the note at "{path}":

---
{note_content}
---

Notes that link to this note:
- /path/to/note-a.md
- /path/to/note-b.md

Identify non-obvious conceptual connections between this note and the rest of the vault. What themes link them? What ideas are worth exploring further?
```

If no backlinks exist, that section is omitted.

---

### `research_note`

**Description:** Synthesises vault knowledge related to a note by searching on its section headings.

**Parameters:**
- `path: String` — vault-relative path to the note.
- `max_results: Option<u32>` — maximum related notes to include; defaults to 5.

**Data fetched:**
1. Full note text via `get_note_text`.
2. Note chunks via `get_note_chunks` — extract the unique set of breadcrumb leaf headings (the most-specific heading per chunk).
3. Run `search_notes` for each unique heading; merge and deduplicate results, excluding the source note itself; cap at `max_results`.
4. Load full text of each result via `get_note_text`.

**Returned message (user role):**

```
Here is the note at "{path}":

---
{note_content}
---

Related notes found by searching section topics ({comma-separated topic list}):

=== /related/note-a.md ===
{full_text}

=== /related/note-b.md ===
{full_text}

Synthesize what the vault knows about this topic. What key ideas are captured? What gaps exist? What questions remain unanswered?
```

If no related notes are found, the related-notes section is replaced with "No related notes found in the vault."

---

### `brainstorm`

**Description:** Searches the vault for a topic and asks the LLM to generate new ideas that build on existing notes, with a suggestion of which note to update.

**Parameters:**
- `topic: String` — free-text topic to brainstorm.

**Data fetched:**
1. `search_notes(topic)` — top 5 results.
2. Full text of each result via `get_note_text`.
3. Suggested append target: the first search result's path (most relevant hit).

**Returned message (user role):**

```
I want to brainstorm ideas about: "{topic}"

Here is relevant content from my vault:

=== /relevant/note-a.md ===
{full_text}

=== /relevant/note-b.md ===
{full_text}

Based on my existing notes:
1. Generate 5–10 new ideas related to "{topic}" that build on what's already captured
2. Avoid repeating existing content
3. Suggested note to append new ideas to: /relevant/note-a.md
```

If no vault content is found, the vault-context section is omitted and the suggestion line is removed.

## Error Handling

- **Note not found / no journal entry** — return a graceful user message within `Vec<PromptMessage>`; never return `Err(McpError)`. Missing content is feedback, not a protocol error.
- **Invalid date format** — return a graceful message: `"Invalid date '{d}' — expected YYYY-MM-DD"`.
- **Vault I/O or DB failure** — return `Err(McpError::internal_error(...))`.
- **`research_note` with no search results** — prompt still returns; vault-context section replaced with a "no related notes found" note.
- **`brainstorm` with no search results** — prompt still returns without the vault-context section or suggestion line.

## Testing

- Unit tests in `tui/src/cli/commands/mcp/prompts.rs`, one per prompt.
- Each test uses the same `make_handler()` helper as the tool tests: `TempDir` + `NoteVault::new` + `validate_and_init`.
- Assertions: returned `Vec<PromptMessage>` is non-empty; first message text contains expected substrings (note content, date, topic, etc.).
- Smoke test (`tui/tests/mcp_smoke.rs`) gains a `prompts/list` assertion alongside the existing `tools/list` check, verifying all four prompt names appear.

## Out of Scope

- Prompt templates with assistant-role messages (multi-turn scaffolding).
- `weekly_review` or `inbox_triage` prompts (future work).
- Configurable prompt wording via `kimun_config.toml`.
