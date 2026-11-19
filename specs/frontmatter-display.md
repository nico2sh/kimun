# Frontmatter display in the editors

Status: draft, 2026-11-18. Scope: the TUI editor and the GUI editor.

## Problem

Both editors show a note's frontmatter as plain text. The `---` fences and the property lines look the same as the note body. The block takes screen space at the top of every note. If the reader does not want to edit the block, the block still shows in full.

## Goal

The editor draws the frontmatter block as a box. If the mouse is not over the block and the cursor is not inside it, the block is collapsed to one summary row. If the reader points at the block or moves the cursor into it, the block opens in place.

## Decisions already made

- The block collapses unless the mouse is over it.
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

1. The mouse is over any row of the open block, or over the summary row.
2. The cursor is inside the block, on a fence row or a property row.
3. The current find match is inside the block.
4. The reader pinned the block open with a click on the summary row.

A second click on the top rule unpins it. In all other cases the block is collapsed.

A new note starts with the cursor on line 1, and line 1 is the opening fence. The block must not open on every note load. For that reason rule 2 counts only after the first cursor move or edit in the session. Until then the block stays collapsed.

## Cursor and rows

A collapsed block has one display row. Its buffer lines are hidden.

If a cursor move targets a hidden line, the block opens first and the cursor lands on the real line. Moving up from the first body line below the block lands on the closing fence row. Moving down from above lands on the opening fence row. If the cursor leaves the block and the mouse is elsewhere, the block closes again.

A click on the summary row pins the block open and puts the cursor on the first property line.

Opening or closing a block changes the row count. The viewport keeps the first visible buffer line fixed. If the block is above the viewport top, the text below does not jump.

## Hover without flicker

When the mouse enters the summary row, the block opens. The mouse is then over the first row of the open block, so the block stays open. When the mouse leaves the open block, the block closes. The rows below move up and the mouse is over a row outside the block, so the block does not reopen. No close delay is needed. The implementation adds a test for this loop.

## Where the code goes

Core adds one public function that returns the line range of the frontmatter block in a text, built on `frontmatter_bounds`. It returns a range of line numbers and the fence kind. This keeps the fence rule in one place, as the project rules require. Neither frontend parses fences.

Both editors already share `tui/src/logic/editor`, which holds the parsed buffer, the layout, `EditorDisplay` and the `EditorFrame` rows. The new logic goes there:

- A `FrontmatterBlock` value on the parsed buffer holds the line range, the keys and the open state.
- `EditorDisplay` takes a hover input, a row or none, next to the existing viewport input. It combines the hover, the cursor, the find match and the pin into the open state.
- The layout treats a collapsed block as one row. The implementation checks first whether the `RowHints` type in `ropetext` can express a row that stands for many lines. If it cannot, the layout needs a small extension.
- `render_runs` gains a role for the title rule, the property rows and the bottom rule.
- Mouse mapping in `click_at_screen` must account for the collapsed row.

All of this logic is shared, so the TUI and the GUI get the feature from one implementation. The frontends keep only two small jobs. Each painter maps the new run roles to colors, and each painter forwards its mouse move event to `EditorDisplay`. The GUI painter is `gui/src/row_paint.rs` with `gui/src/theme.rs`. The TUI painter is `role_style` in `markdown_style.rs`.

## Tests

- Core: the line range for LF and CRLF files, a byte-order mark, a `+++` block, an empty block and an unclosed block.
- Logic: the open state for each of the four rules, and the first-move rule on note load.
- Logic: the hover loop above does not flicker.
- Frame: a collapsed block gives one row. An open block gives the rule rows and the property rows. Rows below the block keep their buffer line numbers.
- Cursor: up and down moves across a collapsed block, and a click on the summary row.
- TUI screen tests and GUI tests use the existing fixtures.
- The nvim backend cannot run in the test suite. The author checks it by hand: open a note with frontmatter, move through the block, edit a property, save.

## Delivery order

1. Core: the frontmatter line range function, with tests.
2. Shared logic: the frontmatter model, the new run roles and the open box, always open. This step needs no hover and no hidden rows. The TUI painter shows the result first.
3. Shared logic: collapse, hover state, cursor entry and the pin. The TUI painter forwards mouse moves.
4. The GUI: map the new roles to colors and forward mouse moves. The GUI has no users, so its behavior can change freely.

Steps 2 and 3 build the feature once for both frontends. Step 4 is small. Each step ends with the full CI gate set, and the author checks the result before the next step starts.

## Open questions

- Does `RowHints` support a many-line row, or does the layout need a new row kind? The answer sets the size of step 3.
- Do the property rows need a right edge on the box? This spec says no, because a right edge needs padding math that conflicts with line wrap. The background rectangle gives the box shape without it.
- Does the summary list keys only, or also the first value? This spec says keys only, to keep the row short.
