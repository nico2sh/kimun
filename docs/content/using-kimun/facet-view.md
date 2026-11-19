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
- A facet view of a note shows one note. To read many notes at once, open a [collection](#collections).
- It draws at most 100 pieces of text at a time. If there are more, the last row says `Show N more`, where N is how many pieces the next click shows. Each click doubles the limit.
- Kimün finds facets in the text of the note. A tag in a code block or in the front matter is not a facet. Links to web addresses and to attachments are not facets.

## Collections

A collection is the facet view of many notes. It shows the notes that a query returns, one after the other, as one document. The same bar narrows all of them at once. Use it to read one topic across many notes, for example everything tagged `#run`.

### Open a collection

Open FIND and type a query. Then use one of these:

- Click the `Open as collection` button in the tool bar of FIND.
- Open the command palette and run `Open results as collection`. FIND must have the keyboard, or the palette says `Needs FIND focused`.

Neither has a key by default. Both use the query that is now in FIND, not the results that you see. If FIND has no query, a message says `No query to open`. If the query uses `{note}` and the pane has no open note, a message says `This query needs an open note.` The query is filled in with the open note when the collection opens, and it stays so.

The collection opens in the current pane and replaces what the pane shows. Kimün saves the text of the pane's own note first. The pane header, and the tab, show the query as the title.

### What a collection shows

- With nothing picked, the view shows every note whole, in the order of the results. Above the text, a line says `Pick a tag, link or section to narrow these notes.`
- The bar lists the tags, the links, and the headings of all the notes together. A tag that is in 3 notes shows once, with its counts added up. Headings with the same text are one pill.
- A pick narrows each note as it does in a facet view. A note that has no match shows nothing. The summary line adds the notes, for example `Showing text tagged #run · 12 pieces in 5 notes`.
- Each piece has a row above it with a breadcrumb. In a collection, the first part of the breadcrumb is the title of the note, for example `Run log › Summer › Week 3`. It is plain text, not a button. When the row is too narrow, the title is cut with `…` together with the outer headings.
- `Open source` opens the note that the piece belongs to.
- A line of hidden blocks never joins two notes. `Show one more below`, `Show one more above`, and `Show all` work inside one note.
- If the notes have no text, the view says `These notes are empty.`

### Limits of a collection

- A collection holds at most 500 notes. When more than 500 notes match, the collection holds the first 500 in the order of the results, and a line under the bar says `Showing the first 500 of N notes`, where N is how many match. The line shows only when the query hit this limit. When you delete a note of the list, both numbers go down by one.
- The 100-piece limit counts the pieces of all the notes together. The last row says `Show N more`, as in a single note.
- A note that Kimün cannot read is left out, and a message says `Could not read 1 note`, or `Could not read N notes` when there are more.

### Refresh

The list of notes is fixed when the collection opens. A new note that matches the query does not join it, and a note that stops matching does not leave it. To run the query again, use one of these:

- Click the `Refresh results` button. It takes the place of the funnel button in the pane header. A lone pane has no header: the button sits in its top right corner.
- Press `r` when the keyboard is in the view.
- Run `Refresh collection` in the command palette. It has no key by default. In a pane that shows no collection, it says `This pane shows no collection`.

A refresh keeps your picks that still exist and drops the others. Blocks that you showed with the arrows or `Show all` are hidden again, and the view shows the first 100 pieces again.

The text of each note is not fixed. When you save a note in another pane, the collection shows the new text of that note. When you rename or move a note in the list, the collection follows it. When you move a folder, the notes under it follow. When you delete a note, it leaves the list.

### A pane that shows a collection

A collection pane is not a note pane:

- There is no editor to go back to. The funnel button is `Refresh results`, and `Toggle facet view` says `No editor to go back to`.
- The note menu has the layout items only: `Split right`, `Split down`, and `New tab`. It also has `Close pane` when the tab has two panes or more, and `Close tab` when the window has two tabs or more.
- To change a note, click `Open source` on one of its pieces and edit it in its own tab.

When you start Kimün again, the pane opens as a collection with the same picks, and the query runs again. The list can differ from the one you left, because the notes may have changed. A pick that none of the notes in the new list has is dropped, for example when the query now returns no notes. If the query has no results, the view says `No notes match this query.` and has a `Refresh` button.

If the search itself fails when Kimün starts (an error, not a query with no results), the pane stays empty and a message says `Couldn't run`, followed by the query and the error. Kimün keeps the collection in its saved session until the next change to your tabs, your panes, or what they show. After that change, it saves the pane as empty.
