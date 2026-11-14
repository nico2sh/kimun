+++
title = "Keybindings"
weight = 14
+++

# Keybindings

These are the shortcuts of the terminal app. In the app, press `F1` for help and `Ctrl+B ?` for the full list with your own changes. The [desktop app](@/using-kimun/desktop-app.md#shortcuts) has its own shortcuts.

## Main shortcuts

| What it does | Shortcut |
|---|---|
| Quit | `Ctrl+Q` |
| Leader key (start a command sequence) | `Ctrl+B` |
| Command palette | `Ctrl+P` |
| Preferences | `F4` or `Ctrl+,` |
| Search content | `Ctrl+K` |
| Find a note by name | `Ctrl+O` |
| Show or hide the drawer | `Ctrl+T` |
| Show the file list | `Ctrl+E` |
| Find in the note | `Ctrl+F` |
| Follow a link | `Ctrl+Enter` or `Ctrl+N` |
| Today's journal | `Ctrl+J` |
| Quick note | `Ctrl+W` |
| Save a search | `Ctrl+D` |
| Saved searches | `F3` |
| Change the sort order | `Ctrl+R` |
| Rename, move, or delete a note | `F2` |
| Switch workspace | `F5` |
| Move focus right or left | `Ctrl+L` or `Ctrl+H` |
| Bold, italic, strikethrough | `Ctrl+B t b`, `Ctrl+B t i`, `Ctrl+B t s` |
| Help | `F1` |

## Command sequences

Press `Ctrl+B`, then the keys of a group. If you wait, a panel shows your choices.

| Group | Keys |
|---|---|
| `f` find | `f f` notes · `f g` search · `f t` tags · `f b` backlinks · `f r` recent · `f s` saved searches · `f h` headings · `f p` pinned notes |
| `n` note | `n n` new · `n d` today's journal · `n r` rename · `n m` move · `n p` properties · `n D` delete |
| `l` links | `l b` backlinks · `l o` outgoing · `l u` unlinked mentions |
| `o` open | `o f` files · `o q` find · `o t` tags · `o k` links · `o l` outline · `o c` settings |
| `v` workspace | `v s` switch · `v r` rebuild index · `v t` theme · `v p` preferences |
| `w` window | `w z` zen mode · `w l` and `w h` resize the drawer |
| `t` text | `t b` bold · `t i` italic · `t s` strikethrough |
| `m` this note | `m c` copy link · `m y` copy path · `m r` rename · `m i` pin or unpin |
| `1` to `9` | Open a pinned note |
| `p` | Command palette |
| `?` | Help |

## Change a shortcut

Add a `[key_bindings]` section to `config.toml`. It replaces the whole list of defaults, so list every shortcut that you want to keep. Only `Quit` comes back by itself.

```toml
[key_bindings]
Quit = ["ctrl&Q"]
SearchNotes = ["ctrl&K"]
OpenNote = ["ctrl&O"]
NewJournal = ["ctrl&J"]
QuickNote = ["ctrl&W"]
```

Bold, italic, and strikethrough have no `Ctrl` shortcut. Most terminals cannot tell `Ctrl+I` from `Tab`. You can bind your own, for example `TextEditor-Bold = ["ctrl&S"]`.
