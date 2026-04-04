# MCP Tools and Prompts v2 Design

**Date:** 2026-04-03
**Status:** Approved

## Overview

Add three MCP tools (`get_outlinks`, `rename_note`, `move_note`) and two MCP prompt templates (`weekly_review`, `link_suggestions`) to the `kimun mcp` server. Also refactor `NoteVault::get_markdown_and_links` to accept a `&VaultPath` directly instead of a `&NoteDetails`.

## Architecture

All additions follow the existing pattern — no new files required:

- `kimun_core/src/lib.rs` — refactor `get_markdown_and_links` signature
- `tui/src/cli/commands/mcp/mod.rs` — 3 new tools
- `tui/src/cli/commands/mcp/prompts.rs` — 2 new prompts

---

## Prerequisite: Refactor `get_markdown_and_links`

The current signature is `get_markdown_and_links(&self, note: &NoteDetails) -> MarkdownNote`, which forces callers to construct a `NoteDetails` before calling it.

**New signature:** `pub async fn get_markdown_and_links(&self, path: &VaultPath) -> Result<MarkdownNote, VaultError>`

The method loads the note text internally and parses it. All existing callers are updated. This makes the API consistent with other vault methods that accept a `&VaultPath`.

---

## Tools

### `get_outlinks`

**Description:** Return the list of notes that this note links to (outgoing wikilinks).

**Parameters:**
- `path: String` — vault-relative path to the note.

**Implementation:**
1. Resolve path via `VaultPath::note_path_from`.
2. Call `vault.get_markdown_and_links(&vault_path)` (refactored).
3. Filter results for `LinkType::Note(path)` variants.
4. For each linked path, attempt to load its title via `get_note_text` + first heading; fall back to `get_clean_name()` if the note can't be read.
5. Return `"path — title"` lines, matching the `get_backlinks` format.

**Error handling:**
- Note not found → `CallToolResult::error` with "Note not found: {path}".
- Vault I/O failure → `Err(McpError::internal_error(...))`.
- No outlinks → `CallToolResult::success` with "No outlinks found."

---

### `rename_note`

**Description:** Rename a note within its current directory (filename only).

**Parameters:**
- `path: String` — vault-relative path to the note.
- `new_name: String` — new filename stem (no extension, no path separator).

**Implementation:**
1. Validate `new_name` contains no `/` — if it does, return `CallToolResult::error` with "Use move_note to change a note's directory."
2. Resolve `from` via `VaultPath::note_path_from(&path)`.
3. Construct `to` by taking `from`'s parent directory and appending `new_name` (with `.md`).
4. Call `vault.rename_note(&from, &to)` — this automatically rewrites backlinks in all other notes.
5. Return success with "Note renamed: {from} → {to}".

**Error handling:**
- `/` in `new_name` → `CallToolResult::error` with hint to use `move_note`.
- Destination already exists → `CallToolResult::error` surfacing the vault error.
- Vault I/O failure → `Err(McpError::internal_error(...))`.

---

### `move_note`

**Description:** Move a note to a new vault path (different directory and/or name).

**Parameters:**
- `path: String` — vault-relative path to the note.
- `new_path: String` — full destination vault-relative path.

**Implementation:**
1. Resolve `from` via `VaultPath::note_path_from(&path)`.
2. Resolve `to` via `VaultPath::note_path_from(&new_path)`.
3. Call `vault.rename_note(&from, &to)` — backlinks in other notes are rewritten automatically.
4. Return success with "Note moved: {from} → {to}".

**Error handling:**
- Destination already exists → `CallToolResult::error` surfacing the vault error.
- Source not found → `CallToolResult::error`.
- Vault I/O failure → `Err(McpError::internal_error(...))`.

---

## Prompts

### `weekly_review`

**Description:** Loads a full week of journal entries and asks the LLM to synthesise themes, accomplishments, and carry-overs across the week.

**Parameters:**
- `date: Option<String>` — any date in YYYY-MM-DD format within the target week; defaults to today.

**Implementation:**
1. Parse date (default today). Invalid format → graceful message "Invalid date '{d}' — expected YYYY-MM-DD."
2. Compute week boundaries using chrono: Monday = `date - date.weekday().num_days_from_monday() days`; Sunday = Monday + 6 days.
3. For each of the 7 days, attempt `get_note_text` on the journal path. Include content if found; note "(no entry)" if `VaultPathNotFound`.
4. Return a single user-role `PromptMessage` with all days laid out chronologically, followed by the synthesis prompt.

**Returned message:**

```
Week of {Monday date} – {Sunday date}

Monday {date}:
---
{content or "(no entry)"}
---

Tuesday {date}:
---
{content or "(no entry)"}
---

[... remaining days ...]

Please review this week:
1. What were the main themes and accomplishments?
2. What carried over unfinished from day to day?
3. What patterns are worth paying attention to?
4. What should be prioritised next week?
```

**Error handling:**
- Invalid date → graceful `Ok(vec![...])` message, never `Err(McpError)`.
- All 7 days missing → prompt still returns with all days marked "(no entry)".
- Vault I/O failure → `Err(McpError::internal_error(...))`.

---

### `link_suggestions`

**Description:** Finds vault notes that are topically related to the given note but not yet linked to or from it, and asks the LLM to evaluate which connections are worth formalising.

**Parameters:**
- `path: String` — vault-relative path to the note.
- `max_results: Option<u32>` — maximum candidates to include; defaults to 5.

**Implementation:**
1. Load source note text via `get_note_text` (not found → graceful message).
2. Extract unique leaf headings from `get_note_chunks` (same approach as `research_note`).
3. Build the "already linked" exclusion set:
   - Outlinks via `get_markdown_and_links(&vault_path)`, filtered for `LinkType::Note`.
   - Backlinks via `get_backlinks(&vault_path)`.
   - The source note itself.
4. Search each heading with `search_notes`; collect results, deduplicate by path, filter out the exclusion set, cap at `max_results`.
5. Load full text of each candidate via `get_note_text`.
6. Return a single user-role `PromptMessage`.

**Returned message:**

```
Here is the note at "{path}":

---
{note_content}
---

Candidate notes not yet linked to or from this note:

=== /candidate/note-a.md ===
{full_text}

=== /candidate/note-b.md ===
{full_text}

For each candidate:
1. Assess whether a meaningful conceptual connection exists.
2. If yes, suggest the exact [[wikilink]] syntax to add and where in the note it fits.
3. If no clear connection, explain briefly why it was surfaced.
```

**Error handling:**
- Note not found → graceful message, never `Err(McpError)`.
- No candidates found → prompt returns with "No unlinked related notes found in the vault."
- Vault I/O failure → `Err(McpError::internal_error(...))`.

---

## Testing

Unit tests in each file using the existing `make_handler()` helper (`TempDir` + `NoteVault::new` + `validate_and_init`).

### Tool tests (`mod.rs`)

**`get_outlinks`:**
- Note with wikilinks returns the linked paths.
- Note with no links returns "No outlinks found."
- Missing note returns an error result.

**`rename_note`:**
- Note is readable at new path after rename; old path no longer exists.
- `/` in `new_name` returns error result with hint.
- Backlinks in other notes are updated (create a linking note, rename target, verify link updated).

**`move_note`:**
- Note is accessible at new path; old path gone.
- Moving to an existing path returns an error result.

### Prompt tests (`prompts.rs`)

**`weekly_review`:**
- A week with some entries: those days include their content; missing days show "(no entry)".
- A date in the middle of the week lands on the correct Monday–Sunday range.
- Invalid date returns graceful message containing "Invalid date".

**`link_suggestions`:**
- Returns candidate notes not already linked to or from the source.
- Notes already in outlinks or backlinks are excluded.
- Empty vault returns graceful "No unlinked related notes found" message.

---

## Out of Scope

- `rename_directory` / `move_directory` — moving whole subtrees (future work).
- Configurable week start day (always Monday).
- `weekly_review` aggregating non-journal notes modified during the week.
