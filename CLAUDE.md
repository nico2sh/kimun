# Kimun

A note taking app split into two components:

- **core**: all file operations, indexing, and note taking functionality
- **ui** (TUI): interaction and presentation layer only

## Docs

The `docs/` directory is the Kimün user-facing documentation site. Only end-user documentation belongs there. Plans, specs, and other internal working documents must be stored outside of `docs/`.

## Rules

- All file modifications and path manipulation must be implemented in core, never in the TUI
  - The TUI may still read files directly (`read_to_string`, `File::open`, `metadata`) — those behave the same on every platform. What it must not do is call `rename`, `copy`, `remove_file`, `remove_dir*`, `create_dir*`, `canonicalize`, `set_permissions` or a truncating `write`: those go through `kimun_core::system`. Enforced by `.github/scripts/check-host-fs.py` in CI (see `adr/0044`)
- Never hardcode the `.md` extension or `/` path separator — use existing core functions for cleaning up note paths, removing extensions, or splitting paths into slices
- If a new path or file operation is needed, implement it in core
- Core's public API must use `VaultPath` for vault-internal path arguments and return types — never `PathBuf` or `Path` for note/directory operations within a vault
  - Configuration-level OS paths (workspace root, cache file, log directory) use `SystemPath` from `kimun_core::system` — absolute and normalized by construction. Plain `PathBuf`/`Path` only for raw, as-written config values before they are resolved, and for converting a `VaultPath` back to a real filesystem location
- All direct filesystem operations (`std::fs`, `tokio::fs`) in core must live inside one of two modules, never in `lib.rs` or elsewhere:
  - `nfs` — vault-scoped: notes, attachments, backups inside a workspace, addressed by `VaultPath`
  - `system` — host-scoped: the app's own directories, path resolution, cross-volume moves, atomic replace. Every OS-specific rule (verbatim Windows paths, `EXDEV` vs `ERROR_NOT_SAME_DEVICE`, `HOME`/`USERPROFILE`) belongs here and nowhere else
- A workspace's index is `kimun_core::IndexFile`, never a hand-built path: it owns the `.kimuncache` naming and moves/removes its `-wal`/`-shm` siblings with it
- The `NoteVault` abstraction sits on top of the OS filesystem and must work on Windows, macOS, and Linux
  - Only accept characters valid on all three major filesystems
  - Paths are case-insensitive; default to lowercase
