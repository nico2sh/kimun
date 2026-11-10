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

The script downloads the latest stable release and makes sure that its SHA-256 checksum is correct. Then it installs the binary into `~/.local/bin`. To use a different directory, set the `KIMUN_INSTALL_DIR` environment variable. The script also records an install marker (a file that tells Kimün how you installed it). The marker enables in-app self-update, so you can upgrade from inside Kimün and do not need to run the installer again.

To read the script before you run it, download it first:

```sh
curl -fsSLO https://kimun.2co.dev/install.sh && less install.sh && sh install.sh
```

If `~/.local/bin` is not on your `PATH`, the script tells you how to add it.

> Windows: use the PowerShell installer. It needs no admin rights. It installs into
> `%LOCALAPPDATA%\Programs\kimun` and adds that folder to your user `PATH`. It also records
> the same install marker, so the app can update itself:
>
> ```powershell
> irm https://kimun.2co.dev/install.ps1 | iex
> ```
>
> Afterwards, open a new terminal. You can also download the zip from the
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

To install the desktop app, `kimun-gui`, use the same script with `--gui`. The [Desktop app](@/using-kimun/desktop-app.md) page describes the app.

```sh
curl -fsSL https://kimun.2co.dev/install.sh | sh -s -- --gui
```

On Linux, the script also installs the launcher entry and the icons of the app under `~/.local/share`. The app then appears in your application menu. On macOS, the script installs the signed `Kimun.app` in `~/Applications`. Open it from Launchpad or Spotlight. To install it in another folder, set `KIMUN_APP_DIR`. If a release has no `.dmg` file, the script installs the plain binary instead. Then you run `kimun-gui` from a terminal. Run the script once for each app that you want. On Linux, both apps can be installed side by side and share one install marker. On macOS, the app has no install marker. To update it, run the script again.

On Windows, the PowerShell installer takes a `-Gui` switch. The `irm | iex` form cannot pass a switch, so paste this command:

```powershell
& ([scriptblock]::Create((irm https://kimun.2co.dev/install.ps1))) -Gui
```

Or use this one-line form: `$env:KIMUN_GUI = 1; irm https://kimun.2co.dev/install.ps1 | iex`.
The installer adds a Start-menu shortcut. Both apps share one folder and one marker. The overrides of the installer (`KIMUN_INSTALL_DIR`, `KIMUN_VERSION`) work as in `install.sh`.
You can also download `kimun-gui-<version>-windows-x64.zip` from the
[GitHub releases page](https://github.com/nico2sh/kimun/releases) and run
`kimun-gui.exe`.

## Updating

If you installed with the install script, Kimün can update itself in place. The install marker tells the app that it is on the `script` channel. You can also run the install command again at any time to get the latest stable release:

```sh
curl -fsSL https://kimun.2co.dev/install.sh | sh                # the terminal app
curl -fsSL https://kimun.2co.dev/install.sh | sh -s -- --gui    # the desktop app
```

If the install script installed the desktop app, it updates itself the same way.

If you installed with Homebrew or Cargo, use the same tool to update:

```sh
brew upgrade kimun        # Homebrew
cargo install kimun-notes # Cargo (reinstalls the latest)
```

## First Run

If you start Kimün for the first time and no workspace exists, a guided setup dialog opens. The dialog asks you to choose a notes directory, Nerd Fonts, a theme, and an editor. Kimün writes nothing until you accept the choices at the end.

```sh
kimun
```

[Guided Setup](@/getting-started/configuration.md#guided-setup) describes each step.

## Configuration File

Kimün creates the configuration file automatically on the first run:

- Linux and macOS: `~/.config/kimun/config.toml`
- Windows: `%USERPROFILE%\kimun\config.toml`

To use a custom configuration path, run:

```sh
kimun --config /path/to/my-config.toml
```

## Workspace Index

Kimün creates a SQLite search index for each workspace. By default, the index is at `<config_dir>/<workspace>.kimuncache`. The file holds only the index. Your notes are plain `.md` files in the workspace directory. You can delete the cache file. Kimün rebuilds it the next time that it runs.

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

You can change the cache and history locations. See [Configuration → Files Kimün Stores on Disk](@/getting-started/configuration.md#files-kimun-stores-on-disk).

## Optional: the Kimün Server

An optional companion server adds semantic search and AI question answering for your notes. It is a separate binary. Currently you can install it with Cargo only. [Semantic Search & Ask](@/using-kimun/server.md) describes the installation and the setup. Kimün works fully without the server.

## Next Steps

Read about the [Terminal UI](@/using-kimun/tui.md), or set up separate [Workspaces](@/getting-started/workspaces.md) for work notes and personal notes.
