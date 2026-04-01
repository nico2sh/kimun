+++
title = "Kimün"
sort_by = "weight"
+++

# Introduction

Kimün is a terminal-based notes app that helps you organize and search your thoughts with speed and simplicity. Your notes are plain Markdown files stored in a directory you own, indexed into a local SQLite database for fast full-text and structured search. There's no lock-in — your data is always yours, and you can access it with any text editor.

![Kimün TUI screenshot](img/screenshot-tui.png)

## Quick Start

See the [Installation](@/getting-started/installation.md) page for setup instructions.

Then run the terminal UI:

```sh
kimun
```

Or explore the command-line interface:

```sh
kimun --help
```

## Where Your Data Lives

Kimün stores two things at your workspace root:

- **`kimun.sqlite`** — The search index (created automatically, safe to delete and rebuild)
- **Your `.md` files** — Your actual notes (plain Markdown, totally portable)

Kimün also creates a config file for settings and workspace configuration:

- **Linux/macOS:** `~/.config/kimun/kimun_config.toml`
- **Windows:** `%USERPROFILE%\kimun\kimun_config.toml`

Everything is stored locally. No cloud, no subscriptions, no tracking.
