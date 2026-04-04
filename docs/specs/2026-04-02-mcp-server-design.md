# MCP Server Design

**Date:** 2026-04-02
**Status:** Approved

## Overview

Add a `kimun mcp` subcommand that runs kimun as a Model Context Protocol (MCP) server over stdio. This lets any MCP-compatible client (Claude Desktop, Claude Code, Zed, Cursor, etc.) manage notes and search the vault without the skills.md indirection or per-operation CLI subprocess spawning.

The MCP client spawns and manages the `kimun mcp` process automatically — the user never needs to start it manually.

## Architecture

### Location

New module `tui/src/cli/commands/mcp.rs`, registered as a `Mcp` variant in the existing `CliCommand` enum alongside the current CLI subcommands. No new crate is needed.

### Startup

`kimun mcp` initialises a `NoteVault` the same way the CLI does: reads `kimun_config.toml`, picks the active workspace, calls `validate_and_init()` to ensure the index is ready. It then starts the `rmcp` stdio server and runs until the client closes the connection.

### Transport

**stdio** — the MCP client spawns `kimun mcp` as a child process and communicates over its stdin/stdout. This is the standard transport and is supported by every MCP-compatible client.

### Concurrency with the TUI

`kimun mcp` and `kimun` (TUI) can run simultaneously against the same vault. Both use `NoteVault`, which relies on SQLite (concurrent reads safe, writes serialised) and plain markdown files on disk. This is the same situation as two terminal windows running `kimun` today.

### Dependency

Add `rmcp` (official Rust MCP SDK) to `tui/Cargo.toml`. It uses tokio, which the binary already depends on.

## Tools

All tools call `NoteVault` methods directly and return plain text or JSON.

| Tool | Parameters | Maps to |
|---|---|---|
| `create_note` | `path: String`, `content: String` | `NoteVault::create_note` |
| `append_note` | `path: String`, `content: String` | `NoteVault::load_or_create_note` + append + `save_note` |
| `show_note` | `path: String` | `NoteVault::get_note_text` |
| `search_notes` | `query: String` | `NoteVault::search_notes` |
| `list_notes` | `path: Option<String>` | `NoteVault::get_all_notes` filtered by path prefix |
| `journal` | `text: String`, `date: Option<String>` | `NoteVault::journal_entry` + append |
| `get_backlinks` | `path: String` | `NoteVault::get_backlinks` |
| `get_chunks` | `path: String` | `NoteVault::get_note_chunks` |

`search_notes` supports the same query operators as the CLI (`@` filename, `>` heading, `/` path, `-` exclusion).

## Resources

Notes are exposed as MCP resources for clients to browse and attach to context.

**URI scheme:** `note://{path}` — e.g. `note://journal/2026-04-02.md`

**List** (`resources/list`): returns all vault notes, each with:
- `uri` — `note://{path}`
- `name` — title from frontmatter or first heading, falling back to filename
- `mimeType` — `text/markdown`

**Read** (`resources/read`): returns the full markdown content of a note by URI.

Resources are read-only. All mutations go through tools.

## Error Handling

- **Note not found** — return an MCP error response with a descriptive message; never panic
- **Vault not initialised** — `validate_and_init()` runs at startup before the first request is accepted
- **Concurrent write conflicts** — delegated to SQLite and the OS file system, same as today
- **Transport errors** (stdin closed, malformed JSON-RPC) — handled by `rmcp`; the process exits cleanly so the client can respawn it

## Testing

- **Unit tests** — one test per tool handler, constructing a `NoteVault` against a `tempfile` directory (same pattern as existing core tests). Assert on returned content and error cases.
- **Integration smoke test** — spawn `kimun mcp` as a child process, send `initialize` + `tools/list` over stdin, assert the expected tool names are present in the response.

## Client Configuration

### Claude Desktop (`claude_desktop_config.json`)

```json
{
  "mcpServers": {
    "kimun": {
      "command": "kimun",
      "args": ["mcp"]
    }
  }
}
```

### Claude Code

```sh
claude mcp add kimun -- kimun mcp
```

## Out of Scope

- SSE/HTTP transport (stdio covers all target clients)
- RAG/semantic search tools (the `rag` crate is experimental and a separate binary)
- Workspace switching via MCP (uses the active workspace from config)
- Prompt templates or MCP sampling
