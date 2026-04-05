# Kimun

A note taking app split into two components:

- **core**: all file operations, indexing, and note taking functionality
- **ui** (TUI): interaction and presentation layer only

## Rules

- All file modifications and path manipulation must be implemented in core, never in the TUI
- Never hardcode the `.md` extension or `/` path separator — use existing core functions for cleaning up note paths, removing extensions, or splitting paths into slices
- If a new path or file operation is needed, implement it in core
- The `NoteVault` abstraction sits on top of the OS filesystem and must work on Windows, macOS, and Linux
  - Only accept characters valid on all three major filesystems
  - Paths are case-insensitive; default to lowercase
