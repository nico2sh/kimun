+++
title = "MCP Server"
weight = 22
+++

# MCP Server

`kimun mcp` runs kimun as a [Model Context Protocol](https://modelcontextprotocol.io) server over stdio. Any MCP-compatible client (Claude Desktop, Claude Code, Zed, Cursor, and others) can connect to it. The client can manage notes and search the vault without a new process for each operation.

The MCP client starts and manages the `kimun mcp` process. You do not start it yourself.

## Tools

| Tool | Description |
|---|---|
| `create_note` | Create a new note (fails if it already exists) |
| `append_note` | Append text to a note (creates it if absent) |
| `overwrite_note` | Replace the entire body of a note with new content (destructive) |
| `replace_in_note` | Replace text in a note. The match is literal. With `regex: true`, the match is a regex, and `new` can use `$1` or `${name}` captures. The match must be unique, unless you set `replace_all`. With `preview: true`, the tool returns the result and does not write it (destructive) |
| `delete_note` | Delete a note (destructive) |
| `show_note` | Return the full markdown content of a note |
| `get_properties` | Return the frontmatter properties of a note and all its labels, as JSON. The properties are in the order of the note. A key with no value is null. A key that Kimün cannot read is also null, for example a TOML time of day |
| `set_property` | Set a frontmatter property. `value` is a string, a number, true or false, or an array. Kimün gives a string the same type as the values of that key in other notes. If the type does not match, Kimün refuses the change unless you give `type`. For a new key, Kimün picks the type from the form of the string. For a note with no frontmatter yet, `format: "yaml"` selects YAML |
| `remove_property` | Remove a frontmatter property (destructive) |
| `search_notes` | Search the vault with the same [query syntax](@/using-kimun/search.md) as the TUI and CLI |
| `list_notes` | List all notes, optionally filtered by path prefix |
| `journal` | Append to the journal entry of today or of a given date |
| `get_backlinks` | List notes that link to the given note |
| `get_chunks` | Return the sections of a note as structured content |
| `get_outlinks` | List the notes that the given note links to. This includes wikilinks and Markdown links. Each link resolves the way [following it](@/using-kimun/search.md#how-links-point-at-notes) resolves: every note that a shared name matches, and the path that a missing note would get |
| `rename_note` | Rename a note in its current directory. Kimün updates the backlinks in other notes automatically |
| `move_note` | Move a note to a new vault path. Kimün updates the backlinks in other notes automatically |
| `quick_note` | Capture a thought into a timestamped note in the inbox directory |

### Destructive tools and backups

`overwrite_note`, `replace_in_note`, `delete_note`, and `remove_property` carry the MCP `destructive` annotation. A client that supports confirmation can ask you before these tools run.

Like every Kimün edit, destructive operations back up the old content first. This includes the backlink rewrites from `rename_note` and `move_note`. For details, see [Backups](@/using-kimun/cli.md#backups).

## Prompts

Prompt templates load vault content and ask the LLM to reason over it. The MCP client calls a template by name. The server returns a message that is ready to send.

| Prompt | Parameters | Description |
|---|---|---|
| `daily_review` | `date` (optional, YYYY-MM-DD) | Loads the journal entry for a given day (the default is today). Asks the LLM to summarize the accomplishments, find the action items, and note the recurring themes |
| `weekly_review` | `date` (optional, any date in the target week) | Loads all seven journal entries for the week that contains the given date. Asks the LLM to combine the themes, the accomplishments, and the carry-overs |
| `find_connections` | `path` | Loads a note and its backlink list. Asks the LLM to find connections between the note and the rest of the vault that are not obvious |
| `research_note` | `path`, `max_results` (optional) | Searches the vault with the section headings of the note as queries. Asks the LLM to combine what the vault captures and to find the gaps |
| `link_suggestions` | `path`, `max_results` (optional) | Finds vault notes that relate to the topic of the given note but have no link to or from it. Asks the LLM which of these connections to make formal |
| `brainstorm` | `topic` | Searches the vault for a topic. Asks the LLM to generate new ideas that build on the existing notes |
| `triage_inbox` | `max_notes` (optional), `max_context` (optional) | Reviews the inbox notes with related context. Suggests how to organize them: move to the journal, promote to a proper note, or keep in the inbox |

## Resources

The server also exposes notes as MCP resources with the `note://` URI scheme, for example `note://journal/2026-04-02.md`. Clients can browse notes and attach them to their context.

## Client Setup

### Claude Desktop

Add this to `claude_desktop_config.json`:

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

`kimun mcp` and `kimun` (TUI) can run at the same time on the same vault. Both use the same SQLite index, which supports concurrent reads.
