# Spec: No "Create:" row in the desktop search browser

Status: draft. Applies to the GUI only.

## Problem

The desktop search browser (Ctrl/Cmd+K, leader `f g` / `f r`, the palette, a followed `#tag`) runs core's query search.
It also leads every non-blank query with a `Create: <query>` row.
A query such as `#todo "meeting notes" @modified>7d` is not a note name, so the row suggests creating a note named after query syntax.

The suggestion belongs where the text is a name: the file browser's filter and the quick-open finder.

## Decisions

| Topic | Decision |
|---|---|
| Search browser | Never shows a `Create:` row, for any query. |
| Quick-open finder (`FileFinderProvider`) | Unchanged. It matches filenames and keeps its `Create:` row. |
| File browser filter | Unchanged. The `Create:` suggestion still opens the New note dialog pre-filled with the text. |
| TUI search | Unchanged. It already has no `Create:` row. |
| Empty results | The existing no-results state. No replacement hint. |
| Selection | Starts on the first match, as today. Nothing else about selection changes. |
| Preview | Unchanged. It already shows nothing for no selection. |

## Behavior

- Typing any query in the search browser lists only matching notes. An empty query still lists the recent notes.
- A query with no matches shows no rows (the no-results state), not a `Create:` row.
- Up from the first match stays on the first match. There is no row above it.
- Creating a note from a search is done through the file browser, the quick-open finder or the New note dialog.

## Code changes

- `tui/src/logic/pickers/search.rs`: the `create_dir` field and parameter of `SearchNotesProvider`, its `leading_row` override, and `resolving_search_source_with_create` exist only for the GUI search browser. Remove them. `resolving_search_source` stays the one constructor.
- `gui/src/workspace/search.rs`: build the source with `resolving_search_source` and drop the `open_note_dir()` argument. Update the module docs (the `Create:` mentions in the **Rows**, **Preview** and **Opening** paragraphs, and the "only reads" line).
- `gui/src/workspace/search.rs` (`search_open`): the comment about "its `Create:` row is the picker's" becomes stale; reword it.
- Shared `create_row` stays: the finder and the file browser use it.

## Docs

`docs/content/using-kimun/desktop-app.md:57` says "in the FILES filter or in the note finder", which is already correct. Check that no docs page describes `Create:` in the search browser. Add nothing.

## Tests

- Replace the GUI test that asserts `Create:` leads the search rows (`gui/src/workspace/search_tests.rs` around line 340) with one that asserts no `CreateNote` row appears for a typed query, for a query that matches nothing, and for a query-syntax string.
- Update the rest of `search_tests.rs` that expects `CreateNote` (the "Nothing matches" case and the preview test that presses Up to reach `Create:`).
- Replace `the_create_variant_leads_a_typed_query_with_create` in `tui/src/logic/pickers/search.rs` with a check that the search source has no leading row.
- Keep the finder and file browser tests (`picker_tests.rs`, `files_tests.rs`, `pickers/tests.rs`) as they are. They guard the unchanged behavior.

## Non-goals

- Changing the finder's or the file browser's suggestion.
- Adding a new-note shortcut or hint to the search browser.
