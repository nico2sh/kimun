+++
title = "Troubleshooting"
weight = 10
+++

# Troubleshooting

Something acting weird? The log file usually knows why.

## Ctrl+Enter Acts Like Plain Enter

Most terminals can't tell `Ctrl+Enter` from `Enter` unless the [kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/) is active. Kimün requests it automatically, but the terminal has to play along:

- **WezTerm** ships with it **off**. Enable it in `~/.wezterm.lua`:

  ```lua
  config.enable_kitty_keyboard = true
  ```

- **Kitty, Ghostty, foot** support it out of the box.
- On terminals that can't be taught, use `Ctrl+N` — it follows links exactly like `Ctrl+Enter`.

## Middle-Click Paste or Drag-to-Select Doesn't Work

Kimün captures the mouse so it can drive panel dividers, list scroll, and click-to-focus. While it captures, your terminal's own mouse gestures are suppressed — including middle-click paste and drag-to-select-and-copy. This is unavoidable: a terminal either reports the mouse to the application or handles it itself, never both at once.

- **Quick fix, no config:** hold `Shift` to borrow the gesture back for one action — `Shift`+middle-click pastes, `Shift`+drag selects. Works in most terminals (xterm and friends).
- **Permanent:** set `mouse = false` under `[global]` (or untick Preferences → Display → mouse) to hand the mouse fully back to your terminal. Takes effect on the next launch. See [Configuration → Mouse](@/getting-started/configuration.md#mouse).

## Log Files

Kimün writes a log file on every run. In release builds only warnings and errors are recorded, keeping the file small. Debug builds log everything.

### Log file location

| Platform | Path |
|----------|------|
| macOS | `~/Library/Application Support/kimun/kimun.log` |
| Linux | `$XDG_DATA_HOME/kimun/kimun.log` (or `~/.local/share/kimun/kimun.log` if `XDG_DATA_HOME` is not set) |
| Windows | `%APPDATA%\kimun\kimun.log` |

The file is created automatically the first time Kimün runs. It is appended to across restarts — there is no rotation, so you can delete it manually at any time if it grows large.

### Reading the log

Open the file in any text editor. Each line is one event:

```
2026-04-08T10:23:01.456Z  WARN kimun: could not open last workspace: path not found
2026-04-08T10:23:05.789Z ERROR kimun: fatal error: broken pipe
```

### Crash reports

If Kimün crashes, the panic message and a full stack trace are appended to the same log file. Look for a line starting with `[PANIC]`:

```
[PANIC] panicked at 'index out of bounds: the len is 3 but the index is 5', src/...
   0: kimun::app::...
   1: ...
```

When reporting a bug, please include the relevant section of `kimun.log`.

### Fallback location

If Kimün cannot write to the platform directory (for example, because the home directory is unavailable), it falls back to the system temporary directory:

| Platform | Fallback |
|----------|---------|
| macOS / Linux | `/tmp/kimun.log` (or wherever `$TMPDIR` points) |
| Windows | `%TEMP%\kimun.log` |

## Still Stuck?

[Open an issue](https://github.com/nico2sh/kimun/issues) with the relevant section of `kimun.log` — especially any `[PANIC]` lines. If search is misbehaving, `kimun workspace reindex <name>` rebuilds the index from scratch and fixes most weirdness (see [Workspaces](@/getting-started/workspaces.md#reindex-rebuild-the-search-index)).
