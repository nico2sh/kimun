+++
title = "Troubleshooting"
weight = 10
+++

# Troubleshooting

Most problems below are about the terminal app. Terminals cannot send some key combinations. To see which keys your terminal can send, run `kimun doctor` in that terminal.

## Ctrl+Enter acts like Enter

Many terminals cannot tell `Ctrl+Enter` from `Enter`. Press `Ctrl+U` instead. It follows a link in the same way.

Kitty, Ghostty, and foot support `Ctrl+Enter`. In WezTerm, add `config.enable_kitty_keyboard = true` to `~/.wezterm.lua`.

## Ctrl+B does not make text bold

`Ctrl+B` starts a command sequence. For bold, press `Ctrl+B`, then `t`, then `b`. For italic, press `Ctrl+B`, `t`, `i`. In the desktop app, use `Ctrl+Shift+B` in place of `Ctrl+B`.

## Backspace moves focus instead of deleting

Some terminals send the same code for `Backspace` and `Ctrl+H`. To make `Backspace` delete, add this line to `config.toml`:

```toml
ctrl_h = "backspace"
```

Then `Ctrl+H` no longer moves focus left. Use `Ctrl+L` and `Ctrl+H` on a terminal that can tell the keys apart, or bind another key.

## Middle-click paste or text selection does not work

Kimün uses the mouse. To use the mouse of your terminal for one action, hold `Shift`. To give the mouse back to your terminal, set `mouse = false` in the `[global]` section of `config.toml`. Then restart Kimün.

## Search misses a note

Run `kimun workspace reindex <name>`. This rebuilds the search index.

## Kimün says the index was made by a newer version

The message reads "This vault's index was made by a newer version of Kimün". The index is the search data that Kimün keeps for a vault. You use two versions of Kimün on this vault, and the newer one made the index.

Update the older app. In the terminal app, run `kimun update`. In the desktop app, open Preferences, go to Display, and select Check now. The desktop app also shows the version of the terminal app in the same place.

A reindex does not fix this message. The older app will not open the vault until you update it.

## Report a problem

Kimün writes a log file. Here is where you find it:

| System | Path |
|---|---|
| macOS | `~/Library/Application Support/kimun/kimun.log` |
| Linux | `~/.local/share/kimun/kimun.log` |
| Windows | `%APPDATA%\kimun\kimun.log` |

[Open an issue](https://github.com/nico2sh/kimun/issues) and include the last lines of this file. If Kimün crashed, include the lines that start with `[PANIC]`.
