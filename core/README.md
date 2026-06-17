# kimun_core

[![crates.io](https://img.shields.io/crates/v/kimun_core.svg)](https://crates.io/crates/kimun_core)
[![docs.rs](https://img.shields.io/docsrs/kimun_core)](https://docs.rs/kimun_core)

Core library for [Kimün](https://github.com/nico2sh/kimun) for indexing and managing Obsidian-like Markdown vaults — note vault management, cross-platform filesystem abstraction, and SQLite-based indexing and search.

[API documentation on docs.rs](https://docs.rs/kimun_core)

## Usage

```toml
[dependencies]
kimun_core = "0.2"
```

## Overview

The library revolves around `NoteVault`, the main entry point. A vault is a directory of Markdown files. `kimun_core` maintains a SQLite index for fast search and metadata queries — by default at `<vault>/kimun.sqlite`, but the cache location is configurable.

```rust
use kimun_core::{NoteVault, VaultConfig};
use kimun_core::nfs::VaultPath;

// Open a vault with the default index location (<vault>/kimun.sqlite).
let vault = NoteVault::new(VaultConfig::new("/path/to/notes")).await?;

// Or place the index outside the vault and enable pre-edit backups:
let vault = NoteVault::new(
    VaultConfig::new("/path/to/notes")
        .with_db_path("/path/to/cache/notes.kimuncache")
        .with_backup(true),
)
.await?;

// Validate and sync the index with the filesystem
vault.validate_and_init().await?;

// Work with notes
let path = VaultPath::new("/projects/ideas.md");
vault.create_note(&path, "# Ideas\n").await?;
let results = vault.search_notes("ideas").await?;
```

Two principles shape the API:

- **The filesystem is the source of truth.** The index is a rebuildable cache; search, backlinks, and suggestions are served from SQLite, but the Markdown files always win.
- **`VaultPath` everywhere.** All vault-internal paths use `VaultPath` — lowercase, `/`-separated, OS-agnostic. Raw `PathBuf` appears only for configuration values such as the workspace root.

## Key Types

### `NoteVault`

The main handle for all vault operations. Highlights:

| Area | Methods |
| --- | --- |
| Lifecycle | `new(VaultConfig)`, `validate_and_init()`, `recreate_index()`, `index_notes()` |
| Notes | `create_note`, `load_note`, `get_note_text`, `save_note`, `append_to_note`, `delete_note`, `quick_note` |
| Rename | `rename_note`, `rename_directory` — rewrites backlinks in other notes so links never break |
| Replace | `replace_in_note`, `preview_replace` — literal or regex, with dry-run |
| Search | `search_notes` (query DSL with labels, ordering, quoting — see `SearchTerms`), `get_backlinks` |
| Labels | `list_labels`, `label_counts`, `notes_with_label`, `suggest_tags_by_prefix` |
| Browse | `get_notes`, `get_all_notes`, `get_directories`, `browse_vault` |
| Journal | `journal_entry`, `journal_path`, `inbox_path` |
| Attachments | `save_attachment`, `generate_attachment_path`, `default_attachments_path` |
| Saved searches | `list_saved_searches`, `save_search`, `delete_saved_search`, `rename_saved_search` |

Mutating operations take per-note async locks internally, so concurrent in-process writers can't lose updates. With `with_backup(true)`, destructive edits (save, delete, replace, rename) first copy the previous content to `<vault>/.kimun/backups/<YYYY-MM-DD>/`, retained for 30 days.

### `nfs` — Filesystem Abstraction

The only module that touches the OS filesystem directly; everything else goes through it.

- **`VaultPath`** — a vault-relative path (always uses `/` as separator). Use this instead of raw `PathBuf` for all vault operations.
- **`NoteEntryData`** — a note (Markdown file) at a `VaultPath`; browse and listing methods return it paired with `NoteContentData`.
- **`DirectoryEntryData`** — a subdirectory at a `VaultPath`.

Other files in the vault are treated as attachments.

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

Pure content extraction, no I/O: `NoteDetails` (title, metadata), `ContentChunk` for splitting note content into indexable sections based on Markdown headings, and `NoteLink`/`LinkType` for links, images, URLs, and hashtags found in note text.

### Search query DSL

`search_notes` accepts a small query language, parsed by `SearchTerms`. Free-text terms run through FTS; prefixed tokens filter and order:

| Prefix (long / short) | Filters by |
| --- | --- |
| `in:` / `@` | breadcrumb (path segment / parent directory) |
| `name:` / `=` | filename |
| `pt:` / `/` | full path |
| `lb:` / `#` | label |
| `lk:` / `<` | backlinks (notes linking *to* the target) |
| `fwd:` / `>` | forward links (notes the target links *to*) |
| `or:` / `^` | order directive (`or:title`, `^file`, …) |

Any prefix can be negated with a leading `-` (`-#draft`), and values can be quoted with `"` or `'` to include whitespace (`="my note"`). See the [`SearchTerms` docs](https://docs.rs/kimun_core/latest/kimun_core/struct.SearchTerms.html) for the full grammar.

### `error` — Error Types

- `VaultError` — top-level error wrapping all vault operations
- `FSError` — filesystem errors (the `nfs` layer)
- `DBError` — index/SQLite errors

## Notes

- Requires Tokio async runtime
- The index file (`kimun.sqlite`) is created automatically in the vault root unless `with_db_path` overrides it; it is a cache and can be deleted at any time
- Notes must be Markdown files; other files are treated as attachments
