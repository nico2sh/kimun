# kimun_core

Core library for [Kimün](https://github.com/nico2sh/kimun) — handles note vault management, file system abstraction, and SQLite-based indexing.

## Usage

```toml
[dependencies]
kimun_core = "0.1"
```

## Overview

The library revolves around `NoteVault`, the main entry point. A vault is a directory of Markdown files. `kimun_core` maintains a `kimun.sqlite` index at the vault root for fast search and metadata queries.

```rust
use kimun_core::NoteVault;

// Open a vault (creates the index if it doesn't exist)
let vault = NoteVault::new("/path/to/notes").await?;

// Validate and sync the index with the filesystem
vault.init_and_validate().await?;
```

## Key Types

### `NoteVault`

The main handle for all vault operations: indexing, browsing, searching, and file management.

| Method | Description |
|--------|-------------|
| `new(path)` | Open a vault at the given path |
| `init_and_validate()` | Check the index and sync any new/removed notes |
| `force_rebuild()` | Delete and fully rebuild the index |
| `recreate_index()` | Rebuild the index without deleting the database file |

### `nfs` — Filesystem Abstraction

- **`VaultPath`** — a vault-relative path (always uses `/` as separator). Use this instead of raw `PathBuf` for all vault operations.
- **`VaultEntry`** — an entry found at a `VaultPath`. The `data` field is one of:
  - `EntryData::Note(NoteEntryData)` — a Markdown file
  - `EntryData::Directory(DirectoryEntryData)` — a subdirectory
  - `EntryData::Attachment` — any other file

### `note` — Note Parsing

Provides `NoteDetails` (title, date, metadata) and `ContentChunk` for splitting note content into indexable sections based on Markdown headings.

### `error` — Error Types

- `VaultError` — top-level error wrapping all vault operations
- `FSError` — filesystem errors
- `DBError` — database errors

## Notes

- Requires Tokio async runtime
- The index file (`kimun.sqlite`) is created automatically in the vault root
- Notes must be `.md` files; other files are treated as attachments
