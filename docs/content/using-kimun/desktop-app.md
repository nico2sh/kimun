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
- Press `Alt+M` (`Cmd+Option+M` on macOS) to zoom the view that has focus. Press it again to go back. In a tab with several panes, zoom shows the current pane only. The focus keys still move to the other panes, and the zoom then shows the pane they reach.
- To hide or show the rail, press `Alt+A` (`Cmd+Option+A` on macOS). To move it to another edge, use `Rail position` in Preferences.

On macOS, use `Cmd` in place of `Ctrl`.

## Tabs and panes

A tab holds one or more panes. A pane shows one note. Press `Ctrl+T` to open a new tab with an empty pane. Open a note in it with `Ctrl+O`.

- With two tabs or more, a tab strip shows above the editor. Each tab shows the name of its note, and a dot when the tab has unsaved text.
- Click a tab to show it. Press `Ctrl+Tab` and `Ctrl+Shift+Tab` to go to the next and the previous tab.
- Press `Ctrl+W`, or click the small cross on a tab, to close the tab. Kimün saves the text first. In vim mode, `Ctrl+W` closes the tab only in normal mode.
- Press `Alt+Shift+R` to open an empty pane to the right of the current one. Press `Alt+Shift+D` to open one below it. Press `Alt+Shift+W` to close the current pane. If you close the last pane of a tab, the tab closes.
- With two panes or more in a tab, each pane has a header. It shows the note name, a dot for unsaved text, and a small cross that closes the pane.
- A note is open in one pane only. If you open a note that another pane shows, the keyboard moves to that pane.
- Press `Alt+Shift` and an arrow key to move the current pane. It swaps places with the pane on that side. If no pane is on that side, it moves to that edge of the tab and fills the whole width or height of the tab. A pane that already fills that edge stays. While a pane is zoomed, these keys do nothing and a message tells you to leave zoom first.
- On Linux and Windows, `Alt+Shift+Left` and `Alt+Shift+Right` do not select by word in the editor. Use `Ctrl+Shift+Left` and `Ctrl+Shift+Right`.
- To move a pane with the mouse, drag it by its header. Drop it on the edge of another pane to place it on that side. Drop it on the middle of another pane to swap the two. Drop it on a tab to move it into that tab. Drop it on the plus button to move it into a new tab. Press `Esc` to cancel.
- Drag the line between two panes to resize them.
- Kimün remembers the tabs, the panes, and the note in each pane for each workspace. When you start Kimün again, they open as you left them. If a note was deleted or moved outside Kimün, its pane opens empty and Kimün tells you which note is gone.

On macOS, the pane keys use `Cmd+Option+Shift` in place of `Alt+Shift`, and the tab keys use `Cmd` in place of `Ctrl`, except `Ctrl+Tab`.

## The note menu

The note menu acts on a pane and its note. To open it, right-click in a pane (not in a facet view), click the small `...` button, or press `F2`. The button sits in the header of each pane when a tab has two panes or more. In a lone pane, it sits in the top right corner, dimmed until you point at the pane. While the pane shows a facet view, it is not dimmed.

The first items work on the note: Facet view, Properties, Rename, Move, and Delete. Press `F`, `P`, `R`, `M`, or `D` to pick one. In a facet view, the first items are Edit note and Properties (`E` and `P`). Rename, Move, and Delete are hidden there. The next items change the layout: Split right, Split down, New tab, Close pane, and Close tab. They show their keys. The menu hides an item that cannot work. For example, an empty pane shows only the layout items. Press `Esc`, click outside the menu, or press `F2` again to close it.

The menu also has `Facet view`. It shows the note in a read-only view that you can narrow by heading, tag, or link. You can tick tasks in it. See [Facet view](@/using-kimun/facet-view.md). In that view, the menu has `Edit note` to go back. A pane that shows a [collection](@/using-kimun/facet-view.md#collections) has no note, so its menu has the layout items only.

The funnel button beside the `...` button does the same as `Facet view` and `Edit note`: click it to open the facet view, and click it again to go back. The command palette has the same switch as `Toggle facet view`. It has no key by default. A pane that shows a facet view, and its tab, have a small funnel before the title.

## The link menu

Right-click a link or a tag (also called a label) in a note to open the link menu. The menu depends on the target:

- A link to a note: Open, Open in new tab, Open in split right, Open in split down, and Copy link.
- A link to an attachment: Open, Open in new tab, Open in default app, and Copy link.
- A link to a web address (`http` or `https`): Open in browser and Copy link.
- A link that Kimün cannot follow, for example a `mailto:` link: Copy link only.
- A label: Search for label and Copy label.

If the note is already open in a pane, Kimün goes to that pane and does not open it again. In a [facet view](@/using-kimun/facet-view.md#click-in-the-text), the menu has no plain Open item.

If you right-click with text selected in an editor, Kimün copies the text and opens no menu. In a facet view, a right-click with text selected does nothing. A right-click on plain text opens the note menu. While the find bar is open, a right-click on a link also opens the note menu.

## The row menu

Right-click a row in FILES, FIND, LINKS, SEM, or the pinned notes to open the row menu. The items depend on the row:

- A note: Open, Open in new tab, Open in split right, and Open in split down. Then Copy path, Pin note (or Unpin note), and Reveal in file browser. Then Rename…, Move…, and Delete….
- A folder: Copy path. Then Rename…, Move…, and Delete….
- An attachment: Open, Open in new tab, and Open in default app. Then Copy path and Reveal in file browser. Then Rename…, Move…, and Delete…. An attachment has no Pin item. Only notes can be pinned.

In FILES, the menu has no Reveal in file browser item. If the note or file was moved or deleted, Open and Reveal show an error and open nothing.

## New notes and directories

- Press `Ctrl+N` to make a note. Type its name and press `Enter`. The note goes in the folder that FILES shows, or in the folder of the open note. A name with slashes, such as `projects/ideas`, makes the folders for you. Start the name with `/` to begin at the top of the vault.
- Press `Ctrl+Shift+N` to make a directory. It takes the same kind of name.
- The window shows the folder it will use. Click `Change…` to pick another one: type to filter the list, use `Up` and `Down`, and press `Enter` or click a folder. `Esc` closes the list first, then the window.
- The window shows where the entry will go, and tells you if the name is already taken or is not allowed.
- The `+` button in FILES opens a small menu with both choices. If you type a name in the FILES filter or in the note finder and nothing matches, pick `Create: …` to open the same window with that name filled in.
- You can also press `Ctrl+Shift+B n n` for a note and `Ctrl+Shift+B n d` for a directory.

## Search

Click FIND in the rail, or press `Ctrl+K`. Type a query and the results update as you type. Click a result to see more of it. In this example, `<carlos or:title` finds the notes that link to `carlos`, sorted by title.

![The Find view with a query, a list of results, and a preview](/img/gui-find.png)

To read all the results as one document, click `Open as collection` in the tool bar of FIND. See [Collections](@/using-kimun/facet-view.md#collections).

## Write a note

- Type `[[` to link to another note. Type `#` to add a tag. Press `Tab` or `Enter` to accept a suggestion.
- Double-click a link to open it. You can also hold `Ctrl` (`Cmd` on macOS) and click it.
- Press `Ctrl+F` to find text in the note.

### Images

The desktop app shows an image inside the note. Put the image on a line of its own with `![alt](path)` or with `![[name]]`. The image shows below that line. The terminal app shows no images.

- When the cursor is on the line, the line shows as plain text and the image shows below it. Move the cursor away and the line gives way to the image.
- The image must be in your vault. A web address does not work. A line with a web address stays a styled line and shows no image.
- Kimün shows PNG, JPEG, GIF, WebP, and SVG files. A GIF shows its first frame only.
- An image shows at its natural size. It is never wider than the text. It is never taller than 60% of the height of the pane.
- If the image is missing or Kimün cannot read it, a short note shows in its place.
- Point at an image to see its alt text.
- Click an image to put the cursor on its line. Right-click it to open the row menu of the image file. The menu has the items for an attachment, with Reveal in file browser.
- If you rename or move an image, Kimün does not update the notes that use it. Change the path in those notes yourself.

## Properties

Properties are facts about a note, such as `status` or `due`. The footer shows how many a note has. Click it to open the list. Use `Add`, `Edit`, and `Delete`, or press `A`, `E`, and `D`.

![The properties window of a note](/img/gui-properties.png)

## Shortcuts

Press `F1` to see every shortcut. These are the most useful:

| Shortcut | What it does |
|---|---|
| `Ctrl+K` or `Ctrl+Shift+F` | Search your notes |
| `Ctrl+Shift+P` | Command palette: find any command |
| `Ctrl+S` | Save |
| `F5` | Switch workspace |
| `F4` | Preferences |
| `Ctrl+Shift+B` | Start a command sequence. For example, `Ctrl+Shift+B f f` opens the note finder |
| `Ctrl+T` / `Ctrl+W` | New tab / close tab |
| `Ctrl+N` / `Ctrl+Shift+N` | New note / new directory |
| `Alt+Shift+R` / `Alt+Shift+D` / `Alt+Shift+W` | Split the pane right / down, close the pane |
| `Alt+Shift+Left` / `Right` / `Up` / `Down` | Move the pane (or the view that has focus) that way |
| `F2` | Note menu |
| `F7` | In FIND or the search box: build the query from a list of conditions |

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
- Display: the editor font and size, reduce motion, the rail position, and the update check.
- Sorting, Indexing, Editor, and Server: the other settings.

![The Display section of Preferences](/img/gui-preferences.png)

Click `Save` or press `Ctrl+S` to keep your changes.

Reduce motion stops the bouncing logo and the loading spinners. The default, System, follows the animation setting of your operating system. Choose Reduce or Allow to override it.

If the title bar looks wrong on your desktop, add `window_decorations = "system"` to the `[gui]` section of `config.toml`. Restart the app to apply it.

## Updates

The app checks for a new version when it starts. If you installed with the install script, click `Update now` in the message. Your open note is saved first. When the update is installed, click `Restart now` to start the new version, or `Later` to keep working. For other installs, the message shows how to update.

To turn off the check, clear `Check for updates on startup` in Preferences, in the Display section. Click `Check now` to look right away.

## Not available yet

These features are planned but not built in either app: notes from templates, a graph view, git sync, and export.
