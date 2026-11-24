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
- The image can be in your vault or on the web. A web image does not load until you ask. See [Remote images](#remote-images).
- Kimün shows PNG, JPEG, GIF, WebP, and SVG files. A GIF shows its first frame only.
- An image shows at its natural size. It is never wider than the text. It is never taller than 60% of the height of the pane.
- If the image is missing or Kimün cannot read it, a short note shows in its place.
- Point at an image to see its alt text.
- Click an image to put the cursor on its line. Right-click it to open the row menu of the image file. The menu has the items for an attachment, with Reveal in file browser.
- If you rename or move an image, Kimün does not update the notes that use it. Change the path in those notes yourself.

Images also show in other views. They use the same sizes and formats as in the note.

- The facet view shows an image in place of its line. A chunk never shows the raw line.
- The search preview and the Ask source reader keep the raw text of the note. An image line becomes the picture. If the line holds a search match, the raw line shows first and the picture shows below it. In the source reader, the marked section plays the part of the match.
- Only the first 100 images of a note show in a preview. A facet chunk also shows only its first 100 images. A preview reads only the first 256 KB of a note.
- Point at a picture to see its alt text. There is no click and no menu on a picture in these views.

### Remote images

A note can embed an image from the web with `![alt](https://host/path)`. A remote image can tell its server that you opened the note. For this reason Kimün loads nothing until you ask.

Each remote image first shows a placeholder. The placeholder names the host and has two actions.

- **Load** fetches this one image.
- **Always load from** fetches this image and allows the host. An allowed host is a host whose images load on their own.

The placeholder looks the same in the note, the facet view, the search preview, and the Ask source reader. You can use both actions in all four places.

How it works:

- An allowed host is one exact name. `cdn.example.com` does not allow `example.com` or `tracker.example.com`.
- An image on an allowed host loads on its own once it is on screen. In the note, the image on the line that you edit waits until you leave that line.
- Kimün saves each loaded image in a cache in its own folder, never in your vault. A saved image shows with no network request, so it also shows offline. A changed image keeps its old saved copy until you reload it.
- The cache can hold 200 MB. When it is full, the oldest copies go first.
- A redirect to a host that is not allowed stops and names the new host. **Load** there lets that one address follow the redirect until you close the app. **Always load from** allows the new host.
- Kimün never loads an `http` address. The note says "Not loaded: http is not secure".
- A remote address can point at a device on your own network. Click **Load** only for addresses that you trust.
- Kimün never checks where a host name points, so it also loads from an allowed host whose name points at an address in your own network.
- Kimün checks the certificate of the server with the certificate store of your system. On a network that inspects HTTPS traffic, remote images load when the certificate of that network is installed in your system. The `SSL_CERT_FILE` environment variable replaces the system store with the certificates in that file.
- Kimün draws PNG, JPEG, GIF (first frame), and WebP. It does not draw an SVG or any other format.
- An image can be at most 20 MB and at most 64 megapixels. A load stops after 15 seconds without data, and after 60 seconds in total.
- Kimün loads at most 2 images at a time from one host. If a server answers that it is busy or gets too many requests, Kimün waits a few seconds and tries once more.
- If a load fails, the placeholder gives a short reason and a Retry action. Kimün does not save the failure.

In the note, right-click a loaded remote image to open its menu.

- **Open in browser** opens the address in your default browser.
- **Copy image address** copies the address.
- **Reload** fetches the image again. The new image replaces the saved copy. If the fetch fails, Kimün keeps the old saved copy and shows it.
- **Clear image cache** removes the saved copy of this image only. On an allowed host, the image loads again at once.

Double-click a remote image, or press `Ctrl+Enter` on its line, to open the address in your browser. The facet view and the preview show a loaded image with its tooltip only.

If a saved copy is damaged, the note shows a failure note and the image has no menu. Use **Clear cached images** in Preferences to remove it.

Open Preferences and choose Remote images to manage this feature.

- The list shows each allowed host in alphabetical order. Click a host to select it. Or press `Tab` to move to the list, then press Up and Down. **Remove** (or the Delete key) takes the selected host off the list and asks nothing. The saved copies of that host stay, and **Always load from** allows the host again.
- **Clear cached images** removes every saved copy. The cache size shows next to it. Every address starts again from its placeholder. Images on allowed hosts load again once they are on screen.

### Images in the attachment view

When you open an image file, the image shows in the attachment view, below the header. This view is different from a note. The image can use the whole area.

- The image shows at its natural size. It is never larger. If it does not fit, Kimün scales it down to fit the whole area, both width and height. It stays in the center and there is no scrolling.
- The image fits again when you resize the window or the panel.
- A GIF shows its first frame only. A photo that was rotated by the camera shows upright.
- An SVG file shows as a picture. Its text does not show. Use Open externally to read the text.
- If Kimün cannot draw the file, the view shows "No preview" and gives the reason. The file may be missing, over 20 MB, or damaged.
- Other binary files, such as a zip or a PDF, show "No preview" and "Open it externally to view it." Text files still show their text.
- The size and date in the header do not update if the file changes while the view is open. Open the file again to refresh them.

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

You can change a shortcut in [Preferences](#keyboard-shortcuts). You can also edit the file by hand.

Add a `[gui.keys]` section to `config.toml`. Write the action name, then the new shortcut. An empty list removes a shortcut.

```toml
[gui.keys]
SaveNote = "primary+s"
OpenFileBrowser = ["primary+e", "f7"]
```

`primary` means `Ctrl`, or `Cmd` on macOS. The terminal app does not use this list.

These are the same lines as in a [keymap file](#export-and-import), but under `[gui.keys]` and not `[keys]`. Kimün skips a line it cannot load and writes the reason to its log file. It shows no message.

## Preferences

Press `F4` or `Ctrl+,`. The list on the left has these sections:

- Workspaces: add, rename, or remove a workspace.
- Appearance: pick the theme.
- Display: the editor font and size, reduce motion, the rail position, and the update check.
- Keyboard: change the shortcuts. See [Keyboard shortcuts](#keyboard-shortcuts).
- Sorting, Indexing, Editor, and Server: the other settings.

![The Display section of Preferences](/img/gui-preferences.png)

Click `Save` or press `Ctrl+S` to keep your changes. If you changed nothing, the button next to it reads `Close`. Once you change something, it reads `Cancel` and asks before it throws your changes away.

Reduce motion stops the bouncing logo and the loading spinners. The default, System, follows the animation setting of your operating system. Choose Reduce or Allow to override it.

If the title bar looks wrong on your desktop, add `window_decorations = "system"` to the `[gui]` section of `config.toml`. Restart the app to apply it.

### The folder chooser

The folder chooser is a panel where you pick a folder on your computer. The setup assistant opens it. Preferences > Workspaces opens it with `Choose folder…`. In Preferences > Keyboard, `Choose folder…` for Export opens it too. `Choose file…` for Import opens the same folder chooser to pick a keymap file. This page calls it the Import chooser then.

The folder chooser lists the folders, and in the Import chooser the keymap files (`.toml`), inside the folder it shows.

The Location field is the text box at the top of the folder chooser. The cursor starts there.

To narrow the list, type part of a name in the Location field. The list keeps the names that match. It uses a fuzzy match, best first.

To go to another folder, type or paste its path in the Location field. Then press `Enter`. A path starts with `~`, is a full path, or has a path separator in it. The text `..` goes up one folder. A relative path starts from the folder that the folder chooser shows.

If you type a path and then click `Choose this folder` or press `Ctrl+Enter`, the folder chooser chooses that folder.

In the Import chooser, a path to a keymap file picks that file. A path to another kind of file shows `That is not a .toml file`. When you pick a folder, a path to a file shows `That is a file, not a folder`. A path that does not exist shows `That folder does not exist`. The message goes away at your next key.

The list grows and shrinks with the height of the window.

These keys work in the folder chooser:

- `Up` and `Down` move the selection. They do nothing while the Location field holds a path.
- `Enter` opens the selected folder, or goes to the path you typed. In the Import chooser, `Enter` on a selected file picks it.
- `Esc` clears the text in the Location field. When the field is empty, `Esc` closes the folder chooser. If the new folder field is open, `Esc` closes only that field.
- `Alt+Up` goes up one folder. On macOS, press `Cmd+Up`.
- `Backspace` in an empty Location field also goes up one folder.
- `Ctrl+Enter` chooses the path you typed, or else the folder that the folder chooser shows. On macOS, press `Cmd+Enter`. The Import chooser has no `Ctrl+Enter` and no `Ctrl+N`.
- `Ctrl+N` makes a new folder inside the folder that the folder chooser shows. On macOS, press `Cmd+N`.

### Keyboard shortcuts

Open Preferences and choose Keyboard. The list shows every action in groups, such as Notes, Editor, and Tabs. Each row shows the action and its shortcut. A row with no shortcut shows `none`. The leader key is one of the rows, named Leader.

Some shortcuts work only in one place. The row says where, for example "editor only" or "search browser only".

To find an action, type in the `Filter shortcuts` field above the list. Type part of its name, or its keys, for example `Ctrl+E` (`Cmd+E` on macOS). Press `Esc` to empty the field.

Each row has these buttons:

- `Change` sets a new shortcut.
- `Clear` removes the shortcut. The row then shows `none`.
- `Reset` gives the row its default shortcut back. It shows only on a row you changed.

To change a shortcut:

1. Click `Change` on the row.
2. Press the new keys.

The row then shows the new shortcut. While the row waits for keys, its `Change` button reads `Cancel`. Press `Esc` or click `Cancel` to keep the old shortcut. `Change` replaces all the shortcuts of the row with this one.

The section refuses these keys and tells you why.

- A plain letter, number, or symbol. It types text. Add `Ctrl`, `Alt`, or `Cmd` to use it.
- `Enter`, `Tab`, `Backspace`, and other editing keys with no `Ctrl`, `Alt`, or `Cmd`. `Shift` alone does not count. Add one of the three.
- The `+` key, with or without other keys. No other keys fix this. Pick another key.

A function key such as `F6` works alone.

On macOS, `Cmd+Q`, `Cmd+W`, `Cmd+H`, and `Cmd+M` act at once, even while a row waits for keys. You cannot set them as shortcuts.

On macOS, `Option` with a letter can type a special character. A shortcut on such a key can then fail in the editor.

What a row can tell you after a change:

- "{shortcut} was {action}. Saving clears it there." Another action had the shortcut. It moves to this row. The other row shows the same warning.
- "In the editor, this key then runs this shortcut and no longer edits text." The editor uses this key, for example to undo or to copy. The shortcut now takes it.
- "In the editor with vim mode, vim reads this key first." With vim mode on, the editor handles this key before Kimün does.
- A line such as "Where a query is typed, F7 is Query builder." Two actions can share a shortcut when they work in different places. The line says which one wins where.

Nothing is kept until you click `Save`. Click `Cancel` to discard all the changes. Saved changes work at once and need no restart.

`Reset all` puts every shortcut back to its default.

If no shortcut opens Preferences anymore, click CFG in the rail. Then click `Preferences…` in the CFG view. In the Keyboard section, click `Reset all`, then `Save`. You can also remove the `[gui.keys]` table from `config.toml`. Every shortcut then goes back to its default.

### Export and import

Below the list, the Keymap file area has two parts. You can save your changes to a file and load them from a file. This helps you move your shortcuts to another computer.

To export:

1. Check the path in the Export field. It starts as `kimun-keys.toml` in your home folder.
2. Click `Export`.

The path must be a full path or start with `~`. Click `Choose folder…` to pick the folder with the [folder chooser](#the-folder-chooser). If the folder in the path does not exist, Kimün creates it. Export replaces a file with the same name and does not ask first. The file holds only the shortcuts you changed.

To import:

1. Type the path of the file in the Import field, or click `Choose file…`.
2. Click `Import`.

The path must be a full path or start with `~`. The `Choose file…` button opens the same chooser. Import fills the list with the shortcuts of the file. It replaces your unsaved changes. Click `Save` to keep them. Click `Cancel` to throw them away.

Kimün shows how many changes it loaded. It lists each line or shortcut it skipped and gives the reason. Import never takes a shortcut from another action. If a line asks for a shortcut that another action already has, Kimün skips that shortcut and tells you why. A default shortcut of another action counts too. If two lines ask for the same shortcut, only one of them gets it. A line replaces all the default shortcuts of its action. So if Kimün skips the only shortcut of a line, that action has no shortcut at all. To prevent this, clear the action that has the shortcut in the same file, for example with `OpenFileBrowser = []`.

A keymap file looks like this:

```toml
[keys]
SaveNote = "primary+s"
OpenFileBrowser = ["primary+e", "f7"]
NewTab = []
```

- `primary` means `Cmd` on macOS and `Ctrl` on other systems. A file with `primary` works on every system.
- An empty list, as in `NewTab = []`, means no shortcut.
- An action that is not in the file keeps its default shortcut.

## Updates

The app checks for a new version when it starts. If you installed with the install script, click `Update now` in the message. Your open note is saved first. When the update is installed, click `Restart now` to start the new version, or `Later` to keep working. For other installs, the message shows how to update.

To turn off the check, clear `Check for updates on startup` in Preferences, in the Display section. Click `Check now` to look right away.

## Not available yet

These features are planned but not built in either app: notes from templates, a graph view, git sync, and export.
