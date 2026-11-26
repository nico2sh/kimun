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

- Search your notes by words, labels, properties, links, and dates.
- Search your notes by meaning. This needs a Kimün server. See below.
- Read a whole note, or read only one section of it.
- Ask for today's date and the date keys, so a question such as "what did I write last week?" gets the right range.
- Create notes, add text to notes, and change text in notes.
- Delete notes.
- Read and set the properties of a note.
- See which notes link to a note, and which notes it links to.
- Rename or move a note. Kimün updates the links in your other notes.
- Add to your journal, and save quick notes to your inbox.

## Semantic search

The tool `semantic_search` finds notes by meaning. It needs a [Kimün server](@/using-kimun/server.md) with an embedder (the model that turns text into numbers for the search). It does not need an LLM key. The Kimün server is optional. Set its address in Preferences, as the server page describes. Without a server, the tool says so, and the AI app uses word search.

Tell the AI app dates as `YYYY-MM-DD`. The AI app works out phrases such as "last week" itself. Both ends of a date range count.

## Guidance for the AI

The server sends your AI app a short method for answering questions about your notes. The `kimun-ask` [skill](@/using-kimun/ai-skills.md) holds the full method, with recipes.

## Prompts

The server also offers ready-made requests, such as a journal review, a search for links between notes, brainstorming, and sorting your inbox.

Your AI app can ask you before it changes or deletes anything. Kimün also saves a backup of the old text first. See [Backups](@/using-kimun/cli.md#backups).

You can keep the desktop or terminal app open while the AI works.
