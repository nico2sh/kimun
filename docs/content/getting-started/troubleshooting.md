+++
title = "Troubleshooting"
weight = 10
+++

# Troubleshooting

If Kimün does not work as you expect, the log file usually shows the reason. For a problem with keys, start with `kimun doctor`:

```sh
kimun doctor
```

The command shows which keys your terminal can send and what happens to each key binding. Run it in the terminal that you use for Kimün. See [CLI → Doctor](@/using-kimun/cli.md#doctor).

## Ctrl+Enter Acts Like Plain Enter

Most terminals cannot tell `Ctrl+Enter` from `Enter` unless the [kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/) is active. Kimün requests the protocol automatically. The terminal must support the protocol and have it enabled:

- WezTerm has the protocol off by default. To enable it, add this line to `~/.wezterm.lua`:

  ```lua
  config.enable_kitty_keyboard = true
  ```

- Kitty, Ghostty, and foot support the protocol by default.
- If your terminal does not support the protocol, use `Ctrl+N`. It follows links the same way as `Ctrl+Enter`.

## Ctrl+B Does Not Make Text Bold

`Ctrl+B` is the leader gateway. For formatting, press `Ctrl+B` and then `t b` for bold, `t i` for italic, or `t s` for strikethrough.

This is deliberate. In ASCII, `Ctrl+I` and `Tab` are the same byte (`0x09`). A terminal setting cannot change this. Without the [kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/), `Ctrl+I` can only indent. Kimün does not give `Ctrl+B` and `Ctrl+S` a direct binding while `Ctrl+I` cannot work. Therefore all three formatting commands are in the `+text` group of the leader, which works in every terminal.

If you want a direct chord, bind one. See [Keybindings](@/using-kimun/keybindings.md#defaults). A binding on `Ctrl+I` indents instead of italicizing, unless your terminal supports the kitty protocol. These terminals support the protocol: Kitty, Ghostty, foot, WezTerm with `enable_kitty_keyboard = true`, and Windows (the console of Windows reports keys instead of bytes). These terminals do not support it: GNOME Terminal, Terminal.app, xterm, urxvt, and `tmux` or `screen` without extended keys. The same byte-level limit is the reason that `Ctrl+Enter` needs the protocol.

## Backspace Moves Focus Instead of Deleting

If `Backspace` in the editor moves focus to the drawer, your terminal sends the older of the two backspace conventions. `Backspace` and `Ctrl+H` are the same byte (`0x08`), unless the [kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/) is active. `Ctrl+H` is bound to focus left, so Kimün reads the key as the chord.

The byte cannot tell the two keys apart, so you choose which key Kimün uses:

```toml
ctrl_h = "backspace"    # Backspace deletes; the Ctrl+H chord is unreachable
```

With the default value `auto`, Kimün picks `backspace` by itself when the erase character of the tty is `^H`. The first time that you open a note, the footer shows a message about this. Then focus left needs a new key or `ctrl_h = "chord"`.

If you use focus left, bind it to another key. See [Key Bindings](@/getting-started/configuration.md#key-bindings).

There are two other fixes. Each fix keeps both keys working:

- Make your terminal send `0x7F`. In Konsole, open *Settings → Edit Current Profile → Keyboard* and pick a key-bindings set whose backspace is `^?` (the "Default" table). `kimun doctor` prints the current erase character. You can also run `cat -v` and press `Backspace`. The output `^?` is correct. The output `^H` is the problem.
- Use a terminal that supports the kitty protocol (Kitty, Ghostty, foot, WezTerm with `enable_kitty_keyboard = true`, recent Konsole). In these terminals the two keys are different, and you do not need a setting.

[`ctrl_h`](@/getting-started/configuration.md#top-level-fields) lists all values.

## Middle-Click Paste or Drag-to-Select Does Not Work

Kimün captures the mouse for panel dividers, list scrolling, and click-to-focus. While Kimün does this, the mouse gestures of your terminal do not work. This includes middle-click paste and drag-to-select-and-copy. A terminal either reports the mouse to the application or handles the mouse itself. It cannot do both at the same time.

- For one action: hold `Shift` to get the behavior of the terminal. `Shift`+middle-click pastes and `Shift`+drag selects. This works in most terminals, including xterm.
- For all actions: set `mouse = false` under `[global]` (or clear Preferences → Display → mouse) to give the mouse back to your terminal. The change takes effect the next time that you start Kimün. See [Configuration → Mouse](@/getting-started/configuration.md#mouse).

## Log Files

Kimün writes a log file on every run. Release builds record only warnings and errors. Debug builds record everything.

### Log file location

| Platform | Path |
|----------|------|
| macOS | `~/Library/Application Support/kimun/kimun.log` |
| Linux | `$XDG_DATA_HOME/kimun/kimun.log` (or `~/.local/share/kimun/kimun.log` if `XDG_DATA_HOME` is not set) |
| Windows | `%APPDATA%\kimun\kimun.log` |

Kimün creates the file automatically the first time that it runs. Each run appends to the file. Kimün does not rotate the file, so delete it by hand if it becomes large.

### Reading the log

Open the file in any text editor. Each line is one event:

```
2026-04-08T10:23:01.456Z  WARN kimun: could not open last workspace: path not found
2026-04-08T10:23:05.789Z ERROR kimun: fatal error: broken pipe
```

### Crash reports

If Kimün crashes, it appends the panic message and a full stack trace to the same log file. Look for a line that starts with `[PANIC]`:

```
[PANIC] panicked at 'index out of bounds: the len is 3 but the index is 5', src/...
   0: kimun::app::...
   1: ...
```

When you report a bug, include the relevant section of `kimun.log`.

### Fallback location

If Kimün cannot write to the platform directory (for example, because the home directory is not available), it writes the log to the system temporary directory:

| Platform | Fallback |
|----------|---------|
| macOS / Linux | `/tmp/kimun.log` (or wherever `$TMPDIR` points) |
| Windows | `%TEMP%\kimun.log` |

## Reporting Issues

[Open an issue](https://github.com/nico2sh/kimun/issues) and include the relevant section of `kimun.log` with any `[PANIC]` lines. If search returns wrong results, run `kimun workspace reindex <name>`. The command rebuilds the index from the beginning and fixes most index problems (see [Workspaces](@/getting-started/workspaces.md#rebuild-the-search-index-reindex)).
