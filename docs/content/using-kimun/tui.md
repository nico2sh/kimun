+++
title = "TUI"
weight = 10
+++

# TUI Reference

Kimün's terminal UI is a single editor screen with an activity rail (top left side), one collapsible drawer (right after that one), and a two-line status bar (below). There are no editing modes; the only state is which panel has focus. Most commands are also reachable by mouse: click `≡ Commands` in the title bar to open the [command palette](#command-palette), then click the command. A few actions have no mouse route, such as Quick Note (`Ctrl+W` by default) and find/replace in the open note that have a quick keyboard shortcut.

## Layout

```
┌──┬────────────────┬───────────────────────────────┐
│  │                │                               │
│R │     DRAWER     │            EDITOR             │
│A │  (one of:      │                               │
│I │   FIL FND TAG  │                               │
│L │   LNK OUT CFG) │                               │
│  │                │                               │
├──┴────────────────┴───────────────────────────────┤
│ ⌨ EDITOR  hints…                     global hints │
│ path · ln/col · ✓ saved · backlinks · git         │
└───────────────────────────────────────────────────┘
```

- **Activity rail**: the icon strip on the far left. Each cell names a drawer view, and the active one is marked with a green border segment. Click a cell (or focus the rail and press Enter) to switch the drawer to that view. CFG is pinned at the bottom.
- **Drawer**: a single panel that shows one view at a time: FILES (file tree), FIND (query search), TAGS, LINKS, OUTLINE, or CFG (configuration overview). Toggle it with `Ctrl+T`, and drag the divider between drawer and editor to resize it.
- **Editor**: always visible, takes the remaining width.
- **Status bar**: line 1 shows the focused surface (`⌨` when a text field holds the cursor, `≣` for lists) and its key hints. Line 2 shows document state: path, line/column, saved/modified, backlink count, and git summary.

`Tab` / `Shift+Tab` cycle focus across the visible panels, except inside the editor text, where Tab indents. `Ctrl+L` / `Ctrl+H` move focus right / left from anywhere.

## The Leader Key

Press `Ctrl+B` (the *leader*) and then a short key sequence to reach any command. Sequences are grouped by mnemonic: `f` for +find, `n` for +note, `v` for +vault, and so on. A few examples:

```
Ctrl+B f f    open the file picker
Ctrl+B n d    open today's journal
Ctrl+B v t    open the theme picker
```

The full group-by-group tree is on the [Keybindings cheat-sheet](@/using-kimun/keybindings.md#the-leader-tree). Inside the app, `Ctrl+B ?` shows the same tree with your custom bindings applied.

If you pause mid-sequence, a **which-key** panel appears above the status bar showing what each next key does. The delay is `leader_timeout_ms`; change it in Preferences → Display or set it in `config.toml`. In lists (not text fields), a bare `Space` also starts a leader sequence. With the [vim editor backend](@/using-kimun/vim-mode.md), so does `Space` in the editor's Normal mode.

### Command Palette

`Ctrl+P` opens the command palette: every leader command in a fuzzy list, searchable by label or key sequence. Enter runs the selected command. The palette and the leader tree run the same actions.

### Customizing the leader tree

Sequences, additions, removals, and group captions are configurable in `config.toml`. See [Leader tree overrides](@/getting-started/configuration.md#leader-tree-overrides).

## Drawer Views

### FILES

The workspace file tree, with a breadcrumb header (click a segment to jump up), type-to-filter, and sorting (`Ctrl+R` opens the sort dialog: field, order, group-directories). Sort by Property to order the listing by a frontmatter key's value. Notes without that key (or not indexed yet) and attachments come after the rest, in name order. Directories come first when group-directories is on; otherwise they join the rows without a value. A property sort applies for the session but can't be saved as the default (`s`). Enter opens a note, and typing a name that matches nothing offers a *Create* row. Right-click a row, or press `F2`, for the file-operations menu (rename / move / delete).

### FIND

A live [query search](@/using-kimun/search.md) over the vault. It opens empty and shows a short syntax primer; type to search. Queries are syntax-highlighted as you type: tags in aqua, note targets in blue, field keys in yellow, negation in red, and an unterminated quote underlined with a `⚠` reason in the header.

- **Type**: results update live. `#` autocompletes tags, and `?` as the first character autocompletes [saved searches](#saved-searches).
- **Up/Down**: move through results. **Enter**: expand the selected note to show match context; press again for more, and a third time to collapse.
- **Ctrl+Enter** (or **Ctrl+N**): open the selected note in the editor, with the matched text highlighted there.
- **Ctrl+R**: sort dialog (written into the query as an `or:` directive). **Ctrl+D**: save the query under a name.
- Bare `<`, `>` or `=` are shorthand for `<{note}`, `>{note}`, `={note}` (current note's backlinks / forward links / name). The panel titles itself "Backlinks" when the query is any spelling of the backlinks query.

`Ctrl+E` switches to FIND from anywhere.

### TAGS

Every `#tag` in the vault with its note count, filterable. Enter (or click) runs that tag's query in FIND.

### LINKS

Link context for the open note, in three sub-tabs: backlinks, outgoing, and unlinked (mentions of the note's name that don't link to it). Switch tabs with `b` / `o` / `u`, `←`/`→`, or by clicking the tab name. Enter opens the selected note; right-click opens its file-operations menu.

### OUTLINE

The open note's headings as an indented tree, filterable. Enter jumps the editor to that heading.

### CFG

A configuration overview: active theme, leader key, preferences key, which-key timeout, and config file path. `t` (or Enter) opens the theme picker, a list of every theme with live preview. Type to filter the list by name. Moving the selection restyles the app immediately; Enter saves the choice and Esc reverts. `p` opens the full Preferences screen.

## Telescope Search

Two modal pickers open over the editor, with the list on the left and a preview on the right:

- **`Ctrl+K`**: query search (same grammar as FIND). The preview shows the note with matches emphasized and a `filename · N matches` header. `Ctrl+R` opens the sort dialog over it; the results re-sort as you change it, and closing the dialog returns to the search. With no search terms, a sort reorders your recent notes instead of searching the whole vault (the dialog shows **Unsorted** until you pick one).
- **`Ctrl+O`**: fuzzy file finder by name. Typing a new name offers a *Create* row. `Ctrl+R` opens the sort dialog here too, and re-sorts the rows by name, title or a property. Until you pick a sort, the dialog shows **Unsorted** (recent or best-match order); the first toggle picks Name.

Enter opens the selection. Query matches stay highlighted in the editor until your first edit. `Ctrl+D` saves the current query.

## Editor

The editor styles Markdown in place. The text stays plain, editable source, and there is no separate preview mode:

- Headings are bright/bold (H3 yellow), bold and italic are styled, bullets are dimmed, blockquotes get a `▏` bar, and inline and fenced code get a code background.
- `[[wikilinks]]` are blue and underlined, and `#tags` are colored. A single click places the cursor, so link text stays editable; a double-click follows the link or runs the tag query. `Ctrl+N` does the same from the keyboard.
- Task lists: `- [ ]` checkboxes are accented, and `- [x]` rows are dimmed and struck through.
- The cursor line shows the raw markup for editing.
- An empty note shows a ghost tip (`Type to start · [[ to link · # to tag · Ctrl+B for commands`) that disappears on the first keystroke.

When the cursor enters a link or tag, status line 2 shows where it goes: `→ people/maria · 3 backlinks` or `→ #tag · tag query`.

### Following links

With the cursor on a link, `Ctrl+Enter` follows it. `Ctrl+N` does the same, for terminals that can't distinguish Ctrl+Enter from Enter.

- **Wikilink**: opens the note (or a picker if several match). Relative paths and `#fragment` suffixes resolve correctly.
- **Markdown link**: same as a wikilink. A URL opens in your browser, and an image opens in your image viewer.
- **`#tag`**: opens the query search pre-filled with that tag.

### Find in buffer

`Ctrl+F` opens a one-line find bar, and matches are highlighted in the buffer. Press `Ctrl+F` or Enter to go to the next match, `Shift+Enter` to go back, and Esc to close.

The pattern is a regular expression, and case matching is smart: an all-lowercase pattern matches any case, and a pattern with a capital letter matches case exactly. The bar shows which mode applies and how many matches there are.

### Replace in buffer

Press `Tab` in the find bar (or run *Replace in note* from the command palette) to show a replacement field. The bar grows to two rows: the pattern and what it matches on top, and the replacement and its result below.

| Key | Action |
| --- | ------ |
| `Enter` | replace the current match and move to the next |
| `Shift+Enter` | skip this match without replacing |
| `Ctrl+A` | replace every match in the note |
| `Tab` | switch between the find and replace fields |
| `Esc` | close |

While you type, the note shows a live preview: every match is drawn as it would read after replacing, in its own colour. The match that `Enter` will replace next is drawn in the cursor colour. While the bar is open the terminal caret sits in the bar, so that highlight is how you see your position in the note. Nothing is written until you press a key; the preview only changes how the note is drawn.

Other details:

- **Undo is one keystroke.** A replace, of one match or all of them, is undone with a single `Ctrl+Z` (or `u` in vim mode).
- **`$1` works when you captured something.** If the pattern has a capture group, `$1`, `$2` and `${name}` expand in the replacement, so `(\w+)-(\w+)` → `$2 $1` swaps the two words. A `$` that doesn't name a real group is left as typed, so prices and `$x^2$` math are unaffected. Write `$$` for a literal `$` next to a digit that *is* a group number.
- **An empty replacement deletes.** Leaving the field blank and pressing `Ctrl+A` removes every match. Because a blank field can also mean you haven't finished typing, this case asks for a second `Ctrl+A` to confirm.
- Patterns can't span line breaks, and replacing never changes the number of lines in a note.
- Pasting while the bar is open goes into the focused field, not into the note.
- Replace is unavailable on the Neovim backend, which has its own `:%s`.

### Text formatting

Formatting commands are in the leader's `+text` group. They have no `Ctrl` chord by default,
because `Ctrl+I` is indistinguishable from `Tab` in most terminals. See
[Keybindings](@/using-kimun/keybindings.md#defaults) to bind one.

| Action | Binding | Effect |
| ------ | ------- | ------ |
| Bold | `Ctrl+B t b` | `**…**` around the selection |
| Italic | `Ctrl+B t i` | `*…*` |
| Strikethrough | `Ctrl+B t s` | `~~…~~` |

Each applies to the selection, or inserts an empty pair at the cursor. The
editor must have focus.

### Autocomplete

Typing `[[` opens a note list, and `#` (not at line start) opens a tag list. Type to filter, accept with Tab/Enter, and dismiss with Esc. This works in the editor and in every query field. (Textarea backend only; the Neovim backend uses your own completion setup.)

### Pasting

`Ctrl+V` (or the terminal's native paste) depends on what is on the clipboard: plain text is inserted, a URL pasted over a selection becomes `[selection](url)`, and an image is saved to `/assets/` with a relative image link inserted.

## Properties

Open the properties dialog for the current note with `<leader> n p`, from the command palette ("properties"), or by clicking `⊞ N props` (or `⊞ props`) in the status bar. The note is saved first. Every change is written to the file immediately and the editor reloads it.

| Key | Action |
| --- | ------ |
| `↑` `↓` / `j` `k` | Select a property |
| `Enter` / `e` | Edit the selected property |
| `a` | Add a property |
| `d` / `Del` | Delete (asks `y`/`n`, or click `[Yes]`/`[No]`) |
| `Tab` | Move between the list and the buttons |
| `Esc` | Close |

The dialog also works with the mouse: click a row to select it, click the selected row again to edit it, and use the `[+ Add] [Edit] [Delete] [Close]` buttons.

The edit form has three fields:

- **Key** suggests the keys other notes in your vault use.
- **Type** is `auto` by default, so Kimün uses the type the key has elsewhere in your vault. Cycle through `auto`, `text`, `number`, `bool`, `date`, `datetime` and `list` with `←` `→` or `Space`, or click `‹` `›`.
- **Value** is comma-separated for list types and for keys that are always lists (`tags`, `aliases`). For other keys, choose type `list` to enter several items. An empty value is refused.

Move between fields with `Tab` / `Shift+Tab` or `↑` `↓` (while the key suggestions are open, `↑` `↓` move through them instead). `Enter` saves and `Esc` goes back to the list. Renaming a key keeps its position in the frontmatter. Adding a key the note already has is refused; edit it instead. With Type `auto`, if the value does not fit the key's usual type in your vault, the form says so and offers **Store as … anyway**.

While a change is being written, `Esc`, `[Close]` and `[Cancel]` wait for it to finish. If you have unsaved typing in the note when a change lands, your typing is kept and the footer asks you to reopen the note to see the new properties.

If a note's frontmatter is malformed, the first failed write turns the dialog read-only. Fix the frontmatter in the editor and reopen the dialog.

## Mouse

Every mouse gesture has a keyboard equivalent. Buttons, dialog `[key]` hints, status-bar segments, the sort label and `≡ Commands` are drawn in the theme's blue. Everything you can click, except list rows, is highlighted when the pointer is over it.

| Gesture | Effect |
| ------- | ------ |
| Click | focus the panel / select the row |
| Click the selected row again | open it (in FIND and Ask Sources: step the preview) |
| Double-click a row | open it (also in FIND and Ask Sources) |
| Click a search / filter box | type into it (leaves list focus) |
| Click a suggestion in an autocomplete popup | insert it (same as `Tab`); scroll over it to move the highlight |
| Click in the editor | place the cursor |
| Double-click a `[[link]]` / `#tag` in the editor | follow it / run the tag query (same as `Ctrl+N`). Textarea and Vim backends only |
| Right-click a file or note row | file-operations menu |
| Right-click in the editor (no selection) | context menu for the open note |
| Right-click in the editor (with selection) | copy the selection |
| Drag the drawer∕editor divider | resize |
| Click a breadcrumb segment | jump up the tree |
| Click a rail cell / LINKS tab | switch view |
| Click a `[key] action` hint in a dialog | same as pressing that key (e.g. `[Esc] Cancel`) |
| Click outside a menu, picker, help or the command palette | close it (same as `Esc`). Dialogs you type into stay open |
| Click `⊞ props` / `⬆ x available` in the status bar | open properties / the update dialog |
| Click `N backlinks` / `→ target` in the status bar | open LINKS on backlinks / follow the link (same as `Ctrl+N`) |
| Click the sort label (`Name ↑`) on a search box (FILES, FIND, the `Ctrl+K` search and the `Ctrl+O` finder) | open the sort dialog (same as `Ctrl+R`) |
| Click `[F1] Syntax` on the FIND query box, or `[F1] Query syntax` in the `Ctrl+K` search | open the search query syntax reference (`F1` there does the same) |
| Click `≡ Commands` / the workspace name in the title bar | open the command palette / the workspace switcher |
| Click a chip under an Ask answer | send, copy, save as note, regenerate, or start a new conversation |
| Click `Open externally` in an attachment | open it with the default program |
| In Preferences: click a section, a checkbox, `◀`/`▶`, a `[value]`, a button or a `[key]` hint | switch to it / toggle / step / cycle / press it. Clicking the selected row runs it too |
| Click `t theme picker` / `p preferences` in CFG | open them |
| Scroll | scroll the pane under the cursor |

Following a link uses a double-click because `Ctrl`+click and `Cmd`+click don't work in a terminal: the mouse protocol cannot report `Cmd`, and macOS turns `Ctrl`+click into a right-click before Kimün receives it. A double-click behaves the same on Linux, macOS, and Windows.

On the [Neovim backend](@/getting-started/configuration.md#editor-backend), Neovim owns the mouse and a click never moves the cursor, so double-click-to-follow does nothing there. Use `Ctrl+N`, which follows whatever the cursor is on with any backend.

While Kimün captures the mouse, your terminal's own gestures (middle-click paste, drag-to-select) are disabled. Hold `Shift` to use them for one action, or set `mouse = false` to give the mouse to your terminal entirely; every gesture above has a keyboard equivalent. See [Configuration → Mouse](@/getting-started/configuration.md#mouse).

## Saved Searches

`Ctrl+D` saves the active query (from FIND, the query modal, or the panel) under a name. Queries are stored as *templates*, so `{note}` resolves against whichever note is open when the search runs. Open the picker with `F3` or `Ctrl+B f s`: type to filter, `1`–`9` to quick-select, Enter to run it in FIND, and Delete to remove it. You can also type `?name` in any query field.

## Pinned Notes

You can pin up to nine notes per vault and open each with one keystroke. A tenth pin is refused rather than replacing an existing one; unpin a note to make room. With a note open, `Ctrl+B m i` pins it, or unpins it if it is already pinned. `Ctrl+B 1`…`Ctrl+B 9` open pinned notes 1–9. `Ctrl+B f p` opens the pinned-notes dialog: press a number or `Enter` to open that note, `j`/`k` to move, `J`/`K` to reorder, `d` to unpin, and Esc to close. Changes are saved as you make them.

Pins are stored in the vault (`.kimun/pinned-notes.toml`), so they move with your notes. Renaming or moving a pinned note keeps its pin and deleting it removes the pin. A pin whose note was removed outside Kimün is shown as *(missing)* until you unpin it.

## Quick Note & Journal

- **`Ctrl+W`**: quick note dialog. Type a thought and press Enter to save it to your inbox with a timestamp name (Shift+Enter saves and opens it).
- **`Ctrl+J`**: open (or create) today's journal entry.

## Workspaces

`F4` opens the workspace switcher. Create, rename, delete, or re-path workspaces in the Preferences screen under **Workspaces**.

## Preferences Screen

`Ctrl+,` opens Preferences: workspace paths, theme, keybindings, autosave, and indexing. You can also open it from the palette, with `Ctrl+B v p`, or with `p` in the CFG drawer.

> Preferences was previously on `Ctrl+Shift+P`. It moved because that combination is a chord prefix in kitty's default configuration, which swallows the next key.

## Key Bindings

The full default table is on the [Keybindings cheat-sheet](@/using-kimun/keybindings.md). All bindings can be remapped in [Configuration](@/getting-started/configuration.md#key-bindings).

All other commands are behind the leader (`Ctrl+B`). Press it and pause, and the which-key panel shows what is available.
