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
- If a path is not found: print an error for that entry to stderr, continue processing remaining paths, still print output for found notes, and exit non-zero at the end
- Exit code: `0` if all paths resolved successfully, non-zero if any were missing
- Output (text or JSON) for successfully found notes is always printed to stdout regardless of whether other paths failed
- When all paths fail (zero notes found): no stdout output (not even an empty JSON envelope), exit non-zero

## Text Output

Entries are separated by a line of 80 `=` characters with no trailing separator after the final entry. Empty metadata fields (no tags, no links, no backlinks) are omitted.

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

Reuses the existing `JsonOutput` envelope (same shape as `notes list` and `search`). The `notes` array contains one entry per successfully found path; not-found paths are skipped (error to stderr). `total_results` reflects only the found notes. `is_listing` is `false` for `note show`.

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

`backlinks` is a top-level field on `JsonNoteEntry` (not inside the nested `metadata` object) because it is a vault-level relationship computed from the index, not a field extracted from the note's own content. It is added as `Option<Vec<String>>` with `#[serde(skip_serializing_if = "Option::is_none")]` — populated only by `show`, invisible in `list` and `search` output.

The `created` field follows the existing convention in the codebase: it is set to `modified_secs` as a fallback (the vault does not separately track creation time).

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

### `run()` signature update

The existing `run()` function in `note_ops.rs` must be updated to accept `workspace_name`, which is needed by `run_show` for the JSON output envelope:

```rust
pub async fn run(
    subcommand: NoteSubcommand,
    vault: &NoteVault,
    quick_note_path: &str,
    workspace_name: &str,
) -> Result<()>
```

The `Show` arm dispatches to `run_show`.

### Call site update in `tui/src/cli/mod.rs`

In the `CliCommand::Note` branch, `_workspace_name` is currently discarded. Rename it to `workspace_name` and pass it to `note_ops::run()`:

```rust
// Before:
let (settings, workspace_path, _workspace_name) = load_and_resolve_workspace(config_path)?;
return commands::note_ops::run(subcommand, &vault, &quick_note_path).await;

// After:
let (settings, workspace_path, workspace_name) = load_and_resolve_workspace(config_path)?;
return commands::note_ops::run(subcommand, &vault, &quick_note_path, &workspace_name).await;
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
2. Call `vault.load_note(&vault_path).await` (returns `Result<NoteDetails, VaultError>`) — if `VaultError::FSError(FSError::VaultPathNotFound { .. })`, print error to stderr with `eprintln!` and set a `had_errors` flag; skip to next path. Other errors propagate immediately with `?`.
3. Extract `NoteContentData` from the returned `NoteDetails` via `note_details.get_content_data()`. Both `NoteDetails` and `get_content_data()` are public (`kimun_core::note::NoteDetails`, `pub fn get_content_data(&self)`).
4. Build `NoteEntryData` from filesystem metadata:
   ```rust
   let meta = tokio::fs::metadata(vault.path_to_pathbuf(&vault_path)).await?;
   let entry_data = NoteEntryData {
       path: vault_path.clone(),
       size: meta.len(),
       modified_secs: meta.modified()
           .map(|t| t.duration_since(UNIX_EPOCH).unwrap().as_secs())
           .unwrap_or(0),
   };
   ```
   `NoteEntryData` has all public fields. This is consistent with how the vault itself builds `NoteEntryData` in `core/src/nfs/mod.rs` (`NoteEntryData::from_path` does the same `fs::metadata` call).
5. Get backlink paths via `vault.get_backlinks(&vault_path).await` (defined at `core/src/lib.rs:416`, returns `Result<Vec<(NoteEntryData, NoteContentData)>, VaultError>`) — map to `Vec<String>` of path strings

After collecting all results:
- If no entries found at all: no stdout output; return `Err(color_eyre::eyre::eyre!("No notes found — all specified paths were missing"))`
- Text: print each formatted entry joined by `NOTE_SEPARATOR` (no trailing separator)
- JSON: build `Vec<JsonNoteEntry>` manually (see below), wrap in `JsonOutput`, serialize and print
- After printing output: if `had_errors` is true, return `Err(color_eyre::eyre::eyre!("One or more notes could not be found"))` to signal non-zero exit. Individual per-path errors were already printed to stderr via `eprintln!`, so color_eyre will add one more summary error line — this is acceptable and consistent with how the rest of the codebase surfaces partial failures.

### JSON assembly in `run_show`

Do not call `format_notes_with_content_as_json` or `format_notes_as_json` — both return a serialized `String` with no way to inject `backlinks`. Build `Vec<JsonNoteEntry>` directly:

```rust
use crate::cli::metadata_extractor::{extract_tags, extract_links, extract_headers};
use crate::cli::json_output::{JsonNoteEntry, JsonNoteMetadata, JsonOutput, JsonOutputMetadata};
use chrono::Utc;

// Per note:
let content = note_details.raw_text.clone();  // NoteDetails.raw_text holds the note text
let tags = extract_tags(&content);
let links = extract_links(&content);
let headers = extract_headers(&content);
let journal_date = vault.journal_date(&vault_path).map(|d| d.format("%Y-%m-%d").to_string());
let path_str = vault_path.to_string();
let path_with_ext = if path_str.ends_with(".md") { path_str.clone() } else { format!("{}.md", path_str) };

let entry = JsonNoteEntry {
    path: path_with_ext,
    title: content_data.title.clone(),
    content,
    size: entry_data.size,
    modified: entry_data.modified_secs,
    created: entry_data.modified_secs,  // fallback: no separate created timestamp
    hash: format!("{:x}", content_data.hash),
    journal_date,
    metadata: JsonNoteMetadata { tags, links, headers },
    backlinks: Some(backlink_paths),
};

// After collecting all entries:
let output = JsonOutput {
    metadata: JsonOutputMetadata {
        workspace: workspace_name.to_string(),
        workspace_path: vault.workspace_path.to_string_lossy().to_string(),
        total_results: notes.len(),
        query: None,
        is_listing: false,
        generated_at: Utc::now().to_rfc3339(),
    },
    notes,
};
print!("{}", serde_json::to_string(&output)?);
```

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

Prints labeled fields (omitting empty ones), then `---\n[content]`.

### Separator

```rust
const NOTE_SEPARATOR: &str = "================================================================================";
```

Defined in `note_ops.rs`. Printed between entries in text output; no trailing separator after the final entry.

## `JsonNoteEntry` change

In `tui/src/cli/json_output.rs`, add to the existing struct:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub backlinks: Option<Vec<String>>,
```

The existing struct literal in `format_notes_with_content_as_json` must also be updated to include `backlinks: None` — otherwise the struct literal will fail to compile. All existing callers (`notes list`, `search`) produce `None` here.

## Error Cases

| Situation | Behavior |
|-----------|----------|
| Path not found | Error to stderr, skip entry, continue, exit non-zero |
| All paths not found | Error to stderr for each, no stdout output, exit non-zero |
| Empty path argument | Error: "Note path cannot be empty" (from `resolve_note_path`) |
| Path resolves to a directory | Propagated as vault error to stderr, skip entry |
| No `--format` specified | Defaults to `text` |

## Files Changed

| File | Change |
|------|--------|
| `tui/src/cli/commands/note_ops.rs` | Add `Show` variant to `NoteSubcommand`; update `run()` signature to add `workspace_name`; add `run_show`, `format_note_show_text`, `NOTE_SEPARATOR` |
| `tui/src/cli/json_output.rs` | Add `backlinks: Option<Vec<String>>` to `JsonNoteEntry`; update struct literal in `format_notes_with_content_as_json` to set `backlinks: None` |
| `tui/src/cli/mod.rs` | Rename `_workspace_name` to `workspace_name`; pass it to `note_ops::run()` |
