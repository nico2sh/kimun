# Block-level editor decorations live at the view layer

Most markdown styling in the TUI editor is per-line: the spanner maps each
`ElementKind` to a `Style` in `span_style`, looking only at the current line.
Two decorations cannot work that way — the blockquote **bar** (one `│` per
nesting depth, repeated on wrapped continuation rows) and the **code box** (a
background sized to the widest line of the whole block). Both need cross-line
context the per-line spanner does not have: the bar's continuation gutter spans
visual rows of one logical line, and the box width is the max over sibling
lines of the block.

We compute these at the view layer (alongside `fence_ranges`) and feed the
result down to the spanner, rather than expressing them as `ElementKind`s.

## Considered Options

- **Per-line in the spanner (rejected).** Simpler and uniform with all other
  styling, but structurally cannot produce depth bars across wrapped rows or a
  content-width box, because neither is knowable from a single line.
- **View-layer block decorations (chosen).** A small amount of block-level state
  threaded into rendering, in exchange for decorations that respect wrapping and
  block geometry.

## Consequences

The blockquote bar's continuation gutter is the first editor feature to add a
left decoration to *wrapped continuation rows*. That gutter consumes columns
that map to no logical character, so wrap-width calculation and the
rendered-column ↔ logical-column mapping (cursor placement, mouse-click mapping)
must account for it on non-first visual rows — the same accounting the existing
image-placeholder `placeholder_width` already does for first rows.
