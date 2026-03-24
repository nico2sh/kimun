# Markdown Editor View — Design Spec

**Date:** 2026-03-24
**Status:** Approved
**Scope:** `kimun/tui` — `TextEditorComponent` and new sub-module

---

## Overview

Replace the raw `ratatui_textarea` render with a rich view layer that supports:
- **Word wrapping** — text wraps at the editor boundary instead of scrolling horizontally
- **Inline markdown rendering** — bold, italic, inline code, links
- **Block-level markdown rendering** — headings (H1–H3), blockquotes, horizontal rules
- **Element-level expand on cursor** — when the cursor is within a markdown element, that element expands to raw markdown text; all other elements stay rendered

The `TextArea` from `ratatui_textarea` is retained as the input/cursor model. A new view layer sits beside it and handles all rendering.

---

## User-Visible Behaviour

### Rendered elements

| Element | Rendered appearance | Raw when cursor inside? |
|---|---|---|
| `**bold**` | Bold + accent color | Yes — shows `**bold**` |
| `*italic*` / `_italic_` | Italic + secondary color | Yes |
| `` `code` `` | Distinct bg color, monospace | Yes |
| `[text](url)` | Underlined text, url dimmed | Yes — shows `[text](url)` |
| `# Heading` | Dimmed `#` sigil + colored bold text | Yes — shows `# Heading` |
| `## Heading` | Dimmed `##` + colored text (level 2 color) | Yes |
| `### Heading` | Dimmed `###` + colored text (level 3 color) | Yes |
| `> blockquote` | Dimmed `>` prefix + secondary fg | Yes |
| `---` / `***` (HR) | Full-width `─` rule | Yes — shows `---` |
| plain text | Theme fg color | N/A |

### Expand granularity

Element-level: only the specific markdown element whose character range contains the cursor column expands. Other elements on the same line remain rendered. The cursor must be on the same logical line and within `[element_start_char, element_end_char]` (character-count space).

### Heading style

Dimmed sigil + colored text (option D):
- `H1`: dimmed `# ` + bold + `theme.accent`
- `H2`: dimmed `## ` + bold + `theme.fg` (slightly dimmed)
- `H3`: dimmed `### ` + `theme.fg_secondary`

---

## Architecture

### Component split

```
TextEditorComponent
├── TextArea                  (ratatui_textarea — input model, cursor, undo)
└── MarkdownEditorView        (view model — layout, scroll, rendering)
    ├── WordWrapLayout        (pure computation — visual line mapping)
    └── MarkdownSpanner       (pure function — line → styled spans, backed by pulldown-cmark)
```

### File layout

```
src/components/text_editor/
  mod.rs          ← TextEditorComponent (was text_editor.rs)
  view.rs         ← MarkdownEditorView
  word_wrap.rs    ← WordWrapLayout + coordinate transforms
  markdown.rs     ← MarkdownSpanner (pulldown-cmark-backed element parsing + span emission)
```

`src/components/mod.rs` — update module path from `text_editor` file to `text_editor` directory.

---

## Component Contracts

### `WordWrapLayout`

```rust
pub struct VisualLine {
    pub logical_row: usize,
    pub start_col: usize,          // character offset (Unicode scalar) in original line where this visual line begins
    pub end_col: usize,            // character offset where it ends (exclusive)
    pub content: String,           // the substring for this visual line
    pub is_first_visual_line: bool, // true only for the first visual line of a logical row
}

pub struct WordWrapLayout {
    visual_lines: Vec<VisualLine>,
}

impl WordWrapLayout {
    /// Always produces at least one VisualLine, even for an empty input.
    pub fn compute(lines: &[String], width: u16) -> Self;
    pub fn logical_to_visual(&self, row: usize, col: usize) -> (usize, usize);
    pub fn visual_to_logical(&self, vrow: usize, vcol: usize) -> (usize, usize);
    pub fn total_visual_lines(&self) -> usize;
}
```

All coordinate spaces (VisualLine offsets, cursor positions, element boundary ranges) use **Unicode scalar value (character) counts**, matching `TextArea::cursor()` which returns character-based positions. Byte offsets are never used across component boundaries.

Word-wrap algorithm: for each logical line, greedily break on whitespace boundaries at `width` columns (character count). If a single word exceeds `width`, hard-break at `width`.

### `MarkdownSpanner`

```rust
pub struct MarkdownSpanner;

impl MarkdownSpanner {
    /// Renders a single visual line into styled spans.
    /// `logical_line` is the full original line; `content` is the visual substring.
    /// `visual_start_col` is the character offset where `content` begins in `logical_line`.
    /// `cursor_col` is `Some(col)` only when cursor's logical row matches; `col` is in logical_line char space.
    /// `is_first_visual_line` — true when this is the first visual line for the logical row
    ///   (block-level prefixes such as `#` and `>` are only rendered on the first visual line).
    /// Lifetime `'a` is tied to `content` and `logical_line` to allow borrowing substrings.
    pub fn render<'a>(
        content: &'a str,
        logical_line: &'a str,
        visual_start_col: usize,
        cursor_col: Option<usize>,
        is_first_visual_line: bool,
        theme: &'a Theme,
    ) -> Vec<Span<'a>>;
}
```

**Parsing strategy: use `pulldown-cmark`** (not a hand-rolled state machine).

`pulldown-cmark` is used to parse the full logical line (or block) and produces a sequence of `Event`s with offset ranges. These offset ranges are byte-based — they must be converted to character-count positions using `line[..byte_offset].chars().count()` before being stored as `(start_char, end_char, kind)` entries.

Source byte offsets are obtained via `Parser::into_offset_iter()`, which consumes the `Parser` and yields `(Event<'a>, Range<usize>)` tuples.

Why `pulldown-cmark` over `comrak`:
- `pulldown-cmark` emits parse events with source byte offsets via `into_offset_iter()`, making it straightforward to map events back to cursor positions.
- `comrak` produces a full AST with source positions, which is more than needed for a line-level render pass and carries more overhead.
- `pulldown-cmark` is already a common dependency in the Rust ecosystem and is lighter.

The `MarkdownSpanner` calls `pulldown-cmark` on the **full logical line**, collects `(start_char, end_char, ElementKind)` triples, then emits `Vec<Span>` for the visual substring `[visual_start_col..visual_start_col + content.chars().count()]`. Cursor overlap is checked per element: if `cursor_col` is `Some(c)` and `start_char <= c < end_char`, emit raw spans; otherwise emit styled spans.

Block-level elements (`#`, `>`, `---`) are detected at the start of the logical line. They apply only when `is_first_visual_line` is `true`. On continuation visual lines of a wrapped heading or blockquote, the content is rendered in the heading/quote style (color) but without repeating the sigil.

### `MarkdownEditorView`

```rust
pub struct MarkdownEditorView {
    layout: WordWrapLayout,
    visual_scroll_offset: usize,
    // Cached from last update() call — avoids passing lines/cursor twice per frame.
    lines_snapshot: Vec<String>,
    cursor_snapshot: (usize, usize),
}

impl MarkdownEditorView {
    pub fn new() -> Self;

    /// Call once per frame before render. Unconditionally recomputes layout on
    /// every call — notes are small, correctness trumps micro-optimization.
    /// Adjusts scroll to keep cursor visible. Guards against rect.height == 0.
    pub fn update(&mut self, lines: &[String], cursor: (usize, usize), rect: Rect);

    /// Render the editor content into `rect` using data cached by the last `update()`.
    /// Does not draw a border.
    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool);

    /// Convert a mouse click (relative to rect top-left, accounting for scroll)
    /// to a logical cursor position for passing to `CursorMove::Jump`.
    /// Returns `(row as u16, col as u16)` already clamped to `u16::MAX`.
    pub fn visual_to_logical_u16(&self, vrow: usize, vcol: usize) -> (u16, u16);
}
```

**Scroll logic in `update()`:**
0. If `rect.height == 0` → return early (no visible area, nothing to scroll into)
1. Compute `cursor_vrow = layout.logical_to_visual(cursor.0, cursor.1).0`
2. If `cursor_vrow < visual_scroll_offset` → `visual_scroll_offset = cursor_vrow`
3. If `cursor_vrow >= visual_scroll_offset + rect.height as usize` → `visual_scroll_offset = cursor_vrow - rect.height as usize + 1`

**Layout invalidation:** unconditional — `WordWrapLayout::compute` is called on every `update()`. Notes are small; this is correct and simple.

**Cursor rendering:** after assembling and rendering the `Paragraph`, call:
```rust
let (cursor_vrow, visual_col) = layout.logical_to_visual(cursor.0, cursor.1);
if focused {
    f.set_cursor_position(Position {
        x: rect.x + visual_col as u16,
        y: rect.y + (cursor_vrow - visual_scroll_offset) as u16,
    });
}
```
(`ratatui::layout::Position` is required by `Frame::set_cursor_position` in ratatui ≥ 0.28.)

The `Paragraph` is rendered with wrapping disabled (no `Wrap` widget option set). Wrapping is already handled by `WordWrapLayout`, so the paragraph receives one pre-wrapped visual line per `Line`. Lines that exceed `rect.width` (which should not occur in correct operation) are silently truncated by ratatui's `LineTruncator`.

### `TextEditorComponent` changes

- Rename `text_editor.rs` → `text_editor/mod.rs`
- Add field: `view: MarkdownEditorView`
- In `render()`: call `self.view.update(self.text_area.lines(), self.text_area.cursor(), rect)` then `self.view.render(...)`
- In mouse handler: replace `CursorMove::Jump(row, col)` with coords from `self.view.visual_to_logical_u16(vrow, vcol)`

---

## Data Flow

### Key input → render

```
InputEvent::Key → TextArea.input(key)
                        ↓
                 lines(), cursor() extracted
                        ↓
         MarkdownEditorView.update(lines, cursor, rect)
           → WordWrapLayout::compute(lines, width) — unconditional
           → visual_scroll_offset adjusted
                        ↓
         MarkdownEditorView.render(f, rect, theme, focused)  // lines & cursor from self.lines_snapshot / cursor_snapshot
           → for vrow in [scroll_offset .. scroll_offset + rect.height]:
               VisualLine { logical_row, start_col, content, is_first_visual_line }
               cursor_col = if logical_row == cursor.0 { Some(cursor.1) } else { None }
               spans = MarkdownSpanner::render(content, lines[logical_row], start_col, cursor_col, is_first_visual_line, theme)
           → Paragraph::new(Text::from(lines_as_spans))
           → f.set_cursor_position(visual position)
```

### Mouse click → cursor move

```
MouseEventKind::Down at (row, col)
  → in_bounds check
  → (vrow, vcol) = (row - rect.y + scroll_offset, col - rect.x)
  → (logical_row_u16, logical_col_u16) = view.visual_to_logical_u16(vrow, vcol)
  → TextArea.move_cursor(CursorMove::Jump(logical_row_u16, logical_col_u16))
```

`CursorMove::Jump` takes `(u16, u16)`. `visual_to_logical_u16` returns values already cast and clamped to `u16::MAX`, which mirrors `CursorMove::Jump`'s own internal clamping behaviour.

---

## Testing Strategy

- **`WordWrapLayout`**: unit tests for wrapping at boundary, single-word overflow, empty lines, unicode, coordinate round-trips (`logical → visual → logical`).
- **`MarkdownSpanner`**: unit tests for each element type (bold, italic, code, link, heading, blockquote, HR), cursor-inside vs cursor-outside expand, overlapping/nested markers.
- **`MarkdownEditorView`**: unit tests for scroll clamping, empty document (`lines = []` and `lines = [""]`), `rect.height == 0` guard, and that `visual_to_logical_u16` correctly accounts for `visual_scroll_offset`.
- **`TextEditorComponent`**: existing tests unchanged.

---

## Out of Scope

- Nested markdown (bold inside a link, etc.) — not supported in v1; parser is linear
- H4–H6 headings — rendered as H3 style
- Ordered/unordered lists — not styled in v1 (rendered as plain text)
- Selection highlight over styled spans — selection background from `TextArea` is not used; selection rendering is deferred
- Undo/redo — delegated entirely to `TextArea`, unaffected by this change
