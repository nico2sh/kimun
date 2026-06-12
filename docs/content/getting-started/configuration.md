+++
title = "Configuration"
weight = 3
+++

# Configuration

All of Kimün's settings live in one `config.toml`:

- **Linux / macOS:** `~/.config/kimun/config.toml`
- **Windows:** `%USERPROFILE%\kimun\config.toml`
- **Anywhere else:** `kimun --config /path/to/config.toml`

You rarely need to edit it by hand: Kimün creates it on first run, and the TUI's Preferences screen (`Ctrl+,`) writes changes for you. This page is for when you want to get your hands dirty anyway.

## Guided Setup

The first time you launch Kimün with no workspace configured, a centered setup dialog walks you through the essentials. Nothing is written until the final step — you can bail at any point with `Esc`.

### Steps

**1. Welcome** — a brief introduction explaining what the setup covers. Press `Enter` to begin.

**2. Workspace** — choose where your notes live. Kimün suggests `~/kimun-notes` as a starting point. From here:

- Press `b` to open a directory browser and navigate to an existing folder.
- Inside the browser, press `n` to create a new directory on the spot.
- Press `e` to edit the workspace name — the label Kimün uses for the workspace, not its directory path.

**3. Nerd Fonts** — two sample rows of glyphs let you check whether your terminal font includes Nerd Font patches. If the symbols look broken, leave `use_nerd_fonts` off; if they look sharp, turn it on.

**4. Theme** — scroll through available themes with a live preview. You can change your mind later via `Ctrl+,`.

**5. Editor Backend** — choose how you edit notes:

- `textarea` — Kimün's built-in editor (always available).
- `vim` — built-in Vim emulation.
- `nvim` — embedded Neovim (only selectable when `nvim` is found on `PATH`).

**6. Summary** — a recap of every choice. Press `Enter` to apply them all atomically (the config file is written once, here). Press `Esc` to discard everything and leave your config unchanged.

### Running It Again

You can reopen the guided setup any time:

- **Command palette** (`Ctrl+P` inside the editor) → type "guided setup".
- **Leader sequence** `v o` (default leader is `Ctrl+G`, so: `Ctrl+G` then `v` then `o`).

On rerun, the Workspace step is informational — it lists your current workspaces rather than prompting you to set one up. To add, rename, or remove workspaces, use the Preferences screen (`Ctrl+,`) or the `kimun workspace` CLI commands (see [Workspaces](@/getting-started/workspaces.md)).

---

## A Complete Config

This is a full, working config file. Everything else on this page is optional detail:

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

[key_bindings]
Quit = ["ctrl&Q"]
SearchNotes = ["ctrl&K"]
OpenNote = ["ctrl&O"]
NewJournal = ["ctrl&J"]
QuickNote = ["ctrl&W"]
```

## The Settings You'll Actually Touch

Most edits are one of these:

| Setting | What it does |
|---|---|
| `theme` | TUI theme name, e.g. `theme = "Nord"`. Empty = built-in default. See [Themes](@/using-kimun/themes.md). |
| `editor_backend` | `"textarea"` (built-in editor), `"vim"` (built-in vim emulation), or `"nvim"` (embedded Neovim). See [Editor Backend](#editor-backend). |
| `[workspaces.<name>].path` | Where your notes live. |
| `autosave_interval_secs` | How often unsaved changes hit disk (seconds). |
| `use_nerd_fonts` | Fancy glyphs, if your terminal font has Nerd Font patches. |
| `[key_bindings]` | Remap any shortcut (see [Key Bindings](#key-bindings)). |

Everything below is the full reference.

---

## Full Reference

The file has five kinds of contents:

1. **Top-level fields** — app-wide settings.
2. **`[global]`** — which workspace is active.
3. **`[workspaces.<name>]`** — one block per workspace.
4. **`[key_bindings]`** — keyboard shortcuts for the TUI.
5. **`[leader.bind]` / `[leader.labels]`** — leader-key overrides.

### Top-Level Fields

| Field | Type | Default | Description |
|---|---|---|---|
| `config_version` | integer | `6` | Schema version. Managed by the config migration system; do not edit. |
| `cache_dir` | string | `"."` | Directory for per-workspace SQLite caches (`<workspace>.kimuncache`). Resolved relative to the config file's directory. Accepts `~`, relative, or absolute paths. |
| `history_dir` | string | `"history"` | Directory for per-workspace history files (`<workspace>.txt`). Same path resolution as `cache_dir`. |
| `theme` | string | `""` | Active TUI theme name (e.g. `"Nord"`). Empty string = built-in default. See [Themes](@/using-kimun/themes.md). |
| `autosave_interval_secs` | integer | `5` | How often unsaved changes are written to disk (seconds). |
| `leader_timeout_ms` | integer | `400` | Hesitation (milliseconds) before the which-key panel reveals itself during a pending leader sequence. Sequences typed faster never wait. |
| `use_nerd_fonts` | boolean | `false` | Enable Nerd Font glyphs in the TUI. Leave `false` if your terminal's font doesn't include Nerd Font patches. |
| `editor_backend` | string | `"textarea"` | Editor engine. `"textarea"` = built-in editor. `"vim"` = built-in vim emulation. `"nvim"` = embedded Neovim. Also editable from the Preferences window (Editor section). |
| `nvim_path` | string | *(unset)* | Absolute path to the `nvim` binary. Only needed when Neovim is not on `PATH`. |
| `default_sort_field` | string | `"name"` | Sort field for the note browser. One of `"name"`, `"title"`. |
| `default_sort_order` | string | `"ascending"` | Sort direction for the note browser. One of `"ascending"`, `"descending"`. |
| `journal_sort_field` | string | `"name"` | Sort field for the journal view. One of `"name"`, `"title"`. |
| `journal_sort_order` | string | `"descending"` | Sort direction for the journal view. Descending shows newest first. |
| `group_directories` | boolean | `false` | When `true`, the sidebar lists directories first, each group sorted by the chosen field/order. Set live from the sort dialog. |

### `[global]` Section

One field, and it's this one:

```toml
[global]
current_workspace = "default"
```

| Field | Type | Default | Description |
|---|---|---|---|
| `current_workspace` | string | *(unset)* | Workspace Kimün loads at startup. Must match a `[workspaces.<name>]` key. |

(Theme lives at the top level, not here.)

### `[workspaces.<name>]` Sections

One block per workspace. The `<name>` after the dot is the identifier you reference from `[global].current_workspace` and the TUI's workspace switcher. It also names the workspace's cache and history files (`<name>.kimuncache`, `<name>.txt`), so it must be a valid filename (see [Workspace Name Rules](#workspace-name-rules)).

| Field | Type | Default | Description |
|---|---|---|---|
| `path` | string | **required** | Path to the notes directory for this workspace. |
| `inbox_path` | string | `"/inbox"` | Vault-relative directory for quick-captured notes. |
| `quick_note_path` | string | `"/"` | Vault-relative directory where `QuickNote` saves its output. Defaults to the vault root. |
| `created` | string (RFC 3339 timestamp) | *(set by Kimün)* | Creation timestamp. Managed automatically; do not edit. |

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

Workspace names become filenames, so they follow cross-platform filename rules. A new workspace is rejected if the name:

- contains any of `\ / : * ? " < > | [ ] ^ #` or control characters
- is a Windows reserved name (`con`, `prn`, `aux`, `nul`, `com1`–`com9`, `lpt1`–`lpt9`)
- starts with two or more dots, ends with a dot, or has leading/trailing whitespace
- exceeds 64 characters

Names are lowercased before validating, so `MyVault` and `myvault` are the same workspace.

#### Path Formats

The `path` field accepts three formats:

- **Absolute:** `/home/alice/notes` or `C:\Users\alice\notes`
- **Relative:** `../my-notes` or `personal` — resolved against the config file's directory
- **Home-relative:** `~/notes` — `~` expands to `$HOME` on Linux/macOS and `%USERPROFILE%` on Windows

### Editor Backend

By default, Kimün uses a built-in textarea for editing. Two alternatives exist, and you can also switch between them from the Preferences window (Editor section) — the change applies the next time you open a note.

For modal editing without any external dependency, use the built-in vim emulation:

```toml
editor_backend = "vim"
```

It layers vim's Normal/Insert/Replace/Visual modes over the built-in editor: motions, counts, operators (`d`/`c`/`y`, case ops, text objects like `diw` and `ci"`), registers, dot-repeat, `f`/`t` finds, and more — while keeping every textarea feature (autocomplete, auto-surround, smart-Enter, the styled markdown view) in Insert mode. The full command table lives in [Vim Mode](@/using-kimun/vim-mode.md).

Or switch to [Neovim](https://neovim.io/) to get modal editing, custom keymaps, plugins, and whatever else your `init.lua` provides:

```toml
editor_backend = "nvim"

# Optional — only when nvim is not on your PATH
nvim_path = "/usr/local/bin/nvim"
```

Worth knowing:

- Neovim runs as a headless embedded process (`nvim --embed`), so no terminal window opens.
- Your personal config (`init.lua` / `init.vim`) loads normally, so keymaps and plugins work as expected.
- **Tab** inserts 4 spaces (`expandtab` + `tabstop=4` are set automatically so indentation renders correctly in the TUI).
- If Neovim fails to start (binary missing, crash on init), Kimün logs a warning and falls back to the built-in textarea. No drama.

### Key Bindings

The `[key_bindings]` section maps action names to shortcuts:

```toml
[key_bindings]
Quit = ["ctrl&Q"]
Leader = ["ctrl&G"]
OpenCommandPalette = ["ctrl&P"]
SearchNotes = ["ctrl&K"]
FileOperations = ["F2"]
TextEditor-Bold = ["ctrl&B"]
```

#### Format

Each action takes a list of one or more key combinations:

```toml
ActionName = ["ctrl&X"]
ActionName = ["ctrl&X", "alt+shift&Y"]  # multiple bindings for one action
```

The ampersand (`&`) separates the modifier chain from the key. Combine modifiers with `+` (e.g. `"ctrl+shift&P"`).

- **Modifiers:** `ctrl`, `alt` (Option), `shift`
- **Keys:** letters `a`–`z` (case-insensitive), digits `0`–`9`, punctuation `, . / ; ' [ ] \ \` - =` (e.g. `["ctrl&,"]`), and `F1`–`F12` used bare with no modifier (e.g. `["F2"]`)
- **Typos are harmless:** combinations that don't parse are silently ignored at load time, so experiment freely.

#### Bindable Actions

Use these names exactly as shown. For the default shortcuts each one ships with, see the [Keybindings cheat-sheet](@/using-kimun/keybindings.md).

**Navigation & UI**

- `Quit` — Exit Kimün
- `Leader` — The leader gateway (default `Ctrl+G`) — starts a key sequence; see the [leader key](@/using-kimun/tui.md#the-leader-key)
- `OpenCommandPalette` — The command palette (default `Ctrl+P`)
- `OpenSettings` — Open the Preferences screen (default `Ctrl+,`)
- `ToggleSidebar` — Show/hide the drawer (default `Ctrl+T`)
- `OpenFileBrowser` — Open (or switch the drawer to) the file browser (default `Ctrl+E`)
- `ToggleQueryPanel` — Toggle the FIND drawer view (no default binding; `ToggleBacklinks` still parses as an alias)
- `FocusEditor` / `FocusSidebar` — Move focus right / left across the visible panels
- `SwitchWorkspace` — Open the workspace switcher
- `OpenSavedSearches` — Open the saved-searches picker (default `F3`)

**Notes**

- `SearchNotes` — Open the query search modal (default `Ctrl+K`)
- `OpenNote` — Open a note (fuzzy file picker, default `Ctrl+O`)
- `NewJournal` — Create a new journal entry
- `QuickNote` — Open the quick capture dialog
- `FollowLink` — Follow the link under the cursor (default `Ctrl+N`; `Ctrl+Enter` also follows, built-in, on terminals that can distinguish it from Enter)
- `SaveCurrentQuery` — Save the active query under a name (default `Ctrl+D`)
- `FileOperations` — Open the file operations menu (delete, rename, move)
- `FindInBuffer` — Open the in-note find bar; press again to advance to the next match (Textarea backend only — the Nvim backend uses its own `/` search)

**Sorting**

- `OpenSortDialog` — Open the sort dialog for the focused panel (sidebar or query panel): choose sort field (name/title), direction, and (for the sidebar) whether to group directories first. (The legacy action names `CycleSortField` and `SortReverseOrder` still parse and map to this action.)

**Text editing** (only fire while the editor has focus)

- `TextEditor-Bold`
- `TextEditor-Italic`
- `TextEditor-Underline`
- `TextEditor-Strikethrough`
- `TextEditor-Link` — Insert a link
- `TextEditor-Image` — Insert an image
- `TextEditor-ToggleHeader` — Cycle heading level
- `TextEditor-Header1` through `TextEditor-Header6` — Set a specific heading level

### Leader Tree Overrides

The leader key's sequence tree is fully remappable. `[leader.bind]` maps a key sequence (the keys *after* the gateway, space-separated) to an action id, or `"none"` to remove a binding. `[leader.labels]` renames group captions shown in the which-key panel and cheatsheet.

```toml
[leader.bind]
"o f" = "find.files"     # remap: leader o f now opens the file picker
"x"   = "note.daily"     # add:   leader x opens today's journal
"g p" = "none"           # remove a binding

[leader.labels]
"f" = "+search"          # rename the +find group caption
```

Action ids follow a `group.action` scheme (`find.files`, `note.new`, `vault.theme`, `drawer.links`, …). The cheatsheet (`Ctrl+G ?`) and the command palette always reflect your overrides. Unknown ids and malformed sequences are skipped with a log warning; they never break startup. Assigning a single key that currently names a whole group replaces that group (a warning is logged), so prefer two-key sequences unless that is what you want.

### Files Kimün Stores on Disk

Alongside `config.toml`, Kimün writes two more kinds of files. Both live next to `config.toml` by default; relocate them with `cache_dir` and `history_dir`.

| File | Default location | Purpose |
|---|---|---|
| `<workspace>.kimuncache` | `<config_dir>/<workspace>.kimuncache` | Per-workspace SQLite search index. Regenerable and safe to delete; Kimün rebuilds it on the next run. |
| `<workspace>.txt` | `<config_dir>/history/<workspace>.txt` | Per-workspace history of recently-opened notes. Plain text, one path per line, newest first. |

Why keep these outside the workspace? They change on every note open, while your workspace is just Markdown files. Keeping them out means you can sync your notes with Syncthing, iCloud, or Git without churning through SQLite blobs and history rewrites.

### Upgrading from `config_version = 2`

If your config still says `config_version = 2`, the next launch upgrades it automatically:

1. Validates every workspace name against the rules above. If any fails, Kimün aborts the upgrade with an error listing every offending name and leaves `config.toml` at version 2. Rename them and relaunch.
2. Writes a backup at `config.toml.bak.v2` next to your original config, in case you want to roll back.
3. Moves each workspace's `<workspace>/kimun.sqlite` to `<cache_dir>/<workspace>.kimuncache` (defaults to your config dir).
4. Extracts each workspace's `last_paths` into `<history_dir>/<workspace>.txt`.
5. Drops `last_paths` from `config.toml` going forward.

The migration is idempotent: re-running it on an already-upgraded config does nothing, and any step whose destination already exists is skipped.
