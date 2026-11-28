+++
title = "MCP Server"
weight = 22
+++

# MCP server

MCP (Model Context Protocol) is a standard way to connect an AI app to other tools. Kimün includes an MCP server. Your AI app starts it when it needs it. You do not start it yourself.

## Set up

For Claude Code, run this command:

```sh
claude mcp add kimun -- kimun mcp
```

For another app, run this command:

```sh
kimun mcp --print-config
```

It prints the settings to add to your AI app. The JSON block goes to standard output. Copy it into the settings of the app. The `claude mcp add` line goes to standard error, so you can send standard output straight into a settings file. The output names the program as `kimun`. If `kimun` on your path is a different program, or is not on your path, the output shows the full path instead.

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

Then restart your AI app.

## Tools

These tools work without a Kimün server:

- `search_notes` searches by words, labels, properties, and links. It also filters by date, with `date_from`, `date_to`, and `date_key`.
- `get_chunks` reads one section of a note. Give it a heading name in `section`, and it returns the text under that heading. An empty `section` lists the headings. `show_note` reads a whole note.
- `list_notes` lists the notes in a folder.
- `get_backlinks` and `get_outlinks` show which notes link to a note, and which notes it links to.
- `get_properties`, `set_property`, and `remove_property` read and change the properties of a note.
- `create_note`, `append_note`, `replace_in_note`, `overwrite_note`, and `delete_note` write notes.
- `rename_note` and `move_note` rename or move a note. Kimün updates the links in your other notes.
- `journal` adds to your journal. `quick_note` saves a note to your inbox.
- `vault_info` gives today's date, the date keys, the state of the Kimün server, and whether the vault is a [wiki](@/using-kimun/wiki.md).
- `wiki_init`, `wiki_check`, `wiki_catalogue`, `wiki_stamp`, and `wiki_clip` work with a wiki. They do the same as the `kimun wiki` commands. `wiki_clip` is the only tool that reaches the network. It saves a web page as a source, and it refuses addresses that are not public https addresses. See [Wiki](@/using-kimun/wiki.md).

This tool needs a Kimün server:

- `semantic_search` finds notes by meaning. It needs a server with an embedder. It does not need an LLM key.

Tell the AI app dates as `YYYY-MM-DD`. The AI app works out phrases such as "last week" itself. Both ends of a date range count.

## Wiki state

The tool `vault_info` ends with the wiki state. A vault that is not a wiki gives the line `wiki: not declared`. A wiki gives the line `wiki: declared` and then the folders, the log note, the schema note, and the required properties. If the declaration file has a mistake, the tool gives one line that names the mistake, and the rest of the reply stays the same. The AI app reads the schema note before it writes a page.

## Server state

The tool `vault_info` reports the state of the Kimün server and a hint for the next step. The state is one of these:

| State | Meaning |
| --- | --- |
| `none` | No server address is saved. |
| `stopped` | The install script put the server on this computer, and it does not answer. |
| `unreachable` | An address is saved and nothing answers. |
| `unconfigured` | The server answers but has no embedder. |
| `semantic-only` | The server has an embedder and no LLM. `semantic_search` works. |
| `full` | The server has an embedder and an LLM. `semantic_search` works. |

When `semantic_search` cannot run, it gives the same hint. The hint tells the AI app to pass the next step to you, for example `kimun server use http://localhost:7573`. The tools never start the server and never change your computer. You run the step yourself. Then restart your AI app. See [the server page](@/using-kimun/server.md).

## Guidance for the AI

The MCP server sends your AI app a short method for answering questions about your notes. The `kimun-ask` [skill](@/using-kimun/ai-skills.md) holds the full method, with recipes.

## Prompts

The server also offers ready-made requests for a journal review, a weekly review, and sorting your inbox.

Your AI app can ask you before it changes or deletes anything. Kimün also saves a backup of the old text first. See [Backups](@/using-kimun/cli.md#backups).

You can keep the desktop or terminal app open while the AI works.
