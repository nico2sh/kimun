+++
title = "Troubleshooting"
weight = 10
+++

# Troubleshooting

When something misbehaves, the log file usually records why. For anything
key-related, start with `kimun doctor`:

```sh
kimun doctor
```

It reports what your terminal can deliver and what becomes of every key
binding. Run it in the terminal you use Kimün in; see [CLI →
Doctor](@/using-kimun/cli.md#doctor).

## Ctrl+Enter Acts Like Plain Enter

Most terminals can't tell `Ctrl+Enter` from `Enter` unless the [kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/) is active. Kimün requests it automatically, but the terminal has to support it and have it enabled:

- **WezTerm** ships with it off. Enable it in `~/.wezterm.lua`:

  ```lua
  config.enable_kitty_keyboard = true
  ```

- **Kitty, Ghostty, foot** support it by default.
- On terminals without the protocol, use `Ctrl+N`. It follows links the same way as `Ctrl+Enter`.

## Ctrl+B Doesn't Make Text Bold

`Ctrl+B` is the leader gateway. For formatting, press it and then
`t b` for bold, `t i` for italic, or `t s` for strikethrough.

This is deliberate. `Ctrl+I` and `Tab` are the same byte (`0x09`) in ASCII,
which is not something a terminal setting can change, so outside the [kitty keyboard
protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/) `Ctrl+I` can only
indent. Instead of having `Ctrl+B` and `Ctrl+S` work directly while
`Ctrl+I` did nothing, all three formatting commands moved to the leader's `+text` group,
which works in every terminal.

If you want a chord anyway, bind one; see [Keybindings](@/using-kimun/keybindings.md#defaults).
A binding on `Ctrl+I` will indent instead of italicise unless
your terminal supports the kitty protocol. Kitty, Ghostty, foot, WezTerm with
`enable_kitty_keyboard = true`, and Windows (whose console reports keys instead of
bytes) support it. GNOME Terminal, Terminal.app, xterm, urxvt, and
`tmux`/`screen` without extended keys do not. The same byte-level limit is why
`Ctrl+Enter` needs the protocol too.

## Backspace Moves Focus Instead of Deleting

If pressing `Backspace` in the editor jumps focus to the drawer, your terminal
is sending the older of the two backspace conventions. `Backspace` and `Ctrl+H`
are the same byte (`0x08`) unless the [kitty keyboard
protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/) is active, and
`Ctrl+H` is bound to focus left, so Kimün reads the key as the chord.

The byte alone can't tell the two apart, so you choose which one Kimün uses:

```toml
ctrl_h = "backspace"    # Backspace deletes; the Ctrl+H chord is unreachable
```

Under the default `auto`, Kimün picks `backspace` by itself when the tty's
erase character is `^H`. The footer says so the first time you open a note,
and focus left then needs a new key or `ctrl_h = "chord"`.

If you use focus left, rebind it to another key; see [Key
Bindings](@/getting-started/configuration.md#key-bindings).

Two other fixes, either of which keeps both keys working:

- **Make your terminal send `0x7F`.** In Konsole, open *Settings → Edit Current
  Profile → Keyboard* and pick a key-bindings set whose backspace is `^?` (the
  "Default" table). `kimun doctor` prints the current erase character. You can
  also run `cat -v` and press `Backspace`: `^?` is fine, `^H` is the problem.
- **Use a terminal that supports the kitty protocol** (Kitty, Ghostty, foot,
  WezTerm with `enable_kitty_keyboard = true`, recent Konsole). There the two
  keys are distinct and no setting is needed.

See [`ctrl_h`](@/getting-started/configuration.md#top-level-fields) for the
full list of values.

## Middle-Click Paste or Drag-to-Select Doesn't Work

Kimün captures the mouse for panel dividers, list scrolling, and click-to-focus. While it does, your terminal's own mouse gestures are suppressed, including middle-click paste and drag-to-select-and-copy. A terminal either reports the mouse to the application or handles it itself; it cannot do both at once.

- **Per gesture:** hold `Shift` to get the terminal's behaviour for one action. `Shift`+middle-click pastes and `Shift`+drag selects. This works in most terminals, including xterm.
- **Permanently:** set `mouse = false` under `[global]` (or untick Preferences → Display → mouse) to give the mouse back to your terminal. This takes effect on the next launch. See [Configuration → Mouse](@/getting-started/configuration.md#mouse).

## Log Files

Kimün writes a log file on every run. Release builds record only warnings and errors. Debug builds log everything.

### Log file location

| Platform | Path |
|----------|------|
| macOS | `~/Library/Application Support/kimun/kimun.log` |
| Linux | `$XDG_DATA_HOME/kimun/kimun.log` (or `~/.local/share/kimun/kimun.log` if `XDG_DATA_HOME` is not set) |
| Windows | `%APPDATA%\kimun\kimun.log` |

The file is created automatically the first time Kimün runs. Each run appends to it and there is no rotation, so delete it by hand if it grows large.

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

## Reporting Issues

[Open an issue](https://github.com/nico2sh/kimun/issues) with the relevant section of `kimun.log`, including any `[PANIC]` lines. If search returns wrong results, `kimun workspace reindex <name>` rebuilds the index from scratch, which fixes most index problems (see [Workspaces](@/getting-started/workspaces.md#rebuild-the-search-index-reindex)).
