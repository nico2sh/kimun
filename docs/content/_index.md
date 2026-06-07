+++
title = "Kimün"
sort_by = "weight"
+++

# Introduction

**Kimün is a notes app that lives in your terminal.** Fast to open, fast to search, impossible to outgrow.

![Kimün TUI screenshot](img/screenshot-tui.png)

- **Plain Markdown files** — your notes are just `.md` files in a directory you own. Open them with any editor, sync them with anything.
- **Search that actually finds things** — a local SQLite index powers full-text and structured queries (by name, section, path, label, links).
- **No lock-in, ever** — no cloud, no subscriptions, no tracking. Delete Kimün tomorrow and your notes won't notice.

## Quick Start

[Install Kimün](@/getting-started/installation.md), then run the terminal UI:

```sh
kimun
```

Or explore the command-line interface:

```sh
kimun --help
```

## Where Your Data Lives

Your workspace directory holds only your **`.md` files** — plain Markdown, totally portable.

Kimün's own files live under your config directory, separate from your notes:

- **Linux/macOS:** `~/.config/kimun/`
- **Windows:** `%USERPROFILE%\kimun\`

That directory contains:

- `config.toml` — your settings and workspace configuration
- `<workspace>.kimuncache` — per-workspace search index (regenerable; safe to delete)
- `history/<workspace>.txt` — per-workspace history of recently-opened notes

Both the cache and history locations are configurable — see [Configuration](@/getting-started/configuration.md#files-kimun-stores-on-disk).

Everything is stored locally. No cloud, no subscriptions, no tracking.
