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
- **`Ctrl+B`** — Toggle the sidebar (context-sensitive; see Key Bindings below)

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
| Toggle sidebar / Bold (context-sensitive) | `Ctrl+B` |
| Italic | `Ctrl+I` |
| Strikethrough | `Ctrl+S` |
| Toggle header | `Ctrl+T` |
| Focus right (Sidebar → Editor → Backlinks) | `Ctrl+L` |
| Focus left (Backlinks → Editor → Sidebar) | `Ctrl+H` |
| Cycle sort field (name/title) | `Ctrl+N` |
| Reverse sort order | `Ctrl+R` |
| Follow link under cursor | `Ctrl+G` |
| File operations (rename/move/delete) | `F2` |

### Context-Sensitive Bindings

**`Ctrl+B` — Toggle sidebar / Bold:**

- When focus is on the **file browser/sidebar:** Toggles the sidebar's visibility
- When the cursor is **inside the editor pane:** Applies or removes **bold** formatting to the selected text or word

This dual purpose allows efficient use of keyboard space while maintaining logical behavior based on context.

**`Ctrl+G` — Follow link (editor only):**

`Ctrl+G` is only active when the editor pane has focus and the cursor is positioned inside a link. If the cursor is not on a link, the key press is ignored.

## Customizing Key Bindings

All key bindings are fully customizable through your config file. See the [Configuration Reference](@/getting-started/configuration.md) for instructions on how to rebind actions and create custom keyboard shortcuts.
