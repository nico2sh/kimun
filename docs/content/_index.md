+++
title = "Kimün"
sort_by = "weight"
+++

# Introduction

Kimün is a note-taking app for the terminal.

![Kimün TUI screenshot](img/screenshot-tui.png)

- Notes are plain `.md` files in a directory that you own. You can open them with any editor and sync them with any tool.
- A local SQLite index answers full-text and structured queries (by name, section, path, label, and links).
- Everything stays on your machine. Kimün has no cloud service, no account, and no tracking. If you stop using Kimün, your notes stay as they are.

## Quick Start

[Install Kimün](@/getting-started/installation.md), then run the terminal UI:

```sh
kimun
```

To see the command-line interface, run:

```sh
kimun --help
```

## Where Your Data Lives

Your workspace directory holds only your `.md` files.

Kimün stores its own files in your configuration directory, separate from your notes:

- Linux and macOS: `~/.config/kimun/`
- Windows: `%USERPROFILE%\kimun\`

This directory contains:

- `config.toml`: your configuration, including your workspaces
- `<workspace>.kimuncache`: the search index of each workspace (Kimün can rebuild it, so you can delete it)
- `history/<workspace>.txt`: the history of recently opened notes of each workspace

You can change the cache and history locations. See [Configuration](@/getting-started/configuration.md#files-kimun-stores-on-disk).
