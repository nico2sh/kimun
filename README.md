# Kimün

[![Crates.io](https://img.shields.io/crates/v/kimun-notes)](https://crates.io/crates/kimun-notes)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

A terminal-based notes app focused on simplicity and powerful search.

**Check the [docs](https://nico2sh.github.io/kimun/).**

Notes are plain Markdown files stored in a directory you own. Kimün indexes them into a local SQLite database for fast full-text and structured search.

If you are already using another markdown, local-first, note-taking app, you should feel right at home and be able to use Kimün just like your existing app (QownNotes, Obsidian, Logseq, etc.), only that in this case, it is on your terminal emulator.

> Small disclaimer: Although by no means this has been vibe coded — the core has been written manually — there is a good chunk of AI-assisted code (using Claude) with manual reviews. Initially for tedious refactors, data structures I'm too lazy to code myself, but also to help me building the foundations of more complex stuff, especially on the UI side. Use AI as a tool, not as a replacement.

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

## Releasing

Releases are automated via `release.sh`, which uses [`semtag`](./semtag) to determine the next version. Requires [`cargo-edit`](https://github.com/killercup/cargo-edit):

```sh
cargo install cargo-edit
```

```sh
./release.sh           # auto scope (minor or patch based on diff size)
./release.sh -s patch  # force patch bump
./release.sh -s minor  # force minor bump
./release.sh -s major  # force major bump
```

The script will:

1. Calculate the next version from git history
2. Bump the version in `tui/Cargo.toml` (kimun-notes)
3. Commit the change and push a version tag

The tag triggers the CI workflow, which:

- Publishes to crates.io — skipping any crate whose current version is already published
- Pushes a formula to the [homebrew-kimun](https://github.com/nico2sh/homebrew-kimun) tap (final releases only)

**Releasing `kimun_core`:** Core is versioned independently. Update `core/Cargo.toml` and the `kimun_core` entry in the root `Cargo.toml` `[workspace.dependencies]` manually, commit, then run `./release.sh` as usual.

**Required secrets** (set in the repository settings):

- `CARGO_REGISTRY_TOKEN` — crates.io API token
- `HOMEBREW_TAP_TOKEN` — GitHub PAT with write access to `nico2sh/homebrew-kimun`

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
