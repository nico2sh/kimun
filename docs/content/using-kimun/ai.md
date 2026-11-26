+++
title = "AI Integration"
weight = 20
+++

# AI assistants

You can let an AI assistant read and write your notes. Kimün gives the assistant two things. The first is the [MCP server](@/using-kimun/ai-mcp-server.md), a set of tools that the assistant calls. The second is the [skills](@/using-kimun/ai-skills.md), short guide files that teach the assistant a method.

The Kimün server is optional. Most tools work without it. Both the tools and the skills back up old text before they change it, so you can undo a mistake. See [Backups](@/using-kimun/cli.md#backups).

## What needs a server

| Tool | Needs a Kimün server |
| --- | --- |
| `search_notes`, with its date filters | No |
| `get_chunks` and `show_note` | No |
| The links tools and the properties tools | No |
| The writing tools | No |
| `vault_info` | No |
| `semantic_search` | Yes, with an embedder. No LLM key is needed |

An embedder is the model that turns text into numbers for the search. The [Kimün server](@/using-kimun/server.md) holds it. Without a server, the assistant uses word search.

## Quick start

### 1. Add the MCP server

For Claude Code, run this command:

```sh
claude mcp add kimun -- kimun mcp
```

For another app, run this command:

```sh
kimun mcp --print-config
```

The command writes the JSON block to standard output. Copy it into the settings of the app. The `claude mcp add` line goes to standard error, so it stays out of the JSON.

Then restart the app.

### 2. Add the skills

Run this command:

```sh
npx skills add nico2sh/kimun
```

To refresh the skills later, run `npx skills update`. If you do not have Node, run `kimun skill install` instead. See [Skills](@/using-kimun/ai-skills.md).

### 3. Add a server (optional)

This step is optional. To get semantic search, install the server and start it at login. This works on Linux and on macOS with Apple silicon:

```sh
curl -fsSL https://kimun.2co.dev/install-server.sh | sh -s -- --service
```

Then save its address and check it:

```sh
kimun server use http://localhost:7573
kimun server status
```

Restart your AI app after you change the address. The MCP server reads the address at its start.

Windows has no install script. Use Docker or WSL2 (Linux inside Windows). The [server page](@/using-kimun/server.md) has the steps.

## Semantic search and Ask

Kimün can also use AI inside the apps. The server finds notes by meaning and answers questions from your notes. You do not need it to use Kimün.
