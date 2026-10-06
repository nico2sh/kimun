+++
title = "Installation"
weight = 2
+++

# Installation

## Install Kimün

### Install script (recommended — macOS & Linux)

```sh
curl -fsSL https://kimun.2co.dev/install.sh | sh
```

The script downloads the latest stable release, verifies its SHA-256 checksum,
and installs the binary into `~/.local/bin` (override with the
`KIMUN_INSTALL_DIR` environment variable). It also records an install marker
that enables in-app self-update, so you can upgrade from inside Kimün instead
of re-running the installer.

To read the script before running it, download it first:

```sh
curl -fsSLO https://kimun.2co.dev/install.sh && less install.sh && sh install.sh
```

If `~/.local/bin` isn't on your `PATH`, the script tells you how to add it.

> **Windows:** the install script is Unix-only. Use the release archive from the
> [GitHub releases page](https://github.com/nico2sh/kimun/releases), or install
> with Cargo (below).

### Homebrew (macOS & Linux)

```sh
brew tap nico2sh/kimun
brew install kimun
```

### Cargo (Rust ecosystem)

```sh
cargo install kimun-notes
```

## Updating

If you installed with the install script, Kimün can update itself in place,
because the install marker tells the app it is on the `script` channel. You can
also re-run the install command at any time to get the latest stable release:

```sh
curl -fsSL https://kimun.2co.dev/install.sh | sh
```

If you installed with Homebrew or Cargo, update with the same tool:

```sh
brew upgrade kimun        # Homebrew
cargo install kimun-notes # Cargo (reinstalls the latest)
```

## First Run

When you launch Kimün for the first time with no workspace configured, a guided setup dialog walks you through choosing a notes directory, Nerd Fonts, a theme, and an editor. Nothing is written until you confirm at the end.

```sh
kimun
```

See [Guided Setup](@/getting-started/configuration.md#guided-setup) for a step-by-step breakdown.

## Configuration File

The config file is created automatically on first run:

- **Linux / macOS:** `~/.config/kimun/config.toml`
- **Windows:** `%USERPROFILE%\kimun\config.toml`

You can also specify a custom config path:

```sh
kimun --config /path/to/my-config.toml
```

## Workspace Index

Kimün creates a SQLite search index for each workspace, at `<config_dir>/<workspace>.kimuncache` by default. It holds only the index. Your notes are plain `.md` files inside the workspace directory. You can delete the cache file safely, and Kimün rebuilds it the next time it runs.

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

The cache and history locations are configurable. See [Configuration → Files Kimün Stores on Disk](@/getting-started/configuration.md#files-kimun-stores-on-disk).

## Optional: the Kimün Server

An optional companion server adds semantic search and AI question answering
over your notes. It is a separate binary, currently installed with Cargo only.
See [Semantic Search & Ask](@/using-kimun/server.md) for installation and setup.
Kimün works fully without it.

## What's Next

Next, learn your way around the [Terminal UI](@/using-kimun/tui.md), or set up separate [Workspaces](@/getting-started/workspaces.md) for work and personal notes.
