+++
title = "Installation"
weight = 2
+++

# Installation

## Install Kimün

**Homebrew (macOS and Linux):**

```sh
brew tap nico2sh/kimun
brew install kimun
```

**Cargo:**

```sh
cargo install kimun-notes
```

## First Run

When you launch Kimün for the first time, it opens the Preferences screen. Here you can set your notes directory — this becomes your **workspace** where all your notes will be stored.

```sh
kimun
```

## Configuration File

The config file is created automatically on first run:

- **Linux / macOS:** `~/.config/kimun/config.toml`
- **Windows:** `%USERPROFILE%\kimun\config.toml`

You can also specify a custom config path:

```sh
kimun --config /path/to/my-config.toml
```

## Workspace Index

Kimün creates a per-workspace SQLite search index — `<config_dir>/<workspace>.kimuncache` by default. It's the index, not your notes. Your actual notes are plain `.md` files inside the workspace directory. The cache file can be safely deleted; it will be rebuilt automatically the next time Kimün runs.

```
~/.config/kimun/                ← Config directory
├── config.toml                 ← Your config
├── default.kimuncache          ← Search index for the "default" workspace
└── history/
    └── default.txt             ← Recently-opened notes for "default"

your-workspace/                 ← Your workspace directory
├── notes.md                    ← Your notes (plain Markdown)
├── journal.md
└── projects/
    └── my-project.md
```

The cache and history locations are configurable — see [Configuration → Files Kimün Stores on Disk](@/getting-started/configuration.md#files-kimun-stores-on-disk).

## What's Next

You're installed. Now learn your way around the [Terminal UI](@/using-kimun/tui.md), or set up separate [Workspaces](@/getting-started/workspaces.md) for work and personal notes.
