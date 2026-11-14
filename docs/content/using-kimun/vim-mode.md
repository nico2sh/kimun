+++
title = "Vim Mode"
weight = 16
+++

# Vim Mode

Vim mode adds the vim way of editing to the Kimün editor. It is built in. You do not need to install anything.

To turn it on, open Preferences (`Ctrl+,`) and choose the vim editor. You can also add `editor_backend = "vim"` to `config.toml`. The change applies when you open the next note. To use your own Neovim, see [Settings](@/getting-started/configuration.md#neovim).

The footer shows the active mode and the keys that you have typed so far, for example `2d`.

## Modes

| Mode | How to enter | How to leave |
|---|---|---|
| Normal | `Esc` | |
| Insert | `i` `a` `I` `A` `o` `O` | `Esc` |
| Replace | `R` | `Esc` |
| Visual (characters) | `v` | `Esc` |
| Visual (lines) | `V` | `Esc` |

## Move

A number before a motion repeats it. For example, `3w` moves three words.

| Keys | Moves to |
|---|---|
| `h` `j` `k` `l` | left, down, up, right |
| `w` `b` `e` | next word, previous word, end of word |
| `0` `^` `$` | start of line, first character, end of line |
| `gg` `G` | first line, last line |
| `5G` | line 5 |
| `{` `}` | previous or next paragraph |
| `%` | matching bracket |
| `f x` `t x` | the next `x` on the line, or just before it |
| `;` `,` | repeat the last `f` or `t` |

## Change text

An operator waits for a motion. Double it to work on a whole line.

| Keys | What it does |
|---|---|
| `d` (`dd`) | delete |
| `c` (`cc`) | delete and start typing |
| `y` (`yy`) | copy |
| `>` `<` | indent or outdent |
| `gu` `gU` `g~` | lowercase, uppercase, switch case |
| `x` | delete the character |
| `r x` | replace one character with `x` |
| `p` `P` | paste after or before |
| `u` and `Ctrl+r` | undo and redo |
| `.` | repeat the last change |
| `J` | join the next line |

Examples: `dw` deletes a word. `d$` deletes to the end of the line. `ci"` changes the text inside quotes.

### Text objects

After an operator, `i` means inside and `a` means around. Use them with `w` (word), quotes, and brackets `( ) [ ] { } < >`. For example, `diw` deletes a word, and `ya(` copies the text in parentheses, with the parentheses.

## Visual mode

Select text with a motion, then press an operator. `d` deletes, `y` copies, `c` changes, and `p` replaces the selection with what you copied. `>` and `<` indent the lines. `o` jumps to the other end of the selection.

In Visual mode, these keys wrap the selection: `(` `[` `{` `"` `'` `*` `_` `~`. For example, `*` makes it bold.

## Differences from vim

- `Enter` in Normal mode continues a list, like `Enter` in Insert mode. It then starts typing.
- `Space` starts a command sequence, as `Ctrl+B` does.
- `/` opens the find bar. `Tab` in the bar adds a replace field. There is no `:%s`.
- `:` opens the command palette.
- Vim copies go to their own register. They do not use the system clipboard. `Ctrl+C`, `Ctrl+X`, and `Ctrl+V` still use the clipboard.

## Not available

Macros, named registers, and block selection (`Ctrl+v`) are not available.
