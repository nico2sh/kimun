+++
title = "TUI"
weight = 10
+++

# TUI Reference

Kimün's terminal UI provides an intuitive interface for managing and editing your notes. This page describes the available screens, navigation patterns, and key bindings.

## Screens

### Browse

The Browse screen displays a file tree navigator for your workspace directory. You can traverse through folders and files using arrow keys, open notes to edit them, and perform file operations like rename, move, and delete.

**Key features:**
- Navigate the note hierarchy with arrow keys
- Press Enter to open a note in the Editor
- Sort notes by name, title, or reverse the sort order
- Perform file operations (rename, move, delete)

### Editor

The Editor screen is a Markdown editor for writing and editing notes. It features:

- **Sidebar (file browser pane):** A collapsible pane on the left showing the file tree of your workspace
- **Main editor area:** Your note content with Markdown formatting support
- **Preview pane:** A toggleable preview showing how your Markdown renders

The preview pane is toggled with `Ctrl+Y` and shows a live preview of your note as you type.

**Key features:**
- Full Markdown syntax support
- Text formatting shortcuts (bold, italic, strikethrough, headers)
- Autosave functionality
- Navigate between the editor and sidebar using focus commands

### Settings

The Settings screen lets you configure Kimün's behavior and appearance:

- **Notes directory:** Set or change the location of your workspace
- **Active workspace:** Switch between multiple note workspaces
- **Theme:** Choose from available color themes
- **Key bindings:** Customize keyboard shortcuts for any action
- **Other preferences:** Autosave interval, font settings, and more

All settings are stored in your config file (see [Configuration Reference](@/getting-started/configuration.md) for details).

## Workspace Switcher

Press `F4` to open the workspace switcher. It lists all configured workspaces with the current one marked. Use Up/Down to navigate and Enter to switch — the app transitions to the new workspace, validating and indexing it as needed.

Workspace management (create, rename, delete) is available in the Settings screen (`Ctrl+P`) under the **Workspaces** section:
- **n** — create a new workspace (enter a name, then browse for a directory)
- **r** — rename the selected workspace
- **d** — delete the selected workspace (cannot delete the current one)
- **b** — browse to change the selected workspace's directory path

The file browser supports type-ahead: press any letter to jump to the first directory starting with that character. Press the same letter again to cycle through matches.

## Navigation Patterns

### Basic Movement

- **Arrow keys** — Move up/down/left/right through the file tree
- **Enter** — Open the selected note in the Editor

### Focus Management

- **`Ctrl+L`** — Move focus right (Sidebar → Editor → Backlinks)
- **`Ctrl+H`** — Move focus left (Backlinks → Editor → Sidebar)

Focus moves directionally through the visible panels. If the target panel is hidden, it is opened automatically (e.g., pressing `Ctrl+L` from the editor opens the backlinks panel if it's not visible).

### Panels and Views

- **`Ctrl+F`** — Toggle the note browser panel visibility
- **`Ctrl+Y`** — Toggle the preview pane (Editor only)
- **`Ctrl+E`** — Toggle the backlinks panel (right side)
- **`Ctrl+T`** — Toggle the sidebar

### Sorting

- **`Ctrl+N`** — Cycle sort field (filename → title → filename → …)
- **`Ctrl+R`** — Reverse the sort order

### Quick Note

Press `Ctrl+W` to open the quick note dialog. Type a thought and press Enter to save it — the note is created in your inbox directory with a timestamp filename, and you stay on the current note without interruption. Use Shift+Enter to save and immediately open the new note instead.

### Backlinks Panel

Press `Ctrl+E` to toggle the backlinks panel on the right side of the editor. It shows all notes that link to the current note — useful for understanding context and navigating related ideas without leaving your current work.

- **Up/Down** — navigate the backlinks list
- **Enter** — expand the selected backlink to show the paragraph that contains the link. Press Enter again to show the full note content. Press Enter a third time to collapse.
- **Ctrl+G** — open the selected backlink in the editor
- **Ctrl+N / Ctrl+R** — sort backlinks by name or title, toggle sort order
- **Esc** — return focus to the editor

The panel loads backlinks when toggled on and refreshes automatically when you switch notes. Panel visibility is remembered for the session.

### Following Links

When the cursor is inside a link in the editor, **`Ctrl+G`** follows it:

- **Wikilink (`[[note name]]`)** — opens the matching note directly, or shows a picker if multiple notes match
- **Markdown link (`[text](path)`)** — opens the linked note; fragment suffixes (e.g. `#section`) are ignored during lookup
- **URL (`https://...`)** — opens the URL in your default browser
- **Image link (`![alt](path)`)** — opens the image file with the OS default image viewer. Relative paths resolve against the current note's directory; absolute vault paths (e.g. `/assets/foo.png`) resolve from the workspace root
- **Hashtag label (`#tag`)** — hashtag tokens are highlighted in the editor. Pressing `Ctrl+G` while the cursor is on a hashtag opens the search modal pre-filled with that label filter (equivalent to typing `#tag` in search).

### Wikilink and Hashtag Autocomplete

The editor and the search modal both pop up a floating suggestion list when you start a wikilink or a hashtag.

**Triggers:**

- Typing `[[` in the editor opens a popup listing every note in the vault, ordered alphabetically by name. As you keep typing, the list filters by prefix against the note name (the wikilink target — the filename without extension, not the full path). The note's path is shown right-aligned and dimmed so you can disambiguate notes that share a name.
- Typing `#` mid-line in the editor or anywhere in the search box opens a popup listing existing tags, ordered by usage. Filtering works the same way.

**Header disambiguation:**

A `#` at the start of a line is *not* an autocomplete trigger by default — it might be the beginning of a Markdown heading (`# Heading`). The popup opens only after you type the next character, and only if that next character is **not** a space:

- `# Heading` — no popup (heading syntax)
- `#project` — popup opens with prefix `p`

**Key bindings (while the popup is open):**

| Key | Action |
|-----|--------|
| `↑` / `↓` | Move the highlighted suggestion |
| `PageUp` / `PageDown` | Jump by a page |
| `Home` / `End` | Jump to first / last suggestion |
| `Tab` or `Enter` | Accept the highlighted suggestion |
| `Esc` | Dismiss the popup without changing your text |

For wikilinks, accepting a suggestion inserts the note name and automatically closes the `]]` brackets (or preserves them if they already exist), placing the cursor right after the closing brackets.

The popup is non-blocking: you can ignore it and keep typing — it disappears as soon as the trigger context is broken (whitespace, newline, or cursor movement out of range). It also stays out of code spans, fenced blocks, frontmatter, and Markdown link bodies, so `#section` inside `https://example.com#section` does not pop up suggestions.

The popup caps its visible rows (default 8). If more suggestions match, a directional `▲ N more` / `▼ N more` indicator shows that scrolling will reveal them; the popup never grows past its cap regardless of available screen space.

In the search box, the same hashtag autocomplete works after the exclusion prefix `-`: typing `-#proj` and accepting a suggestion preserves the leading `-` so the search still excludes that tag.

> **Note**: Autocomplete is available in the **textarea** editor backend. Users on the embedded Neovim backend should rely on their existing Neovim completion plugins.

### Text Formatting

While the cursor is in the editor, format shortcuts wrap the current selection (or insert empty markers at the cursor when no selection is active):

| Action | Default Binding | Effect |
|--------|-----------------|--------|
| Bold | `Ctrl+B` | wraps selection in `**…**` |
| Italic | `Ctrl+I` | wraps selection in `*…*` |
| Strikethrough | `Ctrl+S` | wraps selection in `~~…~~` |

Examples:

- Selecting `important` and pressing `Ctrl+B` produces `**important**`.
- Pressing `Ctrl+I` on an empty cursor inserts `**` and places the cursor between the markers.

### Pasting Content

The editor adapts paste behaviour to the clipboard contents. Both **`Ctrl+V`** and the terminal's native paste shortcut are supported (on macOS this is `Cmd+V`; the TUI receives it through bracketed paste).

**Plain text** — inserted at the cursor, replacing any active selection.

**URL over selection** — if the clipboard holds an `http`, `https`, `ftp`, `ftps`, or `mailto` URL **and** there is an active selection, the selection is wrapped as a markdown link instead of being replaced:

- Select `Nico`, copy `https://nico.red` to the clipboard, paste → produces `[Nico](https://nico.red)`.

**Image** — if the clipboard contains image bytes (e.g. a screenshot), the image is saved as a PNG under the workspace's `/assets/` directory and a markdown image link is inserted at the cursor, relative to the current note. Generated filenames are time-stamped (e.g. `image_<unix_nanos>.png`) so multiple pastes do not collide.

The inserted image link renders as a placeholder (`[image_<…>.png]`) in the editor for readability — press `Ctrl+G` over the placeholder to open the image with your OS default viewer.

## Key Bindings

Default bindings (all configurable via the [Configuration Reference](@/getting-started/configuration.md)):

| Action | Default Binding |
|--------|-----------------|
| Quit | `Ctrl+Q` |
| Settings | `Ctrl+P` |
| Search notes | `Ctrl+K` |
| Open note (fuzzy finder) | `Ctrl+O` |
| Toggle note browser | `Ctrl+F` |
| Toggle preview | `Ctrl+Y` |
| New journal entry | `Ctrl+J` |
| Quick note | `Ctrl+W` |
| Toggle backlinks panel | `Ctrl+E` |
| Switch workspace | `F4` |
| Toggle sidebar | `Ctrl+T` |
| Bold | `Ctrl+B` |
| Italic | `Ctrl+I` |
| Strikethrough | `Ctrl+S` |
| Focus right (Sidebar → Editor → Backlinks) | `Ctrl+L` |
| Focus left (Backlinks → Editor → Sidebar) | `Ctrl+H` |
| Cycle sort field (name/title) | `Ctrl+N` |
| Reverse sort order | `Ctrl+R` |
| Follow link under cursor | `Ctrl+G` |
| File operations (rename/move/delete) | `F2` |

### Context-Sensitive Bindings

**`Ctrl+G` — Follow link (editor only):**

`Ctrl+G` is only active when the editor pane has focus and the cursor is positioned inside a link. If the cursor is not on a link, the key press is ignored.

## Customizing Key Bindings

All key bindings are fully customizable through your config file. See the [Configuration Reference](@/getting-started/configuration.md) for instructions on how to rebind actions and create custom keyboard shortcuts.
