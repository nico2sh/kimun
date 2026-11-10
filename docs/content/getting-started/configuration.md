+++
title = "Configuration"
weight = 3
+++

# Configuration

Kimün keeps all configuration in one `config.toml`:

- Linux and macOS: `~/.config/kimun/config.toml`
- Windows: `%USERPROFILE%\kimun\config.toml`
- Any other location: `kimun --config /path/to/config.toml`

You rarely need to edit the file by hand. Kimün creates it on the first run, and the Preferences screen of the TUI (`Ctrl+,`) writes changes for you. This page explains how to edit the file directly.

## Guided Setup

If you start Kimün for the first time and no workspace is configured, a setup dialog opens in the center of the screen. The dialog guides you through the essential choices. Kimün writes nothing until the final step. You can leave at any point with `Esc`.

### Steps

1. Welcome: a short introduction to the setup. Press `Enter` to begin.

2. Workspace: choose where your notes live. Kimün suggests `~/kimun-notes`. In this step:

- Press `b` to open a directory browser and navigate to an existing folder.
- Inside the browser, press `n` to create a new directory on the spot.
- Press `e` to edit the workspace name. Kimün uses this label for the workspace. It is different from the directory path.

3. Nerd Fonts: two sample rows of glyphs show if your terminal font includes Nerd Font patches. If the symbols look broken, leave `use_nerd_fonts` off. If the symbols look correct, turn it on.

4. Theme: scroll through the available themes. A live preview shows each theme. You can change the theme later with `Ctrl+,`.

5. Editor Backend: choose how you edit notes:

- `textarea`: the built-in editor of Kimün (always available).
- `vim`: built-in Vim emulation.
- `nvim`: embedded Neovim (you can select it only if `nvim` is on `PATH`).

6. Summary: the list of every choice. Press `Enter` to apply all choices at once. Kimün writes the configuration file only in this step. Press `Esc` to discard everything and keep your configuration unchanged.

### Running It Again

You can open the guided setup again at any time:

- In the editor, open the command palette (`Ctrl+P`) and type "guided setup".
- Use the leader sequence `v o`. The default leader is `Ctrl+B`, so press `Ctrl+B`, then `v`, then `o`.

If you run the setup again, the Workspace step lists your current workspaces. It does not ask you to set up a workspace. To add, rename, or remove workspaces, use the Preferences screen (`Ctrl+,`) or the `kimun workspace` CLI commands (see [Workspaces](@/getting-started/workspaces.md)).

## A Complete Configuration

This is a full configuration file that works. The rest of this page explains each part:

```toml
config_version = 6
cache_dir = "."
history_dir = "history"
autosave_interval_secs = 5
use_nerd_fonts = false
editor_backend = "textarea"
default_sort_field = "name"
default_sort_order = "ascending"
journal_sort_field = "name"
journal_sort_order = "descending"

[global]
current_workspace = "default"

[workspaces.default]
path = "~/Documents/Notes"
inbox_path = "/inbox"

# A [key_bindings] section REPLACES the whole default keymap — it does not
# merge. Anything you omit ends up unbound (except Quit, which is restored).
# The five lines below are illustrative; a real override should list every
# binding you want to keep. See "Key Bindings → Overrides Replace the Defaults" below.
[key_bindings]
Quit = ["ctrl&Q"]
SearchNotes = ["ctrl&K"]
OpenNote = ["ctrl&O"]
NewJournal = ["ctrl&J"]
QuickNote = ["ctrl&W"]
```

## Common Settings

Most edits change one of these settings:

| Setting | What it does |
|---|---|
| `theme` | Name of the TUI theme, for example `theme = "Nord"`. An empty value selects the built-in theme. See [Themes](@/using-kimun/themes.md). |
| `editor_backend` | `"textarea"` (built-in editor), `"vim"` (built-in vim emulation), or `"nvim"` (embedded Neovim). See [Editor Backend](#editor-backend). |
| `[workspaces.<name>].path` | Where your notes live. |
| `autosave_interval_secs` | How often Kimün writes unsaved changes to disk (in seconds). |
| `use_nerd_fonts` | Nerd Font glyphs in the TUI. Use this if your terminal font has Nerd Font patches. |
| `[key_bindings]` | Change any shortcut (see [Key Bindings](#key-bindings)). |

## Full Reference

The file has five kinds of content:

1. Top-level fields: app-wide settings.
2. `[global]`: which workspace is active.
3. `[workspaces.<name>]`: one block per workspace.
4. `[key_bindings]`: keyboard shortcuts for the TUI.
5. `[leader.bind]` / `[leader.labels]`: leader-key overrides.

### Top-Level Fields

| Field | Type | Default | Description |
|---|---|---|---|
| `config_version` | integer | `7` | Schema version. The configuration migration system manages this value. Do not edit it. |
| `cache_dir` | string | `"."` | Directory for the SQLite cache of each workspace (`<workspace>.kimuncache`). Kimün resolves it relative to the directory of the configuration file. It accepts `~`, relative, and absolute paths. |
| `history_dir` | string | `"history"` | Directory for the history file of each workspace (`<workspace>.txt`). Kimün resolves it in the same way as `cache_dir`. |
| `theme` | string | `""` | Name of the active TUI theme (for example `"Nord"`). An empty string selects the built-in theme. See [Themes](@/using-kimun/themes.md). |
| `autosave_interval_secs` | integer | `5` | How often Kimün writes unsaved changes to disk (in seconds). |
| `leader_timeout_ms` | integer | `400` | Delay (in milliseconds) before the which-key panel appears during a leader sequence that is not complete. A sequence that you type faster never waits. You can also edit this value in the Preferences window (Display section). |
| `use_nerd_fonts` | boolean | `false` | Enable Nerd Font glyphs in the TUI. Leave `false` if the font of your terminal does not include Nerd Font patches. |
| `editor_backend` | string | `"textarea"` | Editor engine. `"textarea"` is the built-in editor. `"vim"` is the built-in vim emulation. `"nvim"` is embedded Neovim. You can also edit this value in the Preferences window (Editor section). |
| `nvim_path` | string | *(unset)* | Absolute path to the `nvim` binary. Use it only if Neovim is not on `PATH`. |
| `default_sort_field` | string | `"name"` | Sort field for the note browser. One of `"name"`, `"title"`. |
| `default_sort_order` | string | `"ascending"` | Sort direction for the note browser. One of `"ascending"`, `"descending"`. |
| `journal_sort_field` | string | `"name"` | Sort field for the journal view. One of `"name"`, `"title"`. |
| `journal_sort_order` | string | `"descending"` | Sort direction for the journal view. Descending shows newest first. |
| `group_directories` | boolean | `false` | If `true`, the sidebar lists directories first. Each group uses the sort field and order that you chose. You can set it in the sort dialog. |
| `ctrl_h` | string | `"auto"` | Old terminals send `Backspace` and `Ctrl+H` as the same character (`0x08`), so `Backspace` runs the action that is bound to `Ctrl+H`. `"auto"` keeps the `Ctrl+H` chord where Kimün can tell the two keys apart (under the kitty keyboard protocol, or if the erase character of the terminal is `0x7F`). It gives up the chord only on a terminal whose erase character is `0x08`. `"backspace"` makes the key delete and gives up the chord everywhere. Use it if `Backspace` moves focus instead of deleting. `"chord"` always keeps the chord. See [Troubleshooting](@/getting-started/troubleshooting.md#backspace-moves-focus-instead-of-deleting). |

### `[global]` Section

These settings apply to all workspaces:

```toml
[global]
current_workspace = "default"
```

| Field | Type | Default | Description |
|---|---|---|---|
| `current_workspace` | string | *(unset)* | Workspace that Kimün loads at startup. It must match a `[workspaces.<name>]` key. |
| `update_check` | boolean | `true` | Make Kimün look on GitHub for a newer release at launch. Kimün reads this value only at startup. |
| `mouse` | boolean | `true` | Capture the mouse for use in Kimün (divider drag, list scroll, editor scroll with the built-in editor, click-to-focus). Set `false` to give the mouse back to your terminal. See [Mouse](#mouse). Kimün reads this value only at startup. You can also change it with a checkbox in Preferences (`Ctrl+,` → Display). |
| `kimun_server_url` | string | *(unset)* | Base URL of the optional [Kimün server](@/using-kimun/server.md) (for example `"http://localhost:7573"`). The server adds semantic search and question answering. If the value is unset, these features are off. You can also edit it in Preferences (`Ctrl+,` → Server). |
| `kimun_server_token` | string | *(unset)* | Bearer token for the Kimün server, if the server requires one. |

(`theme` is a top-level field, not part of this section.)

### Mouse

By default, Kimün captures the mouse. You can drag panel dividers, scroll lists and the note editor, and click to focus. The note editor scrolls only with the built-in editor, because Neovim handles its own mouse. Capture is all or nothing: a terminal either reports every button to the application or handles the mouse itself. While Kimün captures the mouse, the mouse gestures of your terminal do not work. This includes middle-click paste (the X11 PRIMARY selection) and drag-to-select-and-copy.

There are two ways to get the native mouse behavior of the terminal back:

- For one action: most terminals (xterm and many others) let you hold `Shift` to bypass the mouse capture of an application. `Shift`+middle-click pastes the primary selection, and `Shift`+drag selects text to copy. This needs no configuration, so try it first.
- For all actions: set `mouse = false` in `[global]` (or clear it in Preferences → Display). Then Kimün never captures the mouse, and the terminal keeps full control of selection, copy, and paste.

```toml
[global]
current_workspace = "default"
mouse = false
```

If you set `mouse = false`, the mouse gestures of Kimün do not work. Each gesture has a keyboard equivalent. Resize the drawer with the leader `Window` actions. Move and scroll lists with the arrow keys. Change the panel focus with the keyboard. Kimün reads the setting once at startup, so a change (in the file or in Preferences) takes effect the next time that you start Kimün.

### `[workspaces.<name>]` Sections

Each workspace has one block. The `<name>` after the dot is the identifier that you use in `[global].current_workspace` and in the workspace switcher of the TUI. It is also the name of the cache file and the history file of the workspace (`<name>.kimuncache`, `<name>.txt`), so it must be a valid file name (see [Workspace Name Rules](#workspace-name-rules)).

| Field | Type | Default | Description |
|---|---|---|---|
| `path` | string | required | Path to the notes directory of this workspace. |
| `inbox_path` | string | `"/inbox"` | Vault-relative directory for quick-captured notes. |
| `quick_note_path` | string | `"/"` | Vault-relative directory where `QuickNote` saves its output. The default is the vault root. |
| `created` | string (RFC 3339 timestamp) | *(set by Kimün)* | Creation timestamp. Kimün manages this value. Do not edit it. |

```toml
[workspaces.default]
path = "~/Documents/Notes"
inbox_path = "/inbox"

[workspaces.work]
path = "~/work-notes"
inbox_path = "/capture"
quick_note_path = "/scratch"

[workspaces.archive]
path = "/Users/alice/archive-notes"
```

#### Workspace Name Rules

Workspace names become file names, so they follow the file name rules of all platforms. Kimün rejects a new workspace if the name:

- contains any of `\ / : * ? " < > | [ ] ^ #` or control characters
- is a Windows reserved name (`con`, `prn`, `aux`, `nul`, `com1`–`com9`, `lpt1`–`lpt9`)
- starts with two or more dots, ends with a dot, or has whitespace at the start or the end
- exceeds 64 characters

Kimün changes names to lowercase before it checks them, so `MyVault` and `myvault` are the same workspace.

#### Path Formats

The `path` field accepts three formats:

- Absolute: `/home/alice/notes` or `C:\Users\alice\notes`
- Relative: `../my-notes` or `personal`. Kimün resolves it relative to the directory of the configuration file.
- Home-relative: `~/notes`. On Linux and macOS, `~` becomes `$HOME`. On Windows, `~` becomes `%USERPROFILE%`.

### Editor Backend

By default, Kimün uses a built-in textarea for editing. There are two alternatives. You can also switch between them in the Preferences window (Editor section). The change applies the next time that you open a note.

For modal editing without an external dependency, use the built-in vim emulation:

```toml
editor_backend = "vim"
```

The emulation adds the Normal, Insert, Replace, and Visual modes of vim to the built-in editor. It supports motions, counts, operators (`d`/`c`/`y`, case operators, text objects such as `diw` and `ci"`), registers, dot-repeat, `f`/`t` finds, and more. Insert mode keeps every textarea feature (autocomplete, auto-surround, smart-Enter, the styled markdown view). [Vim Mode](@/using-kimun/vim-mode.md) has the full command table.

Or switch to [Neovim](https://neovim.io/). Then you get modal editing, custom keymaps, plugins, and everything else that your `init.lua` sets up:

```toml
editor_backend = "nvim"

# Optional: use it only if nvim is not on your PATH
nvim_path = "/usr/local/bin/nvim"
```

The Neovim backend works in this way:

- Neovim runs as a headless embedded process (`nvim --embed`), so no terminal window opens.
- Neovim loads your personal configuration (`init.lua` or `init.vim`), so your keymaps and plugins work.
- `Tab` inserts 4 spaces. Kimün sets `expandtab` and `tabstop=4` automatically, so the indentation looks correct in the TUI.
- Neovim owns the mouse, so a click does not move the cursor of Kimün. Double-click to follow a link does not work. `Ctrl+N` follows the link under the cursor as usual.
- If Neovim does not start (the binary is missing, or it crashes at start), Kimün writes a warning to the log and uses the built-in textarea.

### Key Bindings

The `[key_bindings]` section maps action names to shortcuts:

```toml
[key_bindings]
Quit = ["ctrl&Q"]
Leader = ["ctrl&B"]
OpenCommandPalette = ["ctrl&P"]
SearchNotes = ["ctrl&K"]
FileOperations = ["F2"]
TextEditor-Bold = ["ctrl&S"]
```

#### Overrides Replace the Defaults

A `[key_bindings]` section replaces the entire default keymap. It does not merge with the default keymap. Kimün binds only the actions that you list, and every action that you omit has no shortcut. The snippet above, as written, unbinds everything except those six actions. Preferences, focus movement, and the other actions stop responding.

The one exception is `Quit`. If you omit it, Kimün restores the default `Ctrl+Q`. If `Ctrl+Q` is already mapped to another action, Kimün refuses to load the configuration, so you cannot lock yourself out.

There are two safe ways to handle this:

- Keep the defaults: leave out the `[key_bindings]` section completely. Every default stays, but you cannot change any key.
- List every binding that you want: if the section exists, it is the complete keymap. Copy the full default set (see the [Keybindings cheat-sheet](@/using-kimun/keybindings.md)) and edit the lines that you want to change. The other lines keep their keys.

To change one action such as Preferences, the smallest correct configuration must still list all the other actions that you use:

```toml
[key_bindings]
OpenSettings = ["ctrl&."]   # your new Preferences key
# …plus every other binding you want to keep, e.g.:
Quit               = ["ctrl&Q"]
Leader             = ["ctrl&B"]
OpenCommandPalette = ["ctrl&P"]
SearchNotes        = ["ctrl&K"]
# …and so on for the rest of the defaults
```

`OpenSettings` is the action name of the Preferences screen in the file. `OpenPreferences` is an alias.

#### Format

Each action takes a list of one or more key combinations:

```toml
ActionName = ["ctrl&X"]
ActionName = ["ctrl&X", "alt+shift&Y"]  # multiple bindings for one action
```

The ampersand (`&`) separates the modifiers from the key. Combine modifiers with `+` (for example `"ctrl+shift&P"`).

- Modifiers: `ctrl`, `alt` (Option), `shift`
- Keys: letters `a`–`z` (not case-sensitive), digits `0`–`9`, punctuation `, . / ; ' [ ] \ \` - =` (for example `["ctrl&,"]`), and `F1`–`F12` without a modifier (for example `["F2"]`)
- Invalid combinations: Kimün ignores a combination that it cannot parse when it loads the file. It shows no error.

#### Bindable Actions

Use these names exactly as shown. For the default shortcut of each action, see the [Keybindings cheat-sheet](@/using-kimun/keybindings.md).

Navigation and UI:

- `Quit`: exit Kimün
- `Leader`: The leader gateway (default `Ctrl+B`), which starts a key sequence. See the [leader key](@/using-kimun/tui.md#the-leader-key)
- `OpenCommandPalette`: The command palette (default `Ctrl+P`)
- `OpenSettings`: Open the Preferences screen (default `F4`. `Ctrl+,` is an alias, but not every terminal sends it, so `F4` is the default)
- `ToggleSidebar`: Show/hide the drawer (default `Ctrl+T`)
- `OpenFileBrowser`: Open (or switch the drawer to) the file browser (default `Ctrl+E`)
- `ToggleQueryPanel`: Toggle the FIND drawer view (no default binding. `ToggleBacklinks` is an alias)
- `FocusEditor` / `FocusSidebar`: Move focus right / left across the visible panels
- `SwitchWorkspace`: Open the workspace switcher (default `F5`)
- `OpenSavedSearches`: Open the saved-searches picker (default `F3`)

Notes:

- `SearchNotes`: Open the query search modal (default `Ctrl+K`)
- `OpenNote`: Open a note (fuzzy file picker, default `Ctrl+O`)
- `NewJournal`: Create a new journal entry
- `QuickNote`: Open the quick capture dialog
- `FollowLink`: Follow the link under the cursor (default `Ctrl+N`. `Ctrl+Enter` also follows the link, on terminals that can distinguish it from Enter)
- `SaveCurrentQuery`: Save the active query under a name (default `Ctrl+D`)
- `FileOperations`: Open the file operations menu (delete, rename, move)
- `FindInBuffer`: Open the in-note find bar; press again to go to the next match (Textarea backend only. The Nvim backend uses its own `/` search)

Sorting:

- `OpenSortDialog`: Open the sort dialog for the focused panel (sidebar or query panel): choose sort field (name/title), direction, and (for the sidebar) whether to group directories first. (The legacy action names `CycleSortField` and `SortReverseOrder` still parse and map to this action.)

Text editing (these actions work only if the editor has focus):

These actions have no default binding. You reach formatting through the `+text` group of the leader (`Ctrl+B t b`, `t i`, `t s`). You can bind them if you want a chord. Only `Bold`, `Italic`, and `Strikethrough` are implemented.

- `TextEditor-Bold`
- `TextEditor-Italic`
- `TextEditor-Underline`
- `TextEditor-Strikethrough`
- `TextEditor-Link`: Insert a link
- `TextEditor-Image`: Insert an image
- `TextEditor-ToggleHeader`: Cycle heading level
- `TextEditor-Header1` through `TextEditor-Header6`: Set a specific heading level

### Leader Tree Overrides

You can change the sequence tree of the leader key. `[leader.bind]` maps a key sequence (the keys *after* the gateway, separated by spaces) to an action id. Use `"none"` to remove a binding. `[leader.labels]` renames the group captions in the which-key panel and the cheatsheet.

```toml
[leader.bind]
"o f" = "find.files"     # remap: leader o f now opens the file picker
"x"   = "note.daily"     # add:   leader x opens today's journal
"g p" = "none"           # remove a binding

[leader.labels]
"f" = "+search"          # rename the +find group caption
```

Action ids use the scheme `group.action` (`find.files`, `note.new`, `vault.theme`, `drawer.links`, …). The cheatsheet (`Ctrl+B ?`) and the command palette always show your overrides. Kimün skips unknown ids and malformed sequences and writes a warning to the log. They do not stop the startup. If you assign a single key that is currently the name of a whole group, the new binding replaces that group, and Kimün writes a warning to the log. Use two-key sequences unless you want this.

### Files Kimün Stores on Disk

Kimün writes two more kinds of files in addition to `config.toml`. By default, both are next to `config.toml`. To move them, use `cache_dir` and `history_dir`.

| File | Default location | Purpose |
|---|---|---|
| `<workspace>.kimuncache` | `<config_dir>/<workspace>.kimuncache` | SQLite search index of the workspace. Kimün can rebuild it, so you can delete it. Kimün rebuilds it on the next run. |
| `<workspace>.txt` | `<config_dir>/history/<workspace>.txt` | History of recently opened notes of the workspace. Plain text, one path per line, newest first. |

These files are outside the workspace because they change each time that you open a note. The workspace holds only Markdown files. Because the files are outside, you can sync your notes with Syncthing, iCloud, or Git and not sync SQLite blobs and history rewrites.

### Upgrading from `config_version = 2`

If your configuration still says `config_version = 2`, Kimün upgrades it automatically the next time that you start it:

1. Kimün checks every workspace name against the rules above. If a name is not valid, Kimün stops the upgrade with an error that lists every invalid name. It leaves `config.toml` at version 2. Rename the workspaces and start Kimün again.
2. Kimün writes a backup at `config.toml.bak.v2` next to your original configuration. You can use it to go back.
3. Kimün moves the `<workspace>/kimun.sqlite` of each workspace to `<cache_dir>/<workspace>.kimuncache` (the default is your configuration directory).
4. Kimün extracts the `last_paths` of each workspace into `<history_dir>/<workspace>.txt`.
5. Kimün removes `last_paths` from `config.toml`.

The migration is idempotent (safe to run twice). If you run it on a configuration that is already upgraded, nothing changes. Kimün skips each step whose destination already exists.
