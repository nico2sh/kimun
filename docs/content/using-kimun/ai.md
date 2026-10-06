+++
title = "AI Integration"
weight = 20
+++

# AI Integration

There are two ways to let an AI assistant work with your vault: the [CLI skill](@/using-kimun/ai-skills.md) and the [MCP server](@/using-kimun/ai-mcp-server.md). Both give an AI agent read and write access to your notes. Which one to use depends on the tool you use.

## Choosing an approach

| | [CLI skill](@/using-kimun/ai-skills.md) | [MCP server](@/using-kimun/ai-mcp-server.md) |
|---|---|---|
| **Works with** | Any tool that supports agentskills (Claude Code, Codex, Gemini CLI, …) | Any MCP-compatible client (Claude Desktop, Claude Code, Zed, Cursor, …) |
| **How it works** | The AI runs `kimun` shell commands on your behalf | The AI calls structured tools exposed over the MCP protocol |
| **Setup** | Copy one file to your skills directory | One-line client configuration |
| **Process model** | A new `kimun` process per command | One long-running `kimun mcp` process managed by the client |
| **Best for** | Coding assistants and agents that already run shell commands | Desktop apps and editors with native MCP support |

Use the [CLI skill](@/using-kimun/ai-skills.md) if you mostly work in a terminal-based coding assistant like Claude Code. The skill teaches the agent the `kimun` commands, so it can create, edit, and remove notes, search the vault, and log journal entries during a session.

Use the [MCP server](@/using-kimun/ai-mcp-server.md) if you use a desktop AI client such as Claude Desktop, or an editor with MCP support. The server exposes the same operations as structured tool calls. It also provides prompt templates for journal reviews, finding connections between notes, and brainstorming.

Both offer the same destructive operations (overwrite, replace, delete), and both [back up the old content first](@/using-kimun/cli.md#backups), so you can recover from an AI edit. Both can also run while the TUI is open, since they share the same SQLite index, which supports concurrent reads.

## Semantic search and Ask

Kimün can also use AI inside the TUI. The optional [Kimün server](@/using-kimun/server.md) adds semantic search, which finds notes by meaning rather than by exact words, and Ask, which answers natural-language questions from your notes and cites the sources. It runs as a separate service and can run entirely on your machine. Kimün works the same without it.
