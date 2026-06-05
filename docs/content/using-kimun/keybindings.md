+++
title = "Keybindings"
weight = 11
+++

# Keybindings

Everything on one screen. Looking for how to *change* a binding? That's in [Configuration → Key Bindings](@/getting-started/configuration.md#key-bindings).

> In the app itself: `F1` opens help, and `Ctrl+G ?` shows this same cheatsheet — with your custom bindings applied.

## Defaults

| Action | Default |
| ------ | ------- |
| Quit | `Ctrl+Q` |
| **Leader** (command sequences) | `Ctrl+G` |
| Command palette | `Ctrl+P` |
| Preferences | `Ctrl+,` |
| Query search (telescope) | `Ctrl+K` |
| Open note (fuzzy finder) | `Ctrl+O` |
| Toggle drawer | `Ctrl+T` |
| Open FIND view | `Ctrl+E` |
| Find in buffer | `Ctrl+F` |
| Follow link | `Ctrl+Enter` (modern terminals) / `Ctrl+N` |
| New journal entry | `Ctrl+J` |
| Quick note | `Ctrl+W` |
| Save current query | `Ctrl+D` |
| Saved searches | `F3` |
| Sort dialog | `Ctrl+R` |
| File operations | `F2` |
| Switch workspace | `F4` |
| Focus right / left | `Ctrl+L` / `Ctrl+H` |
| Bold / Italic / Strikethrough | `Ctrl+B` / `Ctrl+I` / `Ctrl+S` |
| Help | `F1` (cheatsheet: `Ctrl+G ?`) |

## The Leader Tree

Everything else lives behind the leader: press `Ctrl+G`, then a short sequence. Pause mid-sequence and the which-key panel shows you what's next.

| Group | Keys | Examples |
| ----- | ---- | -------- |
| `f` +find | `f f` files · `f g` grep/query · `f t` tags · `f b` backlinks · `f r` recent · `f s` saved searches · `f h` headings |
| `n` +note | `n n` new · `n d` daily · `n t` from template · `n r` rename · `n m` move · `n D` delete |
| `l` +links | `l b` backlinks · `l o` outgoing · `l u` unlinked mentions |
| `o` +open | `o f/q/t/k/l/c` open a drawer view directly (files/find/tags/links/outline/config) |
| `g` +git | `g s` status · log/diff/sync are display-only stubs |
| `v` +vault | `v s` switch vault · `v r` reindex · `v c` config panel · `v t` theme picker · `v p` preferences |
| `w` +window | `w z` zen · `w l`/`w h` grow/shrink drawer |
| `m` +this note | `m t` toggle todo · `m p` preview · `m c` copy wikilink · `m y` yank path · `m r` rename |
| `p` | command palette |
| `?` | help / cheatsheet |

How the leader works — and how to remap the whole tree — is covered in the [TUI guide](@/using-kimun/tui.md#the-leader-key) and [Leader Tree Overrides](@/getting-started/configuration.md#leader-tree-overrides).
