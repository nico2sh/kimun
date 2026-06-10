+++
title = "Vim Mode"
weight = 15
+++

# Vim Mode

Kimün ships a built-in vim emulation: vim's modal editing layered directly over the built-in editor — no external process, no plugins to load. Insert mode keeps every editor feature (autocomplete, auto-surround, smart-Enter, the styled markdown view); Normal, Visual, and Replace modes run the vim engine.

Enable it in `config.toml`:

```toml
editor_backend = "vim"
```

or from the Preferences window (Editor section). The change applies the next time you open a note. If you want the real thing instead — your own `init.lua`, plugins and all — use [`editor_backend = "nvim"`](@/getting-started/configuration.md#editor-backend).

## Modes

The footer shows the active mode, plus the pending keys of an in-progress command (e.g. `2d`, `gu`, `di`).

| Mode | Enter | Leave |
|---|---|---|
| **NORMAL** | `Esc` from any mode | — |
| **INSERT** | `i` `a` `I` `A` `o` `O`, or any change operator (`c`, `s`, `S`, …) | `Esc` |
| **REPLACE** | `R` — overwrite chars in place | `Esc` |
| **VISUAL** | `v` (charwise) | `Esc`, or any operator |
| **V-LINE** | `V` (linewise) | `Esc`, or any operator |

## Cursor movement

All motions take a count (`3w`, `5j`). Counts compose with operators: `2d3w` deletes six words.

| Keys | Motion |
|---|---|
| `h` `j` `k` `l`, arrows | left / down / up / right |
| `w` `b` `e` | next word start / previous word start / word end |
| `W` `B` `E` | same over WORDS (any non-blank run — `foo.bar` is one WORD) |
| `ge` `gE` | backward to previous word / WORD end |
| `0` `^` | line start / first non-blank |
| `$` `g_` | line end / last non-blank |
| `gg` `G` | first / last line |
| `5gg` `5G` | go to line 5 (the count is a line number) |
| `{` `}` | previous / next paragraph |
| `%` | matching bracket — `()` `[]` `{}` `<>`, across lines |
| `f x` `F x` | to next / previous occurrence of `x` on the line |
| `t x` `T x` | till just before / after `x` |
| `;` `,` | repeat last find, same / opposite direction |

Count-finds are atomic like vim: `2fx` with only one `x` on the line fails and the cursor stays put.

## Operators

Operators combine with any motion or [text object](#text-objects); doubling one operates on whole lines.

| Keys | Operator | Linewise form |
|---|---|---|
| `d` | delete (fills the register) | `dd` |
| `c` | change — delete and enter Insert | `cc` |
| `y` | yank | `yy` |
| `>` `<` | indent / outdent | `>>` `<<` |
| `gu` `gU` `g~` | lowercase / uppercase / toggle case | `guu` / `gUU` / `g~~` (also `gugu`-style) |
| `D` `C` `Y` | delete / change / yank to line end | — |

Examples: `dw`, `ce` (and vim's `cw` = `ce` rule), `d$`, `dj` (linewise, two lines), `dG`, `d2G`, `dfx`, `dtx`, `d;`, `gUiw`, `g~e`.

A failed motion fails the whole command, exactly like vim: `dfz` with no `z` on the line, `dj` on the last line, or `d%` with no bracket under the cursor delete nothing — and never disturb the register or the `.` command.

## Text objects

Work with any operator (`diw`, `ci"`, `ya(`) and in Visual mode (`vi(`, `va"`). `i` = inner, `a` = around (delimiters included; `aw` takes trailing space).

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
| `.` | repeat the last change — works for operators, `x`, `r`, paste, indents, inserts (`ihello<Esc>`), `cw`+typed text, `cc`, `s`, `R`, … |
| `p` `P` | paste after / before (linewise yanks paste as lines) |

## Visual mode

`v` selects charwise, `V` linewise. Motions, counts, finds (`vf,`), `gg`/`5G`, and text objects (`vi(`, `va"`) all extend or re-aim the selection; `o` jumps to the other end.

| Keys | Action on the selection |
|---|---|
| `d` `x` | delete |
| `c` `s` | change (delete and enter Insert) |
| `y` | yank |
| `p` `P` | replace the selection with the register (the replaced text enters the register — vim's swap) |
| `u` `U` `g~` | lowercase / uppercase / toggle case |
| `>` `<` | indent / outdent the selected lines |
| `J` `gJ` | join the selected lines |
| `(` `[` `{` `<` `"` `'` `` ` `` `*` `_` `~` | **kimün twist**: wraps the selection ([auto-surround](@/using-kimun/tui.md)) instead of vim's behavior — so `*` bolds, `[` `[` builds a wikilink, and `~` wraps for strikethrough (use `g~` for vim's toggle-case) |

## Registers, search, command line

- Every yank **and** every delete/change fills the unnamed register, like vim — `xp` transposes, `ddp` moves a line down. The register is kept separate from the OS clipboard (`Ctrl+C/X/V` keep working independently).
- `/` and `?` open the find bar; Enter confirms, then `n` / `N` jump between matches with the bar closed.
- `:` opens the [command palette](@/using-kimun/tui.md#command-palette).
- A bare `Space` in Normal mode starts a [leader](@/using-kimun/tui.md#the-leader-key) sequence (with the which-key panel), so the whole command tree is reachable without leaving the home row. Space only leads from a *clean* Normal state — mid-command (`d Space`, `f Space`, a pending count) it still acts as the motion/target character.

## Not (yet) supported

Macros (`q`) and named registers (`"a`) are planned — the engine is built for them. Visual block mode (`Ctrl+v`), scroll motions (`zz`, `H`/`M`/`L`, `gj`/`gk`), tag objects (`it`/`at`), `gd`, and `gwip` are not available; `gd` and code-centric commands are unlikely to come to a notes app.
