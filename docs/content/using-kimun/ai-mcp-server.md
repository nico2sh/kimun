+++
title = "MCP Server"
weight = 22
+++

# MCP server

MCP (Model Context Protocol) is a standard way to connect an AI app to other tools. Kimün includes an MCP server. Your AI app starts it when it needs it. You do not start it yourself.

## Set up

Add Kimün to the MCP settings of your AI app:

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

For Claude Code, run this command:

```sh
claude mcp add kimun -- kimun mcp
```

Then restart your AI app.

## What the AI can do

- Search and read your notes.
- Create notes, add text to notes, and change text in notes.
- Delete notes.
- Read and set the properties of a note.
- See which notes link to a note, and which notes it links to.
- Rename or move a note. Kimün updates the links in your other notes.
- Add to your journal, and save quick notes to your inbox.

The server also offers ready-made requests, such as a journal review, a search for links between notes, brainstorming, and sorting your inbox.

Your AI app can ask you before it changes or deletes anything. Kimün also saves a backup of the old text first. See [Backups](@/using-kimun/cli.md#backups).

You can keep the desktop or terminal app open while the AI works.
