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

- Rail: The strip at the edge lists the views: FILES, FIND, TAGS, LINKS, OUTLINE. When a [Kimün server](@/using-kimun/server.md) is connected, it also lists SEM and ASK. Click a view to show it.
- Slots: Views live in slots on either side of the editor. To move a view, drag it by its slot header to another slot. To give it a new slot, drag it onto an edge of the window. The places where it can land light up while you drag. Drag the dividers to resize the slots.
- Zoom: `Ctrl+Shift+M` (`Cmd+Shift+M` on macOS) zooms the focused view to the whole window. The same chord restores it.
- Presets: When you leave Kimün, it saves the layout. To start from a layout that you write yourself, put it in a `layouts.toml` next to `config.toml`. Then apply it from the command palette (`Apply layout preset`).
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
| `editor_font_family` | the name of an installed font family | Sets the font of the editor. If you omit the key, the editor uses the monospace font of your theme. An empty value counts as missing. If the family is not installed, the editor uses the theme font, and the file keeps your value. |
| `editor_font_size` | a number from `8` to `32` | Sets the size of the editor text in points. If you omit the key, the editor uses the size of your theme. A number outside the range becomes the nearest limit. A value of the wrong type counts as missing. |

`[gui.layout]` is the saved layout. Leave it to the app.

## Fonts

The interface uses [Inter](https://rsms.me/inter/). The app embeds the font, and the SIL Open Font License covers it. The editor uses the monospace font of your theme, unless you choose another font.

To choose the editor font, open Preferences > Display and use the Font row. The dropdown lists only the monospace families that the app finds on your machine. You can type to search it. Use the size stepper to set a size from 8 to 32 points. The font applies to the editor only. It takes effect when you save in Preferences.

If the family has no bold face, the editor keeps your family and draws the bold text synthetically. If the family is not installed, the editor uses the theme font. The dropdown then shows the name with "(not installed)".

## Updates

When the desktop app starts, it looks for a new Kimün release and tells you in a toast. To turn this off, use `update_check` in `config.toml`. To look for a release at any time, use Preferences > Display > Check now, or leader `v u`.

The `Install update` button of the update dialog downloads the release binary of the desktop app (`kimun-gui-<version>-<platform>`). It makes sure that the checksum is correct, and then it replaces the running binary. Restart the app to use the new version. The dialog offers this only for a copy that the install script installed or that you placed yourself. In these cases, the dialog shows you the correct way to update instead: Homebrew, `cargo install`, a system package, and the macOS `.app` from the `.dmg`. If a release has no desktop build for your system, the dialog says so.

If a [Kimün server](@/using-kimun/server.md) reports a newer release of itself, you get a toast. Update the server the same way that you installed it.

## Not in the desktop app yet

The leader tree lists these items, but neither the desktop app nor the terminal app has them yet:

- note from a template
- local graph
- git sync, log and diff (both apps show the git summary only)
- two editors side by side (slots hold views, not more editors)
- toggle a todo item
- preview, and export
