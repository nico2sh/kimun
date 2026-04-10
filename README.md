# Kimün

[![Crates.io](https://img.shields.io/crates/v/kimun-notes)](https://crates.io/crates/kimun-notes)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-nico2sh.github.io%2Fkimun-blue)](https://nico2sh.github.io/kimun/)

[![Built With Ratatui](https://ratatui.rs/built-with-ratatui/badge.svg)](https://ratatui.rs/)

<p align="center">
  <img src="docs/static/img/kimun.png" alt="Kimün logo" width="128"/>
</p>


A terminal-based notes app focused on simplicity and powerful search.

It doesn't try to do everything; there are already more powerful tools for more complex workflows and knowledge management. Kimün aims to be simple and give you the tools to integrate with your own workflow. You can even combine it with other note taking apps that support local notes.

**Check the [docs](https://nico2sh.github.io/kimun/) for more on what you can do with Kimün.**

Notes are plain Markdown files stored in a directory you own. Kimün indexes them into a local SQLite database for fast full-text and structured search.

If you are already using another markdown, local-first, note-taking app, you should feel right at home and be able to use Kimün just like your existing app (QownNotes, Obsidian, Logseq, etc.), only that in this case, it is on your terminal emulator.

## Interactive and cli tool

**TUI** — an interactive terminal interface for writing, browsing, and organizing notes. Navigate your vault, search across notes, follow wiki links, and manage files without leaving the terminal.

**CLI** — a scriptable interface for automation and integration. Pipe output, capture command results into notes, log to your journal from cron jobs, or build custom workflows with `jq` and shell scripts:

```sh
# Quick capture from anywhere
kimun note journal "Fixed the auth bug, deploying at 17:00"

# Pipe command output into a note
./run-tests.sh | tail -5 | kimun note append "logs/test-log"

# Search and process results
kimun search "todo" --format json | jq '.notes[] | {title, path}'
```

The CLI can be used with AI tools and agents. An AI assistant can create, append, and search notes on your behalf — logging findings, organizing research, or updating your journal as part of an automated workflow. You can use the skill located under the [skills](skills/) directory, or create your own (in that case, create a pull request here and share yours!)

> Note: There is a fair amount of AI-assisted code (using Claude) with manual reviews, although most of the core was written with my human hands. Initially AI was used for tedious refactors, data structures I'm too lazy to code myself, but also to help me building the foundations of more complex stuff, especially on the UI side. Anyway, I guess the lesson is, use AI as a tool, not as a replacement.
> 
> Oh, and about the app logo: maybe is not the most beautiful one, but I did it myself with Inkscape! (it's a `k`! that looks like an open book! get it?) Pay artists or do it yourself, but don't take away the fun creative things from people.

![Kimün screenshot](docs/static/screenshot.png)

## Quick Start

Kimün is a terminal UI for browsing and editing your Markdown notes with a powerful search engine. Use the TUI to write and organize notes, or the CLI for automation and scripting. Everything is stored as plain `.md` files — no lock-in.

**Homebrew (macOS and Linux):**

```sh
brew tap nico2sh/kimun
brew install kimun
```

**Cargo:**

```sh
cargo install kimun-notes
```

## AI Skills

The `skills/` directory contains ready-made skills for AI coding assistants, so they can use the Kimün CLI on your behalf — capturing notes, appending to your journal, searching your vault, and more.

### Claude Code

```sh
# Copy the skill to your Claude skills directory
cp -r skills/kimun-cli ~/.claude/skills
```

Claude Code will pick it up automatically. In any session, Claude can now create and append notes, log to your journal, and search your vault using the CLI.

### Other AI tools (Codex, Gemini CLI, etc.)

Copy `skills/kimun-cli/SKILL.md` to wherever your tool loads skills from, following that tool's skill installation instructions.

## MCP Server

For richer AI integration, `kimun mcp` runs as a [Model Context Protocol](https://modelcontextprotocol.io) server. MCP gives the AI direct access to your vault through structured tools and prompt templates — no shell commands needed.

```sh
# Claude Code (one-time setup)
claude mcp add kimun -- kimun mcp
```

**Tools** let the AI create, read, search, and reorganise notes. **Prompt templates** load vault content and ask the model to reason over it — reviewing your day, scanning a week of journal entries, finding unlinked related notes, or brainstorming ideas grounded in what you've already written.

See the [MCP Server docs](docs/content/getting-started/mcp-server.md) for the full tool and prompt reference, and setup instructions for Claude Desktop and other clients.

---

## Documentation

Full documentation is available in [`docs/`](docs/content):

- [Getting Started](docs/content/getting-started/)
- [Using Kimün](docs/content/using-kimun/)
- [Guides](docs/content/guides/)

To browse the docs locally with search:

```sh
# Install Zola: https://www.getzola.org/documentation/getting-started/installation/
zola serve docs/
```

## Roadmap

- [ ] Command palette
- [X] Backlinks panel
- [ ] Inline tags and search by tag (`#important`)
- [ ] Resolve relative paths on links and images
- [ ] Paste images into notes
- [ ] Auto-continue list formatting on Enter
- [X] Quick add note
- [X] Multiple workspaces
- [X] Search under Markdown sections
- [X] File management (create, rename, move, delete notes and directories)
- [X] Autosave
- [X] Wikilinks in preview
- [X] Navigate notes via links in preview
- [X] Embed neoVim as an option (currently experimental)

# Contributing

Contributions are welcome! This project uses [Conventional Commits](https://www.conventionalcommits.org/) to automate versioning and changelog generation.

If you want to automate enforcing the Conventional Commits, after cloning enable the included git hook:

```sh
git config core.hooksPath .githooks
```

This will reject commit messages that don't follow the format. Common types:

| Type | When to use |
|---|---|
| `feat:` | New feature (minor version bump) |
| `fix:` | Bug fix (patch bump) |
| `feat!:` | Breaking change (major bump) |
| `chore:`, `docs:`, `ci:` | No release triggered |

# Credits
Built with [Ratatui](https://github.com/ratatui/ratatui) (and [ratatui-textarea](https://github.com/ratatui/ratatui-textarea)), [Nucleo](https://docs.rs/nucleo/latest/nucleo/) for fuzzy searching, [Ignore](https://github.com/BurntSushi/ripgrep/tree/master/crates/ignore) for fast file read, [nvim-rs](https://github.com/killthemule/nvim-rs) for Neovim integration.
Inspired by [Obsidian](https://obsidian.md/), [Logseq](https://logseq.com/) and [QownNotes](https://www.qownnotes.org/).
