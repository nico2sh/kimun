## Context

Kimün indexes notes into a SQLite database (`core/src/db/mod.rs`) with a `notes` table, a `links` table, and an FTS4 virtual table `notesContent`. The query parser in `core/src/db/search_terms.rs` already recognizes prefixes like `in:` / `>`, `at:` / `@`, `pt:` / `/`, plus negation and ordering. The hashtag regex `HASHTAG_RX` (`#[A-Za-z0-9_]+`) and a `LinkType::Hashtag` variant exist in `core/src/note/content_extractor.rs` and `core/src/note/mod.rs:127`, and the TUI editor already has an `ElementKind::WikiLink` / `Link` variant in `tui/src/components/text_editor/markdown.rs`. Hashtags are currently stripped from indexable text (`cleanup_hashtags`) and rendered as plain links — they have never been promoted to first-class searchable metadata.

The TUI has a `SearchNotes` action bound to `Ctrl+K` (`tui/src/keys/action_shortcuts.rs:31`) that opens the `NoteBrowserModal` (`tui/src/components/note_browser/mod.rs`) backed by `SearchNotesProvider`, and a `FollowLink` action bound to `Ctrl+G` (`action_shortcuts.rs:45`, dispatched at `tui/src/app_screen/editor.rs:566`, executed by `follow_link()` at `editor.rs:185`).

Database schema versioning is one constant (`VERSION` at `core/src/db/mod.rs:70`); bumping it triggers `Outdated` and a full rebuild. No incremental migration system exists, which is the simplest path for this change.

## Goals / Non-Goals

**Goals:**
- Persist hashtag-derived labels in the database with an index on label name.
- Extend the existing search query parser with `#<label>` and `lb:<label>` filters that resolve through the label index, not through full-text scan.
- Render hashtag tokens in the TUI editor with a label-specific highlight, separate from generic links.
- Make `Ctrl+G` on a hashtag open the existing `Ctrl+K` search modal pre-filled with `#<label>`.
- Bump the DB `VERSION` so existing vaults rebuild and populate the labels table.

**Non-Goals:**
- A general tag taxonomy, hierarchical tags (`#parent/child`), tag rename / merge UI, or tag aliasing.
- Persisting hashtag-position metadata (offsets, line numbers) — only the (note, label) association is stored.
- A separate "labels panel" in the TUI. Discovery happens through the search modal.
- Hashtag detection in note titles or filenames — body text only, consistent with current `HASHTAG_RX` usage.
- Extending the label character set (Unicode, dashes, slashes). The existing regex `[A-Za-z0-9_]+` is the contract.
- Changing how plain links or wikilinks are followed.

## Decisions

### 1. Storage: dedicated `labels` table, not an FTS column

Add two tables:

```sql
CREATE TABLE labels (
    name TEXT NOT NULL,
    path TEXT NOT NULL,
    PRIMARY KEY (name, path)
);
CREATE INDEX labels_by_name ON labels(name);
CREATE INDEX labels_by_path ON labels(path);
```

`name` is the normalized (lowercased) label. `path` is the note's vault path (the same key used in `notes`). The composite primary key gives us free dedup when extracting and inserting.

**Alternatives considered:**
- *Add a `tags` column to `notesContent` FTS4.* Rejected: FTS4 tokenizes on whitespace and is intended for content search; filtering by a fixed token through FTS is slower than a direct index lookup, and combining label filters with `MATCH` clauses adds parser complexity for no win.
- *Reuse `links` with a synthetic destination like `#tag`.* Rejected: confuses link semantics, pollutes backlinks, and prevents the secondary index on label name without overloading the `destination` column with marker prefixes.

### 2. Label normalization: lowercase, body-only, regex-driven

Labels are normalized to lowercase before insert and before lookup. The vault is already case-insensitive for paths (per project rules), so labels follow the same model. Extraction reuses `HASHTAG_RX`; no new tokenizer.

Hashtags inside fenced code blocks and inline code spans are ignored. Implementation: the markdown is already parsed by `pulldown-cmark` in `content_extractor`; collect labels from `Event::Text` events whose parser state is outside `CodeBlock` / `Code`. This keeps a single, authoritative parse and ensures the highlighting rule in the TUI matches the indexing rule.

### 3. Indexing pipeline

`content_extractor::get_markdown_and_links` (or its callers in the index path) returns the list of `(label_name)` tuples discovered for the note alongside the existing `MarkdownNote` data. The DB insert path that today rewrites `links` for a note is extended in lockstep:

```
for each indexed note:
    DELETE FROM labels WHERE path = ?
    INSERT OR IGNORE INTO labels (name, path) VALUES ...
```

Labels are rebuilt on every reindex of a note, matching the existing behavior for links. Deleting a note deletes its label rows by `path`. No background "garbage collect orphan labels" job is needed because the index is `(name, path)` rather than a separate `label_id` table — labels with no notes simply don't appear in queries.

**Alternative considered:** normalized `labels(id, name)` + `note_labels(note_id, label_id)`. Rejected as overkill: the join saves a few bytes per row but adds a lookup step, a uniqueness invariant, and orphan-cleanup logic the flat table avoids.

### 4. Search syntax: `#` short / `lb:` long, registered in `prefix_table`

Extend `prefix_table()` in `core/src/db/search_terms.rs` with two new entries (and their excluded counterparts):

```rust
("lb:-", "#-", || ElementType::ExcludedLabel),
("lb:",  "#",  || ElementType::Label),
```

Excluded entries must precede positive ones to match longest-prefix-first, consistent with the existing convention. Add `Label` / `ExcludedLabel` to `ElementType`, add `labels` / `excluded_labels` to `SearchTerms`, and lowercase the term before pushing.

`build_search_sql_query_inner` gets a new `add_labels_query` step that joins `notes.path` against `labels.path WHERE labels.name = ?` (positive) and uses `notes.path NOT IN (SELECT path FROM labels WHERE name = ?)` for negation, mirroring the existing exclusion pattern.

Multiple label filters AND together (a note must carry every requested label) — implemented as one join per positive label, which uses `labels_by_name`. Empty result on unknown labels falls out naturally from the join.

### 5. TUI rendering: new `ElementKind::Label`

Add a `Label { name: String }` variant to `ElementKind` in `tui/src/components/text_editor/markdown.rs` and emit it from the line parser when a hashtag matches outside code spans. Style it through the existing theme so users can recolor it. Generic `Link` styling stays unchanged so users still distinguish URLs from labels.

### 6. Follow-link: route hashtag-under-cursor to the search modal

In `tui/src/app_screen/editor.rs::follow_link` (line 185), detect whether the element under the cursor is the new `Label`. If so, instead of resolving a note path, dispatch the existing path that opens `NoteBrowserModal` and pass an initial query `format!("#{}", label)`.

`NoteBrowserModal::open` (or a new `open_with_query`) accepts an optional initial query string. When provided, it pre-fills the input, places the cursor at end, and runs the first query synchronously so results are visible on open. Both the keyboard `Ctrl+K` entry point and the follow-link entry point share the same modal — the only difference is the initial query value.

### 7. Schema version bump

`VERSION` goes from `"0.5"` to `"0.6"`. The existing `DBStatus::Outdated` branch already drops and recreates everything via `create_tables`, so adding the `labels` table to `create_tables` plus the version bump is the full migration. The bump comment above `VERSION` records the reason (matching the format of the `0.5` comment).

## Risks / Trade-offs

- **Cold-start reindex cost** → Mitigation: existing vaults already pay the rebuild cost on version bumps; the labels insert is one extra cheap INSERT per hashtag. No new behavior on the user's part.
- **Hashtag in code spans incorrectly indexed** → Mitigation: pull hashtag extraction from the markdown event stream (skip `Code` / `CodeBlock`), and add a test that asserts `` `#foo` `` produces no label.
- **`#` already overloaded** (markdown headings are `#` at line start, fragment links use `#`) → Mitigation: the existing `HASHTAG_RX` requires no whitespace between `#` and the name and is applied to inline text content, not raw lines, so headings (block-level) are not matched. Markdown link fragments `[x](#anchor)` live inside `MD_LINK_RX` and are extracted as links before label scanning runs.
- **Query parser ambiguity between `#` (label) and `-#` (excluded label)** → Mitigation: the prefix table is ordered longest-first per existing convention; add a unit test for both `#tag` and `-#tag`.
- **Empty label after `#`** (user types bare `#`) → Mitigation: extractor requires `+` in the regex (one or more characters); the parser should drop label tokens with empty names rather than running a no-op query.
- **Label deletion races indexer** → Mitigation: label INSERTs sit in the same transaction as the note's `links` update, so a partial indexer crash leaves both stale or both fresh.

## Migration Plan

1. Land the schema additions and `VERSION` bump together. Users opening an existing vault see the standard "rebuilding index" path and the labels table populates from current note bodies.
2. No rollback path is needed beyond reverting the version bump constant — the labels table is additive and ignored by older builds (older builds open the vault, see a version mismatch the other direction, and rebuild without the labels table).
3. Document the new search syntax in `docs/` (per project rule that user-facing docs live there).

## Open Questions

- Should the autocompletion in the search modal suggest existing labels once at least `#` is typed? (Out of scope for this change; tracked separately if requested.)
- Should the `Label` highlight pick up the link color by default or a fresh theme key? Default proposal: a new `label` key with a sane fallback to the `link` color so themes that haven't updated still look reasonable.
