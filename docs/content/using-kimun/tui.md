+++
title = "TUI"
weight = 10
+++

# TUI Reference

The terminal UI of Kimün is a single editor screen. It has an activity rail at the top left, one collapsible drawer next to the rail, and a two-line status bar below. There are no editing modes. The only state is which panel has focus.

You can reach most commands with the mouse. Click `≡ Commands` in the title bar to open the [command palette](#command-palette), then click the command. A few actions have no mouse route. Quick Note (`Ctrl+W` by default) and find/replace in the open note have only keyboard shortcuts.

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

- Activity rail: the icon strip on the far left. Each cell names a drawer view. A green border segment marks the active view. To switch the drawer to a view, click its cell, or focus the rail and press Enter. CFG is pinned at the bottom.
- Drawer: a single panel that shows one view at a time. The views are FILES (file tree), FIND (query search), TAGS, LINKS, OUTLINE, and CFG (configuration overview). Toggle the drawer with `Ctrl+T`. Drag the divider between the drawer and the editor to resize the drawer.
- Editor: always visible. It takes the remaining width.
- Status bar: line 1 shows the focused surface and its key hints. The symbol is `⌨` when a text field holds the cursor and `≣` for lists. Line 2 shows the state of the document: path, line and column, saved or modified, backlink count, and git summary.

`Tab` and `Shift+Tab` cycle focus across the visible panels. Inside the editor text, `Tab` indents. `Ctrl+L` and `Ctrl+H` move focus right and left from anywhere.

## The Leader Key

Press `Ctrl+B` (the leader key) and then a short key sequence to reach any command. Sequences are grouped by mnemonic: `f` for +find, `n` for +note, `v` for +vault, and so on. Here are three examples:

```
Ctrl+B f f    open the file picker
Ctrl+B n d    open today's journal
Ctrl+B v t    open the theme picker
```

The full tree, group by group, is on the [Keybindings cheat-sheet](@/using-kimun/keybindings.md#the-leader-tree). In the app, `Ctrl+B ?` shows the same tree with your custom bindings applied.

If you pause in the middle of a sequence, a which-key panel appears above the status bar. It shows what each next key does. The delay is `leader_timeout_ms`. You can change it in Preferences → Display or in `config.toml`. In lists (not text fields), a bare `Space` also starts a leader sequence. With the [vim editor backend](@/using-kimun/vim-mode.md), `Space` in the Normal mode of the editor does the same.

### Command Palette

`Ctrl+P` opens the command palette. It lists every leader command in a fuzzy list that you can search by label or key sequence. Enter runs the selected command. The palette and the leader tree run the same actions.

### Customizing the leader tree

You can set sequences, additions, removals, and group captions in `config.toml`. See [Leader tree overrides](@/getting-started/configuration.md#leader-tree-overrides).

## Drawer Views

### FILES

FILES shows the file tree of the workspace. It has a breadcrumb header, and you click a segment to jump up. You can type to filter the tree. `Ctrl+R` opens the sort dialog with three fields: field, order, and group-directories.

Sort by Property orders the list by the value of a frontmatter key. Notes without that key, notes that are not indexed yet, and attachments come after the rest, in name order. When group-directories is on, directories come first. Otherwise they join the rows without a value. A property sort applies for the session. You cannot save it as the default (`s`).

Enter opens a note. If you type a name that matches nothing, the list offers a *Create* row. Right-click a row, or press `F2`, to open the file-operations menu (rename, move, delete).

### FIND

FIND is a live [query search](@/using-kimun/search.md) over the vault. It opens empty and shows a short syntax primer. Type to search. The query is highlighted as you type: tags in aqua, note targets in blue, field keys in yellow, and negation in red. An unterminated quote is underlined, and the header shows a `⚠` reason.

- Type: the results update live. `#` autocompletes tags. `?` as the first character autocompletes [saved searches](#saved-searches).
- Up and Down: move through the results.
- Enter: expand the selected note to show the context of the match. Press Enter again for more context, and a third time to collapse.
- Alt+Right and Alt+Left: grow or shrink the preview one step, without leaving the query box. Some terminals (macOS Terminal by default) do not send Alt+arrow keys. Use Enter there.
- Ctrl+Enter or Ctrl+N: open the selected note in the editor. The matched text is highlighted there.
- Ctrl+R: open the sort dialog. The dialog writes the sort into the query as an `or:` directive.
- Ctrl+D: save the query under a name.
- A bare `<`, `>` or `=` is shorthand for `<{note}`, `>{note}` or `={note}`. These match the backlinks, the forward links, and the name of the current note. The panel title is "Backlinks" when the query is any spelling of the backlinks query.

`Ctrl+E` switches to FIND from anywhere.

### TAGS

TAGS lists every `#tag` in the vault with its note count. You can filter the list. Enter (or a click) runs the query for that tag in FIND.

### LINKS

LINKS shows the link context of the open note in three sub-tabs: backlinks, outgoing, and unlinked. Unlinked lists the mentions of the name of the note that do not link to it. Switch tabs with `b`, `o` or `u`, with `←` or `→`, or by a click on the tab name. Enter opens the selected note. Right-click opens its file-operations menu.

Outgoing lists each note that a link [points at](@/using-kimun/search.md#how-links-point-at-notes). If several notes share a name, it lists every match. For a missing note, it lists the path that the note would get, and Enter offers to create it.

### OUTLINE

OUTLINE shows the headings of the open note as an indented tree. You can filter the tree. Enter moves the editor to that heading.

### CFG

CFG is an overview of the configuration: active theme, leader key, preferences key, which-key timeout, and path of the configuration file. `t` (or Enter) opens the theme picker. The theme picker is a list of every theme with a live preview. Type to filter the list by name. When you move the selection, the app changes style at once. Enter saves the choice, and Esc reverts it. `p` opens the full Preferences screen.

## Telescope Search

Two modal pickers open over the editor, with the list on the left and a preview on the right:

- `Ctrl+K`: query search with the same grammar as FIND. The preview shows the note with the matches emphasized and a `filename · N matches` header. `Ctrl+R` opens the sort dialog over it. The results sort again when you change the dialog, and when you close the dialog you return to the search. If there are no search terms, a sort reorders your recent notes. It does not search the whole vault. The dialog shows Unsorted until you pick a sort.
- `Ctrl+O`: fuzzy file finder by name. If you type a new name, the list offers a *Create* row. `Ctrl+R` opens the sort dialog here too. It sorts the rows by name, title, or a property. The dialog shows Unsorted (recent or best-match order) until you pick a sort. The first toggle picks Name.

Enter opens the selection. The query matches stay highlighted in the editor until your first edit. `Ctrl+D` saves the current query.

## Editor

The editor styles Markdown in place. The text stays plain source that you can edit. There is no separate preview mode.

- Headings are bright and bold (H3 is yellow). Bold and italic text has a style. Bullets are dimmed. Blockquotes get a `▏` bar. Inline code and fenced code get a code background.
- `[[wikilinks]]` are blue and underlined, and `#tags` have a color. A single click places the cursor, so the link text stays editable. A double-click follows the link or runs the tag query. `Ctrl+N` does the same from the keyboard.
- In task lists, `- [ ]` checkboxes have an accent color. `- [x]` rows are dimmed and struck through.
- The cursor line shows the raw markup for editing.
- An empty note shows a ghost tip: `Type to start · [[ to link · # to tag · Ctrl+B for commands`. The tip disappears on the first keystroke.

When the cursor enters a link or tag, status line 2 shows where it goes: `→ people/maria · 3 backlinks` or `→ #tag · tag query`. The count is the number of backlinks of the note that the link opens. If several notes match the link, or none, the line shows no count.

### Following links

With the cursor on a link, `Ctrl+Enter` follows the link. `Ctrl+N` does the same for terminals that cannot tell Ctrl+Enter from Enter.

- Wikilink: opens the note that it names, in any folder. If several notes match, a picker opens. Folders narrow the name from the end. For example, `[[work/plan]]` finds `/work/plan.md` or `/a/work/plan.md` from anywhere. `[[/work/plan]]`, `[[./plan]]` and `[[../work/plan]]` are exact paths. A `#fragment` does not change the note.
- Markdown link: an exact path, relative to the folder of the note. `[text](plan.md)` is the `plan.md` beside the note, never a note elsewhere. A URL opens in your browser. An image opens in your image viewer. A link to any other file, for example `[report](files/report.pdf)`, opens in its default program.
- `#tag`: opens the query search with that tag already filled in.
- A link to no note offers to create the note.

See [How links point at notes](@/using-kimun/search.md#how-links-point-at-notes) for the full rules.

### Find in buffer

`Ctrl+F` opens a one-line find bar, and the buffer highlights the matches. Press `Ctrl+F` or Enter to go to the next match. Press `Shift+Enter` to go back. Press Esc to close the bar.

The pattern is a regular expression. Case matching is smart. An all-lowercase pattern matches any case. A pattern with a capital letter matches the case exactly. The bar shows which mode applies and how many matches there are.

### Replace in buffer

To show a replacement field, press `Tab` in the find bar, or run *Replace in note* from the command palette. The bar grows to two rows. The top row shows the pattern and what it matches. The bottom row shows the replacement and its result.

| Key | Action |
| --- | ------ |
| `Enter` | replace the current match and move to the next |
| `Shift+Enter` | skip this match without replacing |
| `Ctrl+A` | replace every match in the note |
| `Tab` | switch between the find and replace fields |
| `Esc` | close |

While you type, the note shows a live preview. The preview draws every match as it will read after the replacement, in its own color. The match that `Enter` will replace next uses the cursor color. While the bar is open, the terminal caret sits in the bar, so that highlight shows your position in the note. Kimün writes nothing until you press a key. The preview only changes how the note is drawn.

Other details:

- Undo takes one keystroke. A single `Ctrl+Z` (or `u` in vim mode) undoes a replace of one match or of all matches.
- `$1` works when the pattern captures something. If the pattern has a capture group, `$1`, `$2` and `${name}` expand in the replacement. For example, `(\w+)-(\w+)` → `$2 $1` swaps the two words. A `$` that does not name a real group stays as you typed it, so prices and `$x^2$` math are not changed. Write `$$` for a literal `$` next to a digit that is a group number.
- An empty replacement deletes. If you leave the field blank and press `Ctrl+A`, Kimün removes every match. A blank field can also mean that you did not finish typing. For this reason, Kimün asks for a second `Ctrl+A` to confirm.
- Patterns cannot span line breaks, and a replace never changes the number of lines in a note.
- A paste while the bar is open goes into the focused field, not into the note.
- Replace is not available on the Neovim backend. That backend has its own `:%s`.

### Text formatting

The formatting commands are in the `+text` group of the leader. They have no `Ctrl` chord by default, because most terminals cannot tell `Ctrl+I` from `Tab`. To bind a chord, see [Keybindings](@/using-kimun/keybindings.md#defaults).

| Action | Binding | Effect |
| ------ | ------- | ------ |
| Bold | `Ctrl+B t b` | `**…**` around the selection |
| Italic | `Ctrl+B t i` | `*…*` |
| Strikethrough | `Ctrl+B t s` | `~~…~~` |

Each command applies to the selection, or inserts an empty pair at the cursor. The editor must have focus.

### Autocomplete

If you type `[[`, a note list opens. If you type `#` not at the start of a line, a tag list opens. Type to filter the list, accept with Tab or Enter, and dismiss with Esc. This works in the editor and in every query field. It works on the textarea backend only. The Neovim backend uses your own completion configuration.

### Pasting

The result of `Ctrl+V` (or the native paste of the terminal) depends on what is on the clipboard. Plain text is inserted. A URL pasted over a selection becomes `[selection](url)`. An image is saved to `/assets/`, and Kimün inserts a relative image link.

## Properties

To open the properties dialog for the current note, use `<leader> n p`, use the command palette ("properties"), or click `⊞ N props` (or `⊞ props`) in the status bar. Kimün saves the note first. Kimün writes every change to the file at once, and the editor reloads the file.

| Key | Action |
| --- | ------ |
| `↑` `↓` / `j` `k` | Select a property |
| `Enter` / `e` | Edit the selected property |
| `a` | Add a property |
| `d` / `Del` | Delete (asks `y`/`n`, or click `[Yes]`/`[No]`) |
| `Tab` | Move between the list and the buttons |
| `Esc` | Close |

The dialog also works with the mouse. Click a row to select it, and click the selected row again to edit it. You can also use the `[+ Add] [Edit] [Delete] [Close]` buttons.

The edit form has three fields:

- Key: suggests the keys that other notes in your vault use.
- Type: `auto` by default. With `auto`, Kimün uses the type that the key has elsewhere in your vault. Cycle through `auto`, `text`, `number`, `bool`, `date`, `datetime` and `list` with `←` `→` or `Space`, or click `‹` `›`.
- Value: comma-separated for list types and for keys that are always lists (`tags`, `aliases`). For other keys, choose the type `list` to enter several items. Kimün refuses an empty value.

Move between the fields with `Tab` and `Shift+Tab`, or with `↑` and `↓`. While the key suggestions are open, `↑` and `↓` move through the suggestions instead. `Enter` saves, and `Esc` goes back to the list. If you rename a key, it keeps its position in the frontmatter. Kimün refuses to add a key that the note already has. Edit that key instead. If the type is `auto` and the value does not fit the usual type of the key in your vault, the form says so and offers Store as … anyway.

While Kimün writes a change, `Esc`, `[Close]` and `[Cancel]` wait until the write ends. If you have unsaved typing in the note when a change arrives, Kimün keeps your typing. The footer then asks you to reopen the note to see the new properties.

If the frontmatter of a note is malformed, the first failed write turns the dialog read-only. Fix the frontmatter in the editor, and reopen the dialog.

## Mouse

Every mouse gesture has a keyboard equivalent. Buttons, dialog `[key]` hints, status-bar segments, the sort label, and `≡ Commands` use the blue of the theme. Everything that you can click, except list rows, is highlighted when the pointer is over it.

| Gesture | Effect |
| ------- | ------ |
| Click | focus the panel / select the row |
| Click the selected row again | open it (in FIND and Ask Sources: step the preview) |
| Click the arrow at the end of the selected row, or in the full preview header | grow or shrink the preview (FIND, semantic search, Ask Sources). It never opens the note |
| Double-click a row | open it (also in FIND and Ask Sources) |
| Click a search / filter box | type into it (leaves list focus) |
| Click a suggestion in an autocomplete popup | insert it (same as `Tab`). Scroll over it to move the highlight |
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

To follow a link, you double-click. `Ctrl`+click and `Cmd`+click do not work in a terminal. The mouse protocol cannot report `Cmd`, and macOS turns `Ctrl`+click into a right-click before Kimün receives it. A double-click works the same on Linux, macOS, and Windows.

On the [Neovim backend](@/getting-started/configuration.md#editor-backend), Neovim owns the mouse and a click never moves the cursor. For this reason, double-click-to-follow does nothing there. Use `Ctrl+N`. It follows whatever is under the cursor with any backend.

While Kimün captures the mouse, the own gestures of your terminal (middle-click paste, drag-to-select) are disabled. Hold `Shift` to use them for one action. To give the mouse to your terminal entirely, set `mouse = false`. Every gesture above has a keyboard equivalent. See [Configuration → Mouse](@/getting-started/configuration.md#mouse).

## Saved Searches

`Ctrl+D` saves the active query under a name. The query can come from FIND, the query modal, or the panel. Kimün stores queries as templates (queries with placeholders). The placeholder `{note}` resolves against the note that is open when the search runs.

Open the picker with `F3` or `Ctrl+B f s`. In the picker, type to filter, press `1` to `9` to quick-select, press Enter to run the query in FIND, and press Delete to remove it. You can also type `?name` in any query field.

## Pinned Notes

You can pin up to nine notes per vault. You open each pinned note with one keystroke. Kimün refuses a tenth pin and does not replace an existing pin. To make room, unpin a note.

With a note open, `Ctrl+B m i` pins the note, or unpins it if it is already pinned. `Ctrl+B 1` to `Ctrl+B 9` open the pinned notes 1 to 9. `Ctrl+B f p` opens the pinned-notes dialog. In the dialog, press a number or `Enter` to open that note, `j` or `k` to move, `J` or `K` to reorder, `d` to unpin, and Esc to close. Kimün saves each change as you make it.

Kimün stores the pins in the vault (`.kimun/pinned-notes.toml`), so they move with your notes. If you rename or move a pinned note, it keeps its pin. If you delete it, the pin is removed. If a note was removed outside Kimün, its pin shows as *(missing)* until you unpin it.

## Quick Note & Journal

- `Ctrl+W`: opens the quick note dialog. Type a thought and press Enter to save it to your inbox with a timestamp name. Shift+Enter saves the note and opens it.
- `Ctrl+J`: opens today's journal entry, or creates it.

## Workspaces

`F5` opens the workspace switcher. To create, rename, delete, or re-path workspaces, use the Preferences screen under Workspaces.

## Preferences Screen

`Ctrl+,` opens Preferences: workspace paths, theme, keybindings, autosave, and indexing. You can also open it from the palette, with `Ctrl+B v p`, or with `p` in the CFG drawer.

Preferences was on `Ctrl+Shift+P` before. It moved because that combination is a chord prefix in the default configuration of kitty, and kitty swallows the next key.

## Key Bindings

The full default table is on the [Keybindings cheat-sheet](@/using-kimun/keybindings.md). You can remap all bindings in [Configuration](@/getting-started/configuration.md#key-bindings).

All other commands are behind the leader (`Ctrl+B`). Press the leader and pause. The which-key panel then shows the available commands.
