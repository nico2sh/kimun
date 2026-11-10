+++
title = "Vim Mode"
weight = 15
+++

# Vim Mode

Kimün has built-in vim emulation. It adds the modal editing of vim on top of the built-in editor, with no external process and no plugins. Insert mode keeps every editor feature: autocomplete, auto-surround, smart-Enter, and the styled markdown view. Normal, Visual, and Replace modes run the vim engine. The one exception is a bare `Enter` in Normal mode. Kimün handles it differently from the `<CR>` motion of vim. See [Editing](#editing).

Enable it in `config.toml`:

```toml
editor_backend = "vim"
```

You can also set it in the Preferences window (Editor section). The change applies the next time you open a note. To use Neovim itself, with your own `init.lua` and plugins, set [`editor_backend = "nvim"`](@/getting-started/configuration.md#editor-backend).

## Modes

The footer shows the active mode and the pending keys of a command in progress, for example `2d`, `gu` or `di`.

| Mode | Enter | Leave |
|---|---|---|
| NORMAL | `Esc` from any mode | — |
| INSERT | `i` `a` `I` `A` `o` `O`, or any change operator (`c`, `s`, `S`, …) | `Esc` |
| REPLACE | `R` (overwrite chars in place) | `Esc` |
| VISUAL | `v` (charwise) | `Esc`, or any operator |
| V-LINE | `V` (linewise) | `Esc`, or any operator |

## Cursor movement

All motions take a count, for example `3w` or `5j`. Counts combine with operators. For example, `2d3w` deletes six words.

| Keys | Motion |
|---|---|
| `h` `j` `k` `l`, arrows | left / down / up / right |
| `w` `b` `e` | next word start / previous word start / word end |
| `W` `B` `E` | same over WORDS (any non-blank run, so `foo.bar` is one WORD) |
| `ge` `gE` | backward to previous word / WORD end |
| `0` `^` | line start / first non-blank |
| `$` `g_` | line end / last non-blank |
| `gg` `G` | first / last line |
| `5gg` `5G` | go to line 5 (the count is a line number) |
| `{` `}` | previous / next paragraph |
| `%` | matching bracket (`()` `[]` `{}` `<>`), across lines |
| `f x` `F x` | to next / previous occurrence of `x` on the line |
| `t x` `T x` | till just before / after `x` |
| `;` `,` | repeat last find, same / opposite direction |

Count-finds are atomic, as in vim. If the line has only one `x`, `2fx` fails and the cursor stays where it is.

## Operators

Operators combine with any motion or [text object](#text-objects). If you double an operator, it works on whole lines.

| Keys | Operator | Linewise form |
|---|---|---|
| `d` | delete (fills the register) | `dd` |
| `c` | change (delete and enter Insert) | `cc` |
| `y` | yank | `yy` |
| `>` `<` | indent / outdent | `>>` `<<` |
| `gu` `gU` `g~` | lowercase / uppercase / toggle case | `guu` / `gUU` / `g~~` (also `gugu`-style) |
| `D` `C` `Y` | delete / change / yank to line end | — |

Examples: `dw`, `ce` (and vim's `cw` = `ce` rule), `d$`, `dj` (linewise, two lines), `dG`, `d2G`, `dfx`, `dtx`, `d;`, `gUiw`, `g~e`.

As in vim, a failed motion fails the whole command. The commands `dfz` with no `z` on the line, `dj` on the last line, and `d%` with no bracket under the cursor delete nothing. They leave the register and the `.` command unchanged.

## Text objects

Text objects work with any operator (`diw`, `ci"`, `ya(`) and in Visual mode (`vi(`, `va"`). `i` means inner. `a` means around, and it includes the delimiters. `aw` takes the trailing space.

| Keys | Object |
|---|---|
| `iw` / `aw` | word |
| `i(` `i)` `ib` / `a(` … | `(…)` block |
| `i{` `i}` `iB` / `a{` … | `{…}` block |
| `i[` `i]` / `a[` … | `[…]` block |
| `i<` `i>` / `a<` … | `<…>` block |
| `i"` `i'` `` i` `` / `a"` … | quoted string |

Text objects work on a single line only.

## Editing

| Keys | Action |
|---|---|
| `x` `X` | delete char under / before cursor (never joins lines; `xp` swaps chars) |
| `r x` | replace one char with `x` |
| `R` | Replace mode: overwrite until `Esc`. Backspace restores the original char. Arrows reposition |
| `s` `S` | substitute char / line (delete and enter Insert) |
| `J` | join next line with one space, indent stripped |
| `gJ` | join verbatim, no space handling |
| `~` | toggle case of the char under the cursor |
| `u` / `Ctrl+r` | undo / redo |
| `.` | repeat the last change. Works for operators, `x`, `r`, paste, indents, inserts (`ihello<Esc>`), `cw`+typed text, `cc`, `s`, `R`, … |
| `p` `P` | paste after / before (linewise yanks paste as lines) |
| `Enter` | Differs from vim. It continues a list (or carries or dedents an indent) the same way Enter does in Insert mode. If there is no list or indent, it splits the row at the cursor. In both cases it enters Insert, like `o`. The `<CR>` motion of vim (first non-blank of the next line) is not implemented. With a count or a pending operator (`3<CR>`, `d<CR>`), Enter is unmapped |

## Visual mode

`v` selects by character, and `V` selects by line. Motions, counts, finds (`vf,`), `gg` and `5G`, and text objects (`vi(`, `va"`) all extend the selection or aim it again. `o` jumps to the other end. `gv` selects the last selection again. In Visual mode, it swaps with the current selection. For example, `V j > gv >` indents the same rows twice, and so does `V j > .`.

| Keys | Action on the selection |
|---|---|
| `d` `x` | delete |
| `c` `s` | change (delete and enter Insert) |
| `y` | yank |
| `p` `P` | replace the selection with the register (as in vim, the replaced text enters the register) |
| `u` `U` `g~` | lowercase / uppercase / toggle case |
| `>` `<` | indent / outdent the selected lines (`3>` shifts three steps) |
| `J` `gJ` | join the selected lines |
| `(` `[` `{` `"` `'` `` ` `` `*` `_` `~` | Differs from vim. It wraps the selection ([auto-surround](@/using-kimun/tui.md)). The wrapped text stays selected in Visual mode, so you can chain wraps. `*` makes bold, `[` `[` builds a wikilink, and `~` wraps for strikethrough. Use `g~` for the toggle-case of vim |

In Visual mode, a mouse drag selects exactly what it covers. `Ctrl+C` and right-click copy the selection the same way, then return to Normal mode.

`.` after any Visual edit repeats the edit at the cursor on a region of the same shape. For `V`, the region has the same number of lines. For a `v` on one row, it has the same number of characters. For a `v` across rows, it has the same number of rows, and it ends at the same column.

## Registers, search, command line

- Every yank and every delete or change fills the unnamed register, as in vim. For example, `xp` transposes two characters, and `ddp` moves a line down. The register is separate from the OS clipboard. `Ctrl+C`, `Ctrl+X` and `Ctrl+V` keep working on their own.
- `/` and `?` open the [find bar](@/using-kimun/tui.md#find-in-buffer). Enter goes to the next match, and `Shift+Enter` goes back. Esc closes the bar. After that, `n` and `N` keep jumping between matches. The bar uses the same keys with both editor backends.
- There is no `:%s`. Search and replace is in the find bar. Press `Tab` to show the replacement field. Then press `Enter` to replace one match, or `Ctrl+A` to replace all matches. See [replace in buffer](@/using-kimun/tui.md#replace-in-buffer). `u` undoes a whole replace in one step. The pattern is a Rust regex, so vim regex syntax such as `\v` and `\zs` does not apply.
- `:` opens the [command palette](@/using-kimun/tui.md#command-palette).
- A bare `Space` in Normal mode starts a [leader](@/using-kimun/tui.md#the-leader-key) sequence, with the which-key panel. Space starts a sequence only from a clean Normal state. In the middle of a command (`d Space`, `f Space`, a pending count), it still acts as the motion or target character.

## Not (yet) supported

Macros (`q`) and named registers (`"a`) are planned. These are not available: Visual block mode (`Ctrl+v`), scroll motions (`zz`, `H`/`M`/`L`, `gj`/`gk`), tag objects (`it`/`at`), `gd`, and `gwip`. `gd` and code-centric commands will probably not come to a notes app.
