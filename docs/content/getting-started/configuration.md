+++
title = "Configuration"
weight = 3
+++

# Configuration Reference

Kimün stores its configuration in `kimun_config.toml`, typically located in your config directory (e.g., `~/.config/kimun/` on Linux/macOS or `%APPDATA%\kimun\` on Windows).

## Top-Level Fields

| Field | Type | Default | Description |
|---|---|---|---|
| `config_version` | integer | `2` | Config schema version. Do not change manually. |
| `theme` | string | `""` | Unused at top level — set theme in `[global]` instead. |
| `autosave_interval_secs` | integer | `5` | How often unsaved changes are written to disk (seconds). |
| `use_nerd_fonts` | boolean | `true` | Enable Nerd Font icons in the TUI. Set to `false` if your terminal lacks Nerd Font support. |
| `editor_backend` | string | `"textarea"` | Editor engine. `"textarea"` uses the built-in editor; `"nvim"` uses Neovim. |
| `nvim_path` | string | *(unset)* | Path to the `nvim` binary. Only needed when Neovim is not on `PATH`. |
| `default_sort_field` | string | `"name"` | Default sort field for the note browser. Options: `"name"`, `"title"`. |
| `default_sort_order` | string | `"ascending"` | Default sort direction. Options: `"ascending"`, `"descending"`. |
| `journal_sort_field` | string | `"name"` | Sort field used in the journal view. Options: `"name"`, `"title"`. |
| `journal_sort_order` | string | `"descending"` | Sort direction for journal entries. Descending shows newest first. |

## `[global]` Section

The `[global]` section contains workspace-level and theme settings.

- **`current_workspace`** — name of the currently active workspace (e.g. `"default"`). This workspace will be loaded when Kimün starts.
- **`theme`** — active theme name (e.g. `"Nord"`). This overrides the `theme` field at the top level. See [Themes](@/using-kimun/themes.md) for built-in options and how to create custom themes.

Example:
```toml
[global]
current_workspace = "default"
theme = "Nord"
```

## `[workspaces.<name>]` Sections

Each workspace is a separate section with the naming pattern `[workspaces.<name>]`. A workspace maps a name to a directory of notes.

- **`path`** — absolute path to the notes directory for this workspace. Kimün will read and write markdown files from this location.
- **`inbox_path`** — vault-relative directory for quick notes (default: `/inbox`). Quick notes created via `Ctrl+W` (TUI), `kimun note quick` (CLI), or the `quick_note` MCP tool are stored here with timestamp-based filenames.

Example:
```toml
[workspaces.default]
path = "/Users/alice/Documents/Notes"
inbox_path = "/inbox"

[workspaces.work]
path = "/Users/alice/work-notes"
inbox_path = "/capture"

[workspaces.archive]
path = "/Users/alice/archive-notes"
```

## Editor Backend

By default, Kimün uses a built-in textarea for note editing. You can switch to [Neovim](https://neovim.io/) as the editor backend to get full modal editing, custom keymaps, plugins, and anything else your `init.lua` / `init.vim` provides.

### Requirements

- Neovim must be installed and available on your `PATH` (or you can point Kimün at a specific binary — see below).
- Neovim is launched as a headless embedded process (`nvim --embed`); no terminal window opens.

### Configuration

| Field | Type | Default | Description |
|---|---|---|---|
| `editor_backend` | string | `"textarea"` | Editor engine. Set to `"nvim"` to enable the Neovim backend. |
| `nvim_path` | string | *(unset)* | Absolute path to the `nvim` binary. Omit to use the `nvim` found on `PATH`. |

```toml
editor_backend = "nvim"

# Optional — only needed if nvim is not on your PATH:
nvim_path = "/usr/local/bin/nvim"
```

### Behaviour notes

- **Tab key** inserts 4 spaces (`expandtab`, `tabstop=4` are set automatically so indentation renders correctly in the TUI).
- Your personal Neovim config (`init.lua` / `init.vim`) is loaded normally, so custom keymaps and plugins work as expected.
- If Neovim fails to start (binary not found, crash on init, etc.), Kimün logs a warning and falls back to the built-in textarea automatically.

## `[key_bindings]` Section

Key bindings map action names to keyboard shortcuts. This lets you customize how Kimün responds to your input.

### Format

Each action is a list of key combinations:

```toml
ActionName = ["ctrl&X"]
ActionName = ["ctrl&X", "alt+shift&Y"]  # Multiple bindings for one action
```

### Supported Modifiers

- `ctrl` — Control key
- `alt` — Alt/Option key
- `shift` — Shift key
- Combine modifiers with `+` (e.g., `"ctrl+shift&P"`)

### Supported Keys

- Letters (a–z), uppercase or lowercase
- F-keys (F1–F12), used bare with no modifier (e.g., `["F2"]`)

### Unrecognised Combinations

Any key combination that doesn't follow these rules is silently ignored at load time. This allows you to safely comment out or experiment with bindings without breaking the config.

### Bindable Actions

Use these action names exactly as shown:

- **Navigation & UI**
  - `Quit` — Exit Kimün
  - `OpenSettings` — Open the settings dialog
  - `ToggleNoteBrowser` — Show/hide the note browser panel
  - `SearchNotes` — Open the search/fuzzy finder
  - `OpenNote` — Open a note (fuzzy file picker)
  - `TogglePreview` — Show/hide the preview pane
  - `ToggleSidebar` — Show/hide the sidebar
  - `ToggleBacklinks` — Show/hide the backlinks panel
  - `FocusEditor` — Move focus right (Sidebar → Editor → Backlinks)
  - `FocusSidebar` — Move focus left (Backlinks → Editor → Sidebar)
  - `SwitchWorkspace` — Open the workspace switcher

- **Note Management**
  - `NewJournal` — Create a new journal entry
  - `QuickNote` — Open the quick note dialog to capture a thought

- **Sorting**
  - `CycleSortField` — Cycle the sort field (filename → title → filename → …)
  - `SortReverseOrder` — Toggle sort direction (ascending ↔ descending)

- **File Operations**
  - `FileOperations` — Open the file operations menu (delete, rename, move)

- **Text Formatting**
  - `TextEditor-Italic` — Insert italic markers
  - `TextEditor-Image` — Insert image markup
  - `TextEditor-ToggleHeader` — Toggle header level
  - `TextEditor-Underline` — Insert underline markers
  - `TextEditor-Strikethrough` — Insert strikethrough markers

### Examples

```toml
Quit = ["ctrl&Q"]
OpenSettings = ["ctrl&P"]
SearchNotes = ["ctrl&E"]
FileOperations = ["F2"]
TextEditor-Italic = ["ctrl&I"]
TextEditor-Image = ["ctrl+shift&L"]
```

## Minimal Example

Here's a complete, minimal config file to get started:

```toml
config_version = 2
theme = ""
autosave_interval_secs = 5
use_nerd_fonts = true
default_sort_field = "name"
default_sort_order = "ascending"
journal_sort_field = "name"
journal_sort_order = "descending"

[global]
current_workspace = "default"
theme = "Nord"

[workspaces.default]
path = "/Users/alice/Documents/Notes"

[key_bindings]
Quit = ["ctrl&Q"]
OpenSettings = ["ctrl&P"]
ToggleNoteBrowser = ["ctrl&F"]
SearchNotes = ["ctrl&K"]
OpenNote = ["ctrl&O"]
NewJournal = ["ctrl&J"]
QuickNote = ["ctrl&W"]
ToggleBacklinks = ["ctrl&E"]
TogglePreview = ["ctrl&Y"]
FileOperations = ["F2"]
SwitchWorkspace = ["F4"]
```
