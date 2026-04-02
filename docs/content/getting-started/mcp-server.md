+++
title = "MCP Server"
weight = 5
+++

# MCP Server

`kimun mcp` runs kimun as a [Model Context Protocol](https://modelcontextprotocol.io) server over stdio. Any MCP-compatible client (Claude Desktop, Claude Code, Zed, Cursor, etc.) can connect to manage notes and search the vault without spawning a new process per operation.

The MCP client spawns and manages the `kimun mcp` process automatically — you never need to start it manually.

## Tools

| Tool | Description |
|---|---|
| `create_note` | Create a new note (fails if it already exists) |
| `append_note` | Append text to a note (creates it if absent) |
| `show_note` | Return the full markdown content of a note |
| `search_notes` | Search with `@filename`, `>heading`, `/path`, `-exclusion` operators |
| `list_notes` | List all notes, optionally filtered by path prefix |
| `journal` | Append to today's (or a specific date's) journal entry |
| `get_backlinks` | List notes that link to the given note |
| `get_chunks` | Return note sections as structured content |

## Resources

Notes are also exposed as MCP resources with the `note://` URI scheme (e.g. `note://journal/2026-04-02.md`). Clients can browse and attach notes directly to their context.

## Client Setup

### Claude Desktop

Add to `claude_desktop_config.json`:

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

## Running Alongside the TUI

`kimun mcp` and `kimun` (TUI) can run simultaneously against the same vault. Both use the same SQLite index with safe concurrent reads.
