+++
title = "Configuration"
weight = 3
+++

# Configuration Reference

Kimün stores its configuration in `kimun_config.toml`. By default it lives in your OS config directory (`~/.config/kimun/` on Linux/macOS, `%USERPROFILE%\kimun\` on Windows), and can be overridden with `kimun --config /path/to/config.toml`.

The file has three kinds of contents:

1. **Top-level fields** — app-wide settings that apply everywhere.
2. **`[global]`** — which workspace is currently active.
3. **`[workspaces.<name>]`** — one block per workspace, mapping a name to a notes directory.
4. **`[key_bindings]`** — keyboard shortcuts for the TUI.

You don't need to write this file by hand. Kimün creates it on first run and updates it when you change settings from the TUI's Settings screen. This page is the reference for when you *do* want to edit it.

## Top-Level Fields

| Field | Type | Default | Description |
|---|---|---|---|
| `config_version` | integer | `2` | Schema version. Managed by the config migration system — do not edit. |
| `theme` | string | `""` | Active TUI theme name (e.g. `"Nord"`). An empty string uses the built-in default. See [Themes](@/using-kimun/themes.md). |
| `autosave_interval_secs` | integer | `5` | How often unsaved changes are written to disk (seconds). |
| `use_nerd_fonts` | boolean | `false` | Enable Nerd Font glyphs in the TUI. Leave `false` if your terminal's font doesn't include Nerd Font patches. |
| `editor_backend` | string | `"textarea"` | Editor engine. `"textarea"` = built-in editor. `"nvim"` = embedded Neovim. |
| `nvim_path` | string | *(unset)* | Absolute path to the `nvim` binary. Only needed when Neovim is not on `PATH`. |
| `default_sort_field` | string | `"name"` | Sort field for the note browser. One of `"name"`, `"title"`. |
| `default_sort_order` | string | `"ascending"` | Sort direction for the note browser. One of `"ascending"`, `"descending"`. |
| `journal_sort_field` | string | `"name"` | Sort field for the journal view. One of `"name"`, `"title"`. |
| `journal_sort_order` | string | `"descending"` | Sort direction for the journal view. One of `"ascending"`, `"descending"`. Descending shows newest first. |

## `[global]` Section

| Field | Type | Default | Description |
|---|---|---|---|
| `current_workspace` | string | *(unset)* | Name of the workspace Kimün loads at startup. Must match a `[workspaces.<name>]` key. |

That's the only field. Theme lives at the top level, not here.

```toml
[global]
current_workspace = "default"
```

## `[workspaces.<name>]` Sections

Each workspace is a separate block. The `<name>` after the dot is the identifier you reference from `[global].current_workspace` and from the workspace switcher in the TUI.

| Field | Type | Default | Description |
|---|---|---|---|
| `path` | string | **required** | Path to the notes directory for this workspace. |
| `inbox_path` | string | `"/inbox"` | Vault-relative directory for quick-captured notes. |
| `quick_note_path` | string | `"/"` | Vault-relative directory where `QuickNote` saves its output. Defaults to the vault root. |
| `created` | string (RFC 3339 timestamp) | *(set by Kimün)* | Creation timestamp. Managed automatically — do not edit. |
| `last_paths` | array of strings | *(set by Kimün)* | Internal history of recently-opened notes. Managed automatically — do not edit. |

### Path formats

The `path` field accepts three formats:

- **Absolute:** `/home/alice/notes` or `C:\Users\alice\notes`
- **Relative:** `../my-notes` or `personal` — resolved against the config file's directory
- **Home-relative:** `~/notes` — `~` expands to `$HOME` on Linux/macOS and `%USERPROFILE%` on Windows

### Example

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

## Editor Backend

By default, Kimün uses a built-in textarea for note editing. You can switch to [Neovim](https://neovim.io/) as the editor backend to get modal editing, custom keymaps, plugins, and anything else your `init.lua` / `init.vim` provides.

### Requirements

- Neovim must be installed and available on your `PATH` (or point Kimün at a specific binary via `nvim_path`).
- Neovim is launched as a headless embedded process (`nvim --embed`); no terminal window opens.

### Configuration

```toml
editor_backend = "nvim"

# Optional — only when nvim is not on your PATH
nvim_path = "/usr/local/bin/nvim"
```

### Behaviour notes

- **Tab key** inserts 4 spaces. `expandtab` and `tabstop=4` are set automatically so indentation renders correctly in the TUI.
- Your personal Neovim config (`init.lua` / `init.vim`) loads normally, so custom keymaps and plugins work as expected.
- If Neovim fails to start (binary not found, crash on init, etc.), Kimün logs a warning and falls back to the built-in textarea automatically.

## `[key_bindings]` Section

Key bindings map action names to keyboard shortcuts.

### Format

Each action is a list of one or more key combinations:

```toml
ActionName = ["ctrl&X"]
ActionName = ["ctrl&X", "alt+shift&Y"]  # multiple bindings for one action
```

The ampersand (`&`) separates the modifier chain from the key. Combine multiple modifiers with `+`.

### Supported modifiers

- `ctrl` — Control
- `alt` — Alt / Option
- `shift` — Shift
- Example: `"ctrl+shift&P"`

### Supported keys

- Letters `a`–`z` (case-insensitive)
- Function keys `F1`–`F12`, used bare with no modifier (e.g. `["F2"]`)

### Unrecognised combinations

Any key combination that doesn't parse is silently ignored at load time. You can safely comment out or experiment with bindings without breaking the config.

### Bindable actions

Use these action names exactly as shown.

**Navigation & UI**

- `Quit` — Exit Kimün
- `OpenSettings` — Open the settings dialog
- `ToggleSidebar` — Show/hide the sidebar
- `ToggleBacklinks` — Show/hide the backlinks panel
- `TogglePreview` — Show/hide the preview pane
- `FocusEditor` — Move focus right (Sidebar → Editor → Backlinks)
- `FocusSidebar` — Move focus left (Backlinks → Editor → Sidebar)
- `SwitchWorkspace` — Open the workspace switcher

**Notes**

- `SearchNotes` — Open the search / fuzzy finder
- `OpenNote` — Open a note (fuzzy file picker)
- `NewJournal` — Create a new journal entry
- `QuickNote` — Open the quick capture dialog
- `FollowLink` — Follow the wiki link under the cursor
- `FileOperations` — Open the file operations menu (delete, rename, move)

**Sorting**

- `CycleSortField` — Cycle the sort field (`name` → `title` → `name` → …)
- `SortReverseOrder` — Toggle sort direction (ascending ↔ descending)

**Text editing** (only fire while the editor has focus)

- `TextEditor-Bold`
- `TextEditor-Italic`
- `TextEditor-Underline`
- `TextEditor-Strikethrough`
- `TextEditor-Link` — Insert a link
- `TextEditor-Image` — Insert an image
- `TextEditor-ToggleHeader` — Cycle heading level
- `TextEditor-Header1` through `TextEditor-Header6` — Set a specific heading level

### Examples

```toml
[key_bindings]
Quit = ["ctrl&Q"]
OpenSettings = ["ctrl&P"]
SearchNotes = ["ctrl&K"]
OpenNote = ["ctrl&O"]
NewJournal = ["ctrl&J"]
QuickNote = ["ctrl&W"]
ToggleSidebar = ["ctrl&T"]
ToggleBacklinks = ["ctrl&E"]
TogglePreview = ["ctrl&Y"]
FileOperations = ["F2"]
SwitchWorkspace = ["F4"]
FollowLink = ["ctrl&G"]
TextEditor-Bold = ["ctrl&B"]
TextEditor-Italic = ["ctrl&I"]
TextEditor-Link = ["ctrl&L"]
TextEditor-Image = ["ctrl+shift&L"]
```

## Minimal Example

A complete, minimal config file:

```toml
config_version = 2
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
