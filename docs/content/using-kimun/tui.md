+++
title = "TUI"
weight = 10
+++

# TUI Reference

Kimün's terminal UI is built around a single editor screen with an activity rail, one collapsible drawer, and a two-line status bar. There are no editing modes: **focus is the only state**, and everything reachable by keyboard is also reachable by mouse.

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

- **Activity rail** — the icon strip on the far left. Each cell names a drawer view; the active one is marked with a green border segment. Click a cell (or focus the rail and press Enter) to switch the drawer to that view. CFG is pinned at the bottom.
- **Drawer** — a single panel that shows one view at a time: **FILES** (file tree), **FIND** (query search), **TAGS**, **LINKS**, **OUTLINE**, or **CFG** (configuration overview). Toggle it with `Ctrl+T`; drag the divider between drawer and editor to resize it.
- **Editor** — always visible, takes the remaining width.
- **Status bar** — line 1 shows the focused surface (`⌨` when a text field holds the cursor, `≣` for lists) and its key hints; line 2 shows document state: path, line/column, saved/modified, backlink count, git summary.

`Tab` / `Shift+Tab` cycle focus across the visible panels (when focus is not inside the editor text — there Tab indents). `Ctrl+L` / `Ctrl+H` move focus right / left from anywhere.

## The Leader Key

Press **`Ctrl+G`** (the *leader*) and then a short key sequence to reach any command. Sequences are grouped mnemonically — `f` for +find, `n` for +note, `v` for +vault, and so on. A few examples:

```
Ctrl+G f f    open the file picker
Ctrl+G n d    open today's journal
Ctrl+G v t    open the theme picker
```

The full group-by-group tree is on the [Keybindings cheat-sheet](@/using-kimun/keybindings.md#the-leader-tree).

Hesitate mid-sequence and a **which-key** panel pops up above the status bar showing what each next key does (the delay is configurable — `leader_timeout_ms`). In **lists** (not text fields), a bare `Space` also starts a leader sequence — and with the [vim editor backend](@/using-kimun/vim-mode.md), so does `Space` in the editor's Normal mode.

The full tree, with your custom bindings applied, is in the cheatsheet: `Ctrl+G ?`.

### Command Palette

**`Ctrl+P`** opens the command palette: every leader command as a fuzzy list, searchable by label *or* key sequence. Enter runs the selected command. It is the same action set as the leader tree — never a second implementation.

### Customizing the leader tree

Sequences, additions, removals, and group captions are configurable in `config.toml` — see [Leader tree overrides](@/getting-started/configuration.md#leader-tree-overrides).

## Drawer Views

### FILES

The workspace file tree with a breadcrumb header (click a segment to jump up), type-to-filter, and sorting (`Ctrl+R` opens the sort dialog: field, order, group-directories). Enter opens a note; typing a name that matches nothing offers a *Create* row. Right-click a row for the file-operations menu (rename / move / delete), also on `F2`.

### FIND

A live [query search](@/using-kimun/search.md) over the vault. It opens **empty**, showing a short syntax primer; type to search. Queries are syntax-highlighted as you type (tags aqua, note targets blue, field keys yellow, negation red, an unterminated quote underlined with a `⚠` reason in the header).

- **Type** — results update live; `#` autocompletes tags, `?` (first char) autocompletes [saved searches](#saved-searches)
- **Up/Down** — move through results · **Enter** — expand the selected note to show match context, again for more, a third time to collapse
- **Ctrl+Enter** (or **Ctrl+N**) — open the selected note in the editor; the matched text lights up there
- **Ctrl+R** — sort dialog (written into the query as an `or:` directive) · **Ctrl+D** — save the query under a name
- Bare `<`, `>` or `=` are shorthand for `<{note}`, `>{note}`, `={note}` (current note's backlinks / forward links / name). The panel titles itself "Backlinks" when the query is any spelling of the backlinks query.

`Ctrl+E` toggles straight to FIND from anywhere.

### TAGS

Every `#tag` in the vault with its note count, filterable. Enter (or click) runs that tag's query in FIND.

### LINKS

Link context for the open note, in three sub-tabs — **backlinks · outgoing · unlinked** (mentions of the note's name that don't link to it). Switch tabs with `b` / `o` / `u`, `←`/`→`, or by clicking the tab name. Enter opens the selected note; right-click opens its file-operations menu.

### OUTLINE

The open note's headings as an indented tree, filterable. Enter jumps the editor to that heading.

### CFG

A configuration overview: active theme, leader key, preferences key, which-key timeout, and config file path. `t` (or Enter) opens the **theme picker** — a live-preview list of every theme; moving the selection restyles the app instantly, Enter persists, Esc reverts. `p` opens the full Preferences screen.

## Telescope Search

Two modal pickers float over the editor, list on the left, preview on the right:

- **`Ctrl+K`** — query search (same grammar as FIND); the preview shows the note with matches emphasized and a `filename · N matches` header.
- **`Ctrl+O`** — fuzzy file finder by name; typing a new name offers a *Create* row.

Enter opens the selection (query matches stay highlighted in the editor until your first edit). `Ctrl+D` saves the current query.

## Editor

The editor renders Markdown styled in place — still plain editable source, no separate mode:

- Headings bright/bold (H3 yellow), bold/italic styled, bullets dimmed, blockquotes get a `▏` bar, inline and fenced code get a code background
- `[[wikilinks]]` blue and underlined, `#tags` colored — both **clickable** (click follows / runs the tag query; click again on the same spot to place the cursor for editing)
- Task lists: `- [ ]` checkboxes accented, `- [x]` rows dimmed and struck through
- The cursor line reveals raw markup for editing
- An empty note shows a ghost tip (`Type to start · [[ to link · # to tag · Ctrl+G for commands`) that vanishes on the first keystroke

When the cursor enters a link or tag, status line 2 shows where it goes: `→ people/maria · 3 backlinks` or `→ #tag · tag query`.

### Following links

With the cursor on a link, **`Ctrl+Enter`** follows it (**`Ctrl+N`** does the same on terminals that can't distinguish Ctrl+Enter from Enter):

- **Wikilink** — opens the note (picker if several match); relative paths and `#fragment` suffixes resolve correctly
- **Markdown link** — same; **URL** — opens in your browser; **image** — opens in your image viewer
- **`#tag`** — opens the query search pre-filled with that tag

### Find in buffer

`Ctrl+F` opens a one-line find bar; matches highlight in the buffer; press `Ctrl+F` / Enter to advance. Esc closes.

### Text formatting

| Action | Binding | Effect |
| ------ | ------- | ------ |
| Bold | `Ctrl+B` | `**…**` around the selection |
| Italic | `Ctrl+I` | `*…*` |
| Strikethrough | `Ctrl+S` | `~~…~~` |

### Autocomplete

Typing `[[` pops up a note list; `#` (not at line start) pops up tags — filter by typing, accept with Tab/Enter, dismiss with Esc. Works in the editor and in every query field. (Textarea backend only; the Neovim backend uses your own completion setup.)

### Pasting

`Ctrl+V` (or the terminal's native paste) adapts to the clipboard: plain text inserts; a URL over a selection wraps it as `[selection](url)`; an image saves to `/assets/` and inserts a relative image link.

## Mouse

Full parity with the keyboard:

| Gesture | Effect |
| ------- | ------ |
| Click | focus the panel / select the row |
| Click the selected row again | open it |
| Click a `[[link]]` / `#tag` in the editor | follow it / run the tag query |
| Right-click a file or note row | file-operations menu |
| Right-click in the editor (no selection) | context menu for the open note |
| Right-click in the editor (with selection) | copy the selection |
| Drag the drawer∕editor divider | resize |
| Click a breadcrumb segment | jump up the tree |
| Click a rail cell / LINKS tab | switch view |
| Scroll | scroll the pane under the cursor |

## Saved Searches

`Ctrl+D` saves the active query (from FIND, the query modal, or the panel) under a name; queries are stored as *templates*, so `{note}` re-resolves against whichever note is open when run. Open the picker with **`F3`** or **`Ctrl+G f s`**: type to filter, `1`–`9` quick-select, Enter runs it in FIND, Delete removes it. Or type `?name` directly in any query field.

## Quick Note & Journal

- **`Ctrl+W`** — quick note dialog: type a thought, Enter saves it to your inbox with a timestamp name (Shift+Enter saves *and* opens it).
- **`Ctrl+J`** — open (or create) today's journal entry.

## Workspaces

**`F4`** opens the workspace switcher. Manage workspaces (create/rename/delete/re-path) in the Preferences screen under **Workspaces**.

## Preferences Screen

**`Ctrl+,`** opens Preferences: workspace paths, theme, keybindings, autosave, indexing. Also reachable via the palette, `Ctrl+G v p`, or the CFG drawer's `p`.

> Preferences was previously on `Ctrl+Shift+P`; it moved because that combination is a chord prefix in kitty's default configuration, which swallows the next key.

## Key Bindings

The full default table lives on the [Keybindings cheat-sheet](@/using-kimun/keybindings.md) — one screen, everything on it. All bindings are remappable in [Configuration](@/getting-started/configuration.md#key-bindings).

Everything else lives behind the leader (`Ctrl+G`) — press it and pause: the which-key panel pops up and shows you everything available.
