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
| `default_sort_field` | string | `"name"` | Default sort field for the note browser. Options: `"name"`, `"title"`. |
| `default_sort_order` | string | `"ascending"` | Default sort direction. Options: `"ascending"`, `"descending"`. |
| `journal_sort_field` | string | `"name"` | Sort field used in the journal view. Options: `"name"`, `"title"`. |
| `journal_sort_order` | string | `"descending"` | Sort direction for journal entries. Descending shows newest first. |

## `[global]` Section

The `[global]` section contains workspace-level and theme settings.

- **`current_workspace`** — name of the currently active workspace (e.g. `"default"`). This workspace will be loaded when Kimün starts.
- **`theme`** — active theme name (e.g. `"Nord"`). This overrides the `theme` field at the top level.

Example:
```toml
[global]
current_workspace = "default"
theme = "Nord"
```

## `[workspaces.<name>]` Sections

Each workspace is a separate section with the naming pattern `[workspaces.<name>]`. A workspace maps a name to a directory of notes.

- **`path`** — absolute path to the notes directory for this workspace. Kimün will read and write markdown files from this location.

Example:
```toml
[workspaces.default]
path = "/Users/alice/Documents/Notes"

[workspaces.work]
path = "/Users/alice/work-notes"

[workspaces.archive]
path = "/Users/alice/archive-notes"
```

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
  - `FocusEditor` — Move focus to the editor
  - `FocusSidebar` — Move focus to the sidebar

- **Note Management**
  - `NewJournal` — Create a new journal entry

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
SearchNotes = ["ctrl&E"]
OpenNote = ["ctrl&O"]
NewJournal = ["ctrl&J"]
TogglePreview = ["ctrl&Y"]
FileOperations = ["F2"]
```
