# Kimun

A note taking app split into two components:

- **core**: all file operations, indexing, and note taking functionality
- **ui** (TUI): interaction and presentation layer only

## Docs

The `docs/` directory is the Kimün user-facing documentation site. Only end-user documentation belongs there. Plans, specs, and other internal working documents must be stored outside of `docs/`.

## Rules

- All file modifications and path manipulation must be implemented in core, never in the TUI
- Never hardcode the `.md` extension or `/` path separator — use existing core functions for cleaning up note paths, removing extensions, or splitting paths into slices
- If a new path or file operation is needed, implement it in core
- Core's public API must use `VaultPath` for vault-internal path arguments and return types — never `PathBuf` or `Path` for note/directory operations within a vault
  - Exceptions: OS path types are fine for configuration-level values (workspace root path, log directory) and for converting a `VaultPath` back to an OS path when the caller needs the real filesystem location
- All direct filesystem operations (`std::fs`, `tokio::fs`) in core must live inside the `nfs` module, not in `lib.rs` or other modules
- The `NoteVault` abstraction sits on top of the OS filesystem and must work on Windows, macOS, and Linux
  - Only accept characters valid on all three major filesystems
  - Paths are case-insensitive; default to lowercase
