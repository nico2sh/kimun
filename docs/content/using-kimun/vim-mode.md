+++
title = "Vim Mode"
weight = 15
+++

# Vim Mode

Kimün has built-in vim emulation: vim's modal editing on top of the built-in editor, with no external process or plugins. Insert mode keeps every editor feature (autocomplete, auto-surround, smart-Enter, the styled markdown view). Normal, Visual, and Replace modes run the vim engine. The one exception is bare `Enter` in Normal mode, which Kimün handles differently from vim's `<CR>` motion; see [Editing](#editing).

Enable it in `config.toml`:

```toml
editor_backend = "vim"
```

or from the Preferences window (Editor section). The change applies the next time you open a note. To use Neovim itself, with your own `init.lua` and plugins, use [`editor_backend = "nvim"`](@/getting-started/configuration.md#editor-backend).

## Modes

The footer shows the active mode, plus the pending keys of an in-progress command (e.g. `2d`, `gu`, `di`).

| Mode | Enter | Leave |
|---|---|---|
| **NORMAL** | `Esc` from any mode | — |
| **INSERT** | `i` `a` `I` `A` `o` `O`, or any change operator (`c`, `s`, `S`, …) | `Esc` |
| **REPLACE** | `R` (overwrite chars in place) | `Esc` |
| **VISUAL** | `v` (charwise) | `Esc`, or any operator |
| **V-LINE** | `V` (linewise) | `Esc`, or any operator |

## Cursor movement

All motions take a count (`3w`, `5j`). Counts compose with operators: `2d3w` deletes six words.

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

Count-finds are atomic, as in vim: `2fx` with only one `x` on the line fails and the cursor stays put.

## Operators

Operators combine with any motion or [text object](#text-objects); doubling one operates on whole lines.

| Keys | Operator | Linewise form |
|---|---|---|
| `d` | delete (fills the register) | `dd` |
| `c` | change (delete and enter Insert) | `cc` |
| `y` | yank | `yy` |
| `>` `<` | indent / outdent | `>>` `<<` |
| `gu` `gU` `g~` | lowercase / uppercase / toggle case | `guu` / `gUU` / `g~~` (also `gugu`-style) |
| `D` `C` `Y` | delete / change / yank to line end | — |

Examples: `dw`, `ce` (and vim's `cw` = `ce` rule), `d$`, `dj` (linewise, two lines), `dG`, `d2G`, `dfx`, `dtx`, `d;`, `gUiw`, `g~e`.

A failed motion fails the whole command, as in vim: `dfz` with no `z` on the line, `dj` on the last line, or `d%` with no bracket under the cursor deletes nothing and leaves the register and the `.` command unchanged.

## Text objects

Text objects work with any operator (`diw`, `ci"`, `ya(`) and in Visual mode (`vi(`, `va"`). `i` = inner, `a` = around (delimiters included; `aw` takes trailing space).

| Keys | Object |
|---|---|
| `iw` / `aw` | word |
| `i(` `i)` `ib` / `a(` … | `(…)` block |
| `i{` `i}` `iB` / `a{` … | `{…}` block |
| `i[` `i]` / `a[` … | `[…]` block |
| `i<` `i>` / `a<` … | `<…>` block |
| `i"` `i'` `` i` `` / `a"` … | quoted string |

Text objects are single-line for now.

## Editing

| Keys | Action |
|---|---|
| `x` `X` | delete char under / before cursor (never joins lines; `xp` swaps chars) |
| `r x` | replace one char with `x` |
| `R` | Replace mode: overwrite until `Esc`; Backspace restores the original char; arrows reposition |
| `s` `S` | substitute char / line (delete and enter Insert) |
| `J` | join next line with one space, indent stripped |
| `gJ` | join verbatim, no space handling |
| `~` | toggle case of the char under the cursor |
| `u` / `Ctrl+r` | undo / redo |
| `.` | repeat the last change. Works for operators, `x`, `r`, paste, indents, inserts (`ihello<Esc>`), `cw`+typed text, `cc`, `s`, `R`, … |
| `p` `P` | paste after / before (linewise yanks paste as lines) |
| `Enter` | **Differs from vim**: continues a list (or carries/dedents an indent) the same way Insert mode's Enter does, or otherwise splits the row at the cursor. Either way it enters Insert, like `o`. Vim's `<CR>` motion (next line's first non-blank) is not implemented. With a count or pending operator (`3<CR>`, `d<CR>`), Enter is unmapped |

## Visual mode

`v` selects charwise, `V` linewise. Motions, counts, finds (`vf,`), `gg`/`5G`, and text objects (`vi(`, `va"`) all extend or re-aim the selection; `o` jumps to the other end. `gv` reselects the last selection (in Visual it swaps with it), so `V j > gv >` indents the same rows twice, as does `V j > .`.

| Keys | Action on the selection |
|---|---|
| `d` `x` | delete |
| `c` `s` | change (delete and enter Insert) |
| `y` | yank |
| `p` `P` | replace the selection with the register (the replaced text enters the register, as in vim) |
| `u` `U` `g~` | lowercase / uppercase / toggle case |
| `>` `<` | indent / outdent the selected lines (`3>` shifts three steps) |
| `J` `gJ` | join the selected lines |
| `(` `[` `{` `"` `'` `` ` `` `*` `_` `~` | **Differs from vim**: wraps the selection ([auto-surround](@/using-kimun/tui.md)). The wrapped text stays selected in Visual, so wraps can be chained: `*` bolds, `[` `[` builds a wikilink, and `~` wraps for strikethrough (use `g~` for vim's toggle-case) |

A mouse drag selects exactly what it covers, in Visual. `Ctrl+C` and right-click copy the selection the same way, then return to Normal.

`.` after any Visual edit repeats it at the cursor on a region of the same shape: as many lines for `V`, as many characters for a one-row `v`, and for a `v` across rows the same number of rows ending at the same column.

## Registers, search, command line

- Every yank and every delete/change fills the unnamed register, as in vim: `xp` transposes, `ddp` moves a line down. The register is separate from the OS clipboard, and `Ctrl+C/X/V` keep working independently.
- `/` and `?` open the [find bar](@/using-kimun/tui.md#find-in-buffer). Enter steps to the next match and `Shift+Enter` back; Esc closes the bar, and `n` / `N` keep jumping between matches afterwards. The bar uses the same keys with both editor backends.
- There is no `:%s`. Search and replace lives in the find bar: press `Tab` to show the replacement field, then `Enter` to replace one match or `Ctrl+A` for all. See [replace in buffer](@/using-kimun/tui.md#replace-in-buffer). `u` undoes a whole replace in one step. The pattern is Rust regex, so vim regex syntax such as `\v` and `\zs` doesn't apply.
- `:` opens the [command palette](@/using-kimun/tui.md#command-palette).
- A bare `Space` in Normal mode starts a [leader](@/using-kimun/tui.md#the-leader-key) sequence (with the which-key panel). Space starts a sequence only from a clean Normal state. Mid-command (`d Space`, `f Space`, a pending count) it still acts as the motion/target character.

## Not (yet) supported

Macros (`q`) and named registers (`"a`) are planned. Visual block mode (`Ctrl+v`), scroll motions (`zz`, `H`/`M`/`L`, `gj`/`gk`), tag objects (`it`/`at`), `gd`, and `gwip` are not available; `gd` and code-centric commands are unlikely to come to a notes app.
