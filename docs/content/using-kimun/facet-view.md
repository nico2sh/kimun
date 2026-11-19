+++
title = "Facet view"
weight = 18
+++

# Facet view

A facet view shows one note in read-only form. You pick headings, tags, and note links from a bar, and the view shows only the parts of the note that belong to them. Use it to read one topic out of a long note, such as a journal.

The facet view is in the [desktop app](@/using-kimun/desktop-app.md). The terminal app does not have it.

## Open a facet view

1. Open the note in a pane.
2. Open the [note menu](@/using-kimun/desktop-app.md#the-note-menu): right-click in the pane, or press `F2`.
3. Pick `Facet view`.

The pane now shows the note as read-only text. To go back, open the note menu and pick `Edit note`.

Kimün saves the text of the note before it opens the view. If another pane changes and saves the note, the facet view updates. When you start Kimün again, the pane opens as a facet view with the same picks.

## Pick facets

A bar above the text lists the facets of the note. A facet is a heading, a tag, or a link to another note. Each kind has its own row of pills.

- Click a pill to pick it. Click it again to remove the pick.
- With nothing picked, the view shows the whole note.
- A pick of a tag or a link shows the paragraph, list item, or table row that holds it. A pick of a heading shows the whole section under it, with its sub-headings.
- Picks of the same kind add up: the view shows the parts that match any of them. Picks of different kinds narrow the view: it shows only the parts that match every kind that you picked.
- A row shows 12 pills. If the note has more, the row ends with `+N more`. Click it to open the full list, and type in `Filter` to search it. Click a pill in the list to pick it.

## Click in the text

- Click a tag, a note link, or a heading in the text to pick it, as you would click its pill.
- Hold `Ctrl` (`Cmd` on macOS) and click a link or a tag to follow it. A link to a note opens that note in a new tab. A link to an attachment opens it in a new tab. A link to a web address opens in your browser. A tag opens a search for that tag.
- The view shows the note as pieces of text. A piece is one block, or several blocks that are next to each other. For example, a heading section is one piece. Each piece has an `Open source` button. It opens the note in a new tab. If the note is already open in a pane, Kimün goes to that pane instead. The note opens at the top, not at the piece.

## Show hidden blocks

When you pick a facet, the view can skip blocks of the note. A block is a paragraph, a list item, a table row, or a similar part. Between two shown pieces, and before the first and after the last, a line tells you how many blocks are hidden, for example `3 hidden blocks`.

Click the down arrow on that line to show the block after the one above it. Click the up arrow to show the block before the one below it. The first line has only an up arrow and the last has only a down arrow. Each click shows one block.

## Limits

- The view is read-only. To change the note, go back with `Edit note`.
- It shows one note.
- It draws at most 100 pieces of text at a time. If there are more, the last row says `Show N more`, where N is how many pieces the next click shows. Each click doubles the limit.
- Kimün finds facets in the text of the note. A tag in a code block or in the front matter is not a facet. Links to web addresses and to attachments are not facets.
