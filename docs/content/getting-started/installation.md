+++
title = "Installation"
weight = 2
+++

# Installation

## Install Kimün

### Install script (macOS and Linux, recommended)

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

> **Windows:** use the PowerShell installer. It needs no admin rights, installs into
> `%LOCALAPPDATA%\Programs\kimun`, adds that folder to your user `PATH`, and records
> the same install marker, so the app can update itself:
>
> ```powershell
> irm https://kimun.2co.dev/install.ps1 | iex
> ```
>
> Open a new terminal afterwards. You can also take the zip from the
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

## Desktop app

The desktop app, `kimun-gui`, is installed the same way with `--gui`. See the
[Desktop app](@/using-kimun/desktop-app.md) page for what it is.

```sh
curl -fsSL https://kimun.2co.dev/install.sh | sh -s -- --gui
```

On Linux the script also installs the app's launcher entry and icons under
`~/.local/share`, so it shows up in your application menu. On macOS it installs
the plain binary (run `kimun-gui` from a terminal); for a double-clickable
app, download the signed `kimun-gui-<version>-<platform>.dmg` from the
[releases page](https://github.com/nico2sh/kimun/releases). Run the script once for each app you want: both can sit side by side
and share one install marker.

On Windows, the PowerShell installer takes a `-Gui` switch. `irm | iex` cannot
pass one, so this is the form to paste:

```powershell
& ([scriptblock]::Create((irm https://kimun.2co.dev/install.ps1))) -Gui
```

or, in one line, `$env:KIMUN_GUI = 1; irm https://kimun.2co.dev/install.ps1 | iex`.
It adds a Start-menu shortcut. Both apps share one folder and one marker, and the
installer's overrides (`KIMUN_INSTALL_DIR`, `KIMUN_VERSION`) work as in `install.sh`.
You can also take `kimun-gui-<version>-windows-x64.zip` from the
[GitHub releases page](https://github.com/nico2sh/kimun/releases) and run
`kimun-gui.exe`.

## Updating

If you installed with the install script, Kimün can update itself in place,
because the install marker tells the app it is on the `script` channel. You can
also re-run the install command at any time to get the latest stable release:

```sh
curl -fsSL https://kimun.2co.dev/install.sh | sh                # the terminal app
curl -fsSL https://kimun.2co.dev/install.sh | sh -s -- --gui    # the desktop app
```

The desktop app updates itself the same way when the install script put it
there.

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
