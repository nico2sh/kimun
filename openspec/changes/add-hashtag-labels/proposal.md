## Why

Notes currently lack lightweight, in-text categorization. Users can link notes together but cannot tag a note with a topic without creating extra link infrastructure. Hashtags inside note text are a familiar, low-friction way to attach topical labels and to surface every note about a topic in one search.

The underlying hashtag tokenizer already exists (`HASHTAG_RX`, `LinkType::Hashtag`), but extracted hashtags are not indexed as labels, not queryable through search, and following one does nothing useful. This change wires those pieces into a real labels feature.

## What Changes

- Extract hashtags from note text on indexing and persist them as labels associated with the note.
- Add a `labels` table (label name + note id) with an index on label name for fast lookup.
- Extend the search query syntax with a `#<label>` filter (and the equivalent `lb:<label>`) that restricts results to notes carrying that label.
- Rebuild a note's labels on every reindex (no orphan labels).
- Render hashtags in the TUI editor with a distinct highlight (separate from generic links).
- Make "follow link" (default `Ctrl+G`) on a hashtag open the search modal (the same modal opened by `Ctrl+K`) pre-filled with `#<label>`.
- Bump the database `VERSION` constant so existing vaults rebuild the index with the new label table populated.

## Capabilities

### New Capabilities
- `note-labels`: hashtag-driven labels associated with notes, persisted as a queryable index in the database, and exposed through the core API.
- `label-search`: search query syntax extension and end-to-end search behavior for filtering notes by label using `#<label>` or `lb:<label>`.
- `label-rendering`: TUI rendering of hashtags as labels (distinct highlight) and follow-link behavior that opens the search modal pre-filled with the label query.

### Modified Capabilities
<!-- None: no specs exist yet in openspec/specs/ -->

## Impact

- **Core (`core/`)**:
  - `core/src/db/mod.rs`: new `labels` table + index, schema `VERSION` bump, label insertion path in the note ingestion flow, label-aware search SQL builder.
  - `core/src/db/search_terms.rs`: register `#` and `lb:` prefixes; new label term kind.
  - `core/src/note/content_extractor.rs`: expose extracted hashtag labels (currently dropped or inlined as links) to the indexing pipeline.
  - `core/src/lib.rs` (`NoteVault`): search API surface accepts label filters via the existing query string; no new method needed if syntax is handled in the parser.
- **TUI (`tui/`)**:
  - `tui/src/components/text_editor/markdown.rs`: new `Label` (or `Hashtag`) `ElementKind` and parsing.
  - `tui/src/app_screen/editor.rs`: `follow_link()` recognizes hashtag-under-cursor and dispatches the existing `SearchNotes` modal pre-filled with `#<label>`.
  - `tui/src/components/note_browser/`: accept an initial query string when the modal is opened programmatically.
- **Migrations / on-disk state**: existing vaults trigger a full reindex on first run after upgrade (already the behavior on `VERSION` bump). No user data loss.
- **Docs (`docs/`)**: end-user docs updated for the new search syntax and hashtag behavior.
