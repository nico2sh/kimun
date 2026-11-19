+++
title = "Facet view"
weight = 18
+++

# Facet view

A facet view shows one note in read-only form. You pick headings, tags, and note links from a bar, and the view shows only the parts of the note that belong to them. Use it to read one topic out of a long note, such as a journal.

The facet view is in the [desktop app](@/using-kimun/desktop-app.md). The terminal app does not have it.

## Open a facet view

Use any of these:

- Click the funnel button in the header of the pane. A lone pane has no header: the button sits in its top right corner, next to the note menu button. Its tooltip says `Facet view`.
- Open the [note menu](@/using-kimun/desktop-app.md#the-note-menu): right-click in the pane, or press `F2`. Pick `Facet view`.
- Open the command palette and run `Toggle facet view`. It has no key by default.

The pane now shows the note as read-only text. The funnel button looks pressed, and its tooltip says `Back to the editor`. When a tab has two panes or more, each pane has a header, and a pane that shows a facet view has a small funnel before its title there. A lone pane has no header: its corner button looks pressed instead. When the window has two tabs or more, a tab whose pane shows a facet view also has a small funnel before its title. To go back, click the funnel button again, run `Toggle facet view`, or open the note menu and pick `Edit note`.

Kimün saves the text of the note before it opens the view. If another pane changes and saves the note, the facet view updates. When you start Kimün again, the pane opens as a facet view with the same picks.

## Pick facets

A bar at the top of the view lists the facets of the note. A facet is a tag, a link to another note, or a heading. The bar stays in view when you scroll the text. Each kind has its own row, in this order: tags, links, headings. An icon starts each row. Hover it to see the name: `Tags`, `Links`, or `Sections`.

- Click a pill to pick it. Click it again to remove the pick.
- With nothing picked, the view shows the whole note. Above the text, a line says `Pick a tag, link or section to narrow this note.` It goes away when you make your first pick.
- A pick of a tag or a link shows the paragraph, list item, or table row that holds it. A pick of a heading shows the whole section under it, with its sub-headings.
- Picks of the same kind add up: the view shows the parts that match any of them. Picks of different kinds narrow the view: it shows only the parts that match every kind that you picked.
- A row shows 12 pills. If the note has more tags or links, the row ends with `+N more`. If it has more than 12 headings, the row shows only the headings that you picked, then `Sections N`, where N is the number of headings.
- Click `+N more` or `Sections N` to open a list of all the facets of that kind. Type in `Filter` to search it. Click a pill to pick it. The list stays open for your next pick. Press `Esc` or click outside the list to close it. A click outside the list also does what that click does: for example, a click on a pill in the bar also picks it or removes its pick.
- The search button at the top right of the bar opens the same kind of list for all kinds at once. Its tooltip says `Search facets (/)`.
- A note with no tags, links, or headings has no bar.

### The summary line

While you have picks, a line at the top of the bar tells you what the view shows, for example `Showing text tagged #run or #garden, in 2026-07-03 · 12 pieces`. The `Clear` button at the right end of that line removes all picks. When the picks match nothing, the line has `Clear` only.

### When nothing matches

If no part of the note belongs to all the picked kinds, the view says which combination is empty, for example `No text tagged #run and in 2026-07-03.` Click `Remove last pick` under it to take back your latest pick.

If the note has no text, the view says `This note is empty.`

## Click in the text

- Click a tag, a note link, or a heading in the text to pick it, as you would click its pill.
- Hold `Ctrl` (`Cmd` on macOS) and click a link or a tag to follow it. A link to a note opens that note in a new tab. A link to an attachment opens it in a new tab. A link to a web address opens in your browser. A tag opens a search for that tag.
- The view shows the note as pieces of text. A piece is one block, or several blocks that are next to each other. For example, a heading section is one piece. A piece has no frame. A slim row above it holds the breadcrumb and the `Open source` button.
- The breadcrumb lists the headings that contain the piece, from the outside in, with `›` between them, for example `Summer › July › Week 3`. Click a heading in it to pick that heading. A piece that sits under no heading has no breadcrumb. If the breadcrumb is too long for the row, Kimün shortens the headings with `…`, the outer ones first, so that the last heading stays readable for as long as possible. `Open source` always shows in full.
- `Open source` opens the note in a new tab. If the note is already open in a pane, Kimün goes to that pane instead. The note opens at the top, not at the piece.

## Show hidden blocks

When you pick a facet, the view can skip blocks of the note. A block is a paragraph, a list item, a table row, or a similar part. Between two shown pieces, and before the first and after the last, a line tells you how many blocks are hidden, for example `3 hidden`.

Click the down arrow on that line to show the block after the one above it. Click the up arrow to show the block before the one below it. The first line has only an up arrow and the last has only a down arrow. Each click shows one block. Hover an arrow to see its tooltip: `Show one more below` or `Show one more above`.

When a line hides 3 blocks or fewer, it also has a `Show all` button. It shows all of them at once.

## Use the keyboard

These keys work in the facet view, also when the keyboard is inside a piece of text. The keyboard goes into a piece when you click in its text.

| Key | What it does |
| --- | --- |
| `/` | Opens a search over all the facets of the note. If the note has no facets, it does nothing. |
| `Up`, `Down` | In a list, move to the previous or next facet. |
| `Enter` | In a list, picks the facet that is lit, or removes its pick. The list stays open. |
| `j`, `k` | Go to the next or previous piece. |
| `Esc` | Closes the open list. If no list is open and the keyboard is inside a piece, it takes the keyboard out of the piece, and your picks stay. Otherwise it clears all picks, and with no picks it does nothing. |
| `Tab`, `Shift+Tab` | Move to the next or previous button: pills, `Clear`, the breadcrumb headings, `Open source`, and the hidden-block buttons. From inside a piece, they go to the button after or before the text of that piece. The button that has the keyboard has a ring. |
| `Enter`, `Space` | Press the button that has the keyboard. |

While you type in the `Filter` field of a list, letters go to the field.

## Limits

- The view is read-only. To change the note, go back with `Edit note`.
- It shows one note.
- It draws at most 100 pieces of text at a time. If there are more, the last row says `Show N more`, where N is how many pieces the next click shows. Each click doubles the limit.
- Kimün finds facets in the text of the note. A tag in a code block or in the front matter is not a facet. Links to web addresses and to attachments are not facets.
