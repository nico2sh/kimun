# TUI Properties (sort + properties dialog) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let TUI users sort a query by a property from the Ctrl+R dialog and view/add/edit/rename/remove the open note's properties in a mouse-friendly dialog.

**Architecture:** Core gains `OrderField::Property`, `NoteVault::property_keys()` and `NoteVault::rename_property()`; every file write stays in core. The TUI gets two reusable widgets (`ButtonRow`, `KeyPicker`), mouse routing into dialogs, a property row in the sort dialog, a new `PropertiesDialog` (list + form states) that spawns core calls itself and reports via `OverlayData`, and three entry points (leader `n p`, command palette — generated from the leader tree — and a clickable status-bar segment). After each write the editor reloads the note from disk.

**Tech Stack:** Rust, ratatui + crossterm, tokio, sqlx (SQLite index), toml_edit 0.25, yaml-rust2.

**Spec:** `2026-10-04-tui-properties-design.md` (repo root). Read it alongside this plan.

## Global Constraints

- No file or path logic in the TUI: all reads/writes go through `NoteVault` / `kimun_core` (`CLAUDE.md`). The TUI must not call `rename`, `copy`, `remove_file`, `create_dir*`, truncating `write` (enforced by `.github/scripts/check-host-fs.py`).
- Never hardcode `.md` or `/`; core's public API uses `VaultPath` for vault paths.
- Query strings are built only with core helpers (`with_order_directive`, `quote_query_term`).
- New frontmatter blocks are TOML (`FrontmatterFormat::Toml`), same as the CLI default.
- A property sort is per query, never persisted: `SortFieldSetting` is unchanged.
- The user commits; tasks end with a green run, never a `git commit`.
- Tests: core → `cargo test -p kimun_core <filter>`; TUI → `cargo test -p kimun-notes --lib <filter>` (the bin target has 0 tests).
- Final gate set (all seven, CI's lint leg), run in Task 12:
  ```bash
  rtk proxy find core server tui -name '*.rs' -exec touch {} +
  cargo fmt --all -- --check
  grep -rn --include='*.rs' -E '\bcrate::' tui/src/ropetext tui/src/server_client | grep -vE '^[^:]+:[0-9]+: *//' | grep -vE '\bcrate::(ropetext|server_client)::'
  python3 .github/scripts/check-host-fs.py
  cargo clippy --workspace --all-targets -- -D warnings
  cargo check --profile bench --workspace --all-targets
  RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
  cargo test --workspace
  cargo fmt --all -- --check   # again, last
  ```

## Review Focus

1. **Key with spaces/quotes in sort** — `^%"due date"` must round-trip through the dialog unchanged (Task 1 test `property_order_round_trips_quoted_key`, Task 4 test `property_sort_survives_order_toggle`).
2. **Rename onto an existing key, differing only in case** (`Status` → `status`) — must be a case-only respelling, not a "duplicate" refusal (Task 3 test `rename_case_only_respells_in_place`).
3. **Unsaved edits when the dialog opens** — the buffer is flushed first; if it stays dirty, no dialog (Task 8 test `properties_open_flushes_dirty_buffer`).
4. **Double click on Save / Delete** — one write only; buttons disabled while in flight (Task 10 test `second_save_while_busy_is_ignored`).
5. **Click in the gap between buttons / outside a modal** — no action (Task 5 test `gap_between_buttons_hits_nothing`, Task 7 test `click_outside_sort_modal_does_nothing`).

---

## File Structure

| File | Responsibility |
|------|----------------|
| `core/src/index/search_terms.rs` | `OrderField::Property`, rendering in `with_order_directive` |
| `core/src/index/mod.rs` | `IndexDB::property_keys()` SQL read |
| `core/src/note/properties/mod.rs` | `PropertyFormatter::rename`, `NoteProperties::rename` |
| `core/src/note/properties/toml_formatter.rs` | TOML rename keeping position/decor |
| `core/src/note/properties/yaml_formatter.rs` | YAML rename by rewriting the key line |
| `core/src/lib.rs` | `NoteVault::property_keys`, `NoteVault::rename_property` |
| `tui/src/components/file_list.rs` | `SortField::Property(String)` (loses `Copy`) |
| `tui/src/components/query_panel.rs` | map property order both ways, title |
| `tui/src/components/button_row.rs` (new) | clickable `[ Label ]` row with hit-testing |
| `tui/src/components/key_picker.rs` (new) | text field + filtered key suggestions |
| `tui/src/components/dialogs/mod.rs` | mouse routing, new variants, new `OverlayData` arms |
| `tui/src/components/dialogs/sort_dialog.rs` | Property cycle, Key row, mouse |
| `tui/src/components/dialogs/properties_dialog.rs` (new) | the properties dialog |
| `tui/src/components/events.rs` | `FileOp::ShowProperties`, `AppEvent::NoteReloadFromDisk`, `AppEvent::PropertyCountLoaded`, `OverlayData::{PropertyKeysLoaded, PropertiesLoaded, PropertyWritten}` |
| `tui/src/app_screen/editor.rs` | open flow, reload, status-bar click, leader dispatch |
| `tui/src/app_screen/doc_meta.rs` | property count for the status bar |
| `tui/src/components/footer_bar.rs` | `⊞ N props` segment + its rect |
| `tui/src/keys/leader.rs` | `LeaderAction::NoteProperties`, tree leaf `n p` |
| `docs/content/using-kimun/tui.md`, `search.md` | user docs |

---

### Task 1: Core — `OrderField::Property`

**Files:**
- Modify: `core/src/index/search_terms.rs:272-279` (enum), `:565-581` (`with_order_directive`), tests module (near `:1923`)

**Interfaces:**
- Produces: `pub enum OrderField { Title, FileName, Property(String) }` — derives `Debug, Clone, PartialEq, Eq` (no longer `Copy`). `with_order_directive(query: &str, field: OrderField, asc: bool) -> String` keeps its signature.

- [ ] **Step 1: Write the failing tests** (append to the existing `mod tests` in `search_terms.rs`)

```rust
#[test]
fn property_order_round_trips_quoted_key() {
    use super::{with_order_directive, OrderField};
    let q = with_order_directive("#work", OrderField::Property("due date".into()), false);
    assert_eq!(q, "#work -or:prop:\"due date\"");
    let st = SearchTerms::from_query_string(&q);
    match st.order_by.as_slice() {
        [OrderBy::Property { key, asc }] => {
            assert_eq!(key, "due date");
            assert!(!asc);
        }
        other => panic!("expected one property order, got {other:?}"),
    }
}

#[test]
fn property_order_is_replaced_not_piled() {
    use super::{with_order_directive, OrderField};
    let q = with_order_directive("x", OrderField::Property("due".into()), true);
    assert_eq!(q, "x or:prop:due");
    let q = with_order_directive(&q, OrderField::FileName, true);
    assert_eq!(q, "x or:file");
    let q = with_order_directive("x ^%due", OrderField::Property("prio".into()), true);
    assert_eq!(q, "x or:prop:prio");
}
```

- [ ] **Step 2: Run, verify they fail**

Run: `cargo test -p kimun_core property_order_`
Expected: compile error — `no variant named Property` on `OrderField`.

- [ ] **Step 3: Implement**

Replace the `OrderField` enum:

```rust
/// The field a query can be ordered by. The asc/desc choice is carried
/// separately by callers; this names only the column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderField {
    /// Order results by note title.
    Title,
    /// Order results by filename.
    FileName,
    /// Order results by a frontmatter property, the key as the user wrote
    /// it; [`with_order_directive`] quotes it when needed.
    Property(String),
}
```

Replace the body of `with_order_directive`:

```rust
pub fn with_order_directive(query: &str, field: OrderField, asc: bool) -> String {
    let base = strip_order_directive(query);
    let field_term = match field {
        OrderField::Title => "title".to_string(),
        OrderField::FileName => "file".to_string(),
        OrderField::Property(key) => format!("prop:{}", quote_query_term(&key)),
    };
    let directive = if asc {
        format!("{}:{}", ORDER_LETTER, field_term)
    } else {
        format!("-{}:{}", ORDER_LETTER, field_term)
    };
    if base.is_empty() {
        directive
    } else {
        format!("{} {}", base, directive)
    }
}
```

Fix existing callers that relied on `Copy` (only `tui/src/components/query_panel.rs:442` builds an `OrderField`; it is moved into the call, so it compiles unchanged).

- [ ] **Step 4: Run, verify pass**

Run: `cargo test -p kimun_core property_order_ && cargo test -p kimun_core with_order`
Expected: all PASS (old `with_order_*` tests still green).

- [ ] **Step 5: Report green** — no commit (user commits).

---

### Task 2: Core — `NoteVault::property_keys()`

**Files:**
- Modify: `core/src/index/mod.rs` (add method next to `dominant_property_kind`, `:722`)
- Modify: `core/src/lib.rs` (add method after `get_property`, `:1300`); test in `mod property_api_tests` (`:4445`)

**Interfaces:**
- Produces: `pub async fn property_keys(&self) -> Result<Vec<String>, VaultError>` on `NoteVault`. Keys are in search form (lowercased, accent-folded), distinct, sorted.

- [ ] **Step 1: Write the failing test** (in `property_api_tests`)

```rust
#[tokio::test]
async fn property_keys_lists_distinct_search_form_keys_sorted() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&p("/a.md"), "+++\nStatus = \"x\"\ndue = 2024-01-01\n+++\nbody")
        .await
        .unwrap();
    vault
        .create_note(&p("/b.md"), "---\nstatus: y\nRésumé: z\nempty:\n---\nbody")
        .await
        .unwrap();
    assert_eq!(
        vault.property_keys().await.unwrap(),
        vec!["due", "empty", "resume", "status"]
    );
}
```

- [ ] **Step 2: Run, verify it fails**

Run: `cargo test -p kimun_core property_keys_lists`
Expected: compile error — no method `property_keys`.

- [ ] **Step 3: Implement**

In `core/src/index/mod.rs`, beside `dominant_property_kind`:

```rust
    /// Every distinct property key in the vault, in search form, sorted.
    pub(crate) async fn property_keys(&self) -> Result<Vec<String>, DBError> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT DISTINCT key FROM property_keys ORDER BY key")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(k,)| k).collect())
    }
```

In `core/src/lib.rs`, after `get_property`:

```rust
    /// Every property key any note has, in search form (lowercased, accents
    /// stripped — the form `%key` filters and `^%key` sorts by), distinct
    /// and sorted. Index-backed.
    pub async fn property_keys(&self) -> Result<Vec<String>, VaultError> {
        Ok(self.index.property_keys().await?)
    }
```

- [ ] **Step 4: Run, verify pass**

Run: `cargo test -p kimun_core property_keys_lists`
Expected: PASS. (If `create_note` does not index synchronously, the existing `remove_property_only_touches_its_key` test proves it does — it searches right after.)

- [ ] **Step 5: Report green.**

---

### Task 3: Core — `NoteVault::rename_property()`

**Files:**
- Modify: `core/src/note/properties/mod.rs` (trait `PropertyFormatter` `:173`, `NoteProperties` `:223`)
- Modify: `core/src/note/properties/toml_formatter.rs`
- Modify: `core/src/note/properties/yaml_formatter.rs`
- Modify: `core/src/lib.rs` (after `remove_property`, `:1369`); tests in `property_api_tests`

**Interfaces:**
- Produces: `pub async fn rename_property(&self, path: &VaultPath, old: &str, new: &str) -> Result<(), VaultError>`.
  - `old` matched case-insensitively; missing → `VaultError::InvalidProperty { key: old, message: "no such property" }`.
  - `new` already present as a *different* key (by `keys_match`) → `VaultError::InvalidProperty { key: new, message: "'<new>' already exists" }`.
  - Case-only change (`Status` → `status`) is allowed: respells in place.
  - Position, value, inline comment and comment lines above stay.
- Trait addition: `fn rename(&self, block: &str, old: &str, new: &str) -> Result<Option<String>, FrontmatterError>;` — `Ok(None)` when `old` is absent.

- [ ] **Step 1: Write the failing tests** (in `property_api_tests`)

```rust
#[tokio::test]
async fn rename_keeps_position_value_and_comments_toml() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(
            &p("/t.md"),
            "+++\na = 1\n# about b\nb = \"x\" # keep\nc = 3\n+++\nbody",
        )
        .await
        .unwrap();
    vault.rename_property(&p("/t.md"), "B", "beta").await.unwrap();
    assert_eq!(
        vault.get_note_text(&p("/t.md")).await.unwrap(),
        "+++\na = 1\n# about b\nbeta = \"x\" # keep\nc = 3\n+++\nbody"
    );
}

#[tokio::test]
async fn rename_keeps_position_value_and_comments_yaml() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(
            &p("/y.md"),
            "---\na: 1\ntags: # keep\n  - x\n  - y\nc: 3\nbare:\n---\nbody",
        )
        .await
        .unwrap();
    vault.rename_property(&p("/y.md"), "tags", "labels").await.unwrap();
    vault.rename_property(&p("/y.md"), "bare", "due date").await.unwrap();
    assert_eq!(
        vault.get_note_text(&p("/y.md")).await.unwrap(),
        "---\na: 1\nlabels: # keep\n  - x\n  - y\nc: 3\ndue date:\n---\nbody"
    );
}

#[tokio::test]
async fn rename_case_only_respells_in_place() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&p("/n.md"), "---\nStatus: x\nb: 2\n---\nbody")
        .await
        .unwrap();
    vault.rename_property(&p("/n.md"), "Status", "status").await.unwrap();
    assert_eq!(
        vault.get_note_text(&p("/n.md")).await.unwrap(),
        "---\nstatus: x\nb: 2\n---\nbody"
    );
}

#[tokio::test]
async fn rename_refuses_existing_target_and_missing_source() {
    let (_tmp, vault) = new_vault().await;
    let original = "+++\na = 1\nb = 2\n+++\nbody";
    vault.create_note(&p("/n.md"), original).await.unwrap();
    let taken = vault.rename_property(&p("/n.md"), "a", "B").await.unwrap_err();
    assert!(matches!(taken, VaultError::InvalidProperty { .. }), "{taken:?}");
    let missing = vault.rename_property(&p("/n.md"), "zzz", "q").await.unwrap_err();
    assert!(matches!(missing, VaultError::InvalidProperty { .. }), "{missing:?}");
    assert_eq!(vault.get_note_text(&p("/n.md")).await.unwrap(), original);
}

#[tokio::test]
async fn rename_updates_the_index() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&p("/n.md"), "+++\nstatus = \"done\"\n+++\nbody")
        .await
        .unwrap();
    vault.rename_property(&p("/n.md"), "status", "state").await.unwrap();
    assert!(vault.search_notes("prop:status=done").await.unwrap().is_empty());
    assert_eq!(hits(&vault, "prop:state=done").await, vec!["/n.md"]);
}
```

(If `hits` returns paths without the leading `/`, adjust the expected literal to whatever `remove_property`'s neighbouring tests use — check one existing `hits(` assertion in this module first.)

- [ ] **Step 2: Run, verify they fail**

Run: `cargo test -p kimun_core rename_`
Expected: compile error — no method `rename_property`.

- [ ] **Step 3: Trait + `NoteProperties`** (`properties/mod.rs`)

Add to the trait, after `remove`:

```rust
    /// `block` with the entry for `old` keyed `new` instead, in place: same
    /// position, value, inline comment and comment lines. `None` when `old`
    /// is absent. Callers have checked that `new` names no *other* entry.
    fn rename(&self, block: &str, old: &str, new: &str)
        -> Result<Option<String>, FrontmatterError>;
```

Add to `impl NoteProperties`, after `remove`:

```rust
    /// The note's text with `old` renamed to `new` (both normalized), or
    /// `None` when there is no such key. Refuses a `new` that names another
    /// existing entry.
    pub(crate) fn rename(&self, old: &str, new: &str) -> Result<Option<String>, FrontmatterError> {
        if self.span.is_none() {
            return Ok(None);
        }
        let block = crate::nfs::to_lf(self.block());
        let entries = self.formatter.parse(&block)?;
        if !keys_match(old, new) && entries.iter().any(|(k, _)| keys_match(k, new)) {
            return Err(FrontmatterError::Refused(format!("'{new}' already exists")));
        }
        self.formatter
            .rename(self.block(), old, new)?
            .map(|block| self.checked(&block))
            .transpose()
    }
```

- [ ] **Step 4: TOML rename** (`toml_formatter.rs`, inside `impl PropertyFormatter for TomlFormatter`)

toml_edit cannot rename a key in place, so rebuild the root table in order, swapping the one key. Values keep their `Item` (and with it the value's decor, i.e. the trailing `# keep`); the key keeps its leaf decor (the comment lines above).

```rust
    fn rename(
        &self,
        block: &str,
        old: &str,
        new: &str,
    ) -> Result<Option<String>, FrontmatterError> {
        let mut doc = document(block)?;
        let matches = matching_keys(&doc, old)?;
        let Some(target) = matches.first().cloned() else {
            return Ok(None);
        };
        let order: Vec<String> = doc.iter().map(|(k, _)| k.to_string()).collect();
        let table = doc.as_table_mut();
        let mut entries = Vec::with_capacity(order.len());
        for k in &order {
            if let Some(entry) = table.remove_entry(k) {
                entries.push(entry);
            }
        }
        for (key, item) in entries {
            let key = if key.get() == target {
                Key::new(new).with_leaf_decor(key.leaf_decor().clone())
            } else {
                key
            };
            table.insert_formatted(&key, item);
        }
        Ok(Some(doc.to_string()))
    }
```

If `Key::with_leaf_decor` does not exist in toml_edit 0.25, build it with `let mut k = Key::new(new); *k.leaf_decor_mut() = key.leaf_decor().clone();`. Run the TOML test; if `[table]` sections reorder, restrict the rebuild to `Item::Value` entries (tables stay put; only values are removed and re-inserted, and toml_edit prints values before tables).

- [ ] **Step 5: YAML rename** (`yaml_formatter.rs`)

Factor the key-quoting out of `render_entry` so rename writes keys the same way:

```rust
/// `key` as an entry line spells it: plain when the line reader finds it
/// again as written, double-quoted otherwise.
fn render_key(key: &str) -> String {
    let readable = line_key(&format!("{key}: x")).as_deref() == Some(key);
    if plain_ok(key) && !key.contains(':') && readable {
        key.to_string()
    } else {
        double_quote(key)
    }
}
```

and make `render_entry` start with `let k = render_key(key);` (delete its inline copy).

Add a helper returning the byte index of the `:` that ends the key on a top-level entry line:

```rust
/// Byte offset of the `:` ending the key of top-level entry line `line`.
fn key_colon(line: &str) -> Option<usize> {
    for quote in ['"', '\''] {
        if let Some(rest) = line.strip_prefix(quote) {
            let end = rest.find(quote)? + 1; // index in `rest` past the closing quote
            let after = &rest[end..];
            let pad = after.len() - after.trim_start().len();
            return after.trim_start().starts_with(':').then_some(1 + end + pad);
        }
    }
    line.find(": ")
        .or_else(|| line.strip_suffix(':').map(str::len))
}
```

Trait impl:

```rust
    fn rename(
        &self,
        block: &str,
        old: &str,
        new: &str,
    ) -> Result<Option<String>, FrontmatterError> {
        check_root(block, old)?;
        let mut lines: Vec<String> = block.lines().map(str::to_string).collect();
        let entries = entry_ranges(&lines, old);
        let Some((range, _)) = entries.first() else {
            verify(block, old, false)?;
            return Ok(None);
        };
        let line = &lines[range.start];
        let colon = key_colon(line).ok_or_else(|| {
            FrontmatterError::Refused(format!("cannot locate the key '{old}' on its line"))
        })?;
        lines[range.start] = format!("{}{}", render_key(new), &line[colon..]);
        let out = join_lines(&lines);
        verify(&out, new, true)?;
        if !keys_match(old, new) {
            verify(&out, old, false)?;
        }
        Ok(Some(out))
    }
```

`verify_others` is keyed on one property and would flag the renamed key as "changed"; the two `verify` calls plus the value check below are enough. Add a value check: parse `block` and `out` with `self.parse`, and require the entry for `new` in `out` to equal the entry for `old` in `block`:

```rust
        let before = self.parse(block)?;
        let after = self.parse(&out)?;
        let value_of = |es: &[PropertyEntry], k: &str| {
            es.iter().find(|(e, _)| keys_match(e, k)).map(|(_, v)| v.clone())
        };
        if value_of(&before, old) != value_of(&after, new) {
            return Err(FrontmatterError::Refused(format!(
                "renaming '{old}' would change its value; edit the frontmatter by hand"
            )));
        }
```

(place it right before `Ok(Some(out))`).

- [ ] **Step 6: `NoteVault::rename_property`** (`lib.rs`, after `remove_property`)

```rust
    /// Renames frontmatter property `old` to `new` in place — same position,
    /// value and comments. A case-only change respells the key. Refused with
    /// [`VaultError::InvalidProperty`] when `old` is missing or `new` names
    /// another existing property; same locking and malformed-block rules as
    /// [`Self::set_property`].
    pub async fn rename_property(
        &self,
        path: &VaultPath,
        old: &str,
        new: &str,
    ) -> Result<(), VaultError> {
        let old = Self::property_key(old)?;
        let new = Self::property_key(new)?;
        let _guard = self.lock_note(path).await;
        let text = nfs::to_lf(&self.get_note_text(path).await?);
        match properties::NoteProperties::new(&text, FrontmatterFormat::default())
            .rename(&old, &new)
            .map_err(|e| Self::frontmatter_error(path, &new, e))?
        {
            Some(updated) => {
                if updated != text {
                    self.save_note_unlocked(path, updated).await?;
                }
                Ok(())
            }
            None => Err(VaultError::InvalidProperty {
                key: old,
                message: "no such property".to_string(),
            }),
        }
    }
```

- [ ] **Step 7: Run, verify pass**

Run: `cargo test -p kimun_core rename_ && cargo test -p kimun_core properties`
Expected: new tests PASS, all existing property tests PASS.

- [ ] **Step 8: Report green.**

---

### Task 4: TUI — `SortField::Property` in the query panel

**Files:**
- Modify: `tui/src/components/file_list.rs:15-79`
- Modify: `tui/src/components/query_panel.rs:409-452` and the title at `:726-744`
- Fix compile fallout: `components/sidebar.rs`, `components/dialogs/sort_dialog.rs`, `components/dialogs/mod.rs`, `components/events.rs`, `app_screen/editor.rs`, `keys/action_shortcuts.rs`
- Test: `query_panel.rs` tests module (near `:1381`)

**Interfaces:**
- Produces: `#[derive(Clone, PartialEq, Debug)] pub enum SortField { Name, Title, Property(String) }` (no `Copy`). `SortField::label(&self) -> String` (`"N"`, `"T"`, or the key). `SortField::cycle(&self, allow_property: bool) -> SortField` — `Name → Title → Property(String::new()) → Name` when `allow_property`, else `Name ↔ Title`. `From<SortField> for SortFieldSetting` maps `Property(_)` to `Name` (unreachable for the sidebar). `QueryPanel::current_order(&self) -> (SortField, SortOrder)` returns `Property(key)` for property orders.

- [ ] **Step 1: Write the failing tests** (in `query_panel.rs` tests; mirror the setup of the existing test at `:1381` that asserts `(SortField::Title, SortOrder::Descending)`)

```rust
#[tokio::test(flavor = "multi_thread")]
async fn current_order_reports_a_property_sort() {
    let vault = crate::test_support::temp_vault("qp-prop-order").await;
    vault.validate_and_init().await.unwrap();
    let mut panel = make_panel(vault);
    panel.set_active_query("#work -^%due".to_string());
    assert_eq!(
        panel.current_order(),
        (SortField::Property("due".into()), SortOrder::Descending)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn property_sort_survives_order_toggle() {
    let vault = crate::test_support::temp_vault("qp-prop-toggle").await;
    vault.validate_and_init().await.unwrap();
    let mut panel = make_panel(vault);
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    panel.set_active_query("x ^%\"due date\"".to_string());
    let (field, order) = panel.current_order();
    panel.apply_sort(field, order.toggle(), &tx);
    assert_eq!(panel.active_query(), "x -or:prop:\"due date\"");
    panel.apply_sort(SortField::Name, SortOrder::Ascending, &tx);
    assert_eq!(panel.active_query(), "x or:file");
}
```

(`make_panel`, `set_active_query` and `active_query` are the helpers the existing `current_order_reads_query_directive` test uses.)

- [ ] **Step 2: Run, verify they fail**

Run: `cargo test -p kimun-notes --lib current_order_reports_a_property_sort`
Expected: compile error — no variant `Property`.

- [ ] **Step 3: Implement `SortField`** (`file_list.rs`)

```rust
#[derive(Clone, PartialEq, Debug)]
pub enum SortField {
    Name,
    Title,
    /// Query panel only: sort by this property key (search form).
    Property(String),
}
```

```rust
impl From<SortField> for SortFieldSetting {
    fn from(s: SortField) -> Self {
        match s {
            SortField::Name | SortField::Property(_) => Self::Name,
            SortField::Title => Self::Title,
        }
    }
}

impl SortField {
    pub fn label(&self) -> String {
        match self {
            Self::Name => "N".to_string(),
            Self::Title => "T".to_string(),
            Self::Property(key) => key.clone(),
        }
    }

    /// Next field in the dialog's cycle. `allow_property` is false for the
    /// sidebar, which sorts directory listings with no index rows.
    pub fn cycle(&self, allow_property: bool) -> Self {
        match self {
            Self::Name => Self::Title,
            Self::Title if allow_property => Self::Property(String::new()),
            Self::Title | Self::Property(_) => Self::Name,
        }
    }
}
```

Fix every compile error from the lost `Copy` by cloning or borrowing (`field.clone()`, `&self.field`); `label()` callers now get a `String`. The sidebar calls `cycle(false)`.

- [ ] **Step 4: Implement the query-panel mapping** (`query_panel.rs`)

In `current_order`, replace the last arm:

```rust
            Some(OrderBy::Property { key, asc }) => (
                SortField::Property(key.clone()),
                if *asc {
                    SortOrder::Ascending
                } else {
                    SortOrder::Descending
                },
            ),
            None => (SortField::Name, SortOrder::Ascending),
```

In `apply_sort`:

```rust
        let order_field = match field {
            SortField::Name => OrderField::FileName,
            SortField::Title => OrderField::Title,
            SortField::Property(key) => OrderField::Property(key),
        };
```

Title (`:731`): `order_cache` is no longer `Copy` — use `let (sort_field, sort_order) = &self.order_cache;` and:

```rust
        let sort_indicator = match sort_field {
            SortField::Property(key) => format!("sorted by {key} {}", sort_order.label()),
            other => format!("{}{}", other.label(), sort_order.label()),
        };
```

- [ ] **Step 5: Run, verify pass**

Run: `cargo test -p kimun-notes --lib query_panel && cargo test -p kimun-notes --lib sort && cargo test -p kimun-notes --lib sidebar`
Expected: PASS.

- [ ] **Step 6: Report green.**

---

### Task 5: TUI — `ButtonRow` widget

**Files:**
- Create: `tui/src/components/button_row.rs`
- Modify: `tui/src/components/mod.rs` (add `pub mod button_row;`)

**Interfaces:**
- Produces:
  ```rust
  pub struct ButtonRow { /* labels, enabled, focused, rects */ }
  impl ButtonRow {
      pub fn new(labels: &[&str]) -> Self;
      pub fn set_enabled(&mut self, idx: usize, enabled: bool);
      pub fn is_enabled(&self, idx: usize) -> bool;
      pub fn set_focused(&mut self, idx: Option<usize>);
      pub fn focused(&self) -> Option<usize>;
      /// Moves focus to the next/previous enabled button; returns false at the end.
      pub fn focus_next(&mut self) -> bool;
      pub fn focus_prev(&mut self) -> bool;
      pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme);
      /// Index of the enabled button under (col,row) from the last render.
      pub fn hit(&self, col: u16, row: u16) -> Option<usize>;
  }
  ```

- [ ] **Step 1: Write the failing tests** (bottom of the new file)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn drawn(row: &mut ButtonRow) {
        let theme = Theme::gruvbox_dark();
        let mut t = Terminal::new(TestBackend::new(40, 1)).unwrap();
        t.draw(|f| row.render(f, f.area(), &theme)).unwrap();
    }

    #[test]
    fn hit_finds_each_button_after_render() {
        let mut row = ButtonRow::new(&["Save", "Cancel"]);
        drawn(&mut row);
        // Layout: " [ Save ]  [ Cancel ]" → Save at cols 1..=8, Cancel at 11..=20.
        assert_eq!(row.hit(1, 0), Some(0));
        assert_eq!(row.hit(8, 0), Some(0));
        assert_eq!(row.hit(11, 0), Some(1));
        assert_eq!(row.hit(20, 0), Some(1));
    }

    #[test]
    fn gap_between_buttons_hits_nothing() {
        let mut row = ButtonRow::new(&["Save", "Cancel"]);
        drawn(&mut row);
        assert_eq!(row.hit(9, 0), None);
        assert_eq!(row.hit(0, 0), None);
        assert_eq!(row.hit(30, 0), None);
    }

    #[test]
    fn disabled_buttons_are_never_hit_or_focused() {
        let mut row = ButtonRow::new(&["Add", "Edit", "Delete"]);
        row.set_enabled(1, false);
        drawn(&mut row);
        assert_eq!(row.hit(8, 0), None, "Edit is disabled");
        row.set_focused(Some(0));
        assert!(row.focus_next());
        assert_eq!(row.focused(), Some(2), "focus skips the disabled button");
        assert!(!row.focus_next());
    }

    #[test]
    fn no_hit_before_first_render() {
        let row = ButtonRow::new(&["Save"]);
        assert_eq!(row.hit(1, 0), None);
    }
}
```

- [ ] **Step 2: Run, verify they fail**

Run: `cargo test -p kimun-notes --lib button_row`
Expected: compile error (module missing).

- [ ] **Step 3: Implement**

```rust
//! A row of clickable `[ Label ]` buttons. Rects are recorded at render time
//! and hit-tested against later mouse events (same pattern as
//! `search_list`): no render, no hits.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::settings::themes::Theme;

pub struct ButtonRow {
    labels: Vec<String>,
    enabled: Vec<bool>,
    focused: Option<usize>,
    rects: Vec<Rect>,
}

impl ButtonRow {
    pub fn new(labels: &[&str]) -> Self {
        Self {
            labels: labels.iter().map(|l| l.to_string()).collect(),
            enabled: vec![true; labels.len()],
            focused: None,
            rects: Vec::new(),
        }
    }

    pub fn set_enabled(&mut self, idx: usize, enabled: bool) {
        if let Some(e) = self.enabled.get_mut(idx) {
            *e = enabled;
        }
        if !enabled && self.focused == Some(idx) {
            self.focused = None;
        }
    }

    pub fn is_enabled(&self, idx: usize) -> bool {
        self.enabled.get(idx).copied().unwrap_or(false)
    }

    pub fn set_focused(&mut self, idx: Option<usize>) {
        self.focused = idx.filter(|&i| self.is_enabled(i));
    }

    pub fn focused(&self) -> Option<usize> {
        self.focused
    }

    pub fn focus_next(&mut self) -> bool {
        let start = self.focused.map_or(0, |i| i + 1);
        match (start..self.labels.len()).find(|&i| self.is_enabled(i)) {
            Some(i) => {
                self.focused = Some(i);
                true
            }
            None => false,
        }
    }

    pub fn focus_prev(&mut self) -> bool {
        let end = self.focused.unwrap_or(self.labels.len());
        match (0..end).rev().find(|&i| self.is_enabled(i)) {
            Some(i) => {
                self.focused = Some(i);
                true
            }
            None => false,
        }
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme) {
        self.rects.clear();
        let normal = Style::default().fg(theme.fg.to_ratatui()).bg(theme.bg_panel.to_ratatui());
        let dim = Style::default()
            .fg(theme.gray.to_ratatui())
            .bg(theme.bg_panel.to_ratatui())
            .add_modifier(Modifier::DIM);
        let focus = Style::default()
            .fg(theme.selection_fg.to_ratatui())
            .bg(theme.selection_bg.to_ratatui())
            .add_modifier(Modifier::BOLD);
        let mut spans = vec![Span::styled(" ", normal)];
        let mut x = rect.x + 1;
        for (i, label) in self.labels.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("  ", normal));
                x += 2;
            }
            let text = format!("[ {label} ]");
            let w = text.width() as u16;
            let style = if !self.enabled[i] {
                dim
            } else if self.focused == Some(i) {
                focus
            } else {
                normal
            };
            // Clip to the row: a button past the right edge is not clickable.
            let visible = rect.right().saturating_sub(x).min(w);
            self.rects.push(Rect { x, y: rect.y, width: visible, height: 1 });
            spans.push(Span::styled(text, style));
            x += w;
        }
        f.render_widget(Paragraph::new(Line::from(spans)).style(normal), rect);
    }

    pub fn hit(&self, col: u16, row: u16) -> Option<usize> {
        let pos = Position { x: col, y: row };
        self.rects
            .iter()
            .position(|r| r.contains(pos))
            .filter(|&i| self.is_enabled(i))
    }
}
```

- [ ] **Step 4: Run, verify pass**

Run: `cargo test -p kimun-notes --lib button_row`
Expected: 4 PASS.

- [ ] **Step 5: Report green.**

---

### Task 6: TUI — `KeyPicker` widget

**Files:**
- Create: `tui/src/components/key_picker.rs`
- Modify: `tui/src/components/mod.rs` (add `pub mod key_picker;`)

**Interfaces:**
- Consumes: `SingleLineInput` (`components/single_line_input.rs`): `new`, `with_value`, `value`, `set_value`, `handle_key -> InputOutcome`, `render(f, rect, style, value_offset_x, focused)`.
- Produces:
  ```rust
  pub enum PickerOutcome { Consumed, Changed, Accepted(String), Submit, Cancel, NotConsumed }
  pub struct KeyPicker { .. }
  impl KeyPicker {
      pub fn new(initial: &str) -> Self;
      pub fn set_keys(&mut self, keys: Vec<String>);
      pub fn value(&self) -> &str;
      pub fn is_list_open(&self) -> bool;
      pub fn handle_key(&mut self, key: &KeyEvent) -> PickerOutcome;
      /// Field row is `field`; the suggestion list renders below it inside `area`, at most 5 rows.
      pub fn render(&mut self, f: &mut Frame, field: Rect, area: Rect, theme: &Theme, focused: bool);
      /// Click at (col,row): on the field → focus (returns `Consumed`); on a suggestion → `Accepted(key)`; elsewhere → `NotConsumed`.
      pub fn handle_click(&mut self, col: u16, row: u16) -> PickerOutcome;
      pub fn field_rect(&self) -> Option<Rect>;
  }
  ```
  Semantics: typing filters `keys` (case-insensitive substring; empty value → list closed); the list opens on `Changed` when at least one suggestion matches and the value is not already an exact match. `↑↓` move in an open list. `Enter`/`→` with the list open → `Accepted(key)` (value set to it, list closed). `Enter` with the list closed → `Submit`. `Esc` with the list open → closes it, returns `Consumed`; with it closed → `Cancel`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent};
    use ratatui::{Terminal, backend::TestBackend};

    fn picker() -> KeyPicker {
        let mut p = KeyPicker::new("");
        p.set_keys(vec!["due".into(), "status".into(), "priority".into()]);
        p
    }
    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }
    fn type_str(p: &mut KeyPicker, s: &str) {
        for c in s.chars() {
            p.handle_key(&k(KeyCode::Char(c)));
        }
    }

    #[test]
    fn typing_filters_case_insensitively() {
        let mut p = picker();
        type_str(&mut p, "PRI");
        assert!(p.is_list_open());
        assert_eq!(p.suggestions(), vec!["priority"]);
    }

    #[test]
    fn enter_accepts_highlighted_then_submits() {
        let mut p = picker();
        type_str(&mut p, "t");
        assert_eq!(p.suggestions(), vec!["status", "priority"]);
        p.handle_key(&k(KeyCode::Down));
        assert_eq!(p.handle_key(&k(KeyCode::Enter)), PickerOutcome::Accepted("priority".into()));
        assert_eq!(p.value(), "priority");
        assert!(!p.is_list_open());
        assert_eq!(p.handle_key(&k(KeyCode::Enter)), PickerOutcome::Submit);
    }

    #[test]
    fn esc_closes_list_before_cancelling() {
        let mut p = picker();
        type_str(&mut p, "d");
        assert_eq!(p.handle_key(&k(KeyCode::Esc)), PickerOutcome::Consumed);
        assert!(!p.is_list_open());
        assert_eq!(p.handle_key(&k(KeyCode::Esc)), PickerOutcome::Cancel);
    }

    #[test]
    fn free_text_is_kept() {
        let mut p = picker();
        type_str(&mut p, "brand new");
        assert!(!p.is_list_open());
        assert_eq!(p.handle_key(&k(KeyCode::Enter)), PickerOutcome::Submit);
        assert_eq!(p.value(), "brand new");
    }

    #[test]
    fn click_on_suggestion_accepts_it() {
        let mut p = picker();
        type_str(&mut p, "t");
        let theme = Theme::gruvbox_dark();
        let mut t = Terminal::new(TestBackend::new(30, 8)).unwrap();
        t.draw(|f| {
            let area = f.area();
            let field = Rect { height: 1, ..area };
            p.render(f, field, area, &theme, true);
        })
        .unwrap();
        // Suggestions start on the row below the field: row 1 = "status", row 2 = "priority".
        assert_eq!(p.handle_click(3, 2), PickerOutcome::Accepted("priority".into()));
        assert_eq!(p.value(), "priority");
    }
}
```

`suggestions(&self) -> Vec<&str>` is a `pub(crate)` accessor (also used by render).

- [ ] **Step 2: Run, verify they fail**

Run: `cargo test -p kimun-notes --lib key_picker`
Expected: compile error.

- [ ] **Step 3: Implement**

```rust
//! A text field with a filtered list of known property keys under it. Free
//! text is always allowed; a suggestion is a shortcut, never a constraint.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Paragraph};

use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::settings::themes::Theme;

const MAX_ROWS: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOutcome {
    Consumed,
    Changed,
    Accepted(String),
    Submit,
    Cancel,
    NotConsumed,
}

pub struct KeyPicker {
    input: SingleLineInput,
    keys: Vec<String>,
    open: bool,
    highlighted: usize,
    field_rect: Option<Rect>,
    /// (rect, suggestion) of each visible row from the last render.
    row_rects: Vec<(Rect, String)>,
}

impl KeyPicker {
    pub fn new(initial: &str) -> Self {
        Self {
            input: SingleLineInput::with_value(initial),
            keys: Vec::new(),
            open: false,
            highlighted: 0,
            field_rect: None,
            row_rects: Vec::new(),
        }
    }

    pub fn set_keys(&mut self, keys: Vec<String>) {
        self.keys = keys;
    }

    pub fn value(&self) -> &str {
        self.input.value()
    }

    pub fn is_list_open(&self) -> bool {
        self.open
    }

    pub fn field_rect(&self) -> Option<Rect> {
        self.field_rect
    }

    pub(crate) fn suggestions(&self) -> Vec<&str> {
        let needle = self.input.value().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        self.keys
            .iter()
            .filter(|k| k.to_lowercase().contains(&needle))
            .map(String::as_str)
            .collect()
    }

    fn refresh_open(&mut self) {
        let value = self.input.value().to_lowercase();
        let s = self.suggestions();
        self.open = !s.is_empty() && !(s.len() == 1 && s[0].to_lowercase() == value);
        self.highlighted = 0;
    }

    fn accept(&mut self, key: String) -> PickerOutcome {
        self.input.set_value(key.clone());
        self.open = false;
        PickerOutcome::Accepted(key)
    }

    pub fn handle_key(&mut self, key: &KeyEvent) -> PickerOutcome {
        if self.open {
            let count = self.suggestions().len();
            match key.code {
                KeyCode::Up => {
                    self.highlighted = self.highlighted.saturating_sub(1);
                    return PickerOutcome::Consumed;
                }
                KeyCode::Down => {
                    self.highlighted = (self.highlighted + 1).min(count.saturating_sub(1));
                    return PickerOutcome::Consumed;
                }
                KeyCode::Enter | KeyCode::Right => {
                    if let Some(k) = self.suggestions().get(self.highlighted) {
                        let k = k.to_string();
                        return self.accept(k);
                    }
                }
                KeyCode::Esc => {
                    self.open = false;
                    return PickerOutcome::Consumed;
                }
                _ => {}
            }
        }
        match self.input.handle_key(key) {
            InputOutcome::Changed => {
                self.refresh_open();
                PickerOutcome::Changed
            }
            InputOutcome::Submit => PickerOutcome::Submit,
            InputOutcome::Cancel => PickerOutcome::Cancel,
            InputOutcome::Consumed => PickerOutcome::Consumed,
            InputOutcome::NotConsumed => PickerOutcome::NotConsumed,
        }
    }

    pub fn render(&mut self, f: &mut Frame, field: Rect, area: Rect, theme: &Theme, focused: bool) {
        self.field_rect = Some(field);
        let style = Style::default()
            .fg(theme.fg_bright.to_ratatui())
            .bg(theme.bg.to_ratatui());
        self.input.render(f, field, style, 0, focused);
        self.row_rects.clear();
        if !self.open || !focused {
            return;
        }
        let rows: Vec<String> = self
            .suggestions()
            .into_iter()
            .take(MAX_ROWS)
            .map(str::to_string)
            .collect();
        for (i, key) in rows.into_iter().enumerate() {
            let y = field.y + 1 + i as u16;
            if y >= area.bottom() {
                break;
            }
            let rect = Rect { x: field.x, y, width: field.width, height: 1 };
            let st = if i == self.highlighted {
                Style::default()
                    .fg(theme.selection_fg.to_ratatui())
                    .bg(theme.selection_bg.to_ratatui())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg.to_ratatui()).bg(theme.bg_panel.to_ratatui())
            };
            f.render_widget(Clear, rect);
            f.render_widget(Paragraph::new(format!(" {key}")).style(st), rect);
            self.row_rects.push((rect, key));
        }
    }

    pub fn handle_click(&mut self, col: u16, row: u16) -> PickerOutcome {
        let pos = Position { x: col, y: row };
        if let Some((_, key)) = self.row_rects.iter().find(|(r, _)| r.contains(pos)) {
            let key = key.clone();
            return self.accept(key);
        }
        if self.field_rect.is_some_and(|r| r.contains(pos)) {
            return PickerOutcome::Consumed;
        }
        PickerOutcome::NotConsumed
    }
}
```

The suggestion list draws over whatever is below the field (hence `Clear`): hosts must render the picker **last** so the list sits on top.

- [ ] **Step 4: Run, verify pass**

Run: `cargo test -p kimun-notes --lib key_picker`
Expected: 5 PASS.

- [ ] **Step 5: Report green.**

---

### Task 7: TUI — mouse into dialogs + property row in the sort dialog

**Files:**
- Modify: `tui/src/components/events.rs` (`OverlayData`, `:262`)
- Modify: `tui/src/components/dialogs/mod.rs` (`Component for ActiveDialog::handle_input`, `handle_data`, `ActiveDialog::sort`)
- Modify: `tui/src/components/dialogs/sort_dialog.rs`
- Modify: `tui/src/app_screen/editor.rs:1121-1124` (`OverlayOpen::SortQuery`)
- Modify: `tui/src/components/dialogs/help_dialog.rs` only if the Sort dialog rows are listed there (grep `Sort by`)

**Interfaces:**
- Consumes: `SortField::cycle(&self, allow_property)` (Task 4), `KeyPicker`, `PickerOutcome` (Task 6), `NoteVault::property_keys` (Task 2).
- Produces:
  - `OverlayData::PropertyKeysLoaded(Vec<String>)`.
  - `ActiveDialog::sort(target, field, order, group_directories, vault: Option<Arc<NoteVault>>, tx: &AppTx)` — when `vault` is `Some` (query target) it spawns `property_keys()` and sends `PropertyKeysLoaded`.
  - `SortDialog::handle_mouse(&mut self, ev: &MouseEvent, tx: &AppTx) -> EventState`, `SortDialog::set_keys(Vec<String>)`.
  - `ActiveDialog::handle_input` routes `InputEvent::Mouse` to `Sort(d) => d.handle_mouse(..)` (and later `Properties`); every other variant returns `EventState::Consumed` for mouse (swallow — the overlay is modal).

- [ ] **Step 1: Write the failing tests** (in `sort_dialog.rs` tests)

```rust
    use ratatui::{Terminal, backend::TestBackend};
    use crate::settings::themes::Theme;

    fn query_dialog(field: SortField) -> SortDialog {
        SortDialog::new(SortTarget::Query, field, SortOrder::Ascending, false)
    }

    fn draw(d: &mut SortDialog) {
        use crate::components::Component;
        let theme = Theme::gruvbox_dark();
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| d.render(f, f.area(), &theme, true)).unwrap();
    }

    fn mouse(col: u16, row: u16) -> ratatui::crossterm::event::MouseEvent {
        match crate::test_support::mouse_down_at(col, row) {
            InputEvent::Mouse(m) => m,
            _ => unreachable!(),
        }
    }

    #[test]
    fn query_cycle_reaches_property_and_waits_for_a_key() {
        let mut d = query_dialog(SortField::Title);
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char(' ')), &tx);
        assert_eq!(d.field, SortField::Property(String::new()));
        assert!(rx.try_recv().is_err(), "no emit before a key is chosen");
        assert_eq!(d.row_count(), 3, "Key row appears");
    }

    #[test]
    fn accepting_a_key_emits_property_sort() {
        let mut d = query_dialog(SortField::Property(String::new()));
        d.set_keys(vec!["due".into(), "status".into()]);
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Down), &tx); // Order
        d.handle_key(key(KeyCode::Down), &tx); // Key
        for c in "du".chars() {
            d.handle_key(key(KeyCode::Char(c)), &tx);
        }
        d.handle_key(key(KeyCode::Enter), &tx); // accept "due" from the list
        match rx.try_recv() {
            Ok(AppEvent::SortChanged { field, .. }) => {
                assert_eq!(field, SortField::Property("due".into()))
            }
            other => panic!("expected SortChanged, got {other:?}"),
        }
    }

    #[test]
    fn opening_with_a_property_sort_preselects_it() {
        let d = query_dialog(SortField::Property("due".into()));
        assert_eq!(d.row_count(), 3);
        assert_eq!(d.key_value(), "due");
    }

    #[test]
    fn sidebar_never_cycles_to_property() {
        let mut d = sidebar_dialog();
        let (tx, _rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char(' ')), &tx); // Name → Title
        d.handle_key(key(KeyCode::Char(' ')), &tx); // Title → Name
        assert_eq!(d.field, SortField::Name);
    }

    #[test]
    fn click_on_a_row_selects_and_toggles_it() {
        let mut d = sidebar_dialog();
        draw(&mut d);
        let (tx, mut rx) = unbounded_channel();
        let (x, y) = d.row_origin(1).expect("Order row rendered");
        d.handle_mouse(&mouse(x + 2, y), &tx);
        assert_eq!(d.order, SortOrder::Descending);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::SortChanged { .. })));
    }

    #[test]
    fn click_outside_sort_modal_does_nothing() {
        let mut d = sidebar_dialog();
        draw(&mut d);
        let (tx, mut rx) = unbounded_channel();
        d.handle_mouse(&mouse(0, 0), &tx);
        assert!(rx.try_recv().is_err());
        assert_eq!(d.field, SortField::Name);
    }

    #[test]
    fn enter_on_key_row_with_list_closed_closes_overlay() {
        let mut d = query_dialog(SortField::Property("due".into()));
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Down), &tx);
        d.handle_key(key(KeyCode::Down), &tx);
        d.handle_key(key(KeyCode::Enter), &tx);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::CloseOverlay)));
    }
```

Also update the existing `space_toggles_field_and_emits_change` etc. for the non-`Copy` field only where the compiler demands.

- [ ] **Step 2: Run, verify they fail**

Run: `cargo test -p kimun-notes --lib sort_dialog`
Expected: compile errors (`set_keys`, `handle_mouse`, `row_origin`, `key_value` missing).

- [ ] **Step 3: Implement the sort dialog changes**

Add `Row::Key` and fields:

```rust
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Field,
    Order,
    Key,
    GroupDirs,
}

pub struct SortDialog {
    target: SortTarget,
    pub(crate) field: SortField,
    pub(crate) order: SortOrder,
    group_dirs: bool,
    rows: Vec<Row>,
    selected: usize,
    picker: KeyPicker,
    /// Screen rect of each row from the last render, for clicks.
    row_rects: Vec<Rect>,
    popup: Option<Rect>,
}
```

Rows are rebuilt whenever `field` changes:

```rust
    fn rebuild_rows(&mut self) {
        let mut rows = vec![Row::Field, Row::Order];
        if matches!(self.field, SortField::Property(_)) {
            rows.push(Row::Key);
        }
        if self.target == SortTarget::Sidebar {
            rows.push(Row::GroupDirs);
        }
        self.rows = rows;
        self.selected = self.selected.min(self.rows.len() - 1);
    }
```

`new` builds `picker: KeyPicker::new(key)` where `key` is the property key when `field` is `Property(key)`, else `""`, then calls `rebuild_rows()`.

`emit` must not fire for an empty property key:

```rust
    fn emit(&self, tx: &AppTx, persist: bool) {
        if matches!(&self.field, SortField::Property(k) if k.is_empty()) {
            return;
        }
        tx.send(AppEvent::SortChanged {
            target: self.target,
            field: self.field.clone(),
            order: self.order,
            group_directories: self.group_dirs,
            persist,
        })
        .ok();
    }
```

`toggle_selected` (Field row) uses `self.field = self.field.cycle(self.target == SortTarget::Query);` then `rebuild_rows()`; when the new field is `Property(_)` and the picker holds a value, set `self.field = SortField::Property(self.picker.value().to_string())`. `Row::Key` in `toggle_selected` does nothing.

`handle_key`: when the selected row is `Row::Key`, route everything except `Up`/`Down`-with-list-closed and `Esc`-with-list-closed to the picker:

```rust
        if self.rows[self.selected] == Row::Key {
            let list_open = self.picker.is_list_open();
            let passthrough = !list_open && matches!(key.code, KeyCode::Up | KeyCode::Down | KeyCode::Esc);
            if !passthrough {
                match self.picker.handle_key(&key) {
                    PickerOutcome::Accepted(k) => {
                        self.field = SortField::Property(k);
                        self.emit(tx, false);
                    }
                    PickerOutcome::Submit => {
                        let typed = self.picker.value().trim().to_string();
                        if !typed.is_empty() && self.field != SortField::Property(typed.clone()) {
                            self.field = SortField::Property(typed);
                            self.emit(tx, false);
                        } else {
                            tx.send(AppEvent::CloseOverlay).ok();
                        }
                    }
                    PickerOutcome::Cancel => {
                        tx.send(AppEvent::CloseOverlay).ok();
                    }
                    _ => {}
                }
                return EventState::Consumed;
            }
        }
```

(Enter on a typed free-text key applies it first; a second Enter closes. This keeps "Enter closes" for an already-applied key, which the test `enter_on_key_row_with_list_closed_closes_overlay` checks.)

`render`: record `self.popup = Some(popup)`; `self.row_rects` gets each row's `Rect`. The Key row renders its label, then `self.picker.render(f, value_rect, body, theme, selected)` **after** the loop and footer so the list draws on top; `value_rect` is the row rect offset by `3 + 20` columns (the `" {marker} {label:<20}"` prefix). Grow `outer_height` by `5` while the Key row is selected and the list is open so suggestions fit inside the modal.

`handle_mouse`:

```rust
    pub fn handle_mouse(&mut self, ev: &MouseEvent, tx: &AppTx) -> EventState {
        if !matches!(ev.kind, MouseEventKind::Down(MouseButton::Left)) {
            return EventState::Consumed;
        }
        match self.picker.handle_click(ev.column, ev.row) {
            PickerOutcome::Accepted(k) => {
                self.field = SortField::Property(k);
                self.emit(tx, false);
                return EventState::Consumed;
            }
            PickerOutcome::Consumed => {
                if let Some(i) = self.rows.iter().position(|r| *r == Row::Key) {
                    self.selected = i;
                }
                return EventState::Consumed;
            }
            _ => {}
        }
        let pos = Position { x: ev.column, y: ev.row };
        if let Some(i) = self.row_rects.iter().position(|r| r.contains(pos)) {
            self.selected = i;
            if self.rows[i] != Row::Key {
                self.toggle_selected(tx);
            }
        }
        EventState::Consumed
    }
```

Test helpers: `pub(crate) fn set_keys(&mut self, keys: Vec<String>) { self.picker.set_keys(keys) }`, `#[cfg(test)] pub(crate) fn key_value(&self) -> &str { self.picker.value() }`, `#[cfg(test)] pub(crate) fn row_origin(&self, i: usize) -> Option<(u16, u16)> { self.row_rects.get(i).map(|r| (r.x, r.y)) }`.

Footer for the query target: `"  [↑↓] Move  [Space] Toggle  [Enter/Esc] Close  · click a row"`; when the Key row is selected: `"  type a key · [Enter] apply · [Esc] close"`.

`Component::handle_input` for `SortDialog` also accepts `InputEvent::Mouse(m) => self.handle_mouse(m, tx)`.

- [ ] **Step 4: Mouse routing + keys load** (`dialogs/mod.rs`, `events.rs`, `editor.rs`)

`events.rs` — add to `OverlayData`:

```rust
    /// Every property key in the vault (search form), for key pickers.
    PropertyKeysLoaded(Vec<String>),
```

`dialogs/mod.rs`, `impl Component for ActiveDialog::handle_input` — replace the `let InputEvent::Key(key) = event else {..}` guard with:

```rust
        let key = match event {
            InputEvent::Key(key) => key,
            InputEvent::Mouse(m) => {
                return match self {
                    ActiveDialog::Sort(d) => d.handle_mouse(m, tx),
                    // Modal: a click on a dialog without mouse support is
                    // swallowed rather than reaching the panels behind it.
                    _ => EventState::Consumed,
                };
            }
            InputEvent::Paste(_) => return EventState::NotConsumed,
        };
```

and use `*key` below as before.

`handle_data` — new arm:

```rust
            OverlayData::PropertyKeysLoaded(keys) => {
                if let ActiveDialog::Sort(d) = self {
                    d.set_keys(keys.clone());
                }
                OverlayMsg::Consumed
            }
```

`ActiveDialog::sort` gains `vault: Option<Arc<NoteVault>>, tx: &AppTx`:

```rust
    pub fn sort(
        target: SortTarget,
        field: SortField,
        order: SortOrder,
        group_directories: bool,
        vault: Option<Arc<NoteVault>>,
        tx: &AppTx,
    ) -> Self {
        if let Some(vault) = vault {
            spawn_property_keys(vault, tx);
        }
        ActiveDialog::Sort(SortDialog::new(target, field, order, group_directories))
    }
```

with a shared helper (Task 9 reuses it):

```rust
/// Load every property key in the background; arrives as
/// [`OverlayData::PropertyKeysLoaded`]. A failed read leaves pickers empty.
pub(crate) fn spawn_property_keys(vault: Arc<NoteVault>, tx: &AppTx) {
    let tx = tx.clone();
    tokio::spawn(async move {
        if let Ok(keys) = vault.property_keys().await {
            tx.send(AppEvent::OverlayData(OverlayData::PropertyKeysLoaded(keys)))
                .ok();
        }
    });
}
```

`editor.rs` `build_overlay`: `SortQuery` passes `Some(self.vault.clone()), tx`; `SortSidebar` passes `None, tx`. Update the `active_dialog_sort_variant_compiles` test in `dialogs/mod.rs` (needs a `tx`: `let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();`, pass `None, &tx`).

Check the help dialog: `grep -n "Sort by\|Group directories" tui/src/components/dialogs/help_dialog.rs`; if the sort rows are listed, add `Key — property to sort by (query panel)`.

- [ ] **Step 5: Run, verify pass**

Run: `cargo test -p kimun-notes --lib sort_dialog && cargo test -p kimun-notes --lib dialogs && cargo test -p kimun-notes --lib editor`
Expected: PASS.

- [ ] **Step 6: Report green.**

---

### Task 8: TUI — open flow and reload from disk

**Files:**
- Modify: `tui/src/components/events.rs` (`FileOp`, `AppEvent`)
- Modify: `tui/src/app_screen/editor.rs` (`handle_file_op` `:1189`, `handle_app_message` `:2356`)
- Test: `editor.rs` tests module (follow the temp-vault editor tests already there — grep `async fn .*temp_vault` in `editor.rs` for the harness)

**Interfaces:**
- Consumes: `try_save` (`editor.rs:501`), `ActiveDialog::properties` (Task 9 — for this task, stub it: see Step 3).
- Produces:
  - `FileOp::ShowProperties(VaultPath)` — request to open the dialog for a note.
  - `AppEvent::NoteReloadFromDisk(VaultPath)` — a core write changed this note; reload the buffer if it is the open note.

- [ ] **Step 1: Write the failing tests** (editor.rs tests; reuse the existing screen-with-temp-vault helper)

```rust
/// `test_screen` with note `n.md` (text "hi") open in the editor.
async fn screen_on_note() -> (EditorScreen, Arc<NoteVault>, AppTx, tokio::sync::mpsc::UnboundedReceiver<AppEvent>, tempfile::TempDir) {
    let (mut screen, vault, _settings, dir) = test_screen().await;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    vault.create_note(&VaultPath::new("n.md"), "hi").await.unwrap();
    screen.open_path(VaultPath::new("n.md"), None, &tx).await;
    (screen, vault, tx, rx, dir)
}

#[tokio::test]
async fn note_reload_from_disk_replaces_buffer_and_marks_clean() {
    let (mut screen, vault, tx, _rx, _dir) = screen_on_note().await;
    vault
        .save_note(&VaultPath::new("n.md"), "+++\na = 1\n+++\nbody")
        .await
        .unwrap();
    screen
        .handle_app_message(AppEvent::NoteReloadFromDisk(VaultPath::new("n.md")), &tx)
        .await;
    let ed = screen.panels.editor().unwrap();
    assert_eq!(ed.get_text(), "+++\na = 1\n+++\nbody");
    assert!(!ed.is_dirty());
}

#[tokio::test]
async fn properties_open_flushes_dirty_buffer() {
    let (mut screen, vault, tx, _rx, _dir) = screen_on_note().await;
    screen.panels.editor_mut().unwrap().set_text("edited".to_string());
    assert!(screen.panels.editor().unwrap().is_dirty(), "precondition: unsaved edit");
    screen
        .handle_file_op(FileOp::ShowProperties(VaultPath::new("n.md")), &tx)
        .await;
    assert_eq!(vault.get_note_text(&VaultPath::new("n.md")).await.unwrap(), "edited");
    assert!(screen.overlays.is_open(), "dialog opened after a good save");
}
```

`set_text` with different text goes through `load()`; if the precondition assert shows `load()` leaves the buffer clean, dirty it by feeding a key instead: `screen.handle_input(&key_event(KeyCode::Char('x')), &tx)` with the editor focused (helper `key_event` exists in the tests module).

- [ ] **Step 2: Run, verify they fail**

Run: `cargo test -p kimun-notes --lib note_reload_from_disk`
Expected: compile error — variants missing.

- [ ] **Step 3: Implement**

`events.rs`:

```rust
    // in FileOp
    /// Open the properties dialog for a note (leader `n p`, palette, status bar).
    ShowProperties(VaultPath),
```

```rust
    // in AppEvent, near FileOp
    /// A core write (the properties dialog) changed this note on disk: reload
    /// the editor buffer from disk if it is the open note.
    NoteReloadFromDisk(VaultPath),
```

`editor.rs` `handle_file_op`:

```rust
            FileOp::ShowProperties(path) => {
                // The dialog writes the file through core; flush the buffer
                // first so no unsaved edit is lost under it.
                if path.is_like(&self.path) {
                    self.try_save().await;
                    if self.panels.editor().is_some_and(|e| e.is_dirty()) {
                        self.footer
                            .flash("save failed — properties not opened".to_string(), tx);
                        return;
                    }
                }
                self.present_overlay(Box::new(ActiveDialog::properties(
                    path,
                    self.vault.clone(),
                    tx,
                )));
            }
```

`handle_app_message` — new arm (place beside `AutosaveCompleted`):

```rust
            AppEvent::NoteReloadFromDisk(path) => {
                if path.is_like(&self.path) {
                    self.autosave_task.abort();
                    if let Ok(text) = self.vault.get_note_text(&self.path).await
                        && let Some(ed) = self.panels.editor_mut()
                    {
                        ed.set_text(text.clone());
                        ed.mark_saved(text);
                    }
                    self.doc_meta.refresh_properties(&self.path, tx);
                }
            }
```

`refresh_properties` comes from Task 11; until then, leave that line out and add it in Task 11.

Until Task 9 lands, `ActiveDialog::properties` does not exist. Do Task 8 and Task 9 in that order but run Task 8's tests after Task 9 Step 3 — or temporarily add `pub fn properties(path, vault, tx) -> Self { ActiveDialog::Help(HelpDialog::query_syntax()) }` and replace it in Task 9 (delete the stub then; the test only checks that an overlay opened).

- [ ] **Step 4: Run, verify pass**

Run: `cargo test -p kimun-notes --lib note_reload_from_disk && cargo test -p kimun-notes --lib properties_open_flushes`
Expected: PASS.

- [ ] **Step 5: Report green.**

---

### Task 9: TUI — `PropertiesDialog`, list state

**Files:**
- Create: `tui/src/components/dialogs/properties_dialog.rs`
- Modify: `tui/src/components/dialogs/mod.rs` (module, variant, constructor, routing, `set_error`, `handle_data`)
- Modify: `tui/src/components/events.rs` (`OverlayData`)

**Interfaces:**
- Consumes: `ButtonRow` (Task 5), `KeyPicker` (Task 6), `spawn_property_keys` (Task 7), `NoteVault::{get_properties, remove_property}`, `kimun_core::note::{PropertyEntry, PropertyValue, PropertyKind}`.
- Produces:
  - `OverlayData::PropertiesLoaded(Result<Vec<PropertyEntry>, String>)`
  - `OverlayData::PropertyWritten { result: Result<String, PropertyWriteError> }` where
    ```rust
    #[derive(Debug, Clone)]
    pub enum PropertyWriteError {
        /// Core refused the value (`VaultError::InvalidProperty`); `auto` typed saves may retry forced.
        Invalid(String),
        /// Anything else (IO, malformed block).
        Other(String),
    }
    ```
    `Ok(msg)` carries the flash text ("property saved" / "property removed").
  - `ActiveDialog::Properties(PropertiesDialog)`, `ActiveDialog::properties(path: VaultPath, vault: Arc<NoteVault>, tx: &AppTx) -> Self`.
  - `PropertiesDialog::{handle_key, handle_mouse, handle_loaded, handle_written, set_keys}`.

**State model** (both tasks 9 and 10 build on this):

```rust
pub struct PropertiesDialog {
    path: VaultPath,
    vault: Arc<NoteVault>,
    entries: Vec<PropertyEntry>,
    load_error: Option<String>,
    loading: bool,
    /// A write is in flight: every write action is ignored.
    busy: bool,
    selected: usize,
    scroll: usize,
    mode: Mode,
    buttons: ButtonRow,          // [+ Add] [Edit] [Delete] [Close]
    confirm: ButtonRow,          // [Yes] [No]
    focus: ListFocus,
    known_keys: Vec<String>,
    /// Rects of visible list rows from the last render (index into entries).
    row_rects: Vec<(Rect, usize)>,
    /// Last left-click on a row, for "click selected row again → edit".
    last_click_row: Option<usize>,
    pub(crate) error: Option<String>,
}

enum Mode {
    List,
    ConfirmDelete(String),
    Form(Box<PropertyForm>), // Task 10
}

#[derive(Clone, Copy, PartialEq)]
enum ListFocus { Rows, Buttons }

const BTN_ADD: usize = 0;
const BTN_EDIT: usize = 1;
const BTN_DELETE: usize = 2;
const BTN_CLOSE: usize = 3;
```

- [ ] **Step 1: Write the failing tests** (in the new file)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kimun_core::note::PropertyValue;
    use ratatui::crossterm::event::{KeyCode, KeyEvent};
    use ratatui::{Terminal, backend::TestBackend};
    use tokio::sync::mpsc::unbounded_channel;

    use crate::settings::themes::Theme;
    use crate::test_support::{mouse_down_at, temp_vault};

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    async fn dialog_with(entries: Vec<PropertyEntry>) -> (PropertiesDialog, AppTx, tokio::sync::mpsc::UnboundedReceiver<AppEvent>) {
        let vault = temp_vault("props_dialog").await;
        let (tx, rx) = unbounded_channel();
        let mut d = PropertiesDialog::new_unloaded(VaultPath::note_path_from("/n.md"), vault);
        d.handle_loaded(&Ok(entries));
        (d, tx, rx)
    }

    fn sample() -> Vec<PropertyEntry> {
        vec![
            ("status".into(), Some(PropertyValue::Text("draft".into()))),
            ("tags".into(), Some(PropertyValue::List(vec!["work".into(), "q4".into()]))),
            ("due".into(), None),
        ]
    }

    fn draw(d: &mut PropertiesDialog) {
        let theme = Theme::gruvbox_dark();
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| d.render(f, f.area(), &theme)).unwrap();
    }

    fn mouse(col: u16, row: u16) -> MouseEvent {
        match mouse_down_at(col, row) {
            InputEvent::Mouse(m) => m,
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn rows_show_key_type_value() {
        let (d, _tx, _rx) = dialog_with(sample()).await;
        assert_eq!(d.row_cells(0), ("status".into(), "text".into(), "draft".into()));
        assert_eq!(d.row_cells(1), ("tags".into(), "list".into(), "work, q4".into()));
        assert_eq!(d.row_cells(2), ("due".into(), "—".into(), String::new()));
    }

    #[tokio::test]
    async fn arrows_and_jk_move_selection() {
        let (mut d, tx, _rx) = dialog_with(sample()).await;
        d.handle_key(k(KeyCode::Down), &tx);
        d.handle_key(k(KeyCode::Char('j')), &tx);
        assert_eq!(d.selected, 2);
        d.handle_key(k(KeyCode::Down), &tx);
        assert_eq!(d.selected, 2, "stops at the end");
        d.handle_key(k(KeyCode::Char('k')), &tx);
        assert_eq!(d.selected, 1);
    }

    #[tokio::test]
    async fn click_selects_then_second_click_edits() {
        let (mut d, tx, _rx) = dialog_with(sample()).await;
        draw(&mut d);
        let (x, y) = d.row_origin(1).unwrap();
        d.handle_mouse(&mouse(x + 1, y), &tx);
        assert_eq!(d.selected, 1);
        assert!(matches!(d.mode, Mode::List));
        d.handle_mouse(&mouse(x + 1, y), &tx);
        assert!(matches!(d.mode, Mode::Form(_)), "second click opens the form");
    }

    #[tokio::test]
    async fn delete_asks_and_y_removes_from_disk() {
        let (mut d, tx, mut rx) = dialog_with(sample()).await;
        d.vault
            .create_note(&d.path, "+++\nstatus = \"draft\"\n+++\nbody")
            .await
            .unwrap();
        d.handle_key(k(KeyCode::Char('d')), &tx);
        assert!(matches!(d.mode, Mode::ConfirmDelete(ref key) if key == "status"));
        d.handle_key(k(KeyCode::Char('y')), &tx);
        assert!(d.busy, "write in flight");
        // The spawned remove reports back as PropertyWritten.
        let evt = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            evt,
            AppEvent::OverlayData(OverlayData::PropertyWritten { result: Ok(_) })
        ));
        assert!(d.vault.get_properties(&d.path).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_cancelled_by_n_or_esc() {
        for code in [KeyCode::Char('n'), KeyCode::Esc] {
            let (mut d, tx, mut rx) = dialog_with(sample()).await;
            d.handle_key(k(KeyCode::Delete), &tx);
            d.handle_key(k(code), &tx);
            assert!(matches!(d.mode, Mode::List));
            assert!(rx.try_recv().is_err(), "nothing written");
        }
    }

    #[tokio::test]
    async fn malformed_frontmatter_is_read_only() {
        let vault = temp_vault("props_dialog").await;
        let (tx, mut rx) = unbounded_channel();
        let mut d = PropertiesDialog::new_unloaded(VaultPath::note_path_from("/n.md"), vault);
        d.handle_loaded(&Err("'a' appears more than once".into()));
        for code in [KeyCode::Char('a'), KeyCode::Char('e'), KeyCode::Char('d'), KeyCode::Enter] {
            d.handle_key(k(code), &tx);
        }
        assert!(matches!(d.mode, Mode::List));
        assert!(rx.try_recv().is_err());
        assert!(!d.buttons.is_enabled(BTN_ADD));
        assert!(d.buttons.is_enabled(BTN_CLOSE));
    }

    #[tokio::test]
    async fn esc_closes_dialog() {
        let (mut d, tx, mut rx) = dialog_with(sample()).await;
        d.handle_key(k(KeyCode::Esc), &tx);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::CloseOverlay)));
    }

    #[tokio::test]
    async fn successful_write_reloads_note_and_list() {
        let (mut d, tx, mut rx) = dialog_with(sample()).await;
        d.busy = true;
        d.handle_written(&Ok("property saved".into()), &tx);
        assert!(!d.busy);
        let mut saw_reload = false;
        let mut saw_flash = false;
        while let Ok(evt) = rx.try_recv() {
            match evt {
                AppEvent::NoteReloadFromDisk(p) => saw_reload = p == d.path,
                AppEvent::FlashMessage(m) => saw_flash = m == "property saved",
                _ => {}
            }
        }
        assert!(saw_reload && saw_flash);
        assert!(d.loading, "list re-read requested");
    }
}
```

**Read-only trigger.** `get_properties` is lenient: a malformed block reads as *no properties*, not an error, and detecting malformation in the TUI would be format logic (forbidden). So the dialog becomes read-only in two ways: `PropertiesLoaded(Err)` (read failure: missing file, IO), or the first write failing with core's `FSError::InvalidFrontmatter`, mapped to `PropertyWriteError::Malformed` (see `from_vault` below; check the variant's exact shape with `grep -n "InvalidFrontmatter" core/src/error.rs`). The `malformed_frontmatter_is_read_only` test drives the first path; add one more test for the second:

```rust
    #[tokio::test]
    async fn malformed_write_error_turns_read_only() {
        let (mut d, tx, _rx) = dialog_with(sample()).await;
        d.busy = true;
        d.handle_written(&Err(PropertyWriteError::Malformed("bad block".into())), &tx);
        assert!(d.read_only());
        assert!(!d.buttons.is_enabled(BTN_ADD));
    }
```

- [ ] **Step 2: Run, verify they fail**

Run: `cargo test -p kimun-notes --lib properties_dialog`
Expected: compile error.

- [ ] **Step 3: Implement the list state**

Events (`events.rs`):

```rust
    /// The properties dialog's note, read: entries in file order, or the read error.
    PropertiesLoaded(Result<Vec<kimun_core::note::PropertyEntry>, String>),
    /// A properties-dialog write finished: flash text, or why it failed.
    PropertyWritten {
        result: Result<String, crate::components::dialogs::properties_dialog::PropertyWriteError>,
    },
```

Dialog core methods:

```rust
impl PropertiesDialog {
    pub fn new(path: VaultPath, vault: Arc<NoteVault>, tx: &AppTx) -> Self {
        let mut d = Self::new_unloaded(path, vault);
        d.reload(tx);
        super::spawn_property_keys(d.vault.clone(), tx);
        d
    }

    pub(crate) fn new_unloaded(path: VaultPath, vault: Arc<NoteVault>) -> Self {
        Self {
            path,
            vault,
            entries: Vec::new(),
            load_error: None,
            loading: true,
            busy: false,
            selected: 0,
            scroll: 0,
            mode: Mode::List,
            buttons: ButtonRow::new(&["+ Add", "Edit", "Delete", "Close"]),
            confirm: ButtonRow::new(&["Yes", "No"]),
            focus: ListFocus::Rows,
            known_keys: Vec::new(),
            row_rects: Vec::new(),
            last_click_row: None,
            error: None,
        }
    }

    fn reload(&mut self, tx: &AppTx) {
        self.loading = true;
        let vault = self.vault.clone();
        let path = self.path.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let result = vault.get_properties(&path).await.map_err(|e| e.to_string());
            tx.send(AppEvent::OverlayData(OverlayData::PropertiesLoaded(result))).ok();
        });
    }

    pub(crate) fn handle_loaded(&mut self, result: &Result<Vec<PropertyEntry>, String>) {
        self.loading = false;
        match result {
            Ok(entries) => {
                self.entries = entries.clone();
                self.load_error = None;
                self.selected = self.selected.min(self.entries.len().saturating_sub(1));
            }
            Err(e) => {
                self.entries.clear();
                self.load_error = Some(e.clone());
            }
        }
        self.sync_buttons();
    }

    fn read_only(&self) -> bool {
        self.load_error.is_some()
    }

    fn sync_buttons(&mut self) {
        let writable = !self.read_only() && !self.busy && !self.loading;
        let has_row = writable && !self.entries.is_empty();
        self.buttons.set_enabled(BTN_ADD, writable);
        self.buttons.set_enabled(BTN_EDIT, has_row);
        self.buttons.set_enabled(BTN_DELETE, has_row);
        self.buttons.set_enabled(BTN_CLOSE, true);
    }

    pub(crate) fn set_keys(&mut self, keys: Vec<String>) {
        self.known_keys = keys;
        if let Mode::Form(form) = &mut self.mode {
            form.key.set_keys(self.known_keys.clone());
        }
    }

    pub(crate) fn handle_written(&mut self, result: &Result<String, PropertyWriteError>, tx: &AppTx) {
        self.busy = false;
        match result {
            Ok(flash) => {
                self.mode = Mode::List;
                tx.send(AppEvent::NoteReloadFromDisk(self.path.clone())).ok();
                tx.send(AppEvent::FlashMessage(flash.clone())).ok();
                self.reload(tx);
                super::spawn_property_keys(self.vault.clone(), tx);
            }
            Err(PropertyWriteError::Malformed(msg)) => {
                self.mode = Mode::List;
                self.load_error = Some(msg.clone());
            }
            Err(e) => self.form_error(e), // Task 10; in Task 9: self.error = Some(e.message())
        }
        self.sync_buttons();
    }
}
```

Cells:

```rust
    /// (key, type, value) as the list shows them.
    pub(crate) fn row_cells(&self, i: usize) -> (String, String, String) {
        let (key, value) = &self.entries[i];
        match value {
            Some(v) => (key.clone(), v.kind().as_str().to_string(), v.to_string()),
            None => (key.clone(), "—".to_string(), String::new()),
        }
    }
```

Delete:

```rust
    fn start_delete(&mut self) {
        if self.busy || self.read_only() {
            return;
        }
        if let Some((key, _)) = self.entries.get(self.selected) {
            self.mode = Mode::ConfirmDelete(key.clone());
            self.confirm.set_focused(Some(1)); // default No
        }
    }

    fn confirm_delete(&mut self, tx: &AppTx) {
        let Mode::ConfirmDelete(key) = std::mem::replace(&mut self.mode, Mode::List) else {
            return;
        };
        self.busy = true;
        self.sync_buttons();
        let vault = self.vault.clone();
        let path = self.path.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let result = vault
                .remove_property(&path, &key)
                .await
                .map(|_| "property removed".to_string())
                .map_err(PropertyWriteError::from_vault);
            tx.send(AppEvent::OverlayData(OverlayData::PropertyWritten { result })).ok();
        });
    }
```

```rust
#[derive(Debug, Clone)]
pub enum PropertyWriteError {
    /// Core refused the value (`VaultError::InvalidProperty`).
    Invalid(String),
    /// The note's frontmatter block does not parse; the dialog turns read-only.
    Malformed(String),
    /// Anything else (IO, lock timeout).
    Other(String),
}

impl PropertyWriteError {
    fn from_vault(e: kimun_core::error::VaultError) -> Self {
        use kimun_core::error::{FSError, VaultError};
        match e {
            VaultError::InvalidProperty { message, .. } => Self::Invalid(message),
            VaultError::FSError(FSError::InvalidFrontmatter { .. }) => Self::Malformed(e.to_string()),
            other => Self::Other(other.to_string()),
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Invalid(m) | Self::Malformed(m) | Self::Other(m) => m,
        }
    }
}
```

(Verify the `FSError::InvalidFrontmatter` shape — struct vs tuple variant — in `core/src/error.rs` and adjust the pattern; if it is tuple-like use `(..)`. If `e` is moved by the match, bind with `ref` or pre-compute `e.to_string()`.)

Key handling in `Mode::List` (busy/read-only gates write actions):

```rust
    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        match &self.mode {
            Mode::Form(_) => return self.handle_form_key(key, tx), // Task 10
            Mode::ConfirmDelete(_) => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => self.confirm_delete(tx),
                    KeyCode::Enter if self.confirm.focused() == Some(0) => self.confirm_delete(tx),
                    KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                        let next = if self.confirm.focused() == Some(0) { 1 } else { 0 };
                        self.confirm.set_focused(Some(next));
                    }
                    _ => self.mode = Mode::List, // n, N, Esc, Enter-on-No: cancel
                }
                return EventState::Consumed;
            }
            Mode::List => {}
        }
        let can_write = !self.busy && !self.read_only() && !self.loading;
        match key.code {
            KeyCode::Esc => {
                tx.send(AppEvent::CloseOverlay).ok();
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    ListFocus::Rows => {
                        self.buttons.set_focused(None);
                        self.buttons.focus_next();
                        ListFocus::Buttons
                    }
                    ListFocus::Buttons => {
                        self.buttons.set_focused(None);
                        ListFocus::Rows
                    }
                };
            }
            KeyCode::Left if self.focus == ListFocus::Buttons => {
                self.buttons.focus_prev();
            }
            KeyCode::Right if self.focus == ListFocus::Buttons => {
                self.buttons.focus_next();
            }
            KeyCode::Enter if self.focus == ListFocus::Buttons => {
                if let Some(b) = self.buttons.focused() {
                    self.press(b, tx);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1))
            }
            KeyCode::Enter | KeyCode::Char('e') if can_write => self.open_edit(),
            KeyCode::Char('a') if can_write => self.open_add(),
            KeyCode::Char('d') | KeyCode::Delete if can_write => self.start_delete(),
            _ => {}
        }
        EventState::Consumed
    }

    fn press(&mut self, button: usize, tx: &AppTx) {
        match button {
            BTN_ADD => self.open_add(),
            BTN_EDIT => self.open_edit(),
            BTN_DELETE => self.start_delete(),
            BTN_CLOSE => {
                tx.send(AppEvent::CloseOverlay).ok();
            }
            _ => {}
        }
    }
```

`open_add` / `open_edit` build the form (Task 10). In Task 9, implement them as `self.mode = Mode::Form(Box::new(PropertyForm::add(&self.known_keys)))` and `PropertyForm::edit(key, value, &self.known_keys)` with a minimal `PropertyForm` struct (fields from Task 10), so the click test passes; Task 10 fills in its behaviour.

Mouse in `Mode::List`/`ConfirmDelete`:

```rust
    pub fn handle_mouse(&mut self, ev: &MouseEvent, tx: &AppTx) -> EventState {
        if let Mode::Form(_) = self.mode {
            return self.handle_form_mouse(ev, tx); // Task 10
        }
        match ev.kind {
            MouseEventKind::ScrollDown => {
                self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
            }
            MouseEventKind::ScrollUp => self.selected = self.selected.saturating_sub(1),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Mode::ConfirmDelete(_) = self.mode {
                    match self.confirm.hit(ev.column, ev.row) {
                        Some(0) => self.confirm_delete(tx),
                        Some(_) => self.mode = Mode::List,
                        None => {}
                    }
                    return EventState::Consumed;
                }
                if let Some(b) = self.buttons.hit(ev.column, ev.row) {
                    self.press(b, tx);
                    return EventState::Consumed;
                }
                let pos = Position { x: ev.column, y: ev.row };
                if let Some(&(_, i)) = self.row_rects.iter().find(|(r, _)| r.contains(pos)) {
                    let again = self.last_click_row == Some(i) && self.selected == i;
                    self.selected = i;
                    self.focus = ListFocus::Rows;
                    self.last_click_row = Some(i);
                    if again && !self.busy && !self.read_only() {
                        self.last_click_row = None;
                        self.open_edit();
                    }
                }
            }
            _ => {}
        }
        EventState::Consumed
    }
```

Render (`pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme)`): popup `fixed_centered_rect(60, 16, rect)` with `modal_chrome` title `" Properties: <note name> "` (note name from `self.path.get_clean_name()` — check the exact `VaultPath` method name used elsewhere for display, e.g. in `rename_dialog.rs`). Inside:
- row 0: header `"  Key            Type      Value"` in `theme.gray`.
- rows 1..=N (body height − 3): entries from `self.scroll` (keep `selected` visible by adjusting `scroll`), each formatted with key padded/truncated to 14 display columns, type to 9, value truncated with `…` to the remaining width; selected row uses selection colours with a `▶` marker; record `(rect, index)` in `row_rects`.
- `loading` → `"  Loading…"`; empty → `"  No properties. Press a or click + Add."`; `load_error` → `render_error_row(f, row, msg, theme)` plus `"  Read-only: fix the frontmatter in the editor."`.
- `self.error` (non-form) → error row above the buttons.
- second-to-last row: `self.buttons.render(...)`, or in `ConfirmDelete(key)` the text `Delete "<key>"?` followed by `self.confirm.render(...)` in the remaining width.
- last row: footer hint `"  ↑↓ select · Enter edit · a add · d delete · Esc close"`.
In `Mode::Form`, delegate to `render_form` (Task 10).

`dialogs/mod.rs` wiring:
- `pub mod properties_dialog;` + `pub use properties_dialog::PropertiesDialog;`
- variant `Properties(PropertiesDialog)`; `set_error`: `ActiveDialog::Properties(d) => d.error = Some(msg)`.
- constructor: `pub fn properties(path: VaultPath, vault: Arc<NoteVault>, tx: &AppTx) -> Self { ActiveDialog::Properties(PropertiesDialog::new(path, vault, tx)) }` (replace the Task 8 stub).
- `handle_input`: key → `ActiveDialog::Properties(d) => d.handle_key(*key, tx)`; mouse → `ActiveDialog::Properties(d) => d.handle_mouse(m, tx)`.
- render arm: `ActiveDialog::Properties(d) => d.render(f, rect, theme)`.
- `handle_data`:
  ```rust
            OverlayData::PropertiesLoaded(result) => {
                if let ActiveDialog::Properties(d) = self {
                    d.handle_loaded(result);
                }
                OverlayMsg::Consumed
            }
            OverlayData::PropertyWritten { result } => {
                if let ActiveDialog::Properties(d) = self {
                    d.handle_written(result, tx);
                }
                OverlayMsg::Consumed
            }
  ```
  and extend the `PropertyKeysLoaded` arm: `ActiveDialog::Properties(d) => d.set_keys(keys.clone())`.

Test helper: `#[cfg(test)] pub(crate) fn row_origin(&self, i: usize) -> Option<(u16, u16)>` returning the rect origin of entry `i` in `row_rects`.

- [ ] **Step 4: Run, verify pass**

Run: `cargo test -p kimun-notes --lib properties_dialog`
Expected: PASS (the form tests come in Task 10).

- [ ] **Step 5: Report green.**

---

### Task 10: TUI — `PropertiesDialog`, form state, save and mismatch

**Files:**
- Modify: `tui/src/components/dialogs/properties_dialog.rs`

**Interfaces:**
- Consumes: `NoteVault::{set_property_from_input, rename_property}`, `kimun_core::note::{PropertyInput, PropertyKind, PropertyValue, FrontmatterFormat}`, `KeyPicker`, `ButtonRow`.
- Produces: `PropertyForm` and the save flow. No new public API beyond Task 9.

**Form model:**

```rust
struct PropertyForm {
    /// `None` for Add; the original key for Edit.
    orig_key: Option<String>,
    /// The original value's kind (Edit), shown as "(currently <kind>)".
    current_kind: Option<PropertyKind>,
    /// The original entry had no value (YAML `due:`).
    was_bare: bool,
    key: KeyPicker,
    /// Index into TYPES; 0 = auto.
    kind_idx: usize,
    value: SingleLineInput,
    focus: FormFocus,
    buttons: ButtonRow,         // [Save] [Cancel]
    /// Shown under Value: core's refusal or a local check.
    error: Option<String>,
    /// Offered after an `auto` save was refused: retry forced to this kind.
    store_anyway: Option<PropertyKind>,
    store_btn: ButtonRow,       // [Store as <kind> anyway]
    // rects from the last render
    value_rect: Option<Rect>,
    kind_prev_rect: Option<Rect>,
    kind_next_rect: Option<Rect>,
}

#[derive(Clone, Copy, PartialEq)]
enum FormFocus { Key, Kind, Value, StoreAnyway, Buttons }

/// `None` = auto (let core decide against the vault's type).
const TYPES: [Option<PropertyKind>; 7] = [
    None,
    Some(PropertyKind::Text),
    Some(PropertyKind::Number),
    Some(PropertyKind::Bool),
    Some(PropertyKind::Date),
    Some(PropertyKind::DateTime),
    Some(PropertyKind::List),
];
```

Edit pre-fill: key = original key; value = `v.to_string()` (`Display` joins lists with `, `; an empty list displays `[]` — pre-fill an empty `List` as `""` instead); `kind_idx = 0`; `current_kind = v.kind()`.

**Values from the field** (`PropertyForm::input_values(&self) -> Vec<String>`): if the chosen type is `List`, or (type `auto` and the original kind was `List`), split on `,`, trim, drop empties; otherwise `vec![value.to_string()]` (untrimmed — Text keeps spaces; core parses others leniently).

Consequences, all intended: `tags` edited with type `auto` splits (original kind is List; core would split a single tag string anyway); a *new* key with type `auto` and value `a, b` is stored as text `"a, b"` — the user picks `list` to get a list.

- [ ] **Step 1: Write the failing tests** (add to the tests module)

```rust
    async fn vault_note(d: &PropertiesDialog, text: &str) {
        d.vault.create_note(&d.path, text).await.unwrap();
    }

    async fn next_written(rx: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>) -> Result<String, PropertyWriteError> {
        loop {
            let evt = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .expect("timely")
                .expect("open channel");
            if let AppEvent::OverlayData(OverlayData::PropertyWritten { result }) = evt {
                return result;
            }
        }
    }

    fn type_into(d: &mut PropertiesDialog, tx: &AppTx, s: &str) {
        for c in s.chars() {
            d.handle_key(k(KeyCode::Char(c)), tx);
        }
    }

    #[tokio::test]
    async fn add_writes_a_new_property() {
        let (mut d, tx, mut rx) = dialog_with(vec![]).await;
        vault_note(&d, "body").await;
        d.handle_key(k(KeyCode::Char('a')), &tx);
        type_into(&mut d, &tx, "priority");
        d.handle_key(k(KeyCode::Tab), &tx); // → Type
        d.handle_key(k(KeyCode::Tab), &tx); // → Value
        type_into(&mut d, &tx, "2");
        d.handle_key(k(KeyCode::Enter), &tx);
        assert!(next_written(&mut rx).await.is_ok());
        assert_eq!(
            d.vault.get_property(&d.path, "priority").await.unwrap(),
            Some(Some(PropertyValue::Number(2.0)))
        );
    }

    #[tokio::test]
    async fn empty_key_and_empty_value_are_refused_locally() {
        let (mut d, tx, mut rx) = dialog_with(vec![]).await;
        d.handle_key(k(KeyCode::Char('a')), &tx);
        d.handle_key(k(KeyCode::Enter), &tx);
        assert_eq!(d.form_error().as_deref(), Some("key required"));
        type_into(&mut d, &tx, "k");
        d.handle_key(k(KeyCode::Enter), &tx);
        assert_eq!(d.form_error().as_deref(), Some("value required"));
        assert!(rx.try_recv().is_err(), "nothing spawned");
    }

    #[tokio::test]
    async fn key_change_renames_then_sets() {
        let (mut d, tx, mut rx) =
            dialog_with(vec![("status".into(), Some(PropertyValue::Text("draft".into())))]).await;
        vault_note(&d, "+++\na = 1\nstatus = \"draft\"\nz = 2\n+++\nbody").await;
        d.handle_key(k(KeyCode::Enter), &tx); // edit row 0
        // Replace the key: clear it, type "state".
        for _ in 0.."status".len() {
            d.handle_key(k(KeyCode::Backspace), &tx);
        }
        type_into(&mut d, &tx, "state");
        d.handle_key(k(KeyCode::Esc), &tx); // close the suggestion list if open
        d.handle_key(k(KeyCode::Tab), &tx);
        d.handle_key(k(KeyCode::Tab), &tx);
        for _ in 0.."draft".len() {
            d.handle_key(k(KeyCode::Backspace), &tx);
        }
        type_into(&mut d, &tx, "done");
        d.handle_key(k(KeyCode::Enter), &tx);
        assert!(next_written(&mut rx).await.is_ok());
        assert_eq!(
            d.vault.get_note_text(&d.path).await.unwrap(),
            "+++\na = 1\nstate = \"done\"\nz = 2\n+++\nbody"
        );
    }

    #[tokio::test]
    async fn bare_key_rename_keeps_it_valueless() {
        let (mut d, tx, mut rx) = dialog_with(vec![("due".into(), None)]).await;
        vault_note(&d, "---\ndue:\n---\nbody").await;
        d.handle_key(k(KeyCode::Enter), &tx);
        for _ in 0.."due".len() {
            d.handle_key(k(KeyCode::Backspace), &tx);
        }
        type_into(&mut d, &tx, "deadline");
        d.handle_key(k(KeyCode::Esc), &tx);
        d.handle_key(k(KeyCode::Enter), &tx);
        assert!(next_written(&mut rx).await.is_ok());
        assert_eq!(d.vault.get_note_text(&d.path).await.unwrap(), "---\ndeadline:\n---\nbody");
    }

    #[tokio::test]
    async fn mismatch_offers_store_anyway_which_forces_the_kind() {
        let (mut d, tx, mut rx) = dialog_with(vec![]).await;
        // Two other notes make `priority` a number key.
        for n in ["/o1.md", "/o2.md"] {
            d.vault
                .create_note(&VaultPath::note_path_from(n), "+++\npriority = 1\n+++\nx")
                .await
                .unwrap();
        }
        vault_note(&d, "body").await;
        d.handle_key(k(KeyCode::Char('a')), &tx);
        type_into(&mut d, &tx, "priority");
        d.handle_key(k(KeyCode::Esc), &tx);
        d.handle_key(k(KeyCode::Tab), &tx);
        d.handle_key(k(KeyCode::Tab), &tx);
        type_into(&mut d, &tx, "high");
        d.handle_key(k(KeyCode::Enter), &tx);
        let result = next_written(&mut rx).await;
        assert!(matches!(result, Err(PropertyWriteError::Invalid(_))));
        d.handle_written(&result, &tx);
        assert_eq!(d.store_anyway_kind(), Some(PropertyKind::Text));
        d.press_store_anyway(&tx);
        assert!(next_written(&mut rx).await.is_ok());
        assert_eq!(
            d.vault.get_property(&d.path, "priority").await.unwrap(),
            Some(Some(PropertyValue::Text("high".into())))
        );
    }

    #[tokio::test]
    async fn forced_type_parse_error_has_no_store_anyway() {
        let (mut d, tx, mut rx) = dialog_with(vec![]).await;
        vault_note(&d, "body").await;
        d.handle_key(k(KeyCode::Char('a')), &tx);
        type_into(&mut d, &tx, "when");
        d.handle_key(k(KeyCode::Esc), &tx);
        d.handle_key(k(KeyCode::Tab), &tx); // Type
        for _ in 0..4 {
            d.handle_key(k(KeyCode::Right), &tx); // auto→text→number→bool→date
        }
        d.handle_key(k(KeyCode::Tab), &tx);
        type_into(&mut d, &tx, "not a date");
        d.handle_key(k(KeyCode::Enter), &tx);
        let result = next_written(&mut rx).await;
        d.handle_written(&result, &tx);
        assert!(d.form_error().is_some());
        assert_eq!(d.store_anyway_kind(), None);
    }

    #[tokio::test]
    async fn second_save_while_busy_is_ignored() {
        let (mut d, tx, mut rx) = dialog_with(vec![]).await;
        vault_note(&d, "body").await;
        d.handle_key(k(KeyCode::Char('a')), &tx);
        type_into(&mut d, &tx, "k");
        d.handle_key(k(KeyCode::Esc), &tx);
        d.handle_key(k(KeyCode::Tab), &tx);
        d.handle_key(k(KeyCode::Tab), &tx);
        type_into(&mut d, &tx, "v");
        d.handle_key(k(KeyCode::Enter), &tx);
        d.handle_key(k(KeyCode::Enter), &tx); // double press
        assert!(next_written(&mut rx).await.is_ok());
        let second = tokio::time::timeout(std::time::Duration::from_millis(300), next_written(&mut rx)).await;
        assert!(second.is_err(), "only one write was spawned");
    }

    #[tokio::test]
    async fn esc_in_form_returns_to_list_without_writing() {
        let (mut d, tx, mut rx) = dialog_with(sample()).await;
        d.handle_key(k(KeyCode::Char('a')), &tx);
        d.handle_key(k(KeyCode::Esc), &tx); // key list closed already → back to list
        assert!(matches!(d.mode, Mode::List));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn clicking_type_arrows_cycles_kind() {
        let (mut d, tx, _rx) = dialog_with(vec![]).await;
        d.handle_key(k(KeyCode::Char('a')), &tx);
        draw(&mut d);
        let (x, y) = d.kind_next_origin().unwrap();
        d.handle_mouse(&mouse(x, y), &tx);
        assert_eq!(d.form_kind(), Some(PropertyKind::Text));
    }
```

Test accessors (all `#[cfg(test)] pub(crate)`): `form_error(&self) -> Option<String>`, `store_anyway_kind(&self) -> Option<PropertyKind>`, `form_kind(&self) -> Option<PropertyKind>`, `kind_next_origin(&self) -> Option<(u16,u16)>`; `press_store_anyway(&mut self, tx)` is the real handler the `[Store as … anyway]` button calls (not test-only).

- [ ] **Step 2: Run, verify they fail**

Run: `cargo test -p kimun-notes --lib properties_dialog`
Expected: new tests fail (form behaviour missing).

- [ ] **Step 3: Implement the form**

Key handling (`handle_form_key`):

```rust
    fn handle_form_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        let Mode::Form(form) = &mut self.mode else {
            return EventState::Consumed;
        };
        // The key picker's open list owns Enter/Esc/arrows first.
        if form.focus == FormFocus::Key {
            match form.key.handle_key(&key) {
                PickerOutcome::Accepted(_) | PickerOutcome::Changed | PickerOutcome::Consumed => {
                    form.clear_feedback_if_changed();
                    return EventState::Consumed;
                }
                PickerOutcome::Submit => {
                    self.submit(None, tx);
                    return EventState::Consumed;
                }
                PickerOutcome::Cancel => {
                    self.mode = Mode::List;
                    return EventState::Consumed;
                }
                PickerOutcome::NotConsumed => {} // Tab etc. below
            }
        }
        match key.code {
            KeyCode::Esc => self.mode = Mode::List,
            KeyCode::Tab => form.focus_next(),
            KeyCode::BackTab => form.focus_prev(),
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if form.focus == FormFocus::Kind => {
                form.cycle_kind(key.code != KeyCode::Left);
            }
            KeyCode::Enter => match form.focus {
                FormFocus::StoreAnyway => self.press_store_anyway(tx),
                FormFocus::Buttons if form.buttons.focused() == Some(1) => self.mode = Mode::List,
                _ => self.submit(None, tx),
            },
            _ if form.focus == FormFocus::Value => {
                if form.value.handle_key(&key) == InputOutcome::Changed {
                    form.error = None;
                    form.store_anyway = None;
                }
            }
            _ => {}
        }
        EventState::Consumed
    }
```

(`clear_feedback_if_changed` clears `error`/`store_anyway` when the key text changed; `focus_next`/`focus_prev` cycle `Key → Kind → Value → [StoreAnyway if offered] → Buttons → Key`, and on entering `Buttons` focus `[Save]`.) Borrow-checker note: `self.submit`/`self.press_store_anyway` need `&mut self` while `form` borrows `self.mode`; compute an action enum inside the match and act after it, e.g. `enum FormAction { None, Submit, StoreAnyway, Cancel }`.

Submit:

```rust
    /// Validate and spawn the write. `forced` overrides the type cycler
    /// (the "store anyway" retry).
    fn submit(&mut self, forced: Option<PropertyKind>, tx: &AppTx) {
        if self.busy {
            return;
        }
        let Mode::Form(form) = &mut self.mode else { return };
        let new_key = form.key.value().trim().to_string();
        if new_key.is_empty() {
            form.error = Some("key required".into());
            return;
        }
        let values = form.input_values();
        let value_empty = values.iter().all(|v| v.trim().is_empty());
        let renamed = form.orig_key.as_deref().filter(|o| *o != new_key).map(str::to_string);
        // A bare key whose value is still empty: rename only (or nothing).
        let rename_only = form.was_bare && value_empty;
        if value_empty && !rename_only {
            form.error = Some("value required".into());
            return;
        }
        if rename_only && renamed.is_none() {
            self.mode = Mode::List;
            return;
        }
        let kind = forced.or(TYPES[form.kind_idx]);
        let input = PropertyInput::new(values).forced(kind);
        form.error = None;
        form.store_anyway = None;
        self.busy = true;
        self.sync_buttons();
        let vault = self.vault.clone();
        let path = self.path.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let result = async {
                if let Some(old) = &renamed {
                    vault.rename_property(&path, old, &new_key).await?;
                }
                if !rename_only {
                    vault
                        .set_property_from_input(&path, &new_key, &input, FrontmatterFormat::Toml)
                        .await?;
                }
                Ok::<_, kimun_core::error::VaultError>("property saved".to_string())
            }
            .await
            .map_err(PropertyWriteError::from_vault);
            tx.send(AppEvent::OverlayData(OverlayData::PropertyWritten { result })).ok();
        });
    }
```

Rename-then-failed-set: the spawn reports the set's error; the rename already landed. `handle_written` on an `Err` while a rename happened must still reload the editor and the list: track `self.pending_rename: Option<(String, String)>` set in `submit` when `renamed.is_some()`; in `handle_written(Err(_))`, if it is `Some((old,new))`, send `NoteReloadFromDisk`, call `reload`, and set the form's `orig_key = Some(new)` so a retry does not try to rename again. Clear `pending_rename` in every `handle_written`.

Mismatch (`form_error` in `handle_written`):

```rust
    fn form_error(&mut self, e: &PropertyWriteError) {
        let Mode::Form(form) = &mut self.mode else {
            self.error = Some(e.message().to_string());
            return;
        };
        form.error = Some(e.message().to_string());
        form.store_anyway = match e {
            PropertyWriteError::Invalid(_) if TYPES[form.kind_idx].is_none() => {
                Some(form.inferred_kind())
            }
            _ => None,
        };
        if let Some(kind) = form.store_anyway {
            form.store_btn = ButtonRow::new(&[&format!("Store as {kind} anyway")]);
        }
    }
```

`inferred_kind`: `let v = form.input_values(); if v.len() > 1 { PropertyKind::List } else { PropertyValue::infer(v.first().map_or("", String::as_str)).kind() }`.

`press_store_anyway`: `if let Some(kind) = <form>.store_anyway { self.submit(Some(kind), tx) }`.

Mouse (`handle_form_mouse`), left button only:
1. `form.key.handle_click` → `Accepted` (focus Key) / `Consumed` (focus Key) / else continue.
2. `value_rect` contains → focus Value.
3. `kind_prev_rect` / `kind_next_rect` contains → focus Kind, `cycle_kind(false/true)`.
4. `store_btn.hit` → `press_store_anyway`.
5. `buttons.hit` → `Some(0)` submit, `Some(1)` back to list.
Everything else: consumed, no action.

Render (`render_form`), same popup size as the list, title `" Add property "` or `" Edit property "`:
```
 Key    [key picker...........]
 Type   ‹ auto ›   (currently date)
 Value  [value................]
        <error in theme.red>            (if any)
        [ Store as text anyway ]        (if offered)

            [ Save ]  [ Cancel ]
 Tab next field · ←→ type · Enter save · Esc back
```
Labels are 8 columns wide; the key picker renders **last** (its list overlays rows below). `‹` and `›` rects are 1 cell each, recorded for clicks. The focused field gets `theme.accent` label colour; the value field renders with `SingleLineInput::render(.., focused = focus == Value)`. Buttons disabled while `busy`.

- [ ] **Step 4: Run, verify pass**

Run: `cargo test -p kimun-notes --lib properties_dialog`
Expected: all PASS.

- [ ] **Step 5: Report green.**

---

### Task 11: TUI — entry points (leader `n p`, palette, status bar)

**Files:**
- Modify: `tui/src/keys/leader.rs` (enum `:16`, `id()` `:~119`, `from_id`/ALL list `:~192`, `default_label()` `:~274`, tree `:445-456`, tests)
- Modify: `tui/src/app_screen/editor.rs` (`execute_leader_action` `:1881`, `EditorIntent::Mouse` branch `:848`, footer `DocState` `:2319`, `AutosaveCompleted` `:1473`)
- Modify: `tui/src/app_screen/doc_meta.rs`
- Modify: `tui/src/components/footer_bar.rs`
- Modify: `tui/src/components/events.rs` (`AppEvent::PropertyCountLoaded`)

**Interfaces:**
- Consumes: `FileOp::ShowProperties` (Task 8).
- Produces: `LeaderAction::NoteProperties` (id `"note.properties"`, label `"properties"`); `DocMeta::properties(&self) -> Option<usize>`, `DocMeta::refresh_properties(&mut self, path: &VaultPath, tx: &AppTx)`; `AppEvent::PropertyCountLoaded { path: VaultPath, count: usize }`; `DocState.props: Option<usize>`; `FooterBar::props_hit(&self, col: u16, row: u16) -> bool`.

The command palette is built from the leader tree (`CommandPaletteModal::new(&s.leader_tree(), ..)`, `editor.rs:~1104`), so the leaf gives the palette entry for free; F1's cheatsheet also lists it.

- [ ] **Step 1: Write the failing tests**

`leader.rs` tests (beside the `NoteDelete` assertions at `:1025`):

```rust
    #[test]
    fn n_p_fires_note_properties() {
        let mut e = LeaderEngine::new();
        e.start();
        e.feed('n');
        assert_eq!(e.feed('p'), LeaderOutcome::Fired(LeaderAction::NoteProperties));
        assert_eq!(LeaderAction::NoteProperties.id(), "note.properties");
        assert_eq!(LeaderAction::from_id("note.properties"), Some(LeaderAction::NoteProperties));
    }
```

`doc_meta.rs` tests:

```rust
    #[tokio::test]
    async fn property_count_guards_against_stale_paths() {
        let (mut dm, _dir) = meta().await;
        let current = note("/a.md");
        dm.handle(AppEvent::PropertyCountLoaded { path: note("/old.md"), count: 4 }, &current);
        assert_eq!(dm.properties(), None);
        dm.handle(AppEvent::PropertyCountLoaded { path: current.clone(), count: 2 }, &current);
        assert_eq!(dm.properties(), Some(2));
    }
```

`footer_bar.rs` tests:

```rust
    #[test]
    fn props_segment_is_clickable_where_drawn() {
        use ratatui::{Terminal, backend::TestBackend};
        let theme = Theme::gruvbox_dark();
        let mut bar = FooterBar::new();
        let mut t = Terminal::new(TestBackend::new(100, 2)).unwrap();
        let ctx = StatusContext {
            focus_label: "EDITOR",
            editing: true,
            hints: &[],
            global_hints: &[],
            doc: DocState { path: "n.md", props: Some(3), ..Default::default() },
        };
        t.draw(|f| bar.render(f, f.area(), &theme, &ctx)).unwrap();
        let buf = t.backend().buffer().clone();
        let line: String = (0..100).map(|x| buf[(x, 1)].symbol().to_string()).collect();
        let col = line.find("⊞ 3 props").expect("segment drawn");
        // `find` is a byte offset; the path and separators before it are ASCII
        // except "✓" (3 bytes, 1 column) — convert via char count.
        let col = line[..col].chars().count() as u16;
        assert!(bar.props_hit(col, 1));
        assert!(bar.props_hit(col + 8, 1));
        assert!(!bar.props_hit(col.saturating_sub(2), 1));
        assert!(!bar.props_hit(col, 0), "line 1 is not the segment");
    }

    #[test]
    fn no_props_segment_without_count() {
        use ratatui::{Terminal, backend::TestBackend};
        let theme = Theme::gruvbox_dark();
        let mut bar = FooterBar::new();
        let mut t = Terminal::new(TestBackend::new(100, 2)).unwrap();
        let ctx = StatusContext {
            focus_label: "EDITOR",
            editing: true,
            hints: &[],
            global_hints: &[],
            doc: DocState { path: "n.md", ..Default::default() },
        };
        t.draw(|f| bar.render(f, f.area(), &theme, &ctx)).unwrap();
        assert!(!(0..100).any(|c| bar.props_hit(c, 1)));
    }
```

(Adapt `hints`/`global_hints` types to what `StatusContext` declares — `&[Hint]` and the global-hints slice type.)

`editor.rs` test (uses `screen_on_note` from Task 8, `lay_out`/`press_at` from the existing mouse tests):

```rust
#[tokio::test]
async fn clicking_props_segment_opens_properties() {
    let (mut screen, _vault, tx, mut rx, _dir) = screen_on_note().await;
    // Land the async property count so the segment renders.
    screen
        .handle_app_message(
            AppEvent::PropertyCountLoaded { path: VaultPath::new("n.md"), count: 0 },
            &tx,
        )
        .await;
    lay_out(&mut screen);
    let (col, row) = (0..40u16)
        .flat_map(|r| (0..120u16).map(move |c| (c, r)))
        .find(|(c, r)| screen.footer.props_hit(*c, *r))
        .expect("props segment laid out");
    while rx.try_recv().is_ok() {}
    screen.handle_input(&press_at(col, row), &tx);
    let mut opened = false;
    while let Ok(evt) = rx.try_recv() {
        opened |= matches!(evt, AppEvent::FileOp(FileOp::ShowProperties(_)));
    }
    assert!(opened);
}
```

- [ ] **Step 2: Run, verify they fail**

Run: `cargo test -p kimun-notes --lib n_p_fires && cargo test -p kimun-notes --lib property_count_guards && cargo test -p kimun-notes --lib props_segment`
Expected: compile errors.

- [ ] **Step 3: Leader**

`leader.rs`: add `NoteProperties,` after `NoteDelete` in the enum; `LeaderAction::NoteProperties => "note.properties"` in `id()`; add it to the list used by `from_id` (after `NoteDelete`); `LeaderAction::NoteProperties => "properties"` in `default_label()`; tree leaf in `+note`: `('p', leaf("properties", A::NoteProperties)),` after `('m', ...)`.

`editor.rs` `execute_leader_action`:

```rust
            LeaderAction::NoteProperties => {
                if let Some(path) = self.open_note_or_flash(tx) {
                    tx.send(AppEvent::FileOp(FileOp::ShowProperties(path))).ok();
                }
            }
```

Fix any exhaustive `match` on `LeaderAction` elsewhere the compiler reports (e.g. help/cheatsheet category maps).

- [ ] **Step 4: DocMeta count**

`events.rs`:

```rust
    /// Property count of a note for the status bar (async-loaded).
    PropertyCountLoaded { path: VaultPath, count: usize },
```

`doc_meta.rs`: field `property_count: Option<usize>` (init `None`); reader `pub fn properties(&self) -> Option<usize> { self.property_count }`;

```rust
    /// Re-read the open note's property count (note open, save, dialog write).
    pub fn refresh_properties(&mut self, path: &VaultPath, tx: &AppTx) {
        let vault = self.vault.clone();
        let path = path.clone();
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let count = vault.get_properties(&path).await.map(|p| p.len()).unwrap_or_default();
            tx2.send(AppEvent::PropertyCountLoaded { path, count }).ok();
        });
    }
```

In `note_opened`: `self.property_count = None;` and `self.refresh_properties(path, tx);`. In `handle`:

```rust
            AppEvent::PropertyCountLoaded { path, count } => {
                if path == *current_note {
                    self.property_count = Some(count);
                }
                None
            }
```

`editor.rs`: in `AutosaveCompleted`, after `refresh_git`: `if path == self.path { self.doc_meta.refresh_properties(&path, tx); }`. In the Task 8 `NoteReloadFromDisk` arm, add `self.doc_meta.refresh_properties(&self.path, tx);`. Footer `DocState`: `props: self.open_note().and(self.doc_meta.properties()),`.

- [ ] **Step 5: Footer segment**

`DocState` gains:

```rust
    /// Property count of the open note — renders the clickable `⊞ N props`.
    pub props: Option<usize>,
```

`FooterBar` gains `props_rect: Option<Rect>`. In `render`: add the width to `tail_width` (`" · ".width() + props_label(n).width()`), and when building segments, before the `rag` push:

```rust
        self.props_rect = None;
        if let Some(n) = doc.props {
            let label = props_label(n);
            let x_before: u16 = segments.iter().map(|s| s.content.width() as u16).sum::<u16>()
                + " · ".width() as u16;
            let x = rows[1].x + x_before;
            let w = label.width() as u16;
            if x < rows[1].right() {
                self.props_rect = Some(Rect {
                    x,
                    y: rows[1].y,
                    width: w.min(rows[1].right() - x),
                    height: 1,
                });
            }
            push(
                &mut segments,
                Span::styled(label, Style::default().fg(theme.blue.to_ratatui()).add_modifier(Modifier::UNDERLINED)),
            );
        }
```

`push` is a closure borrowing nothing from `self`, so computing `x_before` first is fine. Helpers:

```rust
fn props_label(n: usize) -> String {
    if n == 0 { "⊞ props".to_string() } else { format!("⊞ {n} props") }
}

impl FooterBar {
    /// Whether (col,row) is on the `⊞ props` segment from the last render.
    pub fn props_hit(&self, col: u16, row: u16) -> bool {
        self.props_rect.is_some_and(|r| r.contains(ratatui::layout::Position { x: col, y: row }))
    }
}
```

Place the props segment right after the dirty-state span so the path truncation leaves it visible.

- [ ] **Step 6: Status-bar click**

`editor.rs`, `EditorIntent::Mouse` arm, first lines:

```rust
            EditorIntent::Mouse => {
                if let InputEvent::Mouse(m) = event
                    && matches!(m.kind, MouseEventKind::Down(MouseButton::Left))
                    && self.footer.props_hit(m.column, m.row)
                {
                    if let Some(path) = self.open_note().cloned() {
                        tx.send(AppEvent::FileOp(FileOp::ShowProperties(path))).ok();
                    }
                    return EventState::Consumed;
                }
                // ...existing PanelSet routing unchanged
```

- [ ] **Step 7: Run, verify pass**

Run: `cargo test -p kimun-notes --lib leader && cargo test -p kimun-notes --lib doc_meta && cargo test -p kimun-notes --lib footer && cargo test -p kimun-notes --lib editor`
Expected: PASS.

- [ ] **Step 8: Report green.**

---

### Task 12: Docs, help, full gate run

**Files:**
- Modify: `docs/content/using-kimun/tui.md` (new section "Properties")
- Modify: `docs/content/using-kimun/search.md:237` (sort dialog sentence)
- Modify: `docs/content/using-kimun/keybindings.md` only if it lists leader `n` entries (grep `rename` there)

- [ ] **Step 1: tui.md section** (place after the editor/drawer sections; match the file's heading levels)

```markdown
## Properties

Open the properties dialog for the current note with `<leader> n p`, from
the command palette ("properties"), or by clicking `⊞ N props` in the status
bar. The note is saved first; every change is written to the file
immediately and the editor reloads it.

| Key | Action |
|-----|--------|
| `↑` `↓` / `j` `k` | Select a property |
| `Enter` / `e` | Edit the selected property |
| `a` | Add a property |
| `d` / `Del` | Delete (asks `y`/`n`) |
| `Tab` | Move between the list and the buttons |
| `Esc` | Close |

Everything is clickable: click a row to select it, click it again to edit,
and use the `[+ Add] [Edit] [Delete] [Close]` buttons.

In the edit form, **Key** suggests keys other notes use, **Type** is `auto`
by default (Kimün follows the type the key has elsewhere in your vault), and
**Value** takes a comma-separated list for list properties. If the value
does not fit the key's usual type, the form says so and offers
**Store as … anyway**.
```

- [ ] **Step 2: search.md** — replace the sentence at `:237` with:

```markdown
The directive combines with any filter (`#project -#draft ^title`). The TUI sort dialog (`Ctrl+R`) writes this directive into the query for you; in the query panel its **Sort by** row also offers **Property**, with a **Key** field that suggests the keys in your vault.
```

- [ ] **Step 3: Full gate set** — run every command from Global Constraints in order (touch first; fmt check last again).
Expected: all clean, `cargo test --workspace` all PASS.

- [ ] **Step 4: Manual smoke** (optional, needs a terminal): `cargo run -p kimun-notes`, open a note, `<leader> n p`, add/edit/delete with keys and mouse; Ctrl+R in the query panel → Property → pick a key.

- [ ] **Step 5: Report green** with the gate output summary; the user commits.
