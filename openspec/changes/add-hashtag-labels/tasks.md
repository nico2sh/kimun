## 1. Database schema

- [ ] 1.1 Bump `VERSION` from `"0.5"` to `"0.6"` in `core/src/db/mod.rs` with a comment explaining the labels table addition.
- [ ] 1.2 Extend `create_tables()` in `core/src/db/mod.rs` to create the `labels` table with composite primary key `(name, path)` and the `labels_by_name` and `labels_by_path` indices.
- [ ] 1.3 Add a unit test that opens a fresh DB and asserts the `labels` table and both indices exist.

## 2. Label extraction in core

- [ ] 2.1 In `core/src/note/content_extractor.rs`, change the markdown event walk so it collects hashtag matches from `Event::Text` events only when not inside a `Code` or `CodeBlock` tag.
- [ ] 2.2 Expose extracted labels as a new field on the indexing result (e.g., extend `MarkdownNote` / the ingest tuple with a `labels: Vec<String>`), lowercased and deduped.
- [ ] 2.3 Add unit tests: single hashtag, duplicate hashtags collapse, hashtag inside ``` ` ` ``` ignored, hashtag inside fenced code ignored, `#tag-with-dash` yields `tag`, mixed case yields one normalized entry.

## 3. Label persistence in core DB layer

- [ ] 3.1 Add an internal helper in `core/src/db/mod.rs` (or a sibling module) that takes `(path, labels)` and runs `DELETE FROM labels WHERE path = ?` followed by `INSERT OR IGNORE INTO labels (name, path)` for each label, inside the same transaction as the existing per-note index update.
- [ ] 3.2 Wire the helper into the note ingest path so every reindex of a note rebuilds its label rows.
- [ ] 3.3 Wire label cleanup into the note-deletion path so `DELETE FROM labels WHERE path = ?` runs when a note row is removed.
- [ ] 3.4 Add integration tests against a temp DB: indexing a note populates labels, removing a hashtag from a note removes the row on reindex, deleting a note removes its labels.

## 4. Core API for label lookup

- [ ] 4.1 Add `NoteVault::list_labels() -> Result<Vec<String>>` in `core/src/lib.rs` that returns each distinct label name from the labels table.
- [ ] 4.2 Add `NoteVault::notes_with_label(&str) -> Result<Vec<VaultPath>>` that lowercases its argument and queries by `labels.name`.
- [ ] 4.3 Add tests for both methods, including case-insensitive lookup and unknown-label returning empty.

## 5. Search syntax: `#` and `lb:` filters

- [ ] 5.1 In `core/src/db/search_terms.rs`, add `Label` and `ExcludedLabel` variants to `ElementType`.
- [ ] 5.2 Extend `prefix_table()` with `("lb:-", "#-", … ExcludedLabel)` and `("lb:", "#", … Label)`, placed so longest-prefix-first matching still holds.
- [ ] 5.3 Add `labels: Vec<String>` and `excluded_labels: Vec<String>` fields to `SearchTerms` and populate them in `from_query_string`, lowercasing each term and dropping empty ones.
- [ ] 5.4 Add unit tests for `from_query_string`: `#tag`, `lb:tag`, `#Tag` (normalized), `-#tag`, `lb:-tag`, two labels combined, empty `#`.

## 6. Search SQL: label join

- [ ] 6.1 Add `add_labels_query` step in `core/src/db/mod.rs::build_search_sql_query_inner` that joins `labels` on `notes.path = labels.path` and constrains `labels.name = ?` for each positive label (one join per label, AND semantics).
- [ ] 6.2 For each excluded label, append `notes.path NOT IN (SELECT path FROM labels WHERE name = ?)`, mirroring the existing exclusion shape.
- [ ] 6.3 Add tests that exercise the SQL builder with label-only queries, mixed label + free-text queries, and verify that the labels query uses the `labels_by_name` index (e.g., via `EXPLAIN QUERY PLAN`).
- [ ] 6.4 Add end-to-end test through `NoteVault::search_notes` that creates two notes with overlapping/non-overlapping labels and asserts AND semantics.

## 7. TUI rendering of labels

- [ ] 7.1 Add a `Label { name: String }` variant to `ElementKind` in `tui/src/components/text_editor/markdown.rs`.
- [ ] 7.2 Update the line parser that produces `ParsedLine` to emit `Label` spans for hashtag matches outside code spans (skip ``` ` ` ``` and fenced blocks).
- [ ] 7.3 Add a theme key for `label` (with fallback to the link color) and apply it in the editor render path.
- [ ] 7.4 Add a snapshot or unit test for the line parser asserting `#rust` produces a `Label` span and `` `#rust` `` does not.

## 8. Follow-link to search modal

- [ ] 8.1 In `tui/src/components/note_browser/mod.rs`, extend the modal's open/construct path to accept an optional initial query string; on open, pre-fill the query input, set cursor to end, and synchronously run the first query.
- [ ] 8.2 In `tui/src/app_screen/editor.rs::follow_link` (around line 185), detect when the element under the cursor is the new `Label`; instead of resolving a note path, open `NoteBrowserModal` with initial query `format!("#{}", label)`.
- [ ] 8.3 Confirm `Ctrl+K` (the existing `SearchNotes` dispatch path) still opens the modal with an empty query.
- [ ] 8.4 Add a TUI-level test or smoke test that covers: cursor on `#important` + follow-link triggers the modal with the right initial query.

## 9. Docs and finalization

- [ ] 9.1 Update `docs/` to describe the new search syntax (`#<label>` and `lb:<label>`), the hashtag-in-text behavior, and the new follow-link behavior on hashtags.
- [ ] 9.2 Run `cargo test` across the workspace and a manual smoke run of the TUI: create a note with a few hashtags, verify highlighting, run `#tag` search, follow-link from a hashtag opens the modal pre-filled.
- [ ] 9.3 Verify that opening a pre-existing vault triggers a rebuild and that `NoteVault::list_labels()` returns expected entries.
