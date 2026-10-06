+++
title = "Kimün"
sort_by = "weight"
+++

# Introduction

Kimün is a note-taking app for the terminal.

![Kimün TUI screenshot](img/screenshot-tui.png)

- Notes are plain `.md` files in a directory you own. You can open them with any editor and sync them with any tool.
- A local SQLite index handles full-text and structured queries (by name, section, path, label, and links).
- Everything stays on your machine. There is no cloud service, account, or tracking, and if you stop using Kimün your notes stay as they are.

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

Your workspace directory holds only your `.md` files.

Kimün's own files live under your config directory, separate from your notes:

- **Linux/macOS:** `~/.config/kimun/`
- **Windows:** `%USERPROFILE%\kimun\`

That directory contains:

- `config.toml`: your settings and workspace configuration
- `<workspace>.kimuncache`: the per-workspace search index (regenerable, safe to delete)
- `history/<workspace>.txt`: the per-workspace history of recently opened notes

The cache and history locations are configurable. See [Configuration](@/getting-started/configuration.md#files-kimun-stores-on-disk).
