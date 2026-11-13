+++
title = "Installation"
weight = 2
+++

# Installation

## macOS and Linux

Run this command in a terminal:

```sh
curl -fsSL https://kimun.2co.dev/install.sh | sh
```

It installs the desktop app and the terminal app. On macOS, open `Kimun.app` from Launchpad or Spotlight. On Linux, the desktop app appears in your application menu. The terminal app starts with the command `kimun`.

If the script says that `~/.local/bin` is not on your `PATH`, follow the steps that it prints.

To install only one app, add `-s -- --gui` (desktop app) or `-s -- --tui` (terminal app) after `sh`.

## Windows

Run this command in PowerShell:

```powershell
irm https://kimun.2co.dev/install.ps1 | iex
```

It needs no admin rights. It adds a Start-menu shortcut for the desktop app. Open a new terminal to use the `kimun` command.

## Other ways to install

- Download a file from the [Downloads page](@/releases/_index.md).
- Install with Homebrew: `brew tap nico2sh/kimun` and then `brew install kimun`.
- Install the terminal app with Cargo: `cargo install kimun-notes`.

## First start

When you start Kimün for the first time, a setup guide opens. It has six steps. Click `Next` to move on, or `Back` to go back.

![The first step of the setup guide](/img/gui-setup.png)

The guide asks for these choices:

1. The folder for your notes. Kimün suggests `~/kimun-notes`.
2. A theme.
3. An editor style: normal, or [vim](@/using-kimun/vim-mode.md).

The terminal app also asks whether your terminal font shows Nerd Font symbols. Choose yes only if the sample symbols look correct.

Kimün saves your choices only at the end. Press `Esc` at any step to cancel. To run the guide again, open the command palette and type "guided setup".

## Update

If you used the install script, Kimün tells you when a new version is ready. Click `Update now` in the message. You can also run the install command again.

If you used Homebrew, run `brew upgrade kimun`. If you used Cargo, run `cargo install kimun-notes` again.

## Uninstall

Run the install script with `--uninstall`:

```sh
curl -fsSL https://kimun.2co.dev/install.sh | sh -s -- --uninstall
```

On Windows, run `& ([scriptblock]::Create((irm https://kimun.2co.dev/install.ps1))) -Uninstall` in PowerShell.

This removes the apps. It does not remove your notes or your settings.

## Where Kimün keeps its files

Your notes folder holds only your Markdown files. Kimün keeps its own files in a separate folder:

- Linux and macOS: `~/.config/kimun/`
- Windows: `%USERPROFILE%\kimun\`

That folder holds your settings (`config.toml`) and the search index of each workspace. Kimün can rebuild the index at any time, so you can delete it.
