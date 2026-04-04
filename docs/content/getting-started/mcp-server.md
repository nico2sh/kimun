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
| `get_outlinks` | List notes that the given note links to (outgoing wikilinks) |
| `rename_note` | Rename a note within its current directory; backlinks in other notes are updated automatically |
| `move_note` | Move a note to a new vault path; backlinks in other notes are updated automatically |

## Prompts

Prompt templates load vault content and ask the LLM to reason over it. The MCP client invokes them by name and the server returns a ready-to-send message.

| Prompt | Parameters | Description |
|---|---|---|
| `daily_review` | `date` (optional, YYYY-MM-DD) | Loads the journal entry for a given day (defaults to today) and asks the LLM to summarise accomplishments, identify action items, and note recurring themes |
| `weekly_review` | `date` (optional, any date in the target week) | Loads all seven journal entries for the week containing the given date and asks the LLM to synthesise themes, accomplishments, and carry-overs |
| `find_connections` | `path` | Loads a note and its backlink list, then asks the LLM to identify non-obvious conceptual connections to the rest of the vault |
| `research_note` | `path`, `max_results` (optional) | Searches the vault using the note's section headings as queries and asks the LLM to synthesise what is captured and identify gaps |
| `link_suggestions` | `path`, `max_results` (optional) | Finds vault notes topically related to the given note but not yet linked to or from it, and asks the LLM to evaluate which connections are worth formalising |
| `brainstorm` | `topic` | Searches the vault for a topic and asks the LLM to generate new ideas that build on existing notes |

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
