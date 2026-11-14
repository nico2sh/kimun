+++
title = "Desktop app"
weight = 10
+++

# Desktop app

The desktop app is a normal window. It uses the same notes, settings, themes, and search as the [terminal app](@/using-kimun/tui.md). You can use both apps on one workspace.

The desktop app is new. The developer tests it on Linux. The Windows and macOS builds can have rough edges.

To start it, open Kimün from your application menu. To open one note, run `kimun-gui path/to/note.md`.

## The window

![The desktop app with several views open: Files, Find, the editor, and Ask](/img/gui-views.png)


- The rail is a strip at the edge. It lists the views: FILES, FIND, TAGS, LINKS, and OUTLINE. If you connect a [Kimün server](@/using-kimun/server.md), it also lists SEM (search by meaning) and ASK. Click a view to open it.
- Views sit in slots next to the editor. Drag a view by its slot header to move it. Drag it to an edge of the window to give it a new slot. Drag a divider to resize a slot.
- The footer shows the note path, the cursor position, the backlink count, and whether the note is saved.
- Press `Ctrl+Shift+M` to zoom the view that has focus. Press it again to go back.
- To hide or show the rail, press `Ctrl+Shift+A`. To move it to another edge, use `Rail position` in Preferences.

On macOS, use `Cmd` in place of `Ctrl`.

## Search

Click FIND in the rail, or press `Ctrl+K`. Type a query and the results update as you type. Click a result to see more of it. In this example, `<carlos or:title` finds the notes that link to `carlos`, sorted by title.

![The Find view with a query, a list of results, and a preview](/img/gui-find.png)

## Write a note

- Type `[[` to link to another note. Type `#` to add a tag. Press `Tab` or `Enter` to accept a suggestion.
- Double-click a link to open it.
- Press `Ctrl+F` to find text in the note.

## Properties

Properties are facts about a note, such as `status` or `due`. The footer shows how many a note has. Click it to open the list. Use `Add`, `Edit`, and `Delete`, or press `A`, `E`, and `D`.

![The properties window of a note](/img/gui-properties.png)

## Shortcuts

Press `F1` to see every shortcut. These are the most useful:

| Shortcut | What it does |
|---|---|
| `Ctrl+K` | Search your notes |
| `Ctrl+Shift+P` | Command palette: find any command |
| `Ctrl+S` | Save |
| `F5` | Switch workspace |
| `F4` | Preferences |
| `Ctrl+Shift+B` | Start a command sequence. For example, `Ctrl+Shift+B f f` opens the note finder |

### Change a shortcut

Add a `[gui.keys]` section to `config.toml`. Write the action name, then the new shortcut. An empty list removes a shortcut.

```toml
[gui.keys]
SaveNote = "primary+s"
OpenFileBrowser = ["primary+e", "f7"]
```

`primary` means `Ctrl`, or `Cmd` on macOS. The terminal app does not use this list.

## Preferences

Press `F4` or `Ctrl+,`. The list on the left has these sections:

- Workspaces: add, rename, or remove a workspace.
- Appearance: pick the theme.
- Display: the editor font and size, the rail position, and the update check.
- Sorting, Indexing, Editor, and Server: the other settings.

![The Display section of Preferences](/img/gui-preferences.png)

Click `Save` or press `Ctrl+S` to keep your changes.

If the title bar looks wrong on your desktop, add `window_decorations = "system"` to the `[gui]` section of `config.toml`. Restart the app to apply it.

## Updates

The app checks for a new version when it starts. If you installed with the install script, click `Update now` in the message. Your open note is saved first, and the app restarts. For other installs, the message shows how to update.

To turn off the check, clear `Check for updates on startup` in Preferences, in the Display section. Click `Check now` to look right away.

## Not available yet

These features are planned but not built in either app: notes from templates, a graph view, git sync, two editors side by side, and export.
