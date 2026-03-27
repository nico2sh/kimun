# CLI Note Show Design

**Date:** 2026-03-27
**Status:** Approved

## Overview

Add `kimun note show <path> [<path>...] [--format text|json]` to display one or more notes with their content and metadata from the command line.

## Command

```
kimun note show <path> [<path>...] [--format text|json]
```

- `--format` defaults to `text`
- Accepts one or more paths
- Path resolution: same rules as `note create`/`append` — relative paths are joined with the workspace's `quick_note_path`; paths starting with `/` are absolute from vault root
- If a path is not found: print an error for that entry to stderr and continue processing remaining paths
- Exit code: `0` if all paths resolved successfully, non-zero if any were missing

## Text Output

Entries are separated by a line of 80 `=` characters. Empty metadata fields (no tags, no links, no backlinks) are omitted.

```
Path:      /inbox/note-a.md
Title:     Note A
Tags:      #rust #cli
Links:     /other.md
Backlinks: /ref.md
---
[raw markdown content]

================================================================================

Path:      /inbox/note-b.md
Title:     Note B
---
[raw markdown content]
```

## JSON Output

Reuses the existing `JsonOutput` envelope (same shape as `notes list` and `search`). The `notes` array contains one entry per successfully found path; not-found paths are skipped (error to stderr). `total_results` reflects only the found notes.

```json
{
  "metadata": {
    "workspace": "default",
    "workspace_path": "/Users/user/Notes",
    "total_results": 2,
    "query": null,
    "is_listing": false,
    "generated_at": "2026-03-27T10:00:00Z"
  },
  "notes": [
    {
      "path": "/inbox/note-a.md",
      "title": "Note A",
      "content": "...",
      "size": 1024,
      "modified": 1711530000,
      "created": 1711530000,
      "hash": "abc123",
      "metadata": {
        "tags": ["#rust"],
        "links": ["/other.md"],
        "headers": []
      },
      "backlinks": ["/ref.md"]
    }
  ]
}
```

`backlinks` is added to `JsonNoteEntry` as `Option<Vec<String>>` with `#[serde(skip_serializing_if = "Option::is_none")]` — populated only by `show`, invisible in `list` and `search` output.

## Implementation

### `NoteSubcommand::Show` variant

Added to the existing `NoteSubcommand` enum in `tui/src/cli/commands/note_ops.rs`:

```rust
Show {
    /// One or more note paths (relative to quick_note_path or absolute from vault root)
    paths: Vec<String>,
    #[arg(long, value_enum, default_value = "text")]
    format: OutputFormat,
},
```

### `run_show` function

```rust
async fn run_show(
    vault: &NoteVault,
    path_inputs: &[String],
    quick_note_path: &str,
    format: OutputFormat,
    workspace_name: &str,
) -> Result<()>
```

For each path:
1. Resolve via `resolve_note_path(input, quick_note_path)`
2. Call `vault.get_note_text(&vault_path)` — if `VaultError::FSError(FSError::VaultPathNotFound)`, print error to stderr and set a flag; skip to next path
3. Collect successfully loaded notes with their backlinks via `vault.get_backlinks(&vault_path)`

After collecting all results:
- Text: print each entry with the separator; if no entries found at all, exit with error
- JSON: format using `format_notes_as_json` with backlinks populated; print result
- Return `Err` if any paths were missing (non-zero exit), `Ok` otherwise

### Data assembly for a single note

There is no vault method returning `(NoteEntryData, NoteContentData)` for a single path. Assemble manually:

- `NoteEntryData`: read file metadata via `tokio::fs::metadata(vault.path_to_pathbuf(&vault_path))` to get `size` and `modified_secs`
- `NoteContentData`: use `NoteDetails::new(&vault_path, &content).get_content_data()`

### Text formatting

A dedicated `format_note_show_text` function in `note_ops.rs`:

```rust
fn format_note_show_text(
    path: &VaultPath,
    content: &str,
    title: &str,
    tags: &[String],
    links: &[String],
    backlinks: &[String],
) -> String
```

Prints labeled fields, omitting empty ones, then `---\n[content]`.

### Separator

`const NOTE_SEPARATOR: &str` — a string of 80 `=` characters — defined in `note_ops.rs`, used between entries in text output.

## `JsonNoteEntry` change

In `tui/src/cli/json_output.rs`, add to the existing struct:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub backlinks: Option<Vec<String>>,
```

All existing construction sites set this to `None`. `run_show` populates it.

## Error Cases

| Situation | Behavior |
|-----------|----------|
| Path not found | Error to stderr, skip entry, continue |
| All paths not found | Error to stderr for each, exit non-zero, no output |
| Empty path argument | Error: "Note path cannot be empty" (from `resolve_note_path`) |
| Path resolves to a directory | Propagated as vault error to stderr, skip entry |
| No `--format` specified | Defaults to `text` |

## Files Changed

| File | Change |
|------|--------|
| `tui/src/cli/commands/note_ops.rs` | Add `Show` variant to `NoteSubcommand`; add `run_show`, `format_note_show_text`, `NOTE_SEPARATOR` |
| `tui/src/cli/json_output.rs` | Add `backlinks: Option<Vec<String>>` to `JsonNoteEntry` |
