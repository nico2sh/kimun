# CLI Note Operations Design

**Date:** 2026-03-27
**Status:** Approved

## Overview

Add note-writing CLI commands to Kimun: `note create`, `note append`, and `note journal`. These allow users to create and append to notes directly from the command line without opening the TUI.

## Config Change

### `WorkspaceEntry` — add `quick_note_path`

File: `tui/src/settings/workspace_config.rs`

Add an optional field to `WorkspaceEntry`:

```rust
pub struct WorkspaceEntry {
    pub path: PathBuf,
    pub last_paths: Vec<String>,
    pub created: DateTime<Utc>,
    #[serde(default)]
    pub quick_note_path: Option<String>,
}
```

- Default when absent from TOML: `None` → resolved as `/` (vault root)
- Add helper: `fn quick_note_path(&self) -> &str` returning the inner value or `"/"`
- No migration needed — `#[serde(default)]` handles existing TOML files cleanly

TOML example:
```toml
[workspaces.default]
path = "/Users/user/Notes"
quick_note_path = "/inbox"
```

## Accessing `quick_note_path` in Note Commands

Note operation commands need both the vault and the `quick_note_path`. Rather than use `create_and_init_vault` (which discards settings), note commands use `load_and_resolve_workspace` directly and then resolve `quick_note_path` separately:

```rust
// In run_cli, for Note subcommands:
let (settings, workspace_path, workspace_name) = load_and_resolve_workspace(config_path)?;
let quick_note_path = resolve_quick_note_path(&settings);
let vault = NoteVault::new(&workspace_path).await?;
vault.init_and_validate().await?;
commands::note_ops::run(subcommand, &vault, &quick_note_path).await
```

### `resolve_quick_note_path` helper

Added to `tui/src/cli/helpers.rs`:

```rust
pub fn resolve_quick_note_path(settings: &AppSettings) -> String
```

Logic:
- Phase 1 legacy (`settings.workspace_dir` is `Some`): return `"/"`
- Phase 2 (`settings.workspace_config` is `Some`): get current workspace entry, return `entry.quick_note_path()` (which defaults to `"/"` if not set)
- Fallback: `"/"`

This ensures Phase 1 workspaces always use vault root without error.

## Path Resolution

A shared helper `resolve_note_path(input: &str, quick_note_path: &str) -> Result<VaultPath>` added to `tui/src/cli/helpers.rs`:

1. Strip leading/trailing whitespace from input
2. If the trimmed input is empty → return an error: `"Note path cannot be empty"`
3. If trimmed input starts with `/` → treat as absolute: `VaultPath::note_path_from(input)`
4. Otherwise → join with `quick_note_path`: `VaultPath::note_path_from(format!("{}/{}", quick_note_path, input))`
5. `VaultPath::note_path_from` normalizes the path and ensures `.md` extension — so `dir/note` and `dir/note.md` both resolve to `dir/note.md`

## CLI Command Structure

### New subcommand: `note`

`CliCommand::Note { subcommand: NoteSubcommand }` added to `tui/src/cli/mod.rs`.

In `run_cli`, the `Note` variant is handled similarly to `Workspace`: it calls `load_and_resolve_workspace` directly (not `create_and_init_vault`) so that `quick_note_path` is accessible from settings.

### `NoteSubcommand` enum

New file: `tui/src/cli/commands/note_ops.rs`

```
kimun note create <path> [content]   -- fails if note already exists
kimun note append <path> [content]   -- creates if not exists, appends if exists
kimun note journal [content]         -- appends to today's journal (creates if not exists)
```

### Content input

Both `[content]` arg and stdin are supported:

- If `content` positional arg is provided → use it
- If absent and stdin is not a TTY → read all of stdin as content
- If absent and stdin is a TTY → content is empty string

Detection via `std::io::IsTerminal` (stable since Rust 1.70).

### `note create <path> [content]`

1. Resolve path via `resolve_note_path` (errors on empty path)
2. Call `vault.create_note(&path, content)`
3. `create_note` returns `VaultError::NoteExists` if note exists → surface as: `"Note already exists: <path>"`
4. On success: print `"Note saved: <vault_path>"`

### `note append <path> [content]`

1. Resolve path via `resolve_note_path` (errors on empty path)
2. If content is empty, skip vault operations and exit successfully (no-op — avoids silently growing the file)
3. Try `vault.get_note_text(&path)`:
   - If found: `combined = existing + "\n" + content`, call `vault.save_note(&path, combined)`
   - If `VaultError::FSError(FSError::VaultPathNotFound)`: call `vault.create_note(&path, content)`
   - If `create_note` returns `VaultError::NoteExists` (race condition: note created between the two calls): re-read with `vault.get_note_text`, combine, and call `vault.save_note` — propagate any further errors
4. On success: print `"Note saved: <vault_path>"`

### `note journal [content]`

1. If content is empty, skip vault operations and exit successfully (no-op — avoids silently appending a bare newline)
2. Call `vault.journal_entry()` → returns `(NoteDetails, String)` with today's path and existing content (creates the file if absent)
3. `combined = existing_content + "\n" + content`
4. Call `vault.save_note(&details.path, combined)`
5. On success: print `"Note saved: {}"` using `details.path` — `NoteDetails.path` is a `VaultPath` which implements `Display` (e.g. `/journal/2026-03-27.md`)

## Output

All three commands print a single confirmation line on success:

```
Note saved: /inbox/my-note.md
```

On error, print to stderr and exit with a non-zero code. No `--format` / JSON output — write operations are mutations, not queries.

## Files Changed

| File | Change |
|------|--------|
| `tui/src/settings/workspace_config.rs` | Add `quick_note_path` field + `quick_note_path()` helper to `WorkspaceEntry` |
| `tui/src/cli/helpers.rs` | Add `resolve_quick_note_path` and `resolve_note_path` helpers |
| `tui/src/cli/commands/note_ops.rs` | New file: `NoteSubcommand` enum + `run` function |
| `tui/src/cli/commands/mod.rs` | Add `pub mod note_ops` |
| `tui/src/cli/mod.rs` | Add `CliCommand::Note` variant and dispatch |

## Future Work (not in scope)

- **`note journal --date <YYYY-MM-DD> [content]`** — append to (or create) a journal entry for a specific date, using the same logic as `note journal` but targeting a date-derived path instead of today
- **`note show <path>`** and **`note show --journal [--date <YYYY-MM-DD>]`** — print note content to stdout; the journal variant is syntax sugar that resolves today's (or a given date's) journal path and delegates to the same read logic

## Error Cases

| Situation | Behavior |
|-----------|----------|
| `note create` on existing note | Error: "Note already exists: `<path>`" |
| Empty path argument | Error: "Note path cannot be empty" |
| Empty content for `append`/`journal` | No-op, exit successfully |
| Race condition in `append` (note created between read and create) | Re-read, combine, save |
| Path resolves to a directory | Propagated as vault error |
| Phase 1 legacy workspace (no `WorkspaceEntry`) | `quick_note_path` defaults to `/` |
| Vault not initialized | Existing vault init error handling |
