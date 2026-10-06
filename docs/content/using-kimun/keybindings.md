+++
title = "Keybindings"
weight = 11
+++

# Keybindings

All default key bindings. To change a binding, see [Configuration → Key Bindings](@/getting-started/configuration.md#key-bindings).

> In the app, `F1` opens help and `Ctrl+B ?` shows this cheatsheet with your custom bindings applied.

> A `[key_bindings]` section in your config replaces the entire keymap instead of merging with the defaults. Any action you don't list ends up unbound (only `Quit` is restored automatically). To change one key, copy the full table below into your config and edit the lines you want. Details in [Configuration → Key Bindings](@/getting-started/configuration.md#overrides-replace-the-defaults).

## Defaults

| Action | Default |
| ------ | ------- |
| Quit | `Ctrl+Q` |
| **Leader** (command sequences) | `Ctrl+B` |
| Command palette | `Ctrl+P` |
| Preferences | `F4` / `Ctrl+,` |
| Query search (telescope) | `Ctrl+K` |
| Open note (fuzzy finder) | `Ctrl+O` |
| Toggle drawer | `Ctrl+T` |
| Open file browser (FILES view) | `Ctrl+E` |
| Find in buffer | `Ctrl+F` |
| Replace in buffer | `Tab` from the find bar (no default chord) |
| Follow link | `Ctrl+Enter` (modern terminals) / `Ctrl+N` |
| New journal entry | `Ctrl+J` |
| Quick note | `Ctrl+W` |
| Save current query | `Ctrl+D` |
| Saved searches | `F3` |
| Sort dialog | `Ctrl+R` |
| File operations | `F2` |
| Switch workspace | `F5` |
| Focus right / left | `Ctrl+L` / `Ctrl+H` |
| Bold / Italic / Strikethrough | `Ctrl+B t b` / `t i` / `t s` (see below¹) |
| Help | `F1` (cheatsheet: `Ctrl+B ?`). Over FIND or the `Ctrl+K` search, it shows the query syntax |

¹ Formatting has no `Ctrl` chord by design. `Ctrl+I` and `Tab` send the same
byte (`0x09`) on every terminal without the [kitty keyboard
protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/), so `Ctrl+I`
can't work everywhere. Having `Ctrl+B` and `Ctrl+S` format text while `Ctrl+I`
indented would be inconsistent, so all three live in the leader's `+text`
group, which works in every terminal.

You can still bind a chord to these actions, except `Ctrl+B`, which is the
leader key by default:

```toml
[key_bindings]
TextEditor-Bold = ["ctrl&S"]
```

Remember that a `[key_bindings]` section [replaces the whole
keymap](@/getting-started/configuration.md#overrides-replace-the-defaults).

## The Leader Tree

Everything else lives behind the leader: press `Ctrl+B`, then a short sequence. Pause mid-sequence and the which-key panel shows you what's next.

| Group | Keys | Examples |
| ----- | ---- | -------- |
| `f` +find | `f f` files · `f g` grep/query · `f t` tags · `f b` backlinks · `f r` recent · `f s` saved searches · `f h` headings · `f p` pinned notes |
| `n` +note | `n n` new · `n d` daily · `n t` from template · `n r` rename · `n p` properties · `n m` move · `n D` delete |
| `l` +links | `l b` backlinks · `l o` outgoing · `l u` unlinked mentions |
| `o` +open | `o f/q/t/k/l/c` open a drawer view directly (files/find/tags/links/outline/config) |
| `g` +git | `g s` status · `g p` sync/push · `g l` log · `g d` diff *(log/diff/sync are display-only stubs)* |
| `v` +vault | `v s` switch vault · `v r` reindex · `v c` config panel · `v t` theme picker · `v p` preferences |
| `w` +window | `w z` zen · `w l`/`w h` grow/shrink drawer |
| `t` +text | `t b` bold · `t i` italic · `t s` strikethrough |
| `m` +this note | `m t` toggle todo · `m p` preview · `m c` copy wikilink · `m y` yank path · `m r` rename · `m i` pin / unpin |
| `p` | command palette |
| `?` | help / cheatsheet |
| `1`–`9` | open pinned note 1–9 |

How the leader works, and how to remap the whole tree, is covered in the [TUI guide](@/using-kimun/tui.md#the-leader-key) and [Leader Tree Overrides](@/getting-started/configuration.md#leader-tree-overrides).
