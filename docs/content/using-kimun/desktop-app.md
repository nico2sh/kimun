+++
title = "Desktop app"
weight = 16
+++

# Desktop app

Kimün also has a desktop app, `kimun-gui`: a native window over the same notes, the same `config.toml`, the same themes and the same [search](@/using-kimun/search.md) as the [terminal UI](@/using-kimun/tui.md). You can use both on one workspace.

> **Status.** The desktop app is new. Prebuilt binaries are not published yet, so for now you build it yourself (below). It is developed and tried on Linux; Windows and macOS use the same code but have not been verified there yet. Expect rough edges, and see [what is not there yet](#not-in-the-desktop-app-yet).

## Install and run

There is no installer or release download for the desktop app yet. Build it from a checkout of the repository:

```sh
cargo build --release -p kimun-gui
./target/release/kimun-gui
```

On Linux you need the usual development libraries for a native window first. On Debian and Ubuntu:

```sh
sudo apt-get install pkg-config libwayland-dev libfontconfig-dev \
  libfreetype-dev libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev
```

Give it a note to open on the command line (`kimun-gui path/to/note.md`) or run it bare to start where you left off. The first time, with no workspace configured, a short setup assistant walks you through it.

The desktop app does not update itself like the terminal app does today; see [Updates](#updates).

## The window

- **Rail.** The strip at the edge lists the views: FILES, FIND, TAGS, LINKS, OUTLINE, and, when a [Kimün server](@/using-kimun/server.md) is connected, SEM and ASK. Click one to show it.
- **Slots.** Views live in slots on either side of the editor. Drag a view by its slot header to another slot, or onto an edge of the window to give it a new slot; the places it can land light up as you drag. Drag the dividers to resize.
- **Zoom.** `Ctrl+Shift+M` (`Cmd+Shift+M` on macOS) zooms the focused view to the whole window; the same chord restores it.
- **Presets.** The layout is saved as you leave it. To start from a layout you author yourself, put it in a `layouts.toml` next to `config.toml` and apply it from the command palette (`Apply layout preset`).
- **Footer.** The note's path, caret position, backlink and property counts, saved or unsaved, and a git summary (`git ✓` when clean, `git ●3` for three changed paths) when the workspace is a git repository. Leader `g s` (see [Keyboard](#keyboard)) says it in full.

The editor is the same as the terminal app's: plain or [vim](@/using-kimun/vim-mode.md) mode, wikilinks (double-click a link to follow it), `#hashtags`, and a find bar.

### Completion

Typing `[[` in a note offers the vault's notes, and `#` offers its tags. In FIND, in the search browser (`Ctrl+K`) and in the note finder, `#` completes tags, `<`, `>` and `=` complete note names in link filters, and a leading `?` completes a saved search's name and expands to its query. `Up` and `Down` move through the list, `Tab` or `Enter` accepts, and `Esc` closes it.

## Workspaces

The title bar shows which workspace is open, as a button: click it, or press `F5` (or leader `v s`), to open the workspace switcher. With system window decorations, or in fullscreen, the button moves to the left end of the footer. Switching saves the open note first. Workspaces are managed in Preferences > Workspaces, and are the same ones the terminal app uses.

## Preferences

`F4` (or `Ctrl+,`) opens Preferences. What it saves is `config.toml`, the file the terminal app reads too (see [Configuration](@/getting-started/configuration.md)), plus the desktop app's own display options such as **Reduce motion**. Leader `v t` opens the theme picker.

## Keyboard

Press `F1` for the keyboard reference: every command with the chord it has now, including your own overrides. Some chords worth knowing (`Ctrl` is `Cmd` on macOS):

| Chord | Does |
|---|---|
| `Ctrl+Shift+B` | the leader: then a short sequence reaches any command, as in the terminal app (`Ctrl+Shift+B f f` opens the note finder) |
| `Ctrl+Shift+P` | command palette |
| `Ctrl+K` | search browser |
| `Ctrl+S` | save |
| `F5` | switch workspace |
| `F4` | Preferences |

Chords are overridden in a `[gui.keys]` table of `config.toml`. The key is the action's name, the value one chord or a list; an empty list unbinds. A chord joins modifiers and one key with `+`: `primary` is `Ctrl` (`Cmd` on macOS), the others are `cmd`, `ctrl`, `alt` and `shift`.

```toml
[gui.keys]
SaveNote = "primary+s"
OpenFileBrowser = ["primary+e", "f7"]
OpenQueryInSidebar = []
```

A chord or action the app does not know is skipped and logged; the rest still apply. The desktop app has its own key table: the terminal app's `[key_bindings]` do not apply to it.

## Display options in `config.toml`

All under `[gui]`; Preferences writes them for you.

| Key | Values | Meaning |
|---|---|---|
| `window_decorations` | `"client"` (default), `"system"` | Who draws the title bar. `"client"` is Kimün's own, in the theme's colours. `"system"` is the operating system's, the way out if the custom bar misbehaves on your desktop. Read at start; restart to change. macOS always keeps its native window buttons. |
| `reduce_motion` | `true`, `false` (default) | Keeps decorative motion (the welcome screen's bouncing dots, spinners) still. The system's own setting is not read. |
| `layout_preset` | a preset's name | The preset the saved layout started from. |

`[gui.layout]` is the saved layout itself; leave it to the app.

## Fonts

The interface uses [Inter](https://rsms.me/inter/), embedded in the app and licensed under the SIL Open Font License. The editor uses your theme's monospace font.

## Updates

The desktop app checks for a new Kimün release when it starts (turn it off with `update_check` in `config.toml`) and tells you in a toast. **Preferences > Display > Check now**, or leader `v u`, checks on demand. Since prebuilt desktop binaries are not published yet, the app cannot install an update itself: the dialog shows what is new and points you at the releases page, and a new version means building from source again.

If a [Kimün server](@/using-kimun/server.md) reports a newer release of itself, you get a toast saying so. Update the server the way you installed it.

## Not in the desktop app yet

The leader tree lists these, but they are not built, in the desktop app or in the terminal app:

- note from a template
- local graph
- git sync, log and diff (both apps show the git summary only)
- two editors side by side (slots hold views, not more editors)
- toggle a todo item
- preview, and export
