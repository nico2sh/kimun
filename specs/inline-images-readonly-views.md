# Inline images in the read-only views

Date: 2026-11-23. Status: draft, waiting for the user's approval. Calls made after the builder check are in the last section.

## Intent

The GUI note editor already draws vault images in place of `![alt](path)` and `![[name]]` lines (branch `images`, PR 291). The same images must show in the three read-only surfaces that show note text: the facet view, the Preview pane, and the Source reader. A reader who scans a collection or a search result sees the picture, not the raw markdown.

Remote images come after this work. They are not part of it.

## Goals

- The facet view draws the images in every chunk.
- The Preview pane (search browser, FIND, SEM, Query panel) and the Source reader (Ask) draw the images in the note text they show.
- The three surfaces use one image core. Sizes, formats, limits, failure notes, and the cache are the same as in the editor.
- The preview keeps its row model: uniform row height, the same scroll, the same first-match anchor, the same section anchor.

## Non-goals

- No remote images.
- The Preview pane and the Source reader stay raw-text surfaces. They do not render markdown.
- No click, no right-click menu, and no FollowLink on an image in these three surfaces. A hover tooltip is the only interaction.
- No image in the terminal app.
- No downscaling at decode time. That is a separate follow-up.

## Anti-goals

- A preview whose scroll position or first-match anchor changes because an image loaded.
- A match hidden behind an image. A line that holds a search match must show its raw text.
- Text that moves when pixels arrive. The row count comes from the image header, as in the editor.

## What the user decided

- Images in all three surfaces.
- The match rule: in the Preview pane and the Source reader, an embed line that holds a search match shows its raw text with the image below it. Every other embed line shows only the image.
- Approach 1: one shared image core. The facet chunks reuse the editor path. The preview reserves N fixed-height rows under an embed line, and each row paints its own slice of the image.

## Approaches

1. **Shared image core, reserved fixed rows in the preview (chosen).** The facet chunks reuse the editor code. The preview turns an embed line into N rows of the same height as every other row. Each of the N rows paints its own slice of the image, clipped to the row. The list stays uniform, so scroll, virtual rows, and anchors do not change. The cost is one new slice-painting routine, and moving the image loader out of the editor so all surfaces share it.
2. **Variable-height rows in the preview.** One tall row per image. It paints more easily. But it replaces the uniform list with a measured list. The scroll, first-match, and section anchors all depend on uniform rows. We must rewrite them.
3. **Make the preview a read-only editor.** It reuses the editor path fully. But the preview then shows rendered markdown and not raw text. That changes what a preview is.

## How it works

### The shared core

The editor's image pieces become surface-neutral. These pieces are the embed finder and the resolver (both in core), the loader and cache (`gui/src/inline_image.rs`), the fit and row-count functions, the failure notes, and the pinning. Each surface supplies three things. These are the note path that resolves a bare name, the vault, and the pane size.

### Facet view

Each chunk is a read-only editor made with `EditorView::new_read_only`, and each chunk knows the note it came from (a collection mixes notes). A facet chunk gets the image context with its own note path. Two rules differ from the editor, because a chunk never has a caret (ADR-0062):

- There is no cursor row, so no embed line ever reveals its raw text. Every embed line shows only the image.
- A failed image shows its one-line note in the embed row, as the editor does off the cursor row.

The chunk measure already counts layout lines, and pad lines are layout lines, so a chunk grows to hold its images. The images in a chunk and the images in the open editor share one cache. When a chunk is dropped, its pins are dropped.

A chunk can drop leading indent from later lines. An embed is a whole line, so the finder works on the chunk text unchanged. The finder treats an indented line as it does in the editor.

### Preview pane and Source reader

The preview reads a note, splits it into lines, and wraps each line into fixed-height rows (`PreviewText`, `wrap_lines` in `gui/src/workspace/search.rs`). The change is in the code that makes the rows:

- An embed line is found with the core embed finder, on the capped note text.
- If the line holds a search match, its raw text rows come first, then the image rows.
- If the line does not hold a match, only the image rows follow. If the image cannot be drawn, one row shows the failure note instead, and no image rows follow.
- The image rows number `rows_for(fit(natural, column, 60% of the viewport).1, row height)`, at least one.
- The image sizes come from the headers. The background task that reads the note also reads the headers. The rows are right before the first frame. A late decode never changes a row count.
- Each image row paints its own slice. The row paints the whole image, moved up by its index times the row height, clipped to its own bounds. This works with `uniform_list`, because a row that scrolls into view draws its slice with no help from its neighbors.
- The first-match row and the Source reader's section row are found by line number, as now. They still land on the right row after rows are added.
- A row that is a slice of an image holds no text. It takes no part in the needle matching. The preview's selection and copy skip it.

The Source reader has no search words. Its match is the marked section. An embed line inside the marked section shows its raw text with the image below it, as a line with a search match does in the Preview pane. An embed line outside the section shows only the image. This follows the rule you agreed to, and the raw text is what the answer model read.

### Loading

The loader is the editor's. A surface pins the images it shows and requests a decode only for an image on screen. The visible set changes when the pane scrolls, resizes, or shows a new note. The same no-loop rule holds: a decode completion never causes a request.

### Interaction

A hover tooltip shows the alt text, or the file name for `![[name]]`. There is no click, menu, or follow. My call: these surfaces are for reading and for picking a result, and a click already means something there.

## Constraints

- Core owns every vault path and filesystem step. The GUI only reads files.
- The preview's note cap (256 KB) stays. An embed past the cap is not drawn.
- The terminal app does not change.
- The three surfaces must show the same note the same way as the editor for size, format, and failure.

## Left to the builder

- How the shared core is split into modules, and the names.
- The exact paint calls for a slice, and the clip and offset arithmetic at fractional display scales.
- The theme colors for the placeholder and the note. They are the editor's.
- Where the preview's image sizes are cached between a re-wrap at a new width and the first read.

## Docs

The user docs page for the desktop app gains two sentences: the images show in the facet view and in the previews, and a preview shows the raw line when it holds a search match. The glossary entry "Inline image" changes from "the editor group only" to name all three surfaces.

## Calls made after the builder check

These are my calls. A reviewer read the first draft and asked seven questions. None changes what you chose, so I settled them here. Please check the first one.

1. **The Source reader's match.** The marked section is the match. An embed line inside it shows raw text and the image. An embed line outside it shows only the image. The first draft contradicted itself here, and this version follows your words, "holds the marked section".
2. **Height.** The image row count uses 60% of the viewport height. The Preview pane changes height in its Collapsed, Context, and Full states and when the window resizes. The rows are made again when the width or the height changes. The preview keeps its place by the note line and byte offset of its top row, so the same text stays in view. The first-match anchor and the section anchor are found again by line number after the rows are made.
3. **Pixel width.** The background task receives the text width in pixels and the row height in pixels, next to the character column count. The text width is the column count times the monospace cell width.
4. **Selection and copy.** The preview has no text selection or copy of its own. Its "selection" is the selected result row. So nothing needs skipping. A facet chunk is an editor, so selection and copy of an image-only embed line select and copy the raw markdown, as the editor does.
5. **Tooltip.** The tooltip covers every row of an image, and shows the alt text. For `![[name]]` or an empty alt it shows the file name, as in the editor.
6. **Size mismatch.** The header read uses the editor's turned-size logic. If the decoded size still differs, the image stretches to the reserved box, as in the editor.
7. **Cost of header reads.** A note draws images for its first 100 embeds. Later embeds stay as styled or raw text. Each header read is one file open on the background task. A note with more than 100 embeds is rare.
