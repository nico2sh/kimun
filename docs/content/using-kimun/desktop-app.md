+++
title = "Desktop app"
weight = 16
+++

# Desktop app

Kimün also has a desktop app, `kimun-gui`. It is a native window. It uses the same notes, the same `config.toml`, the same themes, and the same [search](@/using-kimun/search.md) as the [terminal UI](@/using-kimun/tui.md). You can use both apps on one workspace.

> Status: The desktop app is new. The developer builds and tests it on Linux. The Windows and macOS builds come from the same code, but nobody verified them on those systems yet. Expect rough edges. See [what is not there yet](#not-in-the-desktop-app-yet).

## Install and run

On macOS and Linux, the install script installs the desktop app together with the terminal app:

```sh
curl -fsSL https://kimun.2co.dev/install.sh | sh
```

To install only the desktop app, add `-s -- --gui` after `sh`. On a Linux machine with no display, the script skips the desktop app unless you use `--gui`.

On Linux, the script also adds the app to your application menu. It creates a launcher entry and icons under `~/.local/share`. On macOS, the script installs the signed `Kimun.app` in `~/Applications`. Open it from Launchpad or Spotlight. You can also download the signed `kimun-gui-<version>-<platform>.dmg` from the [releases page](https://github.com/nico2sh/kimun/releases) and drag Kimün to Applications.

On Linux, the script installs into `~/.local/bin`. To change the folder, set `KIMUN_INSTALL_DIR`. It also leaves an install marker, so the app can update itself. On macOS, set `KIMUN_APP_DIR` to install `Kimun.app` in another folder. The app does not update itself in place there. To update it, run the script again. On both systems, the script makes sure that the checksum of the download is correct. The app can sit beside the terminal app.

On Windows, the PowerShell installer installs both apps too. To install only the desktop app, run it with `-Gui`:

```powershell
& ([scriptblock]::Create((irm https://kimun.2co.dev/install.ps1))) -Gui
```

The installer needs no admin rights. It installs into `%LOCALAPPDATA%\Programs\kimun`, adds a Start-menu shortcut, and adds the folder to your user `PATH`. To use the new `PATH`, open a new terminal. Or download `kimun-gui-<version>-windows-x64.zip` from the [releases page](https://github.com/nico2sh/kimun/releases), unzip it, and run `kimun-gui.exe`.

To build from source, run these commands in a checkout of the repository:

```sh
cargo build --release -p kimun-gui
./target/release/kimun-gui
```

On Linux, you first need the development libraries for a native window. On Debian and Ubuntu, install them with this command:

```sh
sudo apt-get install pkg-config libwayland-dev libfontconfig-dev \
  libfreetype-dev libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev
```

To open a note, give its path on the command line (`kimun-gui path/to/note.md`). To start where you left off, run the app with no argument. If no workspace exists in the configuration, a short setup assistant guides you the first time.

## The window

- Rail: The strip at the edge lists the views: FILES, FIND, TAGS, LINKS, OUTLINE. When a [Kimün server](@/using-kimun/server.md) is connected, it also lists SEM and ASK. Click a view to show it. The rail starts at the left. To move it to the right, top, or bottom edge, choose `Rail position` in Preferences, or one of the `Rail position` rows of the command palette. To hide or show the rail, press `Ctrl+Shift+A` (`Cmd+Shift+A` on macOS), or click the small panel button in the title bar, after the workspace name. When the window has no Kimün title bar, the button is at the start of the footer. Kimün keeps the position and the hidden state for the next start.
- Slots: Views live in slots on either side of the editor. To move a view, drag it by its slot header to another slot. To give it a new slot, drag it onto an edge of the window. The places where it can land light up while you drag. Drag the dividers to resize the slots.
- Side toggles: `Alt+L` shows or hides the left side, and `Alt+R` the right side. The two buttons near the end of the rail do the same. `Alt+E` and `Alt+F` move the keyboard to the next and the previous panel. On macOS, Option and a letter types a character, so these chords are `Cmd+Option+L`, `Cmd+Option+R`, `Cmd+Option+E` and `Cmd+Option+S`.
- Tabs: `Ctrl+T` (`Cmd+T` on macOS) opens a new tab with an empty editor. Open a note in it with `Ctrl+O`. With two tabs or more, a tab strip shows above the editor. Each tab shows the name of its note, and a dot when the tab has unsaved text. Click a tab to show it, or press `Ctrl+Tab` and `Ctrl+Shift+Tab` for the next and the previous tab. `Ctrl+W` (`Cmd+W` on macOS), or the small cross on a tab, closes the tab. Kimün saves the text of the tab first. When you close the last tab, the editor stays, with no note open. In vim mode, `Ctrl+W` closes the tab only in normal mode. In insert mode the key goes to the editor. In normal mode, `Ctrl+W` does not close the tab while a vim command is half typed (for example after `d`). Press `Esc` first.
- Splits: A tab can show several notes side by side. With the editor focused, `Alt+Shift+R` opens an empty editor to the right of the current one, and `Alt+Shift+D` opens one below it (`Cmd+Option+Shift+R` and `Cmd+Option+Shift+D` on macOS). Open a note in it with `Ctrl+O`. When a tab shows two editors or more, each editor has a header with the name of its note, a dot when its text is unsaved, and a small cross that closes it. `Alt+Shift+W` (`Cmd+Option+Shift+W` on macOS) closes the current editor. Kimün saves its text first. When you close the last editor of a tab, the tab closes. A note is open in one editor only: when you open a note that another editor shows, Kimün moves the keyboard to that editor. When a view has the keyboard, a bar in the accent color marks the editor that a note you open from FILES replaces. Drag the line between two editors to resize them. To move an editor, drag it by its header. Drop it on an edge of another editor to place it on that side, or on the middle of another editor to swap the two. While you drag, the tab bar shows: drop it on a tab to move it into that tab, or on the plus button to move it into a new tab. Its unsaved text moves with it. Press `Esc` to cancel the drag. On a view, the same chords split or close its slot.
- Note menu: One small menu acts on an editor and its note. There are three ways to open it. Right-click in an editor. Click the small `⋯` button, which sits in the header of each editor when a tab shows two editors or more, and dimmed at the top right corner of a lone editor. Or press `F2`, which opens the menu under the top right corner of the current editor. A right-click makes that editor the current one first. The menu acts on that editor. The first items work on the note: Properties, Rename, Move and Delete. Each has a letter, so `P`, `R`, `M` or `D` picks it. A line follows, then the layout items: Split right, Split down, New tab, Close group and Close tab. They show their chords. The menu hides an item that cannot act. An empty editor shows only the layout items. Close group needs two editors or more in the tab. Close tab needs two tabs or more. `Esc`, a click outside the menu, or any chord other than `F2` closes it. `F2` again also closes it. The file list keeps its own row menu for Rename, Move and Delete.
- Your tabs come back: Kimün remembers the tabs of each workspace, the editors in each tab, the note in each editor and their sizes. When you start Kimün again, or switch back to the workspace, they open as you left them. If a note was deleted or moved outside Kimün, its editor opens empty and Kimün tells you which note is gone. When no note of the last session is left, Kimün opens the last note you used, as before. When you start Kimün with a note on the command line, only that note opens. The saved tabs stay saved when you open other notes in that editor. They change only when you add, close or move a tab or an editor. If Kimün cannot read the saved tabs (for example, a newer version of Kimün wrote them), it keeps them as they are until you open another note or change the tabs or the editors. This record stays on your computer and does not sync with the notes.
- Zoom: `Ctrl+Shift+M` (`Cmd+Shift+M` on macOS) zooms the focused view to the whole window. The same chord restores it. On a tab with several editors, zoom shows the current editor only. The focus chords still move to the other editors of the tab, and the zoom then shows the editor they reach. Another tab ends the zoom.
- Presets: When you leave Kimün, it saves the layout. To start from a layout that you write yourself, put it in a `layouts.toml` next to `config.toml`. Then apply it from the command palette (`Apply layout preset`). A preset can also set the rail position. Add `rail = "right"` (or `"left"`, `"top"`, `"bottom"`) next to the `split` of the preset. A preset without a `rail` key leaves the rail where it is.
- Closing (macOS): The red button closes the window, and Kimün keeps running in the Dock. To open a window again, click the Dock icon, or choose File > New Window. These actions end the app: `Cmd+Q`, Kimün > Quit, `Quit` in the command palette, and the quit command of the leader. On Linux and Windows, closing the window ends the app.
- Footer: The footer shows the path of the note, the caret position, the backlink count, the property count, and the saved or unsaved state. When the workspace is a git repository, it also shows a git summary. The summary reads `git ✓` for a clean repository and `git ●3` for three changed paths. Leader `g s` (see [Keyboard](#keyboard)) shows the full summary.

The editor is the same as the editor of the terminal app. It has plain mode or [vim](@/using-kimun/vim-mode.md) mode, wikilinks, `#hashtags`, and a find bar. To follow a wikilink, double-click it.

### Completion

When you type `[[` in a note, Kimün offers the notes of the vault. When you type `#`, it offers the tags. In FIND, in the search browser (`Ctrl+K`), and in the note finder, these rules apply:

- `#` completes tags.
- `<`, `>`, and `=` complete note names in link filters.
- A leading `?` completes the name of a saved search and expands to its query.

`Up` and `Down` move through the list. `Tab` or `Enter` accepts an item. `Esc` closes the list.

## Workspaces

A button in the title bar shows which workspace is open. To open the workspace switcher, click the button, or press `F5` (or leader `v s`). With system window decorations, or in fullscreen, the button moves to the left end of the footer. When you switch, Kimün first saves the open note. You manage workspaces in Preferences > Workspaces. They are the same workspaces that the terminal app uses.

## Preferences

`F4` (or `Ctrl+,`) opens Preferences. Preferences saves to `config.toml`, the file that the terminal app also reads (see [Configuration](@/getting-started/configuration.md)). It also saves the display options of the desktop app, such as Reduce motion. Leader `v t` opens the theme picker.

## Keyboard

Press `F1` for the keyboard reference. It lists every command with its current chord, including your own overrides. These chords are the most useful (`Ctrl` is `Cmd` on macOS):

| Chord | Does |
|---|---|
| `Ctrl+Shift+B` | the leader. A short sequence after it reaches any command, as in the terminal app (`Ctrl+Shift+B f f` opens the note finder) |
| `Ctrl+Shift+P` | command palette |
| `Ctrl+K` | search browser |
| `Ctrl+S` | save |
| `F5` | switch workspace |
| `F4` | Preferences |
| `Ctrl+Shift+A` | hide or show the rail |
| `F2` | note menu |
| `Ctrl+T` / `Ctrl+W` | new tab / close tab |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | next tab / previous tab (`Ctrl` on macOS too) |
| `Alt+Shift+R` / `Alt+Shift+D` / `Alt+Shift+W` | split the editor (or the focused slot) to the right / down, close it (`Cmd+Option+Shift+R`, `D`, `W` on macOS) |
| `Alt+L` / `Alt+R` | show or hide the left side / the right side (`Cmd+Option+L` / `Cmd+Option+R` on macOS) |
| `Alt+E` / `Alt+F` | next panel / previous panel (`Cmd+Option+E` / `Cmd+Option+S` on macOS) |

To override a chord, use the `[gui.keys]` table of `config.toml`. The key is the name of the action. The value is one chord or a list of chords. An empty list removes the binding. A chord joins the modifiers and one key with `+`. `primary` is `Ctrl` (`Cmd` on macOS). The other modifiers are `cmd`, `ctrl`, `alt`, and `shift`.

```toml
[gui.keys]
SaveNote = "primary+s"
OpenFileBrowser = ["primary+e", "f7"]
OpenQueryInSidebar = []
```

The app skips a chord or an action that it does not know, and it writes the problem to the log. The other overrides still apply. The desktop app has its own key table. The `[key_bindings]` of the terminal app do not apply to it.

## Display options in `config.toml`

All keys are under `[gui]`. Preferences writes them for you.

| Key | Values | Meaning |
|---|---|---|
| `window_decorations` | `"client"` (default), `"system"` | Sets which program draws the title bar. `"client"` is the title bar of Kimün, in the colors of the theme. `"system"` is the title bar of the operating system. If the custom bar fails on your desktop, use this value. The app reads this key at start, so restart the app to change it. macOS always keeps its native window buttons. |
| `reduce_motion` | `true`, `false` (default) | Stops decorative motion, such as the bouncing dots of the welcome screen and the spinners. The app does not read the setting of the operating system. |
| `layout_preset` | the name of a preset | The preset that the saved layout started from. |
| `editor_font_family` | the name of an installed font family | Sets the font of the editor. A font family is the name of a typeface, like Menlo. The name must match an installed name. Capital letters do not matter. If you omit the key, the editor uses the monospace font of your theme. In a monospace font, all characters have the same width. An empty value counts as missing. If the family is not installed, the editor uses the theme font, and the file keeps your value. |
| `editor_font_size` | a whole number from `8` to `32` | Sets the size of the editor text in pixels. If you omit the key, the editor uses the size of your theme. A number outside the range becomes the nearest limit. A value of the wrong type counts as missing. |
| `rail` | `"left"` (default), `"right"`, `"top"`, `"bottom"` | The window edge where the rail sits. |
| `rail_hidden` | `true`, `false` (default) | Hides the rail. The button in the title bar (in the footer when there is no title bar) and `Ctrl+Shift+A` show it again. |

`[gui.layout]` is the saved layout. Leave it to the app.

## Fonts

The interface uses [Inter](https://rsms.me/inter/). The app embeds the font, and the SIL Open Font License covers it. The editor uses the monospace font of your theme, unless you choose another font.

To choose the editor font, open Preferences > Display and use the Editor font group. The Family dropdown lists only the monospace families that the app finds on your machine. While the app reads them, it shows "Loading installed fonts..." below the dropdown. You can type to search the dropdown. Use the Size stepper to set a size from 8 to 32 pixels. The font applies to the editor only. It takes effect when you save in Preferences.

If the family has no bold style, the editor keeps your family. It then draws each bold character twice, with a small offset. If the family is not installed, the editor uses the theme font. The dropdown then shows the name with "(not installed)".

## Updates

When the desktop app starts, it looks for a new Kimün release and tells you in a toast. To turn this off, use `update_check` in `config.toml`. To look for a release at any time, use Preferences > Display > Check now, or leader `v u`.

When Kimün can update itself, the toast has an `Update now` button. One click saves your open note, downloads the new version, makes sure that the checksum is correct, and replaces the running app. Then the app restarts by itself. The update dialog has the same action as `Install update`. Open the dialog from the palette to read the release notes or to skip a version. If an update fails, an error appears with a `Retry` button.

Kimün updates itself in these cases:

- You installed it with the install script.
- You placed the binary yourself.
- You use the macOS `Kimun.app`, and the folder that holds it is one that you can write to. The default folder of the install script, `~/Applications`, is such a folder.

For the macOS app, Kimün replaces the whole `Kimun.app`. Before it does, it makes sure that the new app has a valid Developer ID signature and that the same developer team signed it as the app that runs now. If one of these checks fails, Kimün keeps the app that you have. A development build has no developer team and never updates itself.

In all other cases, the toast has a `View` button, and the dialog shows you the correct way to update: Homebrew, `cargo install`, a system package, or the releases page. If a release has no desktop build for your system, the update fails and the message tells you to use the releases page.

If a [Kimün server](@/using-kimun/server.md) reports a newer release of itself, you get a toast. Update the server the same way that you installed it.

## Not in the desktop app yet

The leader tree lists these items, but neither the desktop app nor the terminal app has them yet:

- note from a template
- local graph
- git sync, log and diff (both apps show the git summary only)
- toggle a todo item
- preview, and export
