# Hashtag Labels Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn in-text `#hashtags` into first-class labels that are indexed in the database, filterable through the existing search query syntax (`#<label>` / `lb:<label>`), highlighted in the TUI editor, and reachable via the existing follow-link action which opens the search modal pre-filled with the label query.

**Architecture:** A new `labels(name, path)` table joined to `notes.path` carries the (label, note) edges. Hashtag detection already exists as `HASHTAG_RX` and flows into the `Vec<NoteLink>` returned by `content_extractor::get_markdown_and_links` as `LinkType::Hashtag`. Indexing reuses that pipeline: `NoteBatch` learns to also collect `LabelRow`s from hashtag links and bulk-insert them. The search query parser gains two new prefixes that produce `ElementType::Label` / `ExcludedLabel`, and the SQL builder adds one query per label that joins against `labels`. The TUI gets a new `ElementKind::Label`, a new themable color, and a follow-link branch that opens the existing note-browser modal with an initial query.

**Tech Stack:** Rust 2024 edition, SQLite via `sqlx`, `pulldown-cmark` for markdown, `regex`, `tokio`, `ratatui`-based TUI. Test framework: `cargo test` with `tempfile` for DB tests.

**Spec refs:** `openspec/changes/add-hashtag-labels/proposal.md`, `design.md`, `specs/note-labels/spec.md`, `specs/label-search/spec.md`, `specs/label-rendering/spec.md`, `tasks.md`.

**File map (touched):**

- `core/src/db/mod.rs` — `VERSION` bump (0.5 → 0.6), `create_tables` adds `labels` table + indices, `NoteBatch` gains `labels: Vec<LabelRow>` + `LabelRow` struct + `BulkInsertRow` impl, `delete_notes` deletes from `labels`, `build_search_sql_query_inner` calls new `add_labels_query` helper, new `list_labels` / `notes_with_label` DB functions.
- `core/src/db/search_terms.rs` — new `Label` / `ExcludedLabel` variants on `ElementType`, two new entries in `prefix_table()`, two new fields on `SearchTerms`.
- `core/src/note/content_extractor.rs` — new helper `code_char_ranges()` that returns char-offset ranges of `Code` / `CodeBlock` events; existing `get_markdown_and_links` rewrites hashtag processing to skip ranges from that helper.
- `core/src/lib.rs` — two new public `NoteVault` methods: `list_labels`, `notes_with_label`.
- `tui/src/components/text_editor/markdown.rs` — new `ElementKind::Label { name: String }` variant; line parser emits `Label` spans for hashtag matches outside code spans.
- `tui/src/components/text_editor/view.rs` (or whichever file applies styles to `ElementKind`) — pick a theme key for labels.
- `tui/src/components/note_browser/mod.rs` — `NoteBrowserModal` constructor accepts `Option<String>` initial query; pre-fills the input and runs the first search synchronously.
- `tui/src/app_screen/editor.rs` — `follow_link` (line 185) gets a branch that, when the cursor is on a hashtag, opens the modal with `format!("#{}", label)`.
- `tui/src/keys/action_shortcuts.rs` — confirm `FollowLink` and `SearchNotes` enum variants are unchanged.
- `docs/` — add a section describing the new search syntax and hashtag behavior.

**Conventions:**

- TDD throughout: failing test first, then minimal implementation. Each task ends with a commit.
- Commits use Conventional Commits style (the repo's existing convention; see `git log`).
- Run `cargo test -p kimun-core` for core changes, `cargo test -p kimun-tui` for TUI changes, `cargo test --workspace` before the final commit.
- Do NOT skip git hooks. Investigate failures.
- `VaultPath` is mandatory for vault paths in core (see `CLAUDE.md`).

---

## Task 1: Schema bump and `labels` table

**Files:**
- Modify: `core/src/db/mod.rs` (`VERSION` constant near line 70, `create_tables` near line 208)
- Test: `core/src/db/mod.rs` (`#[cfg(test)] mod tests` at the bottom)

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `core/src/db/mod.rs` (next to `vault_db_new_creates_parent_dir_for_db_path`):

```rust
#[tokio::test]
async fn labels_table_exists_after_create_tables() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("kimun.sqlite");
    let db = super::VaultDB::new(&path).await.unwrap();
    super::create_tables(db.pool()).await.unwrap();

    let row: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM sqlite_master \
         WHERE type='table' AND name='labels'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(row.0, 1, "labels table should exist");

    let idx_name: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM sqlite_master \
         WHERE type='index' AND name='labels_by_name'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(idx_name.0, 1, "labels_by_name index should exist");

    let idx_path: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM sqlite_master \
         WHERE type='index' AND name='labels_by_path'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(idx_path.0, 1, "labels_by_path index should exist");

    db.close().await.unwrap();
}
```

- [ ] **Step 2: Run the test, expect failure**

```
cargo test -p kimun-core db::tests::labels_table_exists_after_create_tables -- --nocapture
```

Expected: FAIL — `labels` table not created.

- [ ] **Step 3: Bump the schema version**

In `core/src/db/mod.rs`, replace:

```rust
// 0.5: BREADCRUMB_SEP changed from `>` to `\x1f`. Bump forces a clean
//      reindex so stale rows with the old separator are rewritten.
const VERSION: &str = "0.5";
```

with:

```rust
// 0.6: Added `labels` table populated from hashtags in note bodies. Bump
//      forces a clean reindex so the table is filled for existing vaults.
// 0.5: BREADCRUMB_SEP changed from `>` to `\x1f`. Bump forces a clean
//      reindex so stale rows with the old separator are rewritten.
const VERSION: &str = "0.6";
```

- [ ] **Step 4: Add the table creation in `create_tables`**

In `core/src/db/mod.rs`, inside `create_tables`, after the `CREATE VIRTUAL TABLE notesContent ...` block and before `tx.commit().await?;`, insert:

```rust
sqlx::query(
    "CREATE TABLE labels (
        name TEXT NOT NULL,
        path TEXT NOT NULL,
        PRIMARY KEY (name, path)
    )",
)
.execute(&mut *tx)
.await?;

sqlx::query(
    "CREATE INDEX labels_by_name
        ON labels(name)",
)
.execute(&mut *tx)
.await?;

sqlx::query(
    "CREATE INDEX labels_by_path
        ON labels(path)",
)
.execute(&mut *tx)
.await?;
```

- [ ] **Step 5: Run the test, expect pass**

```
cargo test -p kimun-core db::tests::labels_table_exists_after_create_tables
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add core/src/db/mod.rs
git commit -m "feat(core/db): add labels table and bump schema to 0.6"
```

---

## Task 2: Detect code-span character ranges in note text

**Why:** Hashtags inside ``` `code` ``` and fenced code blocks must NOT become labels. The existing `HASHTAG_RX` runs on raw markdown text and currently does not respect code spans. This task introduces a pure helper that returns char-offset ranges of code regions so later steps can filter regex matches that fall inside them.

**Files:**
- Modify: `core/src/note/content_extractor.rs` (add new helper near other char-span helpers around line 88-150)
- Test: same file's `#[cfg(test)] mod tests` block.

- [ ] **Step 1: Write the failing tests**

In the `tests` module at the bottom of `core/src/note/content_extractor.rs`, add:

```rust
#[test]
fn code_char_ranges_inline_code() {
    let md = "hello `#notalabel` and #real";
    let ranges = super::code_char_ranges(md);
    assert!(
        ranges.iter().any(|(s, e)| md[*s..*e].contains("notalabel")),
        "inline code span must be reported"
    );
    assert!(
        ranges.iter().all(|(s, e)| !md[*s..*e].contains("#real")),
        "non-code text must not be reported"
    );
}

#[test]
fn code_char_ranges_fenced_block() {
    let md = "before\n```\n#inside\n```\nafter #outside";
    let ranges = super::code_char_ranges(md);
    assert!(
        ranges.iter().any(|(s, e)| md[*s..*e].contains("#inside")),
        "fenced block content must be reported"
    );
    assert!(
        ranges.iter().all(|(s, e)| !md[*s..*e].contains("#outside")),
        "text after fence must not be reported"
    );
}

#[test]
fn code_char_ranges_none_for_plain_text() {
    let md = "no code here, just #tags";
    let ranges = super::code_char_ranges(md);
    assert!(ranges.is_empty(), "plain text yields no code ranges");
}
```

- [ ] **Step 2: Run tests, expect failure**

```
cargo test -p kimun-core note::content_extractor::tests::code_char_ranges -- --nocapture
```

Expected: FAIL — `code_char_ranges` does not exist.

- [ ] **Step 3: Implement `code_char_ranges`**

In `core/src/note/content_extractor.rs`, near the other helpers that walk the `pulldown_cmark::Parser` (after the imports and `LazyLock` regexes, before `cleanup_hashtags`), add:

```rust
/// Returns byte-offset ranges (start, end) within `md_text` covering every
/// inline code span and fenced/indented code block. Used to exclude these
/// regions from hashtag extraction so `#tag` inside code is not promoted to
/// a label.
pub(crate) fn code_char_ranges(md_text: &str) -> Vec<(usize, usize)> {
    let parser = Parser::new(md_text).into_offset_iter();
    let mut ranges = Vec::new();
    let mut depth = 0u32;
    let mut current_start: Option<usize> = None;
    for (event, range) in parser {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                if depth == 0 {
                    current_start = Some(range.start);
                }
                depth += 1;
            }
            Event::End(TagEnd::CodeBlock) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Some(start) = current_start.take() {
                        ranges.push((start, range.end));
                    }
                }
            }
            Event::Code(_) => {
                ranges.push((range.start, range.end));
            }
            _ => {}
        }
    }
    ranges
}
```

- [ ] **Step 4: Run tests, expect pass**

```
cargo test -p kimun-core note::content_extractor::tests::code_char_ranges
```

Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add core/src/note/content_extractor.rs
git commit -m "feat(core/note): add code_char_ranges helper for hashtag exclusion"
```

---

## Task 3: Make hashtag extraction skip code regions

**Why:** Wire `code_char_ranges` into `get_markdown_and_links` so `#tag` inside code is rendered as literal text (no link conversion) and is NOT appended to the `links` list as `LinkType::Hashtag`. The downstream indexer reuses `links` to populate the `labels` table (Task 4), so suppressing the link entry is what keeps the spec scenario "Hashtag inside a code fence is not a label" honest.

**Files:**
- Modify: `core/src/note/content_extractor.rs` (the hashtag pass inside `get_markdown_and_links`, around line 366)
- Test: same file's `tests` module.

- [ ] **Step 1: Write the failing tests**

In `tests` mod, add:

```rust
#[test]
fn hashtag_in_inline_code_is_not_extracted() {
    let path = crate::nfs::VaultPath::note_path_from("/n.md");
    let (text, links) =
        super::get_markdown_and_links(&path, "use `#notalabel` and tag #real");
    assert!(
        links.iter().all(|l| !matches!(&l.ltype, super::super::LinkType::Hashtag)
            || l.text != "notalabel"),
        "hashtag inside inline code must not become a hashtag link"
    );
    assert!(
        links.iter().any(|l| matches!(&l.ltype, super::super::LinkType::Hashtag)
            && l.text == "real"),
        "hashtag outside code is still extracted"
    );
    assert!(
        text.contains("`#notalabel`"),
        "inline code literal is preserved in rendered output: {}",
        text
    );
}

#[test]
fn hashtag_in_fenced_block_is_not_extracted() {
    let path = crate::nfs::VaultPath::note_path_from("/n.md");
    let body = "before\n```\n#inside\n```\nafter #outside";
    let (_text, links) = super::get_markdown_and_links(&path, body);
    let hashtag_names: Vec<&str> = links
        .iter()
        .filter_map(|l| match &l.ltype {
            super::super::LinkType::Hashtag => Some(l.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(hashtag_names, vec!["outside"]);
}

#[test]
fn hashtag_terminates_at_non_label_char() {
    // Per spec: `#tag-with-dash` yields the label `tag` and the rest
    // (`-with-dash`) is treated as following text. `HASHTAG_RX` already
    // enforces this because `[A-Za-z0-9_]+` stops at `-`.
    let path = crate::nfs::VaultPath::note_path_from("/n.md");
    let (_text, links) = super::get_markdown_and_links(&path, "x #tag-with-dash y");
    let hashtag_names: Vec<&str> = links
        .iter()
        .filter_map(|l| match &l.ltype {
            super::super::LinkType::Hashtag => Some(l.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(hashtag_names, vec!["tag"]);
}
```

- [ ] **Step 2: Run tests, expect failure**

```
cargo test -p kimun-core note::content_extractor::tests::hashtag_in_ -- --nocapture
```

Expected: FAIL — current implementation extracts hashtags from code.

- [ ] **Step 3: Replace the hashtag pass in `get_markdown_and_links`**

In `core/src/note/content_extractor.rs`, replace the existing block (around lines 365-370):

```rust
    // Process hashtags and convert them to links
    let clean_md_text = HASHTAG_RX.replace_all(&md_text, |caps: &Captures| {
        let tag = &caps["ht_text"];
        links.push(NoteLink::hashtag(tag));
        format!("[#{}](#{})", tag, tag)
    });
```

with:

```rust
    // Process hashtags and convert them to links, skipping any match that
    // overlaps a code span or fenced code block. Hashtag extraction runs
    // against `md_text` (post-link-rewrite); code regions are calculated
    // against the same text.
    let code_ranges = code_char_ranges(&md_text);
    let mut out = String::with_capacity(md_text.len());
    let mut last_end = 0usize;
    for caps in HASHTAG_RX.captures_iter(&md_text) {
        let m = caps.get(0).unwrap();
        let in_code = code_ranges
            .iter()
            .any(|(s, e)| m.start() >= *s && m.end() <= *e);
        out.push_str(&md_text[last_end..m.start()]);
        if in_code {
            out.push_str(m.as_str());
        } else {
            let tag = &caps["ht_text"];
            links.push(NoteLink::hashtag(tag));
            out.push_str(&format!("[#{}](#{})", tag, tag));
        }
        last_end = m.end();
    }
    out.push_str(&md_text[last_end..]);
    let clean_md_text: std::borrow::Cow<'_, str> = std::borrow::Cow::Owned(out);
```

(The variable name `clean_md_text` and its `Cow` type are preserved so the function's final return line `(clean_md_text.to_string(), links)` continues to compile unchanged.)

- [ ] **Step 4: Run tests, expect pass**

```
cargo test -p kimun-core note::content_extractor::tests
```

Expected: all existing tests + 2 new tests PASS. If any pre-existing tests changed their expected output (because they previously relied on hashtags inside code being converted), update them — but only after confirming the new behavior is what the spec mandates.

- [ ] **Step 5: Commit**

```bash
git add core/src/note/content_extractor.rs
git commit -m "feat(core/note): exclude code spans from hashtag extraction"
```

---

## Task 4: Persist labels via `NoteBatch`

**Why:** The indexer fans out into `NoteBatch` (`core/src/db/mod.rs:820+`) which today filters `LinkType::Note` and inserts into `links`. Extend the same mechanism so `LinkType::Hashtag` produces `LabelRow`s that are bulk-inserted into the new `labels` table. Reuse the existing `BulkInsertRow` trait. Delete existing labels for the note before insert (handled by `NoteBatch::flush` symmetric `bulk_delete_in`).

**Files:**
- Modify: `core/src/db/mod.rs` (add `LabelRow`, extend `NoteBatch`, extend `delete_notes` near line 736)
- Test: `core/src/db/mod.rs` tests module

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `core/src/db/mod.rs`:

```rust
#[tokio::test]
async fn labels_are_persisted_on_note_insert() {
    use crate::nfs::{NoteEntryData, VaultPath};

    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("kimun.sqlite");
    let db = super::VaultDB::new(&db_path).await.unwrap();
    super::create_tables(db.pool()).await.unwrap();

    let path = VaultPath::note_path_from("/n.md");
    let body = "Title\n\nbody with #foo and #Foo and #bar".to_string();
    let entry = NoteEntryData {
        path: path.clone(),
        size: body.len() as u64,
        modified_secs: 0,
    };

    let mut tx = db.pool().begin().await.unwrap();
    super::insert_notes(&mut tx, &[(entry, body)]).await.unwrap();
    tx.commit().await.unwrap();

    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT name, path FROM labels ORDER BY name")
            .fetch_all(db.pool())
            .await
            .unwrap();
    assert_eq!(
        rows,
        vec![
            ("bar".to_string(), path.to_string()),
            ("foo".to_string(), path.to_string()),
        ],
        "labels stored deduped + lowercased"
    );

    db.close().await.unwrap();
}

#[tokio::test]
async fn reindexing_a_note_drops_removed_labels() {
    use crate::nfs::{NoteEntryData, VaultPath};

    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("kimun.sqlite");
    let db = super::VaultDB::new(&db_path).await.unwrap();
    super::create_tables(db.pool()).await.unwrap();

    let path = VaultPath::note_path_from("/n.md");
    let body_v1 = "before #draft #keep".to_string();
    let entry_v1 = NoteEntryData {
        path: path.clone(),
        size: body_v1.len() as u64,
        modified_secs: 0,
    };

    let mut tx = db.pool().begin().await.unwrap();
    super::insert_notes(&mut tx, &[(entry_v1, body_v1)]).await.unwrap();
    tx.commit().await.unwrap();

    let body_v2 = "after #keep only".to_string();
    let entry_v2 = NoteEntryData {
        path: path.clone(),
        size: body_v2.len() as u64,
        modified_secs: 1,
    };

    let mut tx = db.pool().begin().await.unwrap();
    super::update_notes(&mut tx, &[(entry_v2, body_v2)]).await.unwrap();
    tx.commit().await.unwrap();

    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT name FROM labels WHERE path = ? ORDER BY name",
    )
    .bind(path.to_string())
    .fetch_all(db.pool())
    .await
    .unwrap();
    assert_eq!(
        rows.into_iter().map(|(n,)| n).collect::<Vec<_>>(),
        vec!["keep".to_string()],
        "reindex must drop labels no longer present"
    );

    db.close().await.unwrap();
}

#[tokio::test]
async fn labels_are_removed_on_note_delete() {
    use crate::nfs::{NoteEntryData, VaultPath};

    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("kimun.sqlite");
    let db = super::VaultDB::new(&db_path).await.unwrap();
    super::create_tables(db.pool()).await.unwrap();

    let path = VaultPath::note_path_from("/n.md");
    let body = "x #drop".to_string();
    let entry = NoteEntryData {
        path: path.clone(),
        size: body.len() as u64,
        modified_secs: 0,
    };

    let mut tx = db.pool().begin().await.unwrap();
    super::insert_notes(&mut tx, &[(entry, body)]).await.unwrap();
    super::delete_notes(&mut tx, &[path.clone()]).await.unwrap();
    tx.commit().await.unwrap();

    let count: (i64,) =
        sqlx::query_as("SELECT count(*) FROM labels WHERE path = ?")
            .bind(path.to_string())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(count.0, 0);

    db.close().await.unwrap();
}
```

- [ ] **Step 2: Run tests, expect failure**

```
cargo test -p kimun-core db::tests::labels_are_ -- --nocapture
```

Expected: FAIL — no rows inserted into `labels` and the delete path doesn't touch it.

- [ ] **Step 3: Add `LabelRow` and its `BulkInsertRow` impl**

In `core/src/db/mod.rs`, add a `LabelRow` struct alongside `LinkRow` (around line 787):

```rust
struct LabelRow {
    path_idx: usize,
    name: String,
}
```

Then add the `BulkInsertRow` impl alongside the others (after the `LinkRow` impl, around line 1010):

```rust
impl BulkInsertRow for LabelRow {
    const HEADER: &'static str = "INSERT OR IGNORE INTO labels (name, path) VALUES ";
    const FOOTER: &'static str = "";
    const COLS: usize = 2;

    fn bind_to<'q>(
        &'q self,
        q: sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
        paths: &'q [String],
    ) -> sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
        q.bind(&self.name).bind(&paths[self.path_idx])
    }
}
```

- [ ] **Step 4: Extend `NoteBatch` to collect labels**

In the `NoteBatch` struct (line 820), add a `labels` field:

```rust
struct NoteBatch {
    paths: Vec<String>,
    notes: Vec<NoteRow>,
    chunks: Vec<ChunkRow>,
    links: Vec<LinkRow>,
    labels: Vec<LabelRow>,
}
```

In `NoteBatch::with_capacity`, initialize it:

```rust
fn with_capacity(notes: usize, chunks: usize, links: usize) -> Self {
    Self {
        paths: Vec::with_capacity(notes),
        notes: Vec::with_capacity(notes),
        chunks: Vec::with_capacity(chunks),
        links: Vec::with_capacity(links),
        labels: Vec::with_capacity(notes), // typical: a few per note
    }
}
```

In `NoteBatch::push`, after the existing `for l in links { ... }` loop that handles `LinkType::Note`, add a parallel branch:

```rust
for l in &links {
    if let LinkType::Hashtag = &l.ltype {
        // `l.text` already holds the tag without the leading `#`.
        let normalized = l.text.to_lowercase();
        if normalized.is_empty() {
            continue;
        }
        self.labels.push(LabelRow {
            path_idx: idx,
            name: normalized,
        });
    }
}
```

(Place this AFTER the existing `for l in links { ... if let LinkType::Note(p) = &l.ltype { ... } }` block; iterate by reference for the new branch because the original loop consumes `links` — change the original to `for l in &links` as well, or hoist `links` into a variable consumed once and iterate by reference both times. Use a single `for l in &links { match &l.ltype { LinkType::Note(p) => { ... } LinkType::Hashtag => { ... } _ => {} } }` block. Cleaner:)

Replace the existing block:

```rust
for l in links {
    if let LinkType::Note(p) = &l.ltype {
        self.links.push(LinkRow {
            path_idx: idx,
            destination: p.to_string(),
        });
    }
}
```

with:

```rust
for l in &links {
    match &l.ltype {
        LinkType::Note(p) => {
            self.links.push(LinkRow {
                path_idx: idx,
                destination: p.to_string(),
            });
        }
        LinkType::Hashtag => {
            let normalized = l.text.to_lowercase();
            if !normalized.is_empty() {
                self.labels.push(LabelRow {
                    path_idx: idx,
                    name: normalized,
                });
            }
        }
        _ => {}
    }
}
```

(Note: `links` is now a borrow. The signature of `push` already takes `links: Vec<NoteLink>` — it owns the vec; iterating by reference inside is fine.)

- [ ] **Step 5: Extend `NoteBatch::flush` to delete-then-insert labels**

Update `NoteBatch::flush` (around line 873):

```rust
async fn flush(self, tx: &mut Transaction<'_, Sqlite>) -> Result<(), DBError> {
    bulk_upsert_note_rows(tx, &self.notes, &self.paths).await?;
    bulk_delete_in(tx, "notesContent", &["path"], &self.paths).await?;
    bulk_delete_in(tx, "links", &["source"], &self.paths).await?;
    bulk_delete_in(tx, "labels", &["path"], &self.paths).await?;
    bulk_insert(tx, &self.chunks, &self.paths).await?;
    bulk_insert(tx, &self.links, &self.paths).await?;
    bulk_insert(tx, &self.labels, &self.paths).await?;
    Ok(())
}
```

- [ ] **Step 6: Extend `delete_notes`**

In `core/src/db/mod.rs` around line 736, add label cleanup:

```rust
pub async fn delete_notes(
    tx: &mut Transaction<'_, Sqlite>,
    paths: &[VaultPath],
) -> Result<(), DBError> {
    if paths.is_empty() {
        return Ok(());
    }
    let path_strings: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
    bulk_delete_in(tx, "notes", &["path"], &path_strings).await?;
    bulk_delete_in(tx, "notesContent", &["path"], &path_strings).await?;
    bulk_delete_in(tx, "links", &["source", "destination"], &path_strings).await?;
    bulk_delete_in(tx, "labels", &["path"], &path_strings).await?;
    Ok(())
}
```

- [ ] **Step 7: Run tests, expect pass**

```
cargo test -p kimun-core db::tests::labels_are_
```

Expected: both PASS.

- [ ] **Step 8: Sanity check existing tests still pass**

```
cargo test -p kimun-core
```

Expected: no regressions.

- [ ] **Step 9: Commit**

```bash
git add core/src/db/mod.rs
git commit -m "feat(core/db): persist hashtag labels via NoteBatch and delete on note removal"
```

---

## Task 5: Core API — `list_labels` and `notes_with_label`

**Why:** Specs require `NoteVault::list_labels()` and `NoteVault::notes_with_label(name)` to expose the labels table without scanning note bodies. Both go through the `labels` index.

**Files:**
- Modify: `core/src/db/mod.rs` (new public-in-crate functions near other readers like `get_all_notes`)
- Modify: `core/src/lib.rs` (new `NoteVault` methods near `search_notes`)
- Test: `core/src/lib.rs` (new `#[cfg(test)] mod label_api_tests`)

- [ ] **Step 1: Write the failing test**

At the bottom of `core/src/lib.rs`, add (or extend an existing tests module):

```rust
#[cfg(test)]
mod label_api_tests {
    use super::*;
    use crate::nfs::VaultPath;

    async fn new_vault() -> (tempfile::TempDir, NoteVault) {
        let tmp = tempfile::TempDir::new().unwrap();
        let cfg = VaultConfig::new(tmp.path().to_path_buf());
        let vault = NoteVault::new(cfg).await.unwrap();
        (tmp, vault)
    }

    #[tokio::test]
    async fn list_labels_returns_distinct_lowercase_names() {
        let (_tmp, vault) = new_vault().await;
        vault
            .create_note(&VaultPath::note_path_from("/a.md"), "x #Foo and #bar")
            .await
            .unwrap();
        vault
            .create_note(&VaultPath::note_path_from("/b.md"), "y #foo only")
            .await
            .unwrap();

        let mut labels = vault.list_labels().await.unwrap();
        labels.sort();
        assert_eq!(labels, vec!["bar".to_string(), "foo".to_string()]);
    }

    #[tokio::test]
    async fn notes_with_label_is_case_insensitive() {
        let (_tmp, vault) = new_vault().await;
        let a = VaultPath::note_path_from("/a.md");
        let b = VaultPath::note_path_from("/b.md");
        vault.create_note(&a, "x #Important").await.unwrap();
        vault.create_note(&b, "x #important #other").await.unwrap();

        let mut paths = vault.notes_with_label("IMPORTANT").await.unwrap();
        paths.sort_by_key(|p| p.to_string());
        assert_eq!(paths, vec![a, b]);
    }

    #[tokio::test]
    async fn notes_with_unknown_label_returns_empty() {
        let (_tmp, vault) = new_vault().await;
        vault
            .create_note(&VaultPath::note_path_from("/a.md"), "x")
            .await
            .unwrap();
        let paths = vault.notes_with_label("nosuch").await.unwrap();
        assert!(paths.is_empty());
    }
}
```

(If `NoteVault::new(VaultConfig)` is not the constructor's actual signature, replace with the correct one — check `core/src/lib.rs` around the `impl NoteVault` block.)

- [ ] **Step 2: Run tests, expect failure**

```
cargo test -p kimun-core label_api_tests -- --nocapture
```

Expected: FAIL — methods don't exist.

- [ ] **Step 3: Add DB-layer functions**

In `core/src/db/mod.rs`, near `get_all_notes` (line 542), add:

```rust
pub async fn list_labels(pool: &SqlitePool) -> Result<Vec<String>, DBError> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT DISTINCT name FROM labels")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|(n,)| n).collect())
}

pub async fn notes_with_label(
    pool: &SqlitePool,
    name: &str,
) -> Result<Vec<VaultPath>, DBError> {
    let normalized = name.to_lowercase();
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT path FROM labels WHERE name = ?",
    )
    .bind(&normalized)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(p,)| VaultPath::new(p)).collect())
}
```

- [ ] **Step 4: Add `NoteVault` wrappers**

In `core/src/lib.rs`, inside the `impl NoteVault` block, near `search_notes` (line 405), add:

```rust
/// Returns every distinct label persisted in the vault, lowercased.
pub async fn list_labels(&self) -> Result<Vec<String>, VaultError> {
    Ok(db::list_labels(self.vault_db.pool()).await?)
}

/// Returns every note path that carries the given label. The label
/// argument is lowercased before lookup, matching how labels are stored.
pub async fn notes_with_label<S: AsRef<str>>(
    &self,
    name: S,
) -> Result<Vec<VaultPath>, VaultError> {
    Ok(db::notes_with_label(self.vault_db.pool(), name.as_ref()).await?)
}
```

- [ ] **Step 5: Run tests, expect pass**

```
cargo test -p kimun-core label_api_tests
```

Expected: 3 PASS.

- [ ] **Step 6: Commit**

```bash
git add core/src/lib.rs core/src/db/mod.rs
git commit -m "feat(core): NoteVault::list_labels and NoteVault::notes_with_label"
```

---

## Task 6: Search syntax — `#<label>` and `lb:<label>` parser entries

**Why:** Extend `SearchTerms` so the existing query parser recognizes label filters. Add `Label` / `ExcludedLabel` to `ElementType`; register the prefixes longest-first.

**Files:**
- Modify: `core/src/db/search_terms.rs` (the whole file is small; touch `ElementType`, `prefix_table`, `SearchTerms`, `from_query_string`)
- Test: same file's tests module.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `core/src/db/search_terms.rs`:

```rust
#[test]
fn search_label_short() {
    let s = SearchTerms::from_query_string("#important");
    assert_eq!(s.labels, vec!["important".to_string()]);
    assert!(s.terms.is_empty());
}

#[test]
fn search_label_long() {
    let s = SearchTerms::from_query_string("lb:important");
    assert_eq!(s.labels, vec!["important".to_string()]);
}

#[test]
fn search_label_case_normalized() {
    let s = SearchTerms::from_query_string("#Important");
    assert_eq!(s.labels, vec!["important".to_string()]);
}

#[test]
fn search_label_excluded_short() {
    let s = SearchTerms::from_query_string("-#draft");
    // `-#draft` should be parsed as ExcludedTerm "#draft" today; once we
    // register `#-` we want it captured as an excluded label. The canonical
    // excluded forms are `#-draft` and `lb:-draft`.
    let s2 = SearchTerms::from_query_string("#-draft");
    assert_eq!(s2.excluded_labels, vec!["draft".to_string()]);
    let s3 = SearchTerms::from_query_string("lb:-draft");
    assert_eq!(s3.excluded_labels, vec!["draft".to_string()]);
    let _ = s;
}

#[test]
fn search_multiple_labels() {
    let s = SearchTerms::from_query_string("#a #b lb:c");
    let mut labels = s.labels.clone();
    labels.sort();
    assert_eq!(labels, vec!["a", "b", "c"]);
}

#[test]
fn search_label_mixed_with_term() {
    let s = SearchTerms::from_query_string("meeting #important");
    assert_eq!(s.labels, vec!["important".to_string()]);
    assert_eq!(s.terms, vec!["meeting".to_string()]);
}

#[test]
fn search_bare_hash_is_dropped() {
    let s = SearchTerms::from_query_string("#");
    assert!(s.labels.is_empty());
    assert!(s.terms.is_empty());
}
```

- [ ] **Step 2: Run tests, expect failure**

```
cargo test -p kimun-core db::search_terms::tests::search_label -- --nocapture
```

Expected: FAIL — `labels` / `excluded_labels` fields and the new prefix do not exist.

- [ ] **Step 3: Extend `ElementType`**

In `core/src/db/search_terms.rs`, add to the `enum ElementType`:

```rust
enum ElementType {
    Invalid,
    Term,
    In,
    At,
    Path,
    OrderBy { asc: bool },
    ExcludedTerm,
    ExcludedIn,
    ExcludedAt,
    ExcludedPath,
    Label,
    ExcludedLabel,
}
```

- [ ] **Step 4: Register the new prefixes**

Update `prefix_table()` to length 8 and add label entries longest-first:

```rust
type PrefixEntry = (&'static str, &'static str, fn() -> ElementType);

fn prefix_table() -> [PrefixEntry; 8] {
    [
        ("in:-", ">-", || ElementType::ExcludedIn),
        ("at:-", "@-", || ElementType::ExcludedAt),
        ("pt:-", "/-", || ElementType::ExcludedPath),
        ("lb:-", "#-", || ElementType::ExcludedLabel),
        ("in:", ">", || ElementType::In),
        ("at:", "@", || ElementType::At),
        ("pt:", "/", || ElementType::Path),
        ("lb:", "#", || ElementType::Label),
    ]
}
```

- [ ] **Step 5: Add fields and populate**

Extend `SearchTerms`:

```rust
#[derive(Default, Debug)]
pub struct SearchTerms {
    pub terms: Vec<String>,
    pub breadcrumb: Vec<String>,
    pub order_by: Vec<OrderBy>,
    pub filename: Vec<String>,
    pub path: Vec<String>,
    pub labels: Vec<String>,
    pub excluded_terms: Vec<String>,
    pub excluded_breadcrumb: Vec<String>,
    pub excluded_filename: Vec<String>,
    pub excluded_path: Vec<String>,
    pub excluded_labels: Vec<String>,
}
```

In `SearchTerms::from_query_string`, add the new accumulators and match arms:

```rust
let mut labels = vec![];
let mut excluded_labels = vec![];
// ... inside the match:
ElementType::Label => {
    let n = qp.term.to_lowercase();
    if !n.is_empty() {
        labels.push(n);
    }
}
ElementType::ExcludedLabel => {
    let n = qp.term.to_lowercase();
    if !n.is_empty() {
        excluded_labels.push(n);
    }
}
```

And include them in the constructor return at the bottom:

```rust
Self {
    breadcrumb,
    filename,
    order_by,
    terms,
    path,
    labels,
    excluded_terms,
    excluded_breadcrumb,
    excluded_filename,
    excluded_path,
    excluded_labels,
}
```

- [ ] **Step 6: Run tests, expect pass**

```
cargo test -p kimun-core db::search_terms::tests
```

Expected: all (existing + 7 new) PASS.

- [ ] **Step 7: Commit**

```bash
git add core/src/db/search_terms.rs
git commit -m "feat(core/search): parse #label and lb:label query syntax"
```

---

## Task 7: Search SQL — label join

**Why:** Each positive label adds an `INTERSECT` query that constrains by `labels.name = ?`. Each excluded label adds a `NOT IN (SELECT path FROM labels WHERE name = ?)` clause attached to a notes-only base SELECT. The result set is restricted to notes with ALL positive labels and NONE of the excluded ones.

**Files:**
- Modify: `core/src/db/mod.rs` (extend `build_search_sql_query_inner` near line 351 + new `add_labels_query`)
- Test: `core/src/db/mod.rs` tests module

- [ ] **Step 1: Write the failing test (unit + integration)**

Append to `mod tests` in `core/src/db/mod.rs`:

```rust
#[test]
fn test_search_terms_query_label_only() {
    let (sql, params) = build_search_sql_query("#important");
    assert_eq!(params, vec!["important".to_string()]);
    assert!(
        sql.contains("FROM notes") && sql.contains("labels"),
        "query should join notes with labels: {}",
        sql
    );
}

#[test]
fn test_search_terms_query_two_labels_intersect() {
    let (sql, params) = build_search_sql_query("#a #b");
    assert_eq!(params.len(), 2);
    assert!(sql.contains("INTERSECT"), "two labels should INTERSECT: {}", sql);
}

#[tokio::test]
async fn search_by_label_returns_matching_notes() {
    use crate::nfs::{NoteEntryData, VaultPath};
    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("kimun.sqlite");
    let db = super::VaultDB::new(&db_path).await.unwrap();
    super::create_tables(db.pool()).await.unwrap();

    let entries: Vec<(NoteEntryData, String)> = vec![
        (
            NoteEntryData {
                path: VaultPath::note_path_from("/a.md"),
                size: 10,
                modified_secs: 0,
            },
            "a #important #todo".to_string(),
        ),
        (
            NoteEntryData {
                path: VaultPath::note_path_from("/b.md"),
                size: 10,
                modified_secs: 0,
            },
            "b #todo".to_string(),
        ),
        (
            NoteEntryData {
                path: VaultPath::note_path_from("/c.md"),
                size: 10,
                modified_secs: 0,
            },
            "c plain".to_string(),
        ),
    ];

    let mut tx = db.pool().begin().await.unwrap();
    super::insert_notes(&mut tx, &entries).await.unwrap();
    tx.commit().await.unwrap();

    let results = super::search_terms(db.pool(), "#important").await.unwrap();
    let paths: Vec<String> = results.iter().map(|(e, _)| e.path.to_string()).collect();
    assert_eq!(paths, vec!["/a.md".to_string()]);

    let results = super::search_terms(db.pool(), "#important #todo").await.unwrap();
    let paths: Vec<String> = results.iter().map(|(e, _)| e.path.to_string()).collect();
    assert_eq!(paths, vec!["/a.md".to_string()]);

    let results = super::search_terms(db.pool(), "#nope").await.unwrap();
    assert!(results.is_empty());

    db.close().await.unwrap();
}

#[tokio::test]
async fn label_search_uses_labels_by_name_index() {
    // Confirms the labels_by_name index is on the query plan for label
    // filters. Without the index a hashtag filter would degrade to a
    // table scan against `labels`, which would silently get slow for
    // large vaults.
    use crate::nfs::{NoteEntryData, VaultPath};
    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("kimun.sqlite");
    let db = super::VaultDB::new(&db_path).await.unwrap();
    super::create_tables(db.pool()).await.unwrap();

    let entry = NoteEntryData {
        path: VaultPath::note_path_from("/a.md"),
        size: 10,
        modified_secs: 0,
    };
    let mut tx = db.pool().begin().await.unwrap();
    super::insert_notes(&mut tx, &[(entry, "x #important".to_string())])
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let (sql, _) = super::build_search_sql_query("#important");
    let plan_sql = format!("EXPLAIN QUERY PLAN {}", sql);
    let rows: Vec<(i64, i64, i64, String)> =
        sqlx::query_as(&plan_sql).bind("important").fetch_all(db.pool()).await.unwrap();
    let plan_text = rows
        .iter()
        .map(|(_, _, _, detail)| detail.as_str())
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        plan_text.contains("labels_by_name"),
        "labels_by_name should appear in the query plan: {}",
        plan_text
    );

    db.close().await.unwrap();
}
```

- [ ] **Step 2: Run tests, expect failure**

```
cargo test -p kimun-core db::tests::test_search_terms_query_label db::tests::search_by_label -- --nocapture
```

Expected: FAIL — no `add_labels_query` wired in.

- [ ] **Step 3: Add `add_labels_query`**

In `core/src/db/mod.rs`, near `add_path_query` (line 482), add:

```rust
/// Build a notes-only base SELECT identical to `search_base_sql` but without
/// the `notesContent` join, so label-only queries don't pay an FTS scan.
static LABEL_BASE_SQL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    let rest = NOTE_COLUMNS.split_once(", ").unwrap().1;
    format!(
        "SELECT DISTINCT notes.path as path, {} FROM notes",
        rest
    )
});

fn label_base_sql() -> &'static str {
    &LABEL_BASE_SQL
}

fn add_labels_query(
    s: &SearchTerms,
    var_num: &mut usize,
    params: &mut Vec<String>,
    queries: &mut Vec<String>,
) {
    // Each positive label becomes its own INTERSECT branch:
    //   SELECT ... FROM notes WHERE notes.path IN (SELECT path FROM labels WHERE name = ?)
    for label in &s.labels {
        let q = format!(
            "{} WHERE notes.path IN (SELECT path FROM labels WHERE name = ?{})",
            label_base_sql(),
            var_num
        );
        queries.push(q);
        params.push(label.clone());
        *var_num += 1;
    }

    // Excluded labels: NOT IN, packaged in a notes-only SELECT so the
    // INTERSECT machinery handles the join.
    if !s.excluded_labels.is_empty() {
        let mut clauses = Vec::with_capacity(s.excluded_labels.len());
        for label in &s.excluded_labels {
            clauses.push(format!(
                "notes.path NOT IN (SELECT path FROM labels WHERE name = ?{})",
                var_num
            ));
            params.push(label.clone());
            *var_num += 1;
        }
        queries.push(format!(
            "{} WHERE {}",
            label_base_sql(),
            clauses.join(" AND ")
        ));
    }
}
```

- [ ] **Step 4: Wire it into `build_search_sql_query_inner`**

In `build_search_sql_query_inner` (line 351), add the call:

```rust
fn build_search_sql_query_inner(search_terms: &SearchTerms) -> (String, Vec<String>) {
    let mut var_num = 1;
    let mut params: Vec<String> = vec![];
    let mut queries: Vec<String> = vec![];

    add_content_terms_query(search_terms, &mut var_num, &mut params, &mut queries);
    add_breadcrumb_query(search_terms, &mut var_num, &mut params, &mut queries);
    add_filename_query(search_terms, &mut var_num, &mut params, &mut queries);
    add_path_query(search_terms, &mut var_num, &mut params, &mut queries);
    add_labels_query(search_terms, &mut var_num, &mut params, &mut queries);

    if queries.is_empty() {
        debug!("No query provided");
        return (String::new(), vec![]);
    }
    (queries.join(" INTERSECT "), params)
}
```

- [ ] **Step 5: Run tests, expect pass**

```
cargo test -p kimun-core db::tests::test_search_terms_query_label db::tests::search_by_label
```

Expected: PASS.

- [ ] **Step 6: Run the full core test suite**

```
cargo test -p kimun-core
```

Expected: no regressions on the existing search SQL snapshot tests. If a snapshot test fails because `add_labels_query` now changes the parameter count for queries that have no labels — it does NOT, because the helper returns early when both vectors are empty. Confirm.

- [ ] **Step 7: Commit**

```bash
git add core/src/db/mod.rs
git commit -m "feat(core/db): label-join in search SQL with INTERSECT semantics"
```

---

## Task 8: TUI — `ElementKind::Label` and rendering

**Why:** Hashtags must be visually distinct from generic links. Add a new `ElementKind::Label` variant emitted by the line parser, and apply a dedicated style in the render path.

**Files:**
- Modify: `tui/src/components/text_editor/markdown.rs` (add variant + parser branch)
- Modify: the style-application site for `ElementKind` (search for `ElementKind::WikiLink` matches inside `tui/src/components/text_editor/`)
- Test: `tui/src/components/text_editor/markdown.rs` `#[cfg(test)] mod tests`

- [ ] **Step 1: Locate the style-application site**

Run:

```
rg -n "ElementKind::WikiLink" tui/src/components/text_editor
```

Expected: one or more matches in `view.rs` or `mod.rs`. Note the exact file:line — that's where the `Label` arm goes.

- [ ] **Step 2: Write the failing test**

`ElementKind` derives `Copy`, so the new `Label` variant must be a unit (no payload). The label name is derived at call-time from `line[element.start_char..element.end_char]` (strip the leading `#`). `.elements` is a public field on `ParsedLine`. Append to the existing tests module in `tui/src/components/text_editor/markdown.rs`:

```rust
#[test]
fn parse_line_emits_label_for_hashtag() {
    let line = "see #rust later";
    let parsed = ParsedLine::parse(line);
    let label = parsed
        .elements
        .iter()
        .find(|e| matches!(e.kind, ElementKind::Label));
    assert!(label.is_some(), "expected Label element: {:?}", parsed.elements);
    let l = label.unwrap();
    let span: String = line.chars().skip(l.start_char).take(l.end_char - l.start_char).collect();
    assert_eq!(span, "#rust");
}

#[test]
fn parse_line_skips_label_inside_inline_code() {
    let parsed = ParsedLine::parse("use `#foo` here");
    let has_label = parsed
        .elements
        .iter()
        .any(|e| matches!(e.kind, ElementKind::Label));
    assert!(!has_label, "should not emit Label inside inline code");
}
```

- [ ] **Step 3: Run, expect failure**

```
cargo test -p kimun-tui markdown::tests::parse_line -- --nocapture
```

Expected: FAIL.

- [ ] **Step 4: Add the `Label` variant**

In `tui/src/components/text_editor/markdown.rs`, extend the `ElementKind` enum at line 45 (preserving the `Copy` derive — variant is unit):

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ElementKind {
    Bold,
    Italic,
    Strikethrough,
    InlineCode,
    Link,
    HeadingH1,
    HeadingH2,
    HeadingH3,
    Blockquote,
    WikiLink,
    Image,
    Label,
}
```

- [ ] **Step 5: Extend the line parser to emit `Label`**

Find where the parser turns `pulldown-cmark` events into `Element { start_char, end_char, kind }` spans (around line 408 — `out.push(ParsedLine { ... })` — and the helper `tag_to_element_kind` near line 1020). The parser already tracks `InlineCode` ranges via `Event::Code`. After all events for a line have been collected, scan the line's source text for hashtag matches and push `Element { kind: ElementKind::Label, start_char, end_char }` for each match whose char range does NOT overlap any existing `ElementKind::InlineCode` element on the same line.

Concrete: in the per-line finalization where `ParsedLine` is built, after `elements` is populated, run:

```rust
for caps in HASHTAG_RX.captures_iter(line) {
    let m = caps.get(0).unwrap();
    let start_char = line[..m.start()].chars().count();
    let end_char = start_char + m.as_str().chars().count();
    let overlaps_code = elements.iter().any(|e| {
        matches!(e.kind, ElementKind::InlineCode)
            && !(end_char <= e.start_char || start_char >= e.end_char)
    });
    if !overlaps_code {
        elements.push(Element {
            start_char,
            end_char,
            kind: ElementKind::Label,
        });
    }
}
```

Add a `use regex::Regex; use std::sync::LazyLock;` block at the top of the file if not already present, with a private `HASHTAG_RX` mirroring the one in `core/src/note/content_extractor.rs` (do NOT cross-crate import — keep the TUI self-contained):

```rust
static HASHTAG_RX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#(?P<ht_text>[A-Za-z0-9_]+)").unwrap());
```

After pushing `Label` elements, you may need to re-sort `elements` by `start_char` and refresh the precomputed `elem_vis` / `elem_index` arrays — locate where those are built (above the `out.push(ParsedLine { ... })` block) and ensure `Label` elements participate. Fenced code blocks are out of scope here: they're rendered line-by-line and the parser already classifies them as `InlineCode`-equivalent or via a separate path; verify the per-line input does not contain fence content before treating a hashtag as a label. If fenced lines are passed through verbatim, the spec scenario for fenced-block exclusion is satisfied by the core-side label store (Task 3) not surfacing the label, even if the TUI highlights it locally; that's acceptable because the TUI highlight is purely cosmetic when the label is not actually associated with the note.

- [ ] **Step 6: Apply a label style at the render site**

Open the file identified in Step 1. In the `match` arm that styles `ElementKind::Link` / `ElementKind::WikiLink`, add:

```rust
ElementKind::Label => theme.label_style(),
```

If the theme module has no `label_style()` yet, add one in `tui/src/.../theme.rs`:

```rust
pub fn label_style(&self) -> Style {
    // Fall back to link style so themes without an explicit label color
    // still render labels visibly distinct from body text.
    self.label.unwrap_or_else(|| self.link_style())
}
```

And add a `label: Option<Style>` field to the theme struct, default `None`. (Exact path of theme module: find with `rg -n "link_style\|fn link_style" tui/src`.)

- [ ] **Step 7: Run all TUI tests, expect pass**

```
cargo test -p kimun-tui
```

Expected: PASS (including the 2 new parser tests).

- [ ] **Step 8: Commit**

```bash
git add tui/src/components/text_editor/ tui/src/
git commit -m "feat(tui/editor): highlight #hashtag spans as Label elements"
```

---

## Task 9: TUI — search modal accepts initial query

**Why:** Step toward Task 10's follow-link integration. The note-browser modal must accept a pre-filled query string and run the first search synchronously.

**Files:**
- Modify: `tui/src/components/note_browser/mod.rs` (the modal's construction / open path)
- Modify: callers that construct the modal (search for the existing entry points; the `SearchNotes` action dispatches it)
- Test: `tui/src/components/note_browser/mod.rs` `#[cfg(test)] mod tests`

- [ ] **Step 1: Locate the existing construction site**

```
rg -n "NoteBrowserModal::new\|NoteBrowserModal::default\|NoteBrowserModal {" tui/src
```

Note every caller — they all need updating (one of them is the `Ctrl+K` `SearchNotes` dispatch in `tui/src/app_screen/editor.rs:530`).

- [ ] **Step 2: Write the failing test**

In the modal's file, add:

```rust
#[cfg(test)]
mod label_open_tests {
    use super::*;
    // The exact test type depends on how the modal is constructed in tests.
    // Aim for a unit test that constructs the modal with an initial query
    // and asserts the query input text equals the initial query.

    #[test]
    fn modal_constructed_with_initial_query_prefills_input() {
        let modal = NoteBrowserModal::with_initial_query("#important");
        assert_eq!(modal.query_text(), "#important");
        // Cursor at end:
        assert_eq!(modal.cursor_position(), "#important".chars().count());
    }
}
```

(Replace `query_text()` / `cursor_position()` with the modal's actual accessors; if they don't exist, add narrow accessors that return what's needed for the assertion. They can be `#[cfg(test)] pub(super)`.)

- [ ] **Step 3: Run, expect failure**

```
cargo test -p kimun-tui note_browser::label_open_tests -- --nocapture
```

Expected: FAIL — `with_initial_query` does not exist.

- [ ] **Step 4: Add the constructor**

Add to `NoteBrowserModal`:

```rust
pub fn with_initial_query<S: Into<String>>(query: S) -> Self {
    let mut modal = Self::new(); // or whatever default constructor exists
    let q = query.into();
    modal.set_query(&q);
    modal
}

pub fn set_query(&mut self, q: &str) {
    // Set the text and move cursor to end. Use whatever the existing
    // input-component API exposes (likely a TextInput-like type).
    self.query_input.set_text(q);
    self.query_input.move_cursor_to_end();
    // Trigger first search synchronously if the modal supports that;
    // otherwise rely on the existing input-change reaction.
    self.refresh_results();
}
```

(Wire up `refresh_results` to whichever async task the modal already uses to populate results on input change.)

- [ ] **Step 5: Update existing callers**

Every caller from Step 1: replace `NoteBrowserModal::new()` with a default that passes no initial query, or call `with_initial_query("")` only if needed. The `Ctrl+K` path should keep the empty-query behavior — make `new()` (or the default constructor) keep doing what it does today.

- [ ] **Step 5b: Add an explicit `Ctrl+K` regression test**

Add to the same `label_open_tests` mod:

```rust
#[test]
fn modal_default_constructor_has_empty_query() {
    let modal = NoteBrowserModal::new();
    assert_eq!(modal.query_text(), "");
}
```

Run:

```
cargo test -p kimun-tui note_browser::label_open_tests::modal_default_constructor_has_empty_query
```

Expected: PASS. This locks the `Ctrl+K` behavior in place so a future refactor cannot accidentally route the default open path through `with_initial_query("...")`.

- [ ] **Step 6: Run, expect pass**

```
cargo test -p kimun-tui note_browser
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add tui/src/components/note_browser/ tui/src/app_screen/editor.rs
git commit -m "feat(tui/note_browser): accept pre-filled initial query on open"
```

---

## Task 10: TUI — `follow_link` on a hashtag opens the search modal

**Why:** The user's discovery flow says `Ctrl+G` over `#tag` opens the same search modal opened by `Ctrl+K`, pre-filled with `#tag`. Plumb the new modal entry point into `follow_link`.

**Files:**
- Modify: `tui/src/app_screen/editor.rs` (`follow_link`, around line 185)
- Modify: `tui/src/components/text_editor/mod.rs` (`link_at_cursor`, if it needs to surface the new `Label` element kind)
- Test: prefer an existing integration-test entry point in `tui/`; if none exists, a unit test on the dispatch helper used by `follow_link` is fine.

- [ ] **Step 1: Make sure `link_at_cursor` returns Label info**

Open `tui/src/components/text_editor/mod.rs` and find `link_at_cursor`. Add a branch (or extend the existing return type) so it can yield a `Label { name }` variant that `follow_link` will match on. Add a small unit test in that file:

```rust
#[test]
fn link_at_cursor_finds_label() {
    // Build an editor state with cursor on `#rust` in a line `see #rust now`.
    // Replace `<construct>` and `<advance_cursor_to>` with the real test helpers.
    let editor = <construct>("see #rust now");
    <advance_cursor_to>(&editor, "see #ru".chars().count());
    let target = editor.link_at_cursor();
    assert!(matches!(
        target,
        Some(LinkTarget::Label(ref n)) if n == "rust"
    ));
}
```

Define `LinkTarget::Label(String)` alongside the existing variants in whichever enum `link_at_cursor` returns.

- [ ] **Step 2: Run, expect failure**

```
cargo test -p kimun-tui text_editor::link_at_cursor_finds_label -- --nocapture
```

Expected: FAIL.

- [ ] **Step 3: Implement the cursor-on-label detection**

Inside `link_at_cursor`, when iterating the cursor line's parsed elements, find the `Element` whose `kind == ElementKind::Label` and whose `start_char..end_char` range contains the cursor's char column. Derive the label name from the source line by skipping the leading `#`:

```rust
if let Some(e) = parsed
    .elements
    .iter()
    .find(|e| e.kind == ElementKind::Label
        && cursor_col >= e.start_char
        && cursor_col < e.end_char)
{
    let span: String = line
        .chars()
        .skip(e.start_char)
        .take(e.end_char - e.start_char)
        .collect();
    let name = span.trim_start_matches('#').to_string();
    return Some(LinkTarget::Label(name));
}
```

- [ ] **Step 4: Run, expect pass**

```
cargo test -p kimun-tui text_editor::link_at_cursor_finds_label
```

Expected: PASS.

- [ ] **Step 5: Route `LinkTarget::Label` in `follow_link`**

In `tui/src/app_screen/editor.rs::follow_link` (line 185), at the top of the match arm that handles the link target, add:

```rust
match target {
    LinkTarget::Label(name) => {
        let initial = format!("#{}", name);
        self.open_note_browser_with_query(initial);
        return Ok(());
    }
    // ... existing arms ...
}
```

`open_note_browser_with_query(query)` is a thin wrapper around whatever the existing `Ctrl+K` dispatch path does (the one that ends up constructing `NoteBrowserModal`). If `Ctrl+K` is dispatched via an `Action::SearchNotes` enum that is then handled in a central place, add an `Action::SearchNotesWithQuery(String)` variant and handle it next to `SearchNotes`. Either way, the construction site uses `NoteBrowserModal::with_initial_query(query)` from Task 9.

- [ ] **Step 6: Add an integration test (or smoke test)**

If the project has a TUI test harness that can drive key events: simulate `Ctrl+G` on a line with cursor at `#important` and assert that a `NoteBrowserModal` is now present with `query_text() == "#important"`. If no such harness exists, this is acceptable as a manual verification step — note it in the docs/PR description.

- [ ] **Step 7: Run full TUI suite, expect pass**

```
cargo test -p kimun-tui
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add tui/
git commit -m "feat(tui/editor): follow-link on #label opens search modal pre-filled"
```

---

## Task 11: User-facing docs

**Why:** Project rule: end-user documentation lives in `docs/`. Search-syntax additions and the new hashtag behavior are end-user concerns.

**Files:**
- Add a section to whichever doc covers the search syntax (find with `rg -ln "search" docs/`)
- Add a section to whichever doc covers the editor / keybindings

- [ ] **Step 1: Locate the search-syntax page**

```
rg -ln "search\|query\|in:\|at:" docs/
```

If no dedicated search page exists, add one at `docs/search.md` (or follow the docs site's existing naming convention).

- [ ] **Step 2: Add a labels section**

Document:

- `#<label>` filters to notes carrying the label.
- `lb:<label>` is the long form.
- Negation: `#-<label>` or `lb:-<label>`.
- Labels are case-insensitive.
- Labels come from `#<name>` hashtags inside note text (letters/digits/underscore). Hashtags inside code spans are ignored.

Use the same prose style as the existing syntax docs.

- [ ] **Step 3: Add an editor section**

Document that `Ctrl+G` over a hashtag opens the search modal pre-filled with that label, and that hashtags are highlighted in the editor.

- [ ] **Step 4: Run any docs build the project has**

```
rg -n "mkdocs\|hugo\|docusaurus\|astro\|vitepress\|zola" docs/ .github/
```

If a build command is part of CI, run it locally to confirm the new pages parse. Otherwise skip.

- [ ] **Step 5: Commit**

```bash
git add docs/
git commit -m "docs: document #label / lb:label search syntax and follow-link"
```

---

## Task 12: Full workspace verification

- [ ] **Step 1: Run the whole workspace test suite**

```
cargo test --workspace
```

Expected: PASS.

- [ ] **Step 2: Lint**

```
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: clean. Fix any new warnings before moving on.

- [ ] **Step 3: Manual smoke test of the TUI**

Run the TUI against a scratch vault:

```
cargo run -p kimun-tui
```

- Create a note with the body `Test note #rust #important and \`#nope\` and a line of body.`
- Verify `#rust` and `#important` render with the label color; `` `#nope` `` does not.
- Run search `#rust` from `Ctrl+K`; verify the note appears.
- Run search `#rust #important`; verify only notes carrying both appear.
- Run search `#rust #-draft`; verify exclusion.
- Position cursor on `#rust`, press `Ctrl+G`; verify the search modal opens with `#rust` pre-filled and results visible.

- [ ] **Step 4: Verify migration**

Re-open a vault that pre-dates this change. Confirm the rebuild runs once and `NoteVault::list_labels()` (e.g. via a debug print or a temporary `dbg!` in a startup path) returns expected entries.

- [ ] **Step 5: Final commit (only if Steps 1-4 produced fix-up edits)**

```bash
git status
# If there are edits:
git add <files>
git commit -m "chore: post-verification cleanups for hashtag labels"
```

- [ ] **Step 6: Update OpenSpec tasks status**

Mark off each task in `openspec/changes/add-hashtag-labels/tasks.md` (flip `- [ ]` to `- [x]`). When all are checked, run:

```
openspec status --change add-hashtag-labels
```

Expected: `All artifacts complete!` and (per OpenSpec convention) the change is ready to archive once merged. Archival is out of scope for this plan — it happens after merge via `/opsx:archive`.
