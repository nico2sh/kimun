# Installation

## Install Kimün

Install via Cargo:

```sh
cargo install kimun-notes
```

## First Run

When you launch Kimün for the first time, it opens the Settings screen. Here you can set your notes directory — this becomes your **workspace** where all your notes will be stored.

```sh
kimun
```

## Configuration File

The config file is created automatically on first run:

- **Linux / macOS:** `~/.config/kimun/kimun_config.toml`
- **Windows:** `%USERPROFILE%\kimun\kimun_config.toml`

You can also specify a custom config path:

```sh
kimun --config /path/to/my-config.toml
```

## Workspace Index

Kimün creates a `kimun.sqlite` file at the root of your workspace. This is the search index — it's not your notes. Your actual notes are plain `.md` files that live alongside it. The index can be safely deleted and will be rebuilt automatically the next time Kimün runs.

```
your-workspace/
├── kimun.sqlite          ← Search index (auto-generated)
├── notes.md              ← Your notes (plain Markdown)
├── journal.md
└── projects/
    └── my-project.md
```
