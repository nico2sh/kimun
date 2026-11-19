# Frontmatter display in the editors

Status: built in the TUI and the GUI, 2026-11-19. Hover reveal was dropped. Scope: the TUI editor and the GUI editor.

## Problem

Both editors show a note's frontmatter as plain text. The `---` fences and the property lines look the same as the note body. The block takes screen space at the top of every note. If the reader does not want to edit the block, the block still shows in full.

## Goal

The editor draws the frontmatter block as a box. If the cursor is not inside the block, the block is collapsed to one summary row. If the reader moves the cursor into the block, the block opens in place. The mouse does not open the block, because the properties view already shows the properties.

## Decisions already made

- The block collapses unless the cursor is inside it, the find match is inside it, or the reader pinned it open.
- Both frontends get the feature through the shared editor logic. The TUI shows it first.
- The buffer text does not change. The display changes only.

## Non-goals

- No change to how core parses or indexes frontmatter.
- No structured property editing. The properties dialog stays as it is.
- No change to the nvim backend. Nvim keeps its full buffer. Only the render layer hides rows.
- No new setting in the first version.

## What the reader sees

A frontmatter block is a block that starts on line 1 with `---` or `+++` and has a closing fence. An unclosed block is not frontmatter. Core already uses this rule in `frontmatter_bounds`.

Collapsed, the block is one row. The row is dim and reads like `─ properties · tags, due, status ─`. If the row is too narrow, the row cuts the list of property keys with `…`. A block with no keys reads `─ properties ─`.

Open, the block is a box. The opening fence row shows as a top rule with the title `properties`. Each property line shows with a soft background that hugs the widest line, in the same way as a code block. The closing fence row shows as a bottom rule. The rules use the `─` glyph that the horizontal rule already uses. No underline and no new glyphs, as the TUI clickable style requires.

If the cursor is on a fence row, that row shows the raw `---` or `+++`. The editor already does this for any row that holds the cursor.

## When the block is open

Any one of these rules opens the block:

1. The cursor is inside the block, on a fence row or a property row.
2. The current find match is inside the block.
3. The reader pinned the block open with a click on the summary row.

A second click on the top rule unpins it. In all other cases the block is collapsed.

A new note starts with the cursor on line 1, and line 1 is the opening fence. The block must not open on every note load. For that reason rule 1 counts only after the first cursor move or edit in the session. Until then the block stays collapsed.

## Cursor and rows

A collapsed block has one display row. Its buffer lines are hidden.

If a cursor move targets a hidden line, the block opens first and the cursor lands on the real line. Moving up from the first body line below the block lands on the closing fence row. Moving down from the opening fence row lands on the first property row. If the cursor leaves the block, the block closes again.

A click on the summary row pins the block open and puts the cursor on the first property line.

Opening or closing a block changes the row count. The viewport keeps the first visible buffer line fixed. If the block is above the viewport top, the text below does not jump.

## Where the code goes

Core adds one public function that returns the line range of the frontmatter block in a text, built on `frontmatter_bounds`. It returns a range of line numbers and the fence kind. This keeps the fence rule in one place, as the project rules require. Neither frontend parses fences.

Both editors already share `tui/src/logic/editor`, which holds the parsed buffer, the layout, `EditorDisplay` and the `EditorFrame` rows. The new logic goes there:

- A `FrontmatterBlock` value on the parsed buffer holds the line range, the keys and the open state.
- A `FrontmatterReveal` value combines the cursor, the find match and the pin into the open state.
- The layout treats a collapsed block as one row. The implementation checks first whether the `RowHints` type in `ropetext` can express a row that stands for many lines. If it cannot, the layout needs a small extension.
- `render_runs` gains a role for the title rule, the property rows and the bottom rule.
- Mouse mapping in `click_at_screen` must account for the collapsed row.

All of this logic is shared, so the TUI and the GUI get the feature from one implementation. The frontends keep one small job: each painter maps the new run roles to colors. The GUI painter is `gui/src/row_paint.rs` with `gui/src/theme.rs`. The TUI painter is `role_style` in `markdown_style.rs`.

## Tests

- Core: the line range for LF and CRLF files, a byte-order mark, a `+++` block, an empty block and an unclosed block.
- Logic: the open state for each of the three rules, and the first-move rule on note load.
- Frame: a collapsed block gives one row. An open block gives the rule rows and the property rows. Rows below the block keep their buffer line numbers.
- Cursor: up and down moves across a collapsed block, and a click on the summary row.
- TUI screen tests and GUI tests use the existing fixtures.
- The nvim backend cannot run in the test suite. The author checks it by hand: open a note with frontmatter, move through the block, edit a property, save.

## Delivery order

1. Core: the frontmatter line range function, with tests.
2. Shared logic: the frontmatter model, the new run roles and the open box, always open. This step needs no hidden rows. The TUI painter shows the result first.
3. Shared logic: collapse, cursor entry, the find rule and the pin.
4. The GUI: check how the box and the fold look. The color mapping came with step 2. The GUI has no users, so its behavior can change freely.

Steps 2 and 3 build the feature once for both frontends. Step 4 is small. Each step ends with the full CI gate set, and the author checks the result before the next step starts.

## Open questions

- Do the property rows need a right edge on the box? This spec says no, because a right edge needs padding math that conflicts with line wrap. The background rectangle gives the box shape without it.
- Does the summary list keys only, or also the first value? This spec says keys only, to keep the row short.
