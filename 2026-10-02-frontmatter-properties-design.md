# Frontmatter Properties Design

## Goal

Turn frontmatter from an opaque, FTS-only text blob into typed, queryable key-value properties: `core/src/note/content_extractor.rs::remove_frontmatter` currently only locates the `---`/`+++` delimiters and keeps everything between them as one unparsed string. This adds a neutral `PropertyValue` model, an index-backed query/sort DSL (`prop:`/`%`), and a read/write API, with TOML as the native write format and YAML kept for reading (and, on request, writing) Obsidian-authored notes.

---

## Scope

- Properties are both searchable/sortable (new `prop:` query token) and readable/writable through the API — not one or the other.
- Full Obsidian property-type parity: Text, Number, Bool, Date, DateTime, List (list-of-text, matching Obsidian's own List type).
- Both frontmatter formats are parsed: `+++` as TOML, `---` as YAML, selected by delimiter alone (no try-both guessing).
- A frontmatter `tags:` property unifies with the existing inline-`#hashtag` label system: both land in the `labels` table so `lb:`/`#` queries see either form. The union only flows into queries — an inline hashtag is never promoted into frontmatter or vice versa.
- Writing a property through the API is supported in this version (not deferred).

### Out of scope

- Scanning property values for wikilinks. A property like `related: [[Other Note]]` is opaque text, not part of the link graph, and is **not** rewritten on note rename. Revisit only if actually wanted later.
- Converting an existing note's frontmatter from one format to the other. The write path only *picks* a format when a note has none yet.
- Nested/hierarchical tags and embeds — separate specs.
- Any TUI-side property editor or display — presentation layer, not this crate.
- Non-list heterogeneous structures (nested tables/maps inside frontmatter, arrays of tables). A property whose value doesn't map onto `PropertyValue` is simply not indexed; the raw frontmatter text is unaffected.

---

## Data model

New module `core/src/note/properties.rs` (parallel to `content_extractor.rs`; `content_extractor.rs` is already large, this keeps the new parsing logic isolated), re-exported from `note::mod`.

```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum PropertyValue {
    Text(String),
    Number(f64),
    Bool(bool),
    Date(NaiveDate),
    DateTime(DateTime<Utc>),
    List(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrontmatterFormat {
    #[default]
    Toml,
    Yaml,
}
```

Both the TOML and YAML parsers normalize into `PropertyValue`; neither third-party crate's own value type is ever part of core's public API.

---

## Parsing

### Format detection

Extends the existing delimiter check in `remove_frontmatter` (first line `---` vs `+++`) rather than attempting to parse with one format and falling back to the other — deterministic, and consistent with the project's existing (and only) format-detection mechanism.

### Type mapping

| Source | TOML (`+++`) | YAML (`---`) |
|---|---|---|
| Text | `String` | plain scalar |
| Number | `Integer`/`Float` | numeric scalar |
| Bool | `Boolean` | boolean scalar |
| Date / DateTime | native `Datetime` variants (below) | see note below |
| List | `Array`, every element stringified | sequence, every element stringified |

List is always list-of-text, matching Obsidian's own List property (its items display as plain text regardless of what's in them). TOML's Integer and Float both map to `Number(f64)` — large integers beyond `f64`'s 2^53 precision lose exactness, an accepted trade-off that matches Obsidian's own Number property (also a double-precision float under the hood), not a new limitation.

**TOML's four datetime subtypes**, split explicitly since only two map cleanly:
- Local Date (no time component) → `Date`
- Offset Date-Time and Local Date-Time (both carry a time component) → `DateTime`; a Local Date-Time has no offset, so it's read as UTC (documented assumption, not a conversion)
- Local Time (time only, no date) → doesn't fit either variant; treated as an unsupported value (same degrade-gracefully rule as a non-finite float: that key is skipped, siblings still index)

**YAML date risk (flag, not resolved here):** plain YAML has no native date type (that's a YAML 1.1 `!!timestamp` tag; library support varies), and Obsidian writes bare `2024-01-01` for its Date property. Whichever YAML crate is chosen (`serde_yaml` is deprecated/archived — `saphyr` or `yaml-rust2` are the live alternatives) may or may not auto-resolve that to a date; if it hands back a bare string instead, the Date/DateTime branch parses that string itself (see below) rather than trusting a type tag. Confirm during implementation.

### Date/DateTime parsing — reuse, don't reinvent

`"%Y-%m-%d"` is already duplicated three times in core (`lib.rs::get_todays_journal`/`journal_date`, `note_rename.rs:388`, `nfs/backup.rs`'s backup-folder dating/purge), none sharing a helper. This feature is a fourth use site, so it's worth consolidating: pull a small `parse_iso_date`/`format_iso_date` pair (wrapping `NaiveDate` + `"%Y-%m-%d"`) out of that duplication and have all four call sites, including the new `Date` branch, use it. `DateTime` has no prior convention to reuse (journal/backup only ever deal in dates), so it uses chrono's built-in RFC3339 parsing (`DateTime::parse_from_rfc3339`/`to_rfc3339()`) instead of a new ad hoc format string. Exact module location for the shared helper is an implementation-plan detail; it lives outside `nfs`/`system` since it does no I/O.

### Failure handling

A block that fails to parse (malformed YAML/TOML) degrades to today's behavior: kept as an opaque searchable blob, zero properties extracted, nothing fails the indexing pass over it. A property whose value doesn't map onto `PropertyValue` (nested table, non-finite float) is individually skipped; the rest of the block's properties still index normally.

---

## Public API (`NoteVault`, `core/src/lib.rs`)

```rust
pub async fn get_tags(&self, path: &VaultPath) -> Result<Vec<String>, VaultError>;
pub async fn get_properties(&self, path: &VaultPath) -> Result<Vec<(String, PropertyValue)>, VaultError>;
pub async fn get_property(&self, path: &VaultPath, key: &str) -> Result<Option<PropertyValue>, VaultError>;
pub async fn set_property(
    &self,
    path: &VaultPath,
    key: &str,
    value: PropertyValue,
    new_block_format: FrontmatterFormat,
) -> Result<(), VaultError>;
pub async fn remove_property(&self, path: &VaultPath, key: &str) -> Result<bool, VaultError>;
```

- `get_tags` closes a pre-existing gap: today a note's own tags are only reachable by filtering `get_chunks_and_links()`'s `Vec<NoteLink>` for `LinkType::Hashtag`, mixed in with every other link type — no dedicated per-note accessor. `get_tags` is the parallel method to `get_properties`, same shape (single-purpose, path-keyed, index-backed): `SELECT name FROM labels WHERE path = ?`, the mirror image of the existing `notes_with_label` (label → notes). Since frontmatter `tags:` unifies into `labels` (below), this returns the union of inline `#hashtags` and frontmatter `tags:` for that note for free.
- `get_property(s)` on a path that doesn't resolve uses the same `VaultError::FSError`/`NotePathNotFound` convention as `get_note_text`.
- Keys are lowercased at the API boundary, same rule as paths and tags ("case-insensitive, default lowercase" is already the vault-wide convention). Values keep their original casing for display.
- `set_property`/`remove_property` go through the same per-note lock as `save_note`/`replace_in_note` (read current text under the lock, parse frontmatter, mutate the one key, re-serialize, write, backup-if-enabled), not a new concurrency primitive.
- `new_block_format` only matters the first time a note gets a frontmatter block: no existing block → create one in the requested format (default `Toml`). An existing block (either format) is preserved; further single-property writes never flip it. Converting an existing block's format is out of scope (see above).
- **New error case:** `set_property`/`remove_property` against a note whose existing frontmatter fails to parse must not guess or silently corrupt it — this returns an error rather than falling back to the read-side "treat as blob" behavior, because a write has to land somewhere specific. Adds `FSError::InvalidFrontmatter { path: VaultPath, message: String }`, surfaced as `VaultError::FSError(...)`, with a `user_message()` entry (`"Could not parse existing frontmatter in '{path}': {message}"`) alongside the other path/regex variants in `error.rs`. The `user_message` match there is exhaustive by design (see its own doc comment) — this variant must be added to it.

---

## Tags unification

- A frontmatter `tags:` property, List or Text, is read into the same `labels` table that inline `#hashtags` already populate. A note's effective tag set for `lb:`/`#` queries is the union of both forms.
- Direction is one-way for *querying only*: neither form is promoted into the other. An inline `#tag` stays inline text; a frontmatter `tags:` entry stays in frontmatter.
- A bare string value for `tags:` (not a YAML/TOML list) is treated as a single tag whose name is the whole string — matches Obsidian's own behavior, no comma-splitting guesswork.
- Mechanically: `tags:` is also stored as a normal row in `properties` (so `get_properties` round-trips it like any other key) *and* its items are additionally upserted into `labels` during the same reindex pass — both tables are populated directly from the same parse of the note's own text in one transaction, so this isn't a cache-of-a-cache; it's two tables derived from one source, same as `links` and `labels` already are today.

---

## Schema / index (`core/src/index/mod.rs`)

```sql
CREATE TABLE properties (
    path TEXT NOT NULL,
    key TEXT NOT NULL,
    list_index INTEGER NOT NULL DEFAULT 0,  -- 0 for scalars; 0..N for list items, in order
    value_type TEXT NOT NULL,               -- text | number | bool | date | datetime | list
    value_text TEXT,                        -- canonical text form: ISO date/datetime, "true"/"false", display string
    value_num REAL,                         -- set only when value_type = 'number'
    PRIMARY KEY (path, key, list_index)
);

CREATE INDEX properties_by_key_text ON properties(key, value_text);
CREATE INDEX properties_by_key_num  ON properties(key, value_num);
```

`WHERE path = ?` (all properties of one note) is already served by the primary key, no extra index needed for that direction. The two secondary indices back `prop:key<op>value` queries scanning across notes by key.

Reindexing an existing note (every `index.save_note()` call) deletes and reinserts that path's `properties` rows (and reconciles its `tags:`-derived `labels` rows), the same pattern already used for `links`/`labels` on every save — not new architecture, an extension of the existing per-note reindex step.

---

## Query DSL (`core/src/index/search_terms.rs`)

New long and short tokens, following the existing prefix-table pattern:

| Form | Meaning |
|---|---|
| `prop:key<op>value` | property comparison, `op` ∈ `= != < <= > >=` |
| `%key<op>value` | short form of the above |
| `-prop:key<op>value` / `-%key<op>value` | excluded form |
| `or:prop:key` / `-or:prop:key` | sort ascending/descending by property |

No short alias was picked lightly: `$` was considered and rejected — inside double-quoted CLI arguments (the normal way to quote a query), bash/zsh still expand `$foo`, so `"$status=active"` would silently try a shell-variable substitution instead of reaching kimun as text. `%` is free, inert in both single and double quotes, and not already meaningful in the user-facing grammar (it only appears internally as the SQL `LIKE` wildcard the DSL's own `*` glob gets translated to — never something a user types).

**Comparison semantics:**

- `value_type = 'number'` → compares `value_num` numerically.
- Everything else → compares `value_text` lexicographically (correct for ISO dates, exact for text/bool).
- Type mismatch (e.g. `prop:due>5` against a Date) → no match, not an error.
- List properties: `=` means "list contains this exact item" (one row among that key's `list_index` rows matches), `!=` means "doesn't contain". Ordering operators on a List → no match, same as any other type mismatch.

---

## Migration

No migration code to write. `index/mod.rs` keeps a `VERSION` string in the `appData` table; on open, a version mismatch already triggers a drop-and-recreate of the schema (self-heal), which the next sync pass rebuilds in full from the markdown files — the actual source of truth, per the crate's own "index is a cache" design. So: bump `VERSION`, add the `properties` table + indices to `create_tables()`, extend the per-note reindex step as described above, and every existing vault picks this up transparently on next open. The one cost is that first post-upgrade open pays for a full resync, same as any past schema change — worth a one-line release note for large vaults, not a design concern.

---

## Edge cases

| Case | Handling |
|---|---|
| Malformed/unparseable frontmatter (read) | Degrades to today's opaque-blob behavior; zero properties extracted, indexing pass still succeeds |
| Malformed/unparseable frontmatter (write) | `set_property`/`remove_property` returns `FSError::InvalidFrontmatter` rather than guessing |
| Duplicate keys in one block | Not our problem: TOML errors on this itself (falls into the row above); YAML's behavior is whatever the chosen crate does — no second detection layer added |
| Non-finite number (TOML allows `nan`/`inf`) | That one key is skipped; rest of the block still indexes |
| Leading `---` meant as a Markdown `<hr>`, not frontmatter | Pre-existing ambiguity in `remove_frontmatter` (shared by every tool using this convention, including Obsidian), not new — but sharper now that `set_property` *writes* based on that same detection: a false "no frontmatter" read means a new block gets prepended above what the user meant as a rule. Not solved here. |
| Property value referencing a note (e.g. `related: [[Other Note]]`) | Opaque text; not scanned, not rewritten on rename (explicit non-goal above) |

---

## Error handling

| Scenario | Error |
|---|---|
| `get_property(s)` on a non-existent note | `VaultError::FSError(FSError::NotePathNotFound)` — same as `get_note_text` |
| `set_property`/`remove_property` on a note whose existing frontmatter fails to parse | `VaultError::FSError(FSError::InvalidFrontmatter { path, message })` (new variant) |
| Query with a type-mismatched `prop:` comparison | No error — zero matches |
| Frontmatter block present but a specific property fails to map to `PropertyValue` | No error — that key is absent from `get_properties`/index, rest proceed |

---

## Testing

### `note/properties.rs`

- TOML: each type (`Text`, `Number`, `Bool`, `Date`, `DateTime`, `List`) parses to the matching `PropertyValue`
- YAML: each type parses to the matching `PropertyValue`, including the bare-date fallback path if the chosen crate doesn't auto-resolve YAML timestamps
- Malformed block (either format) → empty property set, no panic
- Non-finite TOML float (`nan`/`inf`) → that key skipped, siblings still parsed
- `tags:` as a bare string → single tag equal to the whole string
- `tags:` as a list → one tag per item
- List property with mixed-type elements → every element stringified

### `index/mod.rs`

- Saving a note inserts its `properties` rows; resaving without a previously-present key removes that key's row
- `tags:` list items appear in `labels` alongside any inline `#hashtags` on the same note
- `get_tags`-equivalent query (`labels` by path) returns the union of inline `#hashtags` and frontmatter `tags:` for a note; returns empty for a note with neither
- `prop:key=value` exact match for each `value_type`
- `prop:key>value` / `<` / `>=` / `<=` numeric comparison; same operators against a non-number type → zero results
- `-prop:key=value` / `-%key=value` exclusion
- `prop:key=value` against a List property matches when any item equals `value`; `prop:key!=value` matches when none do
- `or:prop:key` ascending and `-or:prop:key` descending sort
- Schema version bump triggers self-heal and a full resync that populates `properties` for pre-existing notes

### `search_terms.rs`

- `prop:`/`%` prefix detection, each operator (`= != < <= > >=`) extracted correctly from `key<op>value`
- Excluded forms `-prop:`/`-%` parse to the excluded variant

### `lib.rs` (integration)

- `get_properties` round-trips everything `set_property` wrote
- `get_tags` on a note with both an inline `#hashtag` and a frontmatter `tags:` entry returns both, deduplicated
- `set_property` on a note with no frontmatter creates a TOML block by default
- `set_property` with `FrontmatterFormat::Yaml` on a note with no frontmatter creates a YAML block instead
- `set_property` on a note with an existing YAML block preserves YAML; same for an existing TOML block
- `remove_property` removes only the targeted key, siblings untouched
- `set_property`/`remove_property` on a note with unparseable existing frontmatter returns `InvalidFrontmatter`
- Concurrent `set_property` calls on the same note serialize correctly (per-note lock), neither write is lost
