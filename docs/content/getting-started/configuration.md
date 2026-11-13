+++
title = "Settings"
weight = 3
+++

# Settings

Most settings are in the Preferences screen. Press `Ctrl+,` or `F4` to open it. Kimün saves your changes in a file named `config.toml`. Both apps read this file.

You find the file here:

- Linux and macOS: `~/.config/kimun/config.toml`
- Windows: `%USERPROFILE%\kimun\config.toml`

To use a different file, start the terminal app with `kimun --config /path/to/config.toml`.

## Common settings

You can set these in Preferences, or edit them in `config.toml`.

| Setting | What it does |
|---|---|
| `theme` | The name of the color theme, for example `theme = "Nord"`. See [Themes](@/using-kimun/themes.md). |
| `editor_backend` | `"textarea"` is the normal editor. `"vim"` adds [vim keys](@/using-kimun/vim-mode.md). `"nvim"` uses your own Neovim. |
| `autosave_interval_secs` | How often Kimün saves your changes, in seconds. The default is 5. |
| `use_nerd_fonts` | Show extra symbols in the terminal app. Turn it on only if your terminal font has Nerd Font symbols. |
| `mouse` | Set `mouse = false` in the `[global]` section to give the mouse back to your terminal. |
| `[workspaces.<name>]` | One block for each workspace. The `path` line is the notes folder. |

An example:

```toml
theme = "Nord"
editor_backend = "vim"
autosave_interval_secs = 5

[global]
current_workspace = "default"

[workspaces.default]
path = "~/Documents/Notes"
```

## Quick notes

A quick note is a short note that you save with `Ctrl+W`. Kimün saves it in the `/inbox` folder of your workspace. To use a different folder, set `inbox_path` in the workspace block:

```toml
[workspaces.default]
path = "~/Documents/Notes"
inbox_path = "/capture"
```

## Change a shortcut

To change a shortcut, add a `[key_bindings]` section to `config.toml`. This section replaces the whole list of default shortcuts. Copy the full list from [Keybindings](@/using-kimun/keybindings.md) and change the lines that you want. The desktop app has its own list. See the [desktop app](@/using-kimun/desktop-app.md#change-a-shortcut) page.

## Neovim

If you choose `nvim`, Kimün runs your installed Neovim in the background. Your `init.lua` and your plugins work. If Neovim does not start, Kimün uses the normal editor. If Neovim is not on your `PATH`, set `nvim_path = "/path/to/nvim"`.

With Neovim, a mouse click does not move the cursor. Use `Ctrl+N` to follow a link.
