# kimun_core

Core library for [Kimün](https://github.com/nico2sh/kimun) — handles note vault management, file system abstraction, and SQLite-based indexing.

## Usage

```toml
[dependencies]
kimun_core = "0.3.1"
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

| Method                  | Description                                          |
| ----------------------- | ---------------------------------------------------- |
| `new(path)`           | Open a vault at the given path                       |
| `init_and_validate()` | Check the index and sync any new/removed notes       |
| `force_rebuild()`     | Delete and fully rebuild the index                   |
| `recreate_index()`    | Rebuild the index without deleting the database file |

### `nfs` — Filesystem Abstraction

- **`VaultPath`** — a vault-relative path (always uses `/` as separator). Use this instead of raw `PathBuf` for all vault operations.
- **`VaultEntry`** — an entry found at a `VaultPath`. The `data` field is one of:
  - `EntryData::Note(NoteEntryData)` — a Markdown file
  - `EntryData::Directory(DirectoryEntryData)` — a subdirectory
  - `EntryData::Attachment` — any other file

#### Path case handling

Vault files are often synced across operating systems (e.g. Linux and macOS) using tools like Syncthing, iCloud, or Git. macOS and Windows use case-insensitive filesystems by default, while Linux is case-sensitive. Without normalisation, a file created as `Projects/MyNote.md` on macOS and `projects/mynote.md` on Linux would be treated as two different files after syncing, corrupting the vault.

To keep vaults portable and consistent across all platforms, `VaultPath` always normalises path components to **lowercase**, regardless of the operating system. This means `/Projects/MyNote.md` and `/projects/mynote.md` are identical vault paths.

When performing disk I/O (reading, writing, deleting, renaming), the library resolves each path component **case-insensitively** against what is actually on disk. If a directory named `Journal/` exists on a case-sensitive filesystem (e.g. Linux), a vault path of `/journal/2024-01-01.md` will correctly resolve to `Journal/2024-01-01.md` instead of creating a duplicate `journal/` directory.

The resolution strategy:
1. Start from the vault root.
2. For each lowercase component, scan the parent directory for a case-insensitive match.
3. If a match exists, use the on-disk name for that component; otherwise use the lowercase name (for new files/directories being created).

This is implemented in `nfs::resolve_path_on_disk` (async) and `nfs::resolve_path_on_disk_sync`, which are used internally by all file operations.

> **Recommendation:** Create files and directories through the Kimun library or app rather than directly on the filesystem. Kimun will most likely resolve externally created files with uppercase names correctly, but using the app ensures paths are normalised from the start and avoids any potential cross-platform inconsistencies.

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
