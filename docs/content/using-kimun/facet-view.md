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

At the top of the view, a title row shows a small, dim `Note` and, under it, the title of the note in bold. It changes when you rename the note or change its first line. So you can see what the view shows also when the window shows no tabs.

Kimün saves the text of the note before it opens the view. If another pane changes and saves the note, the facet view updates. When you start Kimün again, the pane opens as a facet view with the same picks.

## Pick facets

A bar at the top of the view lists the facets of the note. A facet is a tag, a link to another note, or a heading. The bar stays in view when you scroll the text.

The top row of the bar has one chip for each kind that still has facets to pick: `Tags N`, `Links N`, and `Sections N`. N is the number of facets of that kind that give a result in the view now. A chip with an N of 0 is dimmed, but it still opens its list. A kind with nothing left to pick has no chip, unless its list is open. The chip of the open list is drawn selected. At the right end of the row is the search chip: a magnifier icon and `Find`. Hover it to see its tooltip: `Find (/)`. The row never wraps.

Click a chip to open the list of the facets of that kind. The list starts with a `Filter` field. Type in it to narrow the list. Each row shows the name of a facet at the left. A tag reads `#name`. A link reads `[[name]]`. The right end of the row shows how many pieces the facet gives in the view now.

- A check mark before the name means that you picked the facet. Picked rows come first.
- Rows that give no result are dimmed. You can still pick them. In the tag and link lists, rows with no result come last. Sections stay in note order, and the dimmed ones stay in place.
- The order of the rows is set when the list opens and when you change the `Filter` text. It stays the same while you pick: a row that you pick or unpick keeps its place and shows or loses its tick there. Only the counts change. Open the list again to see your picks first.
- The count leaves out your picks of the same kind. So it tells you what you get when you pick that facet now.
- Click a row, or press `Enter` on it, to pick the facet or to remove the pick. The list stays open.

The search chip, and the `/` key, open one list of all the kinds. It groups the facets under `Tags`, `Links`, and `Sections`.

When the `Filter` text matches nothing, the list says `No tags match "xyz"`. The lists of links and sections say `No links match "xyz"` and `No sections match "xyz"`. The list of all the kinds says `No matches for "xyz"`.

A list shows at most 200 matches. When there are more, the last line says `N more. Type to filter.` Type in the `Filter` field to find the others.

A list stays open while you pick, so you can make more picks. It stays open while it holds any facet, even when every row has a tick. Press `Esc` or click outside the list to close it. A click outside the list also does what that click does. For example, a click on a pick in the bar removes it.

The facets that you picked show in their own row under the chips, in the order that you picked them. Each has a small remove mark. A heading that you picked shows its text with the outline icon. The row wraps onto more lines when it needs them. The bar takes at most 40 percent of the height of the view. When it is taller, it scrolls inside itself.

- Click a picked facet, or its remove mark, to remove the pick.
- With nothing picked, the view shows the whole note. Under the bar, a line says `Pick a tag, link or section to narrow this note. Press / to find. Press j or k to move between pieces. Click a tag or link to open it.` It goes away when you make your first pick.
- A pick of a tag or a link shows the paragraph, list item, or table row that holds it. A pick of a heading shows the whole section under it, with its sub-headings.
- Picks of the same kind add up: the view shows the parts that match any of them. Picks of different kinds narrow the view: it shows only the parts that match every kind that you picked.
- A note with no tags, links, or headings has no bar.

### The summary line

While you have picks, a line under the row of picks tells you what the view shows, for example `Showing text tagged #run or #garden, and in 2026-07-03 · 12 pieces`. The `Clear` button at the right end of that line removes all picks. When the picks match nothing, the line has `Clear` only.

### Clear and undo

`Clear` and `Esc` remove all picks. The summary line then says `Picks cleared` and shows an `Undo` button for 8 seconds. Click `Undo`, or press `Ctrl+Z` (`Cmd+Z` on macOS), to bring the picks back. A new pick or a removed pick takes the button away.

`Ctrl+Z` works also after the button is gone. Press it again to go back further, up to 20 sets of picks. While the cursor is in the `Filter` field, `Ctrl+Z` undoes the typing instead. Kimün does not save cleared picks. They are gone when you close the app.

### When nothing matches

If no part of the note belongs to all the picked kinds, the view says which combination is empty, for example `No text tagged #run, and in 2026-07-03.` Click `Remove last pick` under it to take back your latest pick.

If the note has no text, the view says `This note is empty.`

## Click in the text

- The text of a piece is read-only text. A click in it gives the piece the keyboard. Drag to select text. Double-click plain text to select a word. `Ctrl+C` (`Cmd+C` on macOS) copies the selection of the piece that has the keyboard. A drag across a link copies the link as it is written in the note, for example `[text](note.md)`. When the piece loses the keyboard, for example when you press `Esc` or `Tab` or click somewhere else, its selection is cleared. A click on plain text does not change your picks. To filter, use the chips, the lists, the picked facets, and the breadcrumb buttons.
- Click a link or a tag to follow it. Kimün follows it when you release the button. A link to a note opens that note in a new tab. A link to an attachment opens it in a new tab. A link to a web address opens in your browser. A tag opens a search for that tag.
- A drag that leaves the link or the tag, or that moves more than a few pixels, selects text and follows nothing. So you can drag across part of a long link to copy it. A double click on a link or a tag does not select the word, because the first click already followed. `Ctrl+click` (`Cmd+click` on macOS) follows too. The mouse cursor over every tag and link is a pointing hand.
- Right-click a link or a tag (also called a label) to open the [link menu](@/using-kimun/desktop-app.md#the-link-menu). In a facet view, it has no plain Open item.
- In a normal editor, a plain click on a link or a tag only places the cursor.
- The view shows the note as pieces of text. A piece is one block, or several blocks that are next to each other. For example, a heading section is one piece. A piece has no frame. A slim row above it, a little lighter or darker than the view, holds the breadcrumb and the `Open note` button. The breadcrumb starts at the same left edge as the text of the piece.
- The breadcrumb lists the headings that contain the piece, from the outside in, with `›` between them, for example `Summer › July › Week 3`. The headings are quieter than the text. Click a heading in it to pick that heading, or to remove its pick. A heading that you picked has a filled background. A piece that sits under no heading has no breadcrumb. If the breadcrumb is too long for the row, Kimün shortens the headings with `…`, the outer ones first, so that the last heading stays readable for as long as possible. `Open note` always shows in full.
- `Open note` opens the note in a new tab. If a pane already shows the note, Kimün goes to that pane instead. In both cases the cursor and the view go to the first line of the piece.

## Show hidden blocks

When you pick a facet, the view can skip blocks of the note. A block is a paragraph, a list item, a table row, or a similar part. Between two shown pieces, and before the first and after the last, a line tells you how many blocks are hidden, for example `3 blocks hidden`. For one block, it says `1 block hidden`.

Two buttons show hidden blocks, one block for each click. Each has an arrow that points the way it shows text:

- The button with the down arrow sits on the line with the count, right under the piece above. It shows the block after that piece. Its tooltip is `Show next block`.
- The button with the up arrow sits under a piece's headings, right above its text. It shows the block before that text. Its tooltip is `Show previous block`.

Before the first piece of a note, nothing is above to extend. There, the count moves to the line under the headings, next to the button. After the last piece, the line has only the `Show next block` button.

When a line hides 3 blocks or fewer, it also has a `Show all` button. It shows all of them at once. It is always next to the count.

## Use the keyboard

These keys work in the facet view, also when the keyboard is inside a piece of text. The keyboard goes into a piece when you click in its text.

| Key | What it does |
| --- | --- |
| `/` | Opens the list of all the kinds, the same list as the search chip. If every facet is picked, it does nothing. |
| `Up`, `Down` | In a list, move the light one row up or down. The light also passes over rows that have a tick. |
| `Enter` | In a list, picks the facet that is lit, or removes its pick when it has a tick. The list stays open, and the row and the light stay where they are. |
| `j`, `k` | Go to the next or previous piece. |
| `Esc` | Closes the open list. If no list is open and the keyboard is inside a piece, it takes the keyboard out of the piece, and your picks stay. Otherwise it clears all picks, and with no picks it does nothing. |
| `Ctrl+Z` (`Cmd+Z` on macOS) | Brings back the picks that you cleared. Press it again to go back further. |
| `Tab`, `Shift+Tab` | Move to the next or previous button. The order is the kind chips, the search chip, the picked facets, `Clear` (or `Undo` after a clear), the breadcrumb headings, `Open note`, and the hidden-block buttons. From inside a piece, they go to the button after or before the text of that piece. |
| `Enter`, `Space` | Press the button that has the keyboard. On a kind chip or the search chip, this opens the list, or closes it when it is open. On a picked facet, it removes the pick. On `Clear`, it clears the picks and moves the keyboard to `Undo`. |

When you remove a pick with the keyboard, the keyboard moves to the next picked facet. If there is none, it moves to the one before. If no pick is left, it goes to the view. When the keyboard moves to a picked facet that is below the visible part of the bar, the bar scrolls to show it.

Every button, chip, and picked facet that has the keyboard shows a ring of 2 px around it.

While you type in the `Filter` field of a list, letters go to the field.

## Limits

- The view is read-only. To change the note, go back with `Edit note`.
- A facet view of a note shows one note. To read many notes at once, open a [collection](#collections).
- It draws at most 100 pieces of text at a time. If there are more, the last row says `Show N more`, where N is how many pieces the next click shows. Next to it, the row says how many pieces are drawn, for example `100 of 340 pieces shown`. Each click doubles the limit.
- Kimün finds facets in the text of the note. A tag in a code block or in the front matter is not a facet. Links to web addresses and to attachments are not facets.

## Collections

A collection is the facet view of many notes. It shows the notes that a query returns, one after the other, as one document. The same bar narrows all of them at once. Use it to read one topic across many notes, for example everything tagged `#run`.

### Open a collection

Open FIND and type a query. Then use one of these:

- Click the `Open as collection` button in the tool bar of FIND.
- Open the command palette and run `Open results as collection`. FIND must have the keyboard, or the palette says `Needs FIND focused`.

Neither has a key by default. Both use the query that is now in FIND, not the results that you see. If FIND has no query, a message says `No query to open`. If the query uses `{note}` and the pane has no open note, a message says `This query needs an open note.` The query is filled in with the open note when the collection opens, and it stays so.

The collection opens in the current pane and replaces what the pane shows. Kimün saves the text of the pane's own note first. The pane header, and the tab, show the query as the title. The title row at the top of the view shows a small, dim `Collection` and, under it, the query in bold. A long query is shortened with `…`. Hover it to see the full query.

### What a collection shows

- A line under the bar, the collection line, says what the collection holds. With nothing picked, it reads `23 notes`. With picks, it reads `12 pieces in 5 of 23 notes`.
- The collection line adds ` · first 500 of N` when the query hit the 500-note limit. It ends with ` · list from 10:42`, the time when the list was made. When the 100-piece limit hides pieces, the `Show N more` row at the end says so.
- While a query runs, the line says `Loading…` and the old text stays. For 6 seconds after a refresh, the line says what changed, for example `Refreshed: 2 notes added, 1 removed`. Dropped picks add ` · 1 pick dropped`. A pick, a removed pick, `Clear`, or an undo ends this message at once.
- With nothing picked, the view shows every note whole, in the order of the results. Above the text, a line says `Pick a tag, link or section to narrow these notes. Press / to find. Press j or k to move between pieces. Click a tag or link to open it.`
- The bar lists the tags, the links, and the headings of all the notes together. A tag that is in 3 notes shows once, with its counts added up. Headings with the same text are one facet.
- A pick narrows each note as it does in a facet view. A note that has no match shows nothing. The summary line names the picks, for example `Showing text tagged #run`. The collection line counts the pieces and the notes.
- Each note starts with a dateline: the title of the note, a little to the left of the text, a thin line that fills the rest of the row, and a muted text at the right end. That text is the folder of the note and the number of pieces of the note that the view shows, for example `journal · 2 pieces`. The 100-piece limit can make this number smaller. A note in the root of the vault has the count only. When the row is narrow, Kimün shortens the folder with `…` first. There is space above each dateline except the first. When you press `j` or `k` to go to the first piece of a note, its dateline shows.
- Each piece has a row above it with a breadcrumb. In a collection, the breadcrumb has only the headings inside the note, for example `Summer › Week 3`. It does not have the title of the note: the dateline has it. A piece that sits under no heading has no breadcrumb.
- `Open note` opens the note that the piece belongs to, at the first line of the piece.
- A line of hidden blocks never joins two notes. `Show next block`, `Show previous block`, and `Show all` work inside one note.
- If the notes have no text, the view says `These notes are empty.`

### Limits of a collection

- A collection holds at most 500 notes. When more than 500 notes match, the collection holds the first 500 in the order of the results, and the collection line says `first 500 of N`, where N is how many match. The line shows this only when the query hit this limit. When you delete a note of the list, both numbers go down by one.
- The 100-piece limit counts the pieces of all the notes together. The last row says `Show N more` and `100 of 340 pieces shown`, as in a single note.
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
- To change a note, click `Open note` on one of its pieces and edit it in its own tab.

When you start Kimün again, the pane opens as a collection with the same picks, and the query runs again. The list can differ from the one you left, because the notes may have changed. A pick that none of the notes in the new list has is dropped, for example when the query now returns no notes. If the query has no results, the view says `No notes match this query.` and has a `Refresh` button.

If the search itself fails when Kimün starts (an error, not a query with no results), the pane stays empty and a message says `Couldn't run`, followed by the query and the error. Kimün keeps the collection in its saved session until the next change to your tabs, your panes, or what they show. After that change, it saves the pane as empty.
