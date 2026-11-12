<div align="center">
  <img src="assets/logo.png" alt="Kimün Logo" width="80" height="80" />

  <h1>Kimün</h1>
  <p><strong>Simple note-taking. Fast, expressive search. AI-ready.</strong></p>

  <p>
    <a href="https://nico2sh.github.io/kimun/"><img src="https://img.shields.io/badge/docs-github%20pages-brightgreen?style=flat-square" alt="Documentation"></a>
    <a href="https://github.com/nico2sh/kimun/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="License"></a>
    <a href="https://crates.io/crates/kimun-notes"><img src="https://img.shields.io/crates/v/kimun-notes?style=flat-square&color=orange" alt="Crates.io"></a>
    <a href="https://github.com/nico2sh/kimun/stargazers"><img src="https://img.shields.io/github/stars/nico2sh/kimun?style=flat-square&label=stars" alt="Stars"></a>
    <a href="https://ratatui.rs/"><img src="https://img.shields.io/badge/Built_With_Ratatui-000?logo=ratatui&logoColor=fff", alt="Built With Ratatui"></a>
  </p>

  <em><strong>Kimün</strong> (Mapudungun): Knowledge, learning, or wisdom.</em>
</div>

---

**Kimün** is a terminal notes application. It is fast, it includes the tools you need, and it focuses on simple use and good search. It is local-first, so you own the notes.

Kimün does not try to be an all-in-one life-management tool or a _second brain_ (some of us struggle with a single brain already!). It is a small local vault for your terminal. Your notes are plain Markdown files, so Kimün works next to tools like Obsidian, Logseq, and QOwnNotes. It gives you a fast TUI and CLI for the same notes.

Kimün is in heavy development. A new version can break your configuration file or your custom themes. If the app does not start after an update, remove your configuration file. Kimün then creates a new one.

---

## Quick Start

You need two commands to start taking notes.

1. Install Kimün on macOS or Linux:

```bash
curl -fsSL https://kimun.2co.dev/install.sh | sh
```

2. Run it:

```bash
kimun
```

On the first launch, Kimün shows a setup screen. Choose a folder for your notes. Then you can start to write.

- Press `Ctrl+K` to search.
- Press `Ctrl+W` to write a quick note.
- Type in the search field of the file browser to filter notes or to create a new note.

The server is optional. It adds search by meaning and Ask, which answers questions from your own notes. Kimün works without it, but it has no semantic search and no Ask. To install the server and start it at login, run this command:

```bash
curl -fsSL https://kimun.2co.dev/install-server.sh | sh -s -- --service
```

Then open Preferences (`Ctrl+,`), go to Server, and enter `http://localhost:7573`. When the server starts for the first time, it downloads its embedding model. The download is a few hundred MB, so wait a minute.

For Homebrew, Cargo, or Docker (server only), see [Installation](#installation).

---

## Documentation

The [documentation site](https://nico2sh.github.io/kimun/) has user guides, setup steps, and integration tips.

---

## Key Features

- Search: Kimün indexes your Markdown files into a local SQLite database. You can run full-text, structured, and fuzzy search on the index.
- Semantic search and Ask (optional): The companion [Kimün server](server/README.md) finds notes by meaning. It also answers questions from your own notes (RAG). It uses local embeddings. You choose a local or a cloud LLM.
- Links: You can use `[[wikilinks]]` and standard Markdown links. Backlinks show an interactive preview.
- AI and MCP: Kimün has an MCP (Model Context Protocol) server and LLM tools. An AI model, for example Claude Code, can read your notes, run daily reviews, and update journals.
- Two interfaces: The TUI (terminal user interface) lets you write, browse workspaces, and preview files. The CLI (command line interface) works in scripts. You can pipe output, log entries from cron jobs, and process entries with `jq` and shell tools.
- Workspaces: A workspace is one independent vault. You can keep separate contexts, for example `Personal` and `Work`.
- Vim: You can use `HJKL` and other Vim keys in two ways. The built-in Vim emulation has motions, Visual mode, and Replace mode. The embedded Neovim mode uses your own Neovim configuration. Because the Vim emulation works better, use it if you can.

---

### How the TUI looks
<div align="center">
  <img src="assets/screenshot.png" alt="Kimün Notes Preview and Search Panel Screenshot" width="100%" />
</div>

---

## Installation

### Install script (macOS and Linux)
```bash
curl -fsSL https://kimun.2co.dev/install.sh | sh
```
The script installs the latest release into `~/.local/bin`. To change the folder, set `KIMUN_INSTALL_DIR`. The script also turns on in-app self-update. It verifies the SHA-256 of the download before it installs. The script installs both apps: the terminal app (`kimun`) and the desktop app (`kimun-gui`). On a Linux machine with no display, it skips the desktop app. To install only one app, add `-s -- --tui` or `-s -- --gui` after `sh`.

To read the script before you run it, use these commands:
```bash
curl -fsSLO https://kimun.2co.dev/install.sh && less install.sh && sh install.sh
```

### Homebrew (macOS and Linux)
```bash
brew tap nico2sh/kimun
brew install kimun

```

### Cargo (Rust)

```bash
cargo install kimun-notes

```

### Try It Out

The `example/` directory has ready-made workspaces. They contain linked personal and work notes, journals, and inboxes. Clone the repository and start Kimün in the sample workspace:

```bash
kimun --config ./example/config.toml

```

---

## TUI Keyboard Shortcuts

| Shortcut | Action |
| --- | --- |
| Ctrl + K | Global fuzzy search (like Telescope) |
| Ctrl + N | Follow `[[wikilink]]` under cursor |
| Ctrl + G | Leader gateway (for example, `Ctrl + G` then `l b` opens Backlinks) |
| Ctrl + E | Open the file browser drawer |
| Ctrl + T | Toggle the sidebar drawer |
| Ctrl + W | Quick Note: write a thought to your inbox |
| F5 | Switch Workspaces (for example, toggle Personal / Work) |

---

## Automation with the CLI and AI (MCP)

### CLI

The CLI works in shell pipelines. These two commands show the idea:

```bash
# Quick log to your daily journal from a shell workflow
echo "Finished deploy script successfully" | kimun journal append

# Search your notes structurally
kimun search "refactoring" --json | jq '.[].path'

```

### Model Context Protocol (MCP) and LLMs

The Kimün MCP server lets an AI agent, for example Claude Code, view, update, and query your notes. You can use it in these ways:

- Ask an LLM to read the journal logs of one week and list the action items.
- Give an agent your notes as context for brainstorming.
- Ask an AI tool to suggest notes that are related but not linked.

To set up the MCP server, read the [documentation](https://nico2sh.github.io/kimun/using-kimun/ai-mcp-server/). This command adds it to Claude Code. You run it one time:

```bash
# Claude Code (one-time setup)
claude mcp add kimun -- kimun mcp
```

You can also use skills. Copy the skills into the skills directory of your agent tool:

```bash
# Copy the skill to your Claude skills directory
cp -r skills/kimun-cli ~/.claude/skills
```

The `/skills` directory has more prompt definitions.

### Semantic Search and Ask (RAG Server)

The optional Kimün server adds two functions to the TUI. Semantic search finds notes by meaning. Ask answers a question from your notes and cites the sources. The server never reads your disk, because Kimün only pushes data to it. You can deploy it anywhere, and one server can host many vaults. It can run fully local with embedded SQLite and local embeddings. For question answering, you can add an LLM: Claude, OpenAI, Gemini, Mistral, or a local OpenAI-compatible endpoint.

```bash
# Install script (Linux, macOS Apple Silicon). Run it again to update.
curl -fsSL https://kimun.2co.dev/install-server.sh | sh

# Run with working local defaults (SQLite + local embedder)
kimun-server --default-config
```

The server is also available as a Docker image (`ghcr.io/nico2sh/kimun-server`) and with `cargo install`. The [server README](server/README.md) explains both.

To connect Kimün to the server, open Preferences, go to Server, and enter the URL. You can also set `kimun_server_url = "http://localhost:7573"` in the `[global]` section of your configuration file. The [server README](server/README.md) and the [documentation site](https://nico2sh.github.io/kimun/using-kimun/server/) have more details.

---

## Contributing

You can send a bug fix, an improvement to the SQLite indexing or the text layout, or an LLM prompt template or skill.

1. Fork the repository.
2. Create your feature branch (`git checkout -b feature/amazing-idea`).
3. Commit your changes (`git commit -m 'feat: Add amazing feature'`).
4. Push to the branch (`git push origin feature/amazing-idea`).
5. Open a Pull Request.

---

## Credits

Kimün uses these projects and libraries, and took ideas from them:

- UI: [Ratatui](https://github.com/ratatui/ratatui).
- Search: [Nucleo](https://github.com/helix-editor/nucleo) for fuzzy matching, and [ignore](https://github.com/BurntSushi/ripgrep/tree/master/crates/ignore) for directory walking.
- Editor integration: [nvim-rs](https://github.com/KillTheMule/nvim-rs) for Neovim synchronization.
- Inspiration: [Obsidian](https://obsidian.md/), [Logseq](https://logseq.com/), and [QOwnNotes](https://www.qownnotes.org/).

---
