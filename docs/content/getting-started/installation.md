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

The script downloads the latest stable release and makes sure that its SHA-256 checksum is correct. It installs both Kimün apps: the terminal app, `kimun`, and the desktop app, `kimun-gui`. The binaries go into `~/.local/bin`. To use a different directory, set the `KIMUN_INSTALL_DIR` environment variable. The script also records an install marker (a file that tells Kimün how you installed it). The marker enables in-app self-update, so you can upgrade from inside Kimün and do not need to run the installer again.

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

## Choosing the apps

By default, the install script installs both apps. The [Desktop app](@/using-kimun/desktop-app.md) page describes the desktop app. These options select one app:

```sh
curl -fsSL https://kimun.2co.dev/install.sh | sh -s -- --tui    # the terminal app only
curl -fsSL https://kimun.2co.dev/install.sh | sh -s -- --gui    # the desktop app only
```

If you do not name an app, the desktop app is best effort. If the script cannot install it, the terminal app stays installed. The script then prints the error, a warning, and how to try again, and its last line says that it finished without the desktop app. On a Linux machine with no display, such as a server, the script skips the desktop app and tells you so. If you name an app with `--tui` or `--gui`, that app must install. Otherwise the script stops with an error. You can use `--gui` on a machine with no display too.

On Linux, the script also installs the launcher entry and the icons of the desktop app under `~/.local/share`. The app then appears in your application menu. On macOS, the script installs the signed `Kimun.app` in `~/Applications`. Open it from Launchpad or Spotlight. To install it in another folder, set `KIMUN_APP_DIR`. If a release has no `.dmg` file, a default run installs only the terminal app. With `--gui`, the script installs the plain binary instead. Then you run `kimun-gui` from a terminal. On Linux, both apps share one install marker. On macOS, the desktop app has no install marker. To update it, run the script again.

On Windows, the PowerShell installer also installs both apps. It takes `-Tui` and `-Gui` switches. The `irm | iex` form cannot pass a switch, so paste this command to install one app:

```powershell
& ([scriptblock]::Create((irm https://kimun.2co.dev/install.ps1))) -Gui
```

Or use a one-line form: `$env:KIMUN_GUI = 1; irm https://kimun.2co.dev/install.ps1 | iex`. For the terminal app only, use `-Tui` or `KIMUN_TUI = 1`.
The installer adds a Start-menu shortcut for the desktop app. Both apps share one folder and one marker. The overrides of the installer (`KIMUN_INSTALL_DIR`, `KIMUN_VERSION`) work as in `install.sh`.
You can also download `kimun-gui-<version>-windows-x64.zip` from the
[GitHub releases page](https://github.com/nico2sh/kimun/releases) and run
`kimun-gui.exe`.

## Uninstalling

To remove what the install script installed, run the script with `--uninstall`. This removes both apps. Add `--tui` or `--gui` to remove only one app.

```sh
curl -fsSL https://kimun.2co.dev/install.sh | sh -s -- --uninstall           # both apps
curl -fsSL https://kimun.2co.dev/install.sh | sh -s -- --uninstall --gui     # the desktop app only
```

The script removes the binary. For the desktop app, it also removes the launcher entry and the icons on Linux, or `Kimun.app` on macOS. When no app is left, it removes the install marker. It does not remove your notes or your configuration. To remove the configuration too, delete the `~/.config/kimun` directory yourself.

On Windows, the PowerShell installer takes an `-Uninstall` switch. As for the install, the `irm | iex` form cannot pass a switch, so paste this command. It removes both apps. Add `-Tui` or `-Gui` to remove only one app:

```powershell
& ([scriptblock]::Create((irm https://kimun.2co.dev/install.ps1))) -Uninstall
```

Or use this one-line form: `$env:KIMUN_UNINSTALL = 1; irm https://kimun.2co.dev/install.ps1 | iex`.

The Windows uninstall removes the app, the Start-menu shortcut of the desktop app, and the folder if it is empty. When no app is left, it also removes the install marker and the folder entry in your user `PATH`. If the app is open, close it and run the command again. It does not remove your notes or your configuration in `%USERPROFILE%\kimun`.

To remove the server, see [Server](@/using-kimun/server.md). If you installed Kimün with Homebrew or Cargo, use that tool to remove it.

## Updating

If you installed with the install script, Kimün can update itself in place. The install marker tells the app that it is on the `script` channel. You can also run the install command again at any time to get the latest stable release:

```sh
curl -fsSL https://kimun.2co.dev/install.sh | sh
```

The command updates the apps that are installed. If only one app is installed, the command does not add the other one. To add it, use `--gui` or `--tui`. If the install script installed the desktop app, it also updates itself the same way.

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
