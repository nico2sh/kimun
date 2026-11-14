+++
title = "TUI"
weight = 11
+++

# Terminal app

![The terminal app with the rail on the left, the file list, and the editor](/img/tui-rail.png)

The terminal app has one screen. It has a strip of icons on the left (the rail), a side panel (the drawer), the editor, and a status bar at the bottom. You can use the mouse for most things. Every mouse action also has a keyboard shortcut.

```
┌──┬────────────────┬───────────────────────────────┐
│  │                │                               │
│R │     DRAWER     │            EDITOR             │
│A │  (one of:      │                               │
│I │   FIL FND TAG  │                               │
│L │   LNK OUT CFG) │                               │
│  │                │                               │
├──┴────────────────┴───────────────────────────────┤
│ hints                                             │
│ path · line/col · saved · backlinks               │
└───────────────────────────────────────────────────┘
```

The rail lists the views: FIL (files), FND (find), TAG, LNK (links), OUT (outline), and CFG (settings). If you connect a [Kimün server](@/using-kimun/server.md), it also lists SEM (search by meaning) and ASK.

Click an icon in the rail to change what the drawer shows. Press `Ctrl+T` to hide or show the drawer. Press `Tab` to move focus between the panels.

## Commands

Click `≡ Commands` in the title bar, or press `Ctrl+P`. Type to find a command, then press `Enter`.

Most commands also start with `Ctrl+B`. This is the leader key. Press it, then one or two more keys:

```
Ctrl+B f f    find a note by name
Ctrl+B n d    open today's journal
Ctrl+B v t    pick a theme
```

If you wait after `Ctrl+B`, a panel shows the keys that you can press next. See [Keybindings](@/using-kimun/keybindings.md) for the full list.

## The drawer

- FILES shows your folders and notes. Type to filter. Press `Enter` to open a note. Right-click a note, or press `F2`, to rename, move, or delete it. Press `Ctrl+R` to change the sort order.
- FIND is a live [search](@/using-kimun/search.md). Type a query and the results update. Press `Enter` to see more of a result. Press `Ctrl+Enter` or `Ctrl+N` to open it.
- TAGS lists every `#tag` and how many notes use it. Press `Enter` to find the notes.
- LINKS shows the notes that link to this note (backlinks), the notes that this note links to (outgoing), and notes that mention it without a link (unlinked).
- OUTLINE shows the headings of the note. Press `Enter` to jump to one.
- CFG shows your main settings. Press `t` to pick a theme or `p` to open Preferences.

## Find notes fast

- `Ctrl+O` finds a note by name. If no note matches, it offers to create one.
- `Ctrl+K` searches the content of all notes, with a preview.
- `Ctrl+E` jumps to FIND.

## Write a note

The editor shows Markdown with style, such as bold headings and colored links. You edit plain text. The line with the cursor shows the Markdown marks.

- Type `[[` to link to a note. Type `#` to add a tag. Press `Tab` or `Enter` to accept a suggestion.
- Double-click a `[[link]]` or a `#tag` to follow it. You can also press `Ctrl+Enter` or `Ctrl+N`. If no note matches the link, Kimün offers to create it.
- Press `Ctrl+B t b` for bold, `Ctrl+B t i` for italic, and `Ctrl+B t s` for strikethrough.
- Paste an image with `Ctrl+V`. Kimün saves it in `/assets/` and adds the link.
- Paste a web address over selected text to make a link.

### Find and replace

Press `Ctrl+F` to find text in the note. Press `Enter` for the next match and `Shift+Enter` for the previous one. Press `Tab` to add a replacement field. Then press `Enter` to replace one match, or `Ctrl+A` to replace all. `Ctrl+Z` undoes it. Press `Esc` to close.

## Properties

Properties are facts about a note, such as `status` or `due`. Kimün stores them at the top of the file. Press `Ctrl+B n p`, or click `props` in the status bar, to see them. In the window you can add (`a`), edit (`e`), and delete (`d`) a property.

![The properties window in the terminal app](/img/tui-properties.png)

## Saved searches

Press `Ctrl+D` in a search to save it under a name. Press `F3` to see your saved searches. Type `?` in any search box to use one by name.

## Pinned notes

You can pin up to nine notes. Press `Ctrl+B m i` to pin or unpin the open note. Press `Ctrl+B 1` to `Ctrl+B 9` to open a pinned note.

## Quick note and journal

- `Ctrl+W` opens a box. Type a thought and press `Enter`. Kimün saves it in your inbox. Press `Shift+Enter` to save it and open it.
- `Ctrl+J` opens today's [journal](@/using-kimun/journal.md).

## Workspaces and settings

Press `F5` to switch [workspace](@/getting-started/workspaces.md). Press `Ctrl+,` or `F4` for Preferences.

## Mouse

| Action | What it does |
|---|---|
| Click | Focus a panel or select a row |
| Double-click a row | Open it |
| Right-click a note | Rename, move, or delete it |
| Right-click in the editor | Menu for the open note |
| Drag the divider | Resize the drawer |
| Click `props` or `backlinks` in the status bar | Open properties or the backlinks |

Kimün uses the mouse, so your terminal cannot use it for copy and paste at the same time. Hold `Shift` for one action. To turn this off, see [Troubleshooting](@/getting-started/troubleshooting.md#middle-click-paste-or-text-selection-does-not-work).
