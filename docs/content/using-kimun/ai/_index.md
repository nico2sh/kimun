+++
title = "AI Integration"
weight = 20
sort_by = "weight"
+++

# AI Integration

Kimün offers two ways to let an AI assistant work with your vault: the **CLI skill** and the **MCP server**. Both give an AI agent read and write access to your notes — the right choice depends on what kind of tool you are using and how tightly you want the integration to fit.

## Choosing an approach

| | CLI skill | MCP server |
|---|---|---|
| **Works with** | Any tool that supports agentskills (Claude Code, Codex, Gemini CLI, …) | Any MCP-compatible client (Claude Desktop, Claude Code, Zed, Cursor, …) |
| **How it works** | The AI runs `kimun` shell commands on your behalf | The AI calls structured tools exposed over the MCP protocol |
| **Setup** | Copy one file to your skills directory | One-line client configuration |
| **Process model** | A new `kimun` process per command | One long-running `kimun mcp` process managed by the client |
| **Best for** | Coding assistants and agents that already run shell commands | Desktop apps and editors with native MCP support |

**Use the CLI skill** if you primarily work inside a terminal-based coding assistant like Claude Code. The skill teaches the agent the full `kimun` command surface so it can create notes, search the vault, and log journal entries as part of any session.

**Use the MCP server** if you use a desktop AI client such as Claude Desktop, or an editor with MCP support. The server exposes the same operations as structured tool calls and also provides prompt templates for journal reviews, connection finding, and brainstorming.

Both approaches can run simultaneously — the TUI, the CLI, and the MCP server all share the same SQLite index with safe concurrent reads.
