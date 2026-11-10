+++
title = "AI Integration"
weight = 20
+++

# AI Integration

You can let an AI assistant work with your vault in two ways: the [CLI skill](@/using-kimun/ai-skills.md) and the [MCP server](@/using-kimun/ai-mcp-server.md). Both give an AI agent read and write access to your notes. The best choice depends on the tool you use.

## Choosing an approach

| | [CLI skill](@/using-kimun/ai-skills.md) | [MCP server](@/using-kimun/ai-mcp-server.md) |
|---|---|---|
| Works with | Any tool that supports agentskills (Claude Code, Codex, Gemini CLI, …) | Any MCP-compatible client (Claude Desktop, Claude Code, Zed, Cursor, …) |
| How it works | The AI runs `kimun` shell commands for you | The AI calls structured tools that the MCP protocol exposes |
| Setup | Copy one file to your skills directory | Add one line to the client configuration |
| Process model | A new `kimun` process for each command | One long-running `kimun mcp` process that the client manages |
| Best for | Coding assistants and agents that already run shell commands | Desktop apps and editors with native MCP support |

If you mostly work in a terminal-based coding assistant like Claude Code, use the [CLI skill](@/using-kimun/ai-skills.md). The skill teaches the agent the `kimun` commands. During a session, the agent can create, edit, and remove notes, search the vault, and log journal entries.

If you use a desktop AI client such as Claude Desktop, or an editor with MCP support, use the [MCP server](@/using-kimun/ai-mcp-server.md). The server exposes the same operations as structured tool calls. It also provides prompt templates for journal reviews, for finding connections between notes, and for brainstorming.

Both options offer the same destructive operations (overwrite, replace, delete). Both [back up the old content first](@/using-kimun/cli.md#backups), so you can recover from an AI edit. Both can run while the TUI is open, because they share the same SQLite index, which supports concurrent reads.

## Semantic search and Ask

Kimün can also use AI inside the TUI. The optional [Kimün server](@/using-kimun/server.md) adds two features. Semantic search finds notes by meaning instead of by exact words. Ask answers natural-language questions from your notes and cites the sources. The server runs as a separate service, and it can run entirely on your machine. Kimün works the same without it.
