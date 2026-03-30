# Kimün

A terminal-based notes app focused on simplicity and powerful search.

**Check the [docs](https://nico2sh.github.io/kimun/).**

Notes are plain Markdown files stored in a directory you own. Kimün indexes them into a local SQLite database for fast full-text and structured search.

> Small disclaimer: Although by no means this has been vibe coded — the core has been written manually — there is a good chunk of AI-assisted code (using Claude) with manual reviews. Initially for tedious refactors, data structures I'm too lazy to code myself, but also to help me building the foundations of more complex stuff, especially on the UI side. Use AI as a tool, not as a replacement.

## Quick Start

Kimün is a terminal UI for browsing and editing your Markdown notes with a powerful search engine. Use the TUI to write and organize notes, or the CLI for automation and scripting. Everything is stored as plain `.md` files — no lock-in.

```sh
cargo install kimun-notes
```

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
- [ ] Display key shortcuts in command palette and help modal
- [ ] Backlinks panel
- [ ] Inline tags and search by tag (`#important`)
- [ ] Resolve relative paths on links and images
- [ ] Paste images into notes
- [ ] Calendar view for journal browsing
- [ ] Auto-continue list formatting on Enter
- [X] Multiple workspaces
- [X] Search under Markdown sections
- [X] File management (create, rename, move, delete notes and directories)
- [X] Autosave
- [X] Wikilinks in preview
- [X] Navigate notes via links in preview
- [ ] Embed neoVim as an option
