# Query Panel + Saved Searches Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the right sidebar from a fixed backlinks panel into a generic, query-driven note list (backlinks become the default query `>{note}`), and add saved searches stored in the vault, selectable via a global modal.

**Architecture:** Backlinks become a special case of the existing query language (ADR 0003): the panel runs `vault.search_notes(query)`. A `{note}` *query variable* is resolved in the TUI to the open note's clean name before the query reaches core (core stays ignorant of "current note"). Saved searches persist in-vault under `.kimun/saved-searches.toml` (ADR 0004), owned by core; the TUI presents/creates/manages them. One new key action opens a global picker modal with numeric quick-select.

**Tech Stack:** Rust, ratatui (TUI), tokio (async), sqlx (index), serde + toml (config & saved-search file), async-trait (provider trait).

**Reference ADRs:** `adr/0001-link-query-operator.md`, `adr/0003-query-panel-replaces-backlinks.md`, `adr/0004-saved-searches-stored-in-vault.md`. **Glossary:** `CONTEXT.md` (query variable, saved search, saved searches modal, query panel).

**Execution order:** Phases are sequenced; each ends green and committable. Phase 1 (core) and Phase 2 (query-var + autocomplete) are independent of each other; Phase 3 depends on Phase 2; Phase 4 depends on 1+3.

---

## File Structure

**Create:**
- `core/src/nfs/saved_searches.rs` — `SavedSearch` model + file read/write (all fs ops in `nfs` per CLAUDE.md).
- `tui/src/components/query_vars.rs` — `{note}` template resolution (TUI-only, pure functions).
- `tui/src/components/saved_searches_modal.rs` — the global picker modal.
- `tui/src/components/dialogs/save_search_dialog.rs` — single-line "name this search" dialog.

**Modify:**
- `core/src/error.rs` — add `FSError::SerializationError`.
- `core/src/nfs/mod.rs` — `pub mod saved_searches;` + re-export.
- `core/src/lib.rs` — `NoteVault` saved-search API (`list/save/delete/rename`).
- `tui/src/components/autocomplete/trigger.rs` — `TriggerKind::LinkFilter` + detection for `>`/`->`.
- `tui/src/components/autocomplete/controller.rs` — `AutocompleteMode::SearchQuery`, LinkFilter query branch, `{note}` injection.
- `tui/src/components/backlinks_panel.rs` → rename/rewrite into `QueryPanel` (keep file, rename type).
- `tui/src/keys/action_shortcuts.rs` — rename `ToggleBacklinks`→`ToggleQueryPanel` (+ alias), add `OpenSavedSearches`, `SaveCurrentQuery`.
- `tui/src/keys/mod.rs` + `tui/src/settings/mod.rs` — default bindings for the new actions.
- `tui/src/components/events.rs` — new `AppEvent` variants.
- `tui/src/components/dialog_manager.rs` + `tui/src/components/dialogs/mod.rs` — wire `SaveSearchDialog`.
- `tui/src/app_screen/editor.rs` — store/open the modal, route the new actions, rename panel field.

---

## PHASE 1 — Core saved-search storage

### Task 1: `SerializationError` variant on `FSError`

**Files:**
- Modify: `core/src/error.rs:38-50` (the `FSError` enum)

- [ ] **Step 1: Add the variant**

In `core/src/error.rs`, inside `pub enum FSError`, add after `AlreadyExists`:

```rust
    #[error("Serialization error: {0}")]
    SerializationError(String),
```

- [ ] **Step 2: Build**

Run: `cargo build -p kimun_core`
Expected: compiles (no usages yet).

- [ ] **Step 3: Commit**

```bash
git add core/src/error.rs
git commit -m "feat(core): add FSError::SerializationError for saved-search (de)serialization"
```

### Task 2: `SavedSearch` model + file read/write in `nfs`

**Files:**
- Create: `core/src/nfs/saved_searches.rs`
- Modify: `core/src/nfs/mod.rs` (add `pub mod saved_searches;`)
- Test: inline `#[cfg(test)]` in `core/src/nfs/saved_searches.rs`

- [ ] **Step 1: Write the failing test**

Create `core/src/nfs/saved_searches.rs`:

```rust
//! Saved searches: named queries persisted in the vault under
//! `.kimun/saved-searches.toml`, so they travel with the notes (see
//! `adr/0004-saved-searches-stored-in-vault.md`). All filesystem access
//! lives here per the project rule that fs ops belong in `nfs`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::FSError;

/// A named query. `query` is stored verbatim, including any TUI query
/// variable such as `{note}`; resolution happens in the presentation layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSearch {
    pub name: String,
    pub query: String,
}

/// On-disk wrapper: TOML needs a named array-of-tables at the top level.
#[derive(Debug, Default, Serialize, Deserialize)]
struct SavedSearchFile {
    #[serde(default)]
    search: Vec<SavedSearch>,
}

fn saved_searches_path(workspace_path: &Path) -> std::path::PathBuf {
    workspace_path.join(".kimun").join("saved-searches.toml")
}

/// Read all saved searches. Returns an empty list if the file does not
/// exist yet (a fresh vault has none).
pub async fn read_saved_searches(workspace_path: &Path) -> Result<Vec<SavedSearch>, FSError> {
    let path = saved_searches_path(workspace_path);
    match tokio::fs::read_to_string(&path).await {
        Ok(body) => {
            let parsed: SavedSearchFile =
                toml::from_str(&body).map_err(|e| FSError::SerializationError(e.to_string()))?;
            Ok(parsed.search)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(FSError::ReadFileError(e)),
    }
}

/// Write the full saved-search list, creating `.kimun/` if needed.
pub async fn write_saved_searches(
    workspace_path: &Path,
    searches: &[SavedSearch],
) -> Result<(), FSError> {
    let path = saved_searches_path(workspace_path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let file = SavedSearchFile {
        search: searches.to_vec(),
    };
    let body = toml::to_string_pretty(&file).map_err(|e| FSError::SerializationError(e.to_string()))?;
    tokio::fs::write(&path, body).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_missing_file_returns_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let got = read_saved_searches(dir.path()).await.unwrap();
        assert!(got.is_empty());
    }

    #[tokio::test]
    async fn write_then_read_roundtrips() {
        let dir = tempfile::TempDir::new().unwrap();
        let searches = vec![
            SavedSearch { name: "todo".into(), query: "#todo".into() },
            SavedSearch { name: "backlinks".into(), query: ">{note}".into() },
        ];
        write_saved_searches(dir.path(), &searches).await.unwrap();
        let got = read_saved_searches(dir.path()).await.unwrap();
        assert_eq!(got, searches);
    }

    #[tokio::test]
    async fn write_creates_kimun_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        write_saved_searches(dir.path(), &[]).await.unwrap();
        assert!(dir.path().join(".kimun").join("saved-searches.toml").exists());
    }
}
```

Add to `core/src/nfs/mod.rs` near the top with the other `mod` declarations (the file already has `pub mod filename;` etc. — match that style):

```rust
pub mod saved_searches;
```

- [ ] **Step 2: Run test to verify it fails/builds**

Run: `cargo test -p kimun_core nfs::saved_searches`
Expected: tests compile and PASS (this task is self-contained; if `tempfile` is not a dev-dep of core, see Step 3).

- [ ] **Step 3: Ensure `tempfile` dev-dependency**

Run: `grep -n "tempfile" core/Cargo.toml`
If absent under `[dev-dependencies]`, add `tempfile = "3"` there, then re-run Step 2. Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add core/src/nfs/saved_searches.rs core/src/nfs/mod.rs core/Cargo.toml
git commit -m "feat(core): SavedSearch model + .kimun/saved-searches.toml read/write"
```

### Task 3: `NoteVault` saved-search API

**Files:**
- Modify: `core/src/lib.rs` (add methods on `impl NoteVault`, near `get_backlinks` at line ~609); re-export `SavedSearch`
- Test: inline in `core/src/lib.rs` tests module

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `core/src/lib.rs` (use the existing test-vault helper pattern in that module — find it with `grep -n "async fn .*temp\|VaultConfig::new\|fn test_vault" core/src/lib.rs` and mirror it):

```rust
    #[tokio::test]
    async fn saved_search_crud() {
        let (vault, _tmp) = make_test_vault().await; // mirror existing helper name
        assert!(vault.list_saved_searches().await.unwrap().is_empty());

        vault.save_search("todo", "#todo").await.unwrap();
        vault.save_search("links", ">{note}").await.unwrap();
        let all = vault.list_saved_searches().await.unwrap();
        assert_eq!(all.len(), 2);

        // Upsert by case-insensitive name: overwrites, no duplicate.
        vault.save_search("Todo", "#todo #urgent").await.unwrap();
        let all = vault.list_saved_searches().await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all.iter().find(|s| s.name.eq_ignore_ascii_case("todo")).unwrap().query, "#todo #urgent");

        vault.rename_saved_search("links", "backlinks").await.unwrap();
        assert!(vault.list_saved_searches().await.unwrap().iter().any(|s| s.name == "backlinks"));

        vault.delete_saved_search("todo").await.unwrap();
        let all = vault.list_saved_searches().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "backlinks");
    }
```

> If the existing test helper has a different name/shape, adapt the first line only; the assertions stay.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kimun_core saved_search_crud`
Expected: FAIL — `no method named list_saved_searches`.

- [ ] **Step 3: Implement the API**

In `core/src/lib.rs`, add `use crate::nfs::saved_searches::{self, SavedSearch};` to the imports, re-export with `pub use crate::nfs::saved_searches::SavedSearch;` near other `pub use`s, and add to `impl NoteVault` (after `get_backlinks`):

```rust
    /// List the vault's saved searches (see `SavedSearch`). Empty if none.
    pub async fn list_saved_searches(&self) -> Result<Vec<SavedSearch>, VaultError> {
        Ok(saved_searches::read_saved_searches(self.workspace_path()).await?)
    }

    /// Insert or replace a saved search by name (case-insensitive match,
    /// preserving the existing position on overwrite). Appends if new.
    pub async fn save_search(&self, name: &str, query: &str) -> Result<(), VaultError> {
        let mut all = saved_searches::read_saved_searches(self.workspace_path()).await?;
        let entry = SavedSearch { name: name.to_string(), query: query.to_string() };
        match all.iter_mut().find(|s| s.name.eq_ignore_ascii_case(name)) {
            Some(existing) => *existing = entry,
            None => all.push(entry),
        }
        saved_searches::write_saved_searches(self.workspace_path(), &all).await?;
        Ok(())
    }

    /// Delete a saved search by name (case-insensitive). No-op if absent.
    pub async fn delete_saved_search(&self, name: &str) -> Result<(), VaultError> {
        let mut all = saved_searches::read_saved_searches(self.workspace_path()).await?;
        all.retain(|s| !s.name.eq_ignore_ascii_case(name));
        saved_searches::write_saved_searches(self.workspace_path(), &all).await?;
        Ok(())
    }

    /// Rename a saved search, preserving its position and query. No-op if absent.
    pub async fn rename_saved_search(&self, old: &str, new: &str) -> Result<(), VaultError> {
        let mut all = saved_searches::read_saved_searches(self.workspace_path()).await?;
        if let Some(existing) = all.iter_mut().find(|s| s.name.eq_ignore_ascii_case(old)) {
            existing.name = new.to_string();
        }
        saved_searches::write_saved_searches(self.workspace_path(), &all).await?;
        Ok(())
    }
```

> `workspace_path()` already exists on `NoteVault` (returns `&Path`). Confirm with `grep -n "pub fn workspace_path" core/src/lib.rs`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kimun_core saved_search_crud`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/lib.rs
git commit -m "feat(core): NoteVault saved-search list/save/delete/rename API"
```

---

## PHASE 2 — Query variable resolution + `>` autocomplete

### Task 4: `{note}` query-variable resolver (TUI)

**Files:**
- Create: `tui/src/components/query_vars.rs`
- Modify: `tui/src/components/mod.rs` (add `pub mod query_vars;`)
- Test: inline in `query_vars.rs`

- [ ] **Step 1: Write the failing test + implementation**

Create `tui/src/components/query_vars.rs`:

```rust
//! Query variables: `{name}` placeholders the TUI resolves to runtime
//! values before a query reaches core (see `CONTEXT.md` "Query variable"
//! and `adr/0003`). Core's query language never sees these.

use kimun_core::nfs::VaultPath;

/// The current-note variable. A bare `>` typed in the query panel is sugar
/// that expands to `>{note}` (handled at the input layer, not here).
pub const VAR_NOTE: &str = "{note}";

/// True if `template` contains any query variable. The query panel uses
/// this to decide whether to re-run on note navigation.
pub fn query_has_variables(template: &str) -> bool {
    template.contains(VAR_NOTE)
}

/// Resolve all query variables in `template` against the open note,
/// producing a plain query string for `vault.search_notes`. `{note}`
/// becomes the note's clean name (matching how `>` targets are matched —
/// see ADR 0001). When no note is open, `{note}` resolves to the empty
/// string.
pub fn resolve_query(template: &str, current_note: Option<&VaultPath>) -> String {
    let note_name = current_note
        .map(|p| p.get_clean_name())
        .unwrap_or_default();
    template.replace(VAR_NOTE, &note_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_variables() {
        assert!(query_has_variables(">{note}"));
        assert!(query_has_variables("#todo >{note}"));
        assert!(!query_has_variables("#todo"));
    }

    #[test]
    fn resolves_note_variable() {
        let p = VaultPath::note_path_from("work/spec.md");
        assert_eq!(resolve_query(">{note}", Some(&p)), ">spec");
        assert_eq!(resolve_query("#todo >{note}", Some(&p)), "#todo >spec");
    }

    #[test]
    fn resolves_to_empty_without_note() {
        assert_eq!(resolve_query(">{note}", None), ">");
        assert_eq!(resolve_query("#todo", None), "#todo");
    }
}
```

Add `pub mod query_vars;` to `tui/src/components/mod.rs`.

> Verify `VaultPath::get_clean_name()` returns the bare note name (no extension); `backlinks_panel.rs:371` already uses `self.current_note.get_clean_name()` this way.

- [ ] **Step 2: Run tests**

Run: `cargo test -p kimun --lib query_vars`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tui/src/components/query_vars.rs tui/src/components/mod.rs
git commit -m "feat(tui): {note} query-variable resolution"
```

### Task 5: `LinkFilter` autocomplete trigger (`>` / `->`)

**Files:**
- Modify: `tui/src/components/autocomplete/trigger.rs:8-11` (`TriggerKind`), detection logic (~line 143-320)
- Test: inline in `trigger.rs` tests module (mirror the existing `#[test]` style at ~line 400)

- [ ] **Step 1: Write the failing test**

Add to the tests module in `trigger.rs`:

```rust
    #[test]
    fn detects_link_filter_trigger() {
        // `>` at the start of a query token opens a LinkFilter trigger.
        let t = detect_trigger(">pro", 4).expect("should detect");
        assert_eq!(t.kind, TriggerKind::LinkFilter);
        assert_eq!(t.query, "pro");
    }

    #[test]
    fn detects_excluded_link_filter_trigger() {
        let t = detect_trigger("->dra", 5).expect("should detect");
        assert_eq!(t.kind, TriggerKind::LinkFilter);
        assert_eq!(t.query, "dra");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p kimun --lib trigger::tests::detects_link_filter`
Expected: FAIL — `no variant LinkFilter`.

- [ ] **Step 3: Implement**

Add the variant to `TriggerKind` (trigger.rs:8):

```rust
pub enum TriggerKind {
    Wikilink,
    Hashtag,
    LinkFilter,
}
```

In `detect_trigger_with_oracle` (the core detection fn), add a branch that recognises a `>` token: scan back from the cursor over query-name characters to a `>` that is either at string start, preceded by whitespace, or preceded by `-` which is itself at start/after-whitespace (the `->` exclusion form). Emit:

```rust
        // Link-filter trigger: `>name` / `->name` in the search query.
        // anchor/replace_range cover the text after the `>` (the target prefix).
        return Some(TriggerContext {
            kind: TriggerKind::LinkFilter,
            query: text[target_start..cursor].to_string(),
            replace_range: target_start..cursor,
            anchor_col: target_start,
        });
```

Model the back-scan on the existing Hashtag detection in the same function (it already finds an opener `#` and computes `inner_start`). The LinkFilter opener is `>`; `target_start` is the byte index just after it. Gate it behind the same `TriggerOptions`/zone checks the Hashtag path uses so it does not fire inside code spans (the search box disables those zones anyway via `apply_exclusion_zone: false`).

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p kimun --lib trigger`
Expected: PASS (new + existing trigger tests).

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/autocomplete/trigger.rs
git commit -m "feat(tui): LinkFilter (>/->) autocomplete trigger detection"
```

### Task 6: Controller mode + LinkFilter suggestions + `{note}` injection

**Files:**
- Modify: `tui/src/components/autocomplete/controller.rs` (`AutocompleteMode` at line ~43, the mode filter at ~333, the query branch at ~436)
- Test: inline in `controller.rs` tests (or an integration test mirroring `search_box_autocomplete_accept_inserts_tag` in `note_browser/mod.rs`)

- [ ] **Step 1: Write the failing test**

Add to `controller.rs` tests (mirror existing controller test setup):

```rust
    #[tokio::test]
    async fn link_filter_mode_suggests_note_names_and_note_var() {
        // Vault with one note "projects".
        let vault = crate::test_support::temp_vault("ac_linkfilter").await;
        vault.validate_and_init().await.unwrap();
        vault
            .create_note(&kimun_core::nfs::VaultPath::note_path_from("/projects.md"), "x")
            .await
            .unwrap();
        // Query ">" with empty prefix should offer the {note} variable first.
        let suggestions = super::link_filter_suggestions(&vault, "").await;
        assert!(suggestions.iter().any(|s| s == "{note}"));
        // Prefix "pro" should surface the note name.
        let suggestions = super::link_filter_suggestions(&vault, "pro").await;
        assert!(suggestions.iter().any(|s| s == "projects"));
    }
```

> This assumes a small helper `link_filter_suggestions(vault, prefix) -> Vec<String>` extracted for testability. If the controller's query path is private/async-spawned, extract the pure suggestion-building into this helper and call it from the spawned task.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p kimun --lib link_filter_mode_suggests`
Expected: FAIL — function/mode missing.

- [ ] **Step 3: Implement**

1. Add mode variant (`controller.rs:43`):

```rust
pub enum AutocompleteMode {
    Both,
    HashtagOnly,
    /// Search-query box: hashtags (labels) + link-filter (`>`) note names.
    SearchQuery,
}
```

2. In the mode filter (~line 333), accept `LinkFilter` and `Hashtag` under `SearchQuery`, reject `Wikilink`:

```rust
            (AutocompleteMode::SearchQuery, TriggerKind::Hashtag) => true,
            (AutocompleteMode::SearchQuery, TriggerKind::LinkFilter) => true,
            (AutocompleteMode::SearchQuery, TriggerKind::Wikilink) => false,
```

(and add `(_, TriggerKind::LinkFilter) => false` for the other modes, plus arms so `Both`/`HashtagOnly` compile against the new variant.)

3. Add the suggestion helper + wire it into the query branch (near the `TriggerKind::Wikilink => suggest_notes_by_prefix` at ~436):

```rust
/// Build link-filter suggestions: the `{note}` variable when it matches the
/// prefix, followed by note names. Pure + async so it can be unit-tested.
pub(super) async fn link_filter_suggestions(vault: &NoteVault, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    // Offer {note} when the prefix is empty or a prefix of "note".
    if prefix.is_empty() || "note".starts_with(&prefix.to_lowercase()) {
        out.push("{note}".to_string());
    }
    if let Ok(found) = vault.suggest_notes_by_prefix(prefix, 20).await {
        out.extend(found.into_iter().map(|n| n.name));
    }
    out
}
```

In the spawned query task, branch on `TriggerKind::LinkFilter => link_filter_suggestions(&vault, &query).await` and feed the results into the same suggestion-list state the Wikilink/Hashtag paths use. Accepting a suggestion replaces the trigger range (existing `replace_range_bytes` machinery) — selecting `{note}` writes the literal `{note}` into the query.

> `NoteSuggestion` has a `.name` field (lib.rs:455 docs confirm the inserted target is `name`).

4. Switch the search box to the new mode: in `note_browser/mod.rs:144`, change `AutocompleteMode::HashtagOnly` → `AutocompleteMode::SearchQuery`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p kimun --lib autocomplete`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/autocomplete/controller.rs tui/src/components/note_browser/mod.rs
git commit -m "feat(tui): SearchQuery autocomplete mode — > suggests note names + {note}"
```

---

## PHASE 3 — Query panel (rewrite backlinks panel)

> The existing `BacklinksPanel` (`tui/src/components/backlinks_panel.rs`) already has the list + expand(collapsed/context/full) + scroll + sort + rendering. This phase **adds a query input line**, sources the list from `search_notes` instead of `get_backlinks`, generalises the context preview, and renames the type to `QueryPanel`. Keep the file; preserve `wrap_line`, `find_case_insensitive`, `split_paragraphs`, `ExpandState`, and the render structure — they are unchanged.

### Task 7: Generalise context extraction to a needle list

**Files:**
- Modify: `tui/src/components/backlinks_panel.rs` — `extract_context` (line 576), `highlight_link` (line 711)
- Test: inline tests already exist (lines 771-897); add needle-list cases.

- [ ] **Step 1: Write failing tests**

Add to the tests module:

```rust
    #[test]
    fn extract_context_matches_any_needle() {
        let text = "# Title\n\nIntro line.\n\nA paragraph mentioning widget here.\n";
        // term needle (not a link) should still locate the paragraph
        let result = extract_context_multi(text, &["widget".to_string()]);
        assert!(result.contains("widget"));
    }

    #[test]
    fn highlight_link_highlights_first_needle() {
        let spans = highlight_needles(
            "see widget and gadget",
            &["gadget".to_string()],
            ratatui::style::Color::Gray,
            ratatui::style::Color::Black,
            &crate::settings::themes::Theme::default(),
        );
        assert!(spans.iter().any(|s| s.content.contains("gadget")
            && s.style.add_modifier.contains(Modifier::BOLD)));
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p kimun --lib backlinks_panel::tests::extract_context_matches_any_needle`
Expected: FAIL — `extract_context_multi` not found.

- [ ] **Step 3: Implement**

Generalise the two helpers to take a needle slice, and keep the old single-target signatures as thin wrappers so existing call sites/tests stay green:

```rust
/// Find the first paragraph containing any of `needles` (case-insensitive);
/// fall back to the first non-blank line.
fn extract_context_multi(text: &str, needles: &[String]) -> String {
    let lowered: Vec<String> = needles.iter().map(|n| n.to_lowercase()).collect();
    for para in &split_paragraphs(text) {
        let lower = para.to_lowercase();
        if lowered.iter().any(|n| !n.is_empty() && lower.contains(n)) {
            return para.clone();
        }
    }
    text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").to_string()
}

/// Highlight the earliest occurrence of any needle in `line`.
fn highlight_needles(
    line: &str,
    needles: &[String],
    fg_muted: ratatui::style::Color,
    bg: ratatui::style::Color,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let normal = Style::default().fg(fg_muted).bg(bg);
    let bold = Style::default().fg(theme.accent.to_ratatui()).bg(bg).add_modifier(Modifier::BOLD);
    let mut best: Option<(usize, usize)> = None;
    for needle in needles {
        if needle.is_empty() { continue; }
        if let Some((s, e)) = find_case_insensitive(line, needle) {
            if best.is_none() || s < best.unwrap().0 { best = Some((s, e)); }
        }
    }
    let Some((start, end)) = best else { return vec![Span::styled(line.to_string(), normal)]; };
    let mut spans = Vec::new();
    if start > 0 { spans.push(Span::styled(line[..start].to_string(), normal)); }
    spans.push(Span::styled(line[start..end].to_string(), bold));
    if end < line.len() { spans.push(Span::styled(line[end..].to_string(), normal)); }
    spans
}
```

Build the needle list from a query: the resolved link target(s) (already with `[[`/`(` markers handled by reusing the existing wikilink/markdown needle builders) **plus** free-text terms. Add a helper that, given the active query string, returns the needles via `kimun_core`'s `SearchTerms` — note `SearchTerms` fields `terms`, `links` are `pub` (search_terms.rs:152). Example:

```rust
fn query_needles(query: &str) -> Vec<String> {
    let st = kimun_core::db::search_terms::SearchTerms::from_query_string(query);
    let mut needles = st.terms.clone();
    needles.extend(st.links.clone());
    needles
}
```

> Confirm `SearchTerms` is re-exported for TUI use: `grep -rn "pub use.*SearchTerms\|pub mod search_terms\|pub mod db" core/src/lib.rs`. If `db` is private, add `pub use crate::db::search_terms::SearchTerms;` to `core/src/lib.rs` (small core change; commit with this task).

- [ ] **Step 4: Run tests**

Run: `cargo test -p kimun --lib backlinks_panel`
Expected: PASS (new + existing).

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/backlinks_panel.rs core/src/lib.rs
git commit -m "feat(tui): generalise context preview to query-needle highlighting"
```

### Task 8: Add the query input line + source list from `search_notes`; rename to `QueryPanel`

**Files:**
- Modify: `tui/src/components/backlinks_panel.rs` (struct + `load` + `on_loaded` + `handle_key` + `render`)
- Modify: `tui/src/components/mod.rs`, `tui/src/components/events.rs`, `tui/src/app_screen/editor.rs` (rename references)
- Test: inline

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn query_panel_runs_query_and_lists_results() {
        let vault = crate::test_support::temp_vault("qp").await;
        vault.validate_and_init().await.unwrap();
        vault.create_note(&VaultPath::note_path_from("/a.md"), "alpha #todo").await.unwrap();
        vault.create_note(&VaultPath::note_path_from("/b.md"), "beta").await.unwrap();
        let entries = load_query(&vault, "#todo", None).await;
        assert_eq!(entries.len(), 1);
        assert!(entries[0].filename.contains("a"));
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p kimun --lib query_panel_runs_query`
Expected: FAIL — `load_query` not found.

- [ ] **Step 3: Implement**

Replace the data source. Add a standalone async helper (alongside `load_backlinks`):

```rust
/// Run `query` (already a template; caller resolves variables first) and
/// build panel entries. Mirrors `load_backlinks` but sources from search.
async fn load_query(vault: &NoteVault, query: &str, _note: Option<&VaultPath>) -> Vec<BacklinkEntry> {
    let needles = query_needles(query);
    let results = vault.search_notes(query).await.unwrap_or_default();
    let mut entries = Vec::with_capacity(results.len());
    for (entry_data, content_data) in results {
        let text = vault.get_note_text(&entry_data.path).await.unwrap_or_default();
        let context = extract_context_multi(&text, &needles);
        let (_p, filename) = entry_data.path.get_parent_path();
        entries.push(BacklinkEntry {
            path: entry_data.path,
            title: content_data.title,
            filename,
            context,
            full_text: Some(text),
        });
    }
    entries
}
```

Struct changes (rename `BacklinksPanel` → `QueryPanel`; keep `BacklinkEntry` name or rename to `QueryResultEntry`, your choice — if renaming, update `events.rs:114`):
- Add fields: `query_input: SingleLineInput`, `active_query: String` (the template, e.g. `>{note}`), `current_note: VaultPath` (already present).
- `load(note_path, tx)` becomes `set_note(note_path, tx)`: store the note; if `query_has_variables(&self.active_query)`, re-resolve and re-run; else leave results untouched.
- New `run_query(tx)`: resolve `active_query` against `current_note` via `query_vars::resolve_query`, spawn a task calling `load_query`, send a `QueryResultsLoaded` event (rename `BacklinksLoaded`).
- Default `active_query` = `">{note}"` (set in `new`), so first open shows backlinks (zero regression).
- `handle_key`: forward char/backspace/cursor keys to `query_input` (mirror `SidebarComponent::handle_input`, sidebar.rs:194-201); on change, set `active_query = query_input.value()` and `run_query`. Up/Down/Enter/sort behaviour unchanged. Wire the `SearchQuery` autocomplete controller exactly as `note_browser/mod.rs` does (snapshot host, popup steals Up/Down/Tab/Enter).
- `render`: prepend a search-box row (mirror `sidebar.rs:242-257`), then render the existing list/preview below. Title = `format!("Backlinks ({})", n)` when `active_query == ">{note}"`, else show the query/saved-search name.

Rename the type and all references:

Run: `grep -rln "BacklinksPanel\|backlinks_panel" tui/src` and update each (`editor.rs:15,64,...`, `mod.rs`). Keep the module file name `backlinks_panel.rs` **or** `git mv` it to `query_panel.rs` and fix the `mod` line — your call; if renaming the file, do it as the first sub-step so diffs stay readable.

- [ ] **Step 4: Run tests + build**

Run: `cargo test -p kimun --lib query_panel && cargo build -p kimun`
Expected: PASS + compiles.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(tui): query panel — editable query line sourced from search_notes"
```

### Task 9: Refresh-on-navigation gated by `{note}`

**Files:**
- Modify: `tui/src/app_screen/editor.rs:295-297` (the `if self.backlinks_visible { self.backlinks_panel.load(...) }` call after a note opens)
- Test: inline in `query_panel` tests

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn static_query_survives_navigation() {
        // A panel with a static query (#todo) keeps its results & query text
        // when set_note is called; a {note} query re-runs.
        let vault = crate::test_support::temp_vault("nav").await;
        let mut panel = QueryPanel::new(vault.clone(), /* key_bindings */ Default::default());
        panel.set_active_query("#todo".to_string());
        let before = panel.active_query().to_string();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.set_note(VaultPath::note_path_from("x.md"), tx);
        assert_eq!(panel.active_query(), before); // unchanged, not reset to >{note}
        assert!(!panel.did_rerun_for_test()); // static query → no re-run
    }
```

> Add small test-only accessors (`active_query`, `set_active_query`, `did_rerun_for_test`) under `#[cfg(test)]`.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p kimun --lib static_query_survives_navigation`
Expected: FAIL.

- [ ] **Step 3: Implement**

In `QueryPanel::set_note`, only re-run when `query_vars::query_has_variables(&self.active_query)`. In `editor.rs:295`, the call becomes `self.query_panel.set_note(path.clone(), tx.clone());` (gating lives inside `set_note`).

- [ ] **Step 4: Run tests**

Run: `cargo test -p kimun --lib query_panel && cargo build -p kimun`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(tui): re-run query on navigation only when it uses {note}"
```

---

## PHASE 4 — Saved Searches UX (modal, create, manage, keys)

### Task 10: Key actions — rename + two new, with config migration alias

**Files:**
- Modify: `tui/src/keys/action_shortcuts.rs` (enum + `category` + `label` + `Display` + `TryFrom`)
- Modify: `tui/src/keys/mod.rs` + `tui/src/settings/mod.rs` (default bindings)
- Test: inline in `action_shortcuts.rs`

- [ ] **Step 1: Write failing tests**

Add to `action_shortcuts.rs` tests:

```rust
    #[test]
    fn toggle_query_panel_roundtrip_and_alias() {
        assert_eq!(ActionShortcuts::ToggleQueryPanel.to_string(), "ToggleQueryPanel");
        // New canonical name parses.
        assert_eq!(ActionShortcuts::try_from("ToggleQueryPanel".to_string()), Ok(ActionShortcuts::ToggleQueryPanel));
        // Legacy name still parses to the renamed action (config migration alias).
        assert_eq!(ActionShortcuts::try_from("ToggleBacklinks".to_string()), Ok(ActionShortcuts::ToggleQueryPanel));
    }

    #[test]
    fn saved_search_actions_roundtrip() {
        assert_eq!(ActionShortcuts::try_from("OpenSavedSearches".to_string()), Ok(ActionShortcuts::OpenSavedSearches));
        assert_eq!(ActionShortcuts::try_from("SaveCurrentQuery".to_string()), Ok(ActionShortcuts::SaveCurrentQuery));
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p kimun --lib action_shortcuts::tests::toggle_query_panel`
Expected: FAIL.

- [ ] **Step 3: Implement**

In `action_shortcuts.rs`:
- Rename enum variant `ToggleBacklinks` → `ToggleQueryPanel`; add `OpenSavedSearches`, `SaveCurrentQuery`.
- `category()`: put all three under `ShortcutCategory::Navigation` (group with `ToggleSidebar`).
- `label()`: `ToggleQueryPanel => "Toggle query panel"`, `OpenSavedSearches => "Saved searches"`, `SaveCurrentQuery => "Save current query"`.
- `Display`: emit `"ToggleQueryPanel"`, `"OpenSavedSearches"`, `"SaveCurrentQuery"`.
- `TryFrom<String>`: map `"ToggleQueryPanel"` and the legacy alias `"ToggleBacklinks"` → `ActionShortcuts::ToggleQueryPanel`; add the two new names.

Default bindings: find the existing `ToggleBacklinks` default (`grep -rn "ToggleBacklinks" tui/src/settings/mod.rs tui/src/keys/mod.rs`) and rename to `ToggleQueryPanel`. Add defaults for `OpenSavedSearches` and `SaveCurrentQuery` in the same `batch_add()` chain (pick free `KeyStrike`s consistent with the keymap — e.g. a Ctrl combo not already used; verify no clash by reading the chain).

- [ ] **Step 4: Run tests**

Run: `cargo test -p kimun --lib action_shortcuts && cargo build -p kimun`
Expected: PASS + compiles (editor.rs still references `ToggleBacklinks` — fix those references now: `grep -rln "ToggleBacklinks" tui/src` → rename).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(tui): rename ToggleBacklinks->ToggleQueryPanel (+alias); add saved-search actions"
```

### Task 11: Save-search name dialog

**Files:**
- Create: `tui/src/components/dialogs/save_search_dialog.rs`
- Modify: `tui/src/components/dialogs/mod.rs` (export + `ActiveDialog` variant), `tui/src/components/dialog_manager.rs` (`open_save_search`), `tui/src/components/events.rs` (events)
- Test: inline

- [ ] **Step 1: Read the template dialog**

Run: `sed -n '1,200p' tui/src/components/dialogs/` then open the `CreateNoteDialog` source file (find with `grep -rln "struct CreateNoteDialog" tui/src`). Record its `new`, `handle_input`, `render`, and how it emits its confirm event. **The SaveSearchDialog mirrors it**: one `SingleLineInput` for the name + the read-only query shown above it.

- [ ] **Step 2: Write the failing test**

```rust
    #[test]
    fn save_search_dialog_emits_save_event_on_submit() {
        let mut d = SaveSearchDialog::new(">{note}".to_string());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        for ch in ['l','i','n','k','s'] {
            d.handle_input(&InputEvent::Key(key_char(ch)), &tx);
        }
        d.handle_input(&InputEvent::Key(key_enter()), &tx);
        let evt = rx.try_recv().unwrap();
        assert!(matches!(evt, AppEvent::SaveSearchConfirmed { name, query }
            if name == "links" && query == ">{note}"));
    }
```

> `key_char`/`key_enter` helpers: reuse from `test_support` (grep for existing key-event test helpers).

- [ ] **Step 3: Implement**

Create `SaveSearchDialog { query: String, name: SingleLineInput }`. On `Submit`: if name empty, use `self.query.clone()` as the name (per design); emit `AppEvent::SaveSearchConfirmed { name, query: self.query.clone() }` then `AppEvent::CloseDialog`. On `Cancel`: emit `CloseDialog`. `render`: a centered block — line 1 read-only `Query: <query>`, line 2 the name `SingleLineInput`. Mirror `CreateNoteDialog`'s render/border code.

Add events in `events.rs`:

```rust
    SaveSearchConfirmed { name: String, query: String },
    OpenSavedSearches,
    SavedSearchSelected { query: String, name: String },
```

Add `ActiveDialog::SaveSearch(SaveSearchDialog)` to `dialogs/mod.rs` and route it in `DialogManager::handle_input`/`render` (the manager dispatches to the active dialog generically — confirm at dialog_manager.rs:64-72). Add `DialogManager::open_save_search(&mut self, query: String, current_focus: u8)`.

- [ ] **Step 4: Run tests + build**

Run: `cargo test -p kimun --lib save_search_dialog && cargo build -p kimun`
Expected: PASS + compiles.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(tui): save-search name dialog"
```

### Task 12: Persist on confirm + wire SaveCurrentQuery action

**Files:**
- Modify: `tui/src/app_screen/editor.rs` (handle `SaveCurrentQuery` action; handle `AppEvent::SaveSearchConfirmed`)
- Test: inline editor test (mirror existing editor tests)

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn save_current_query_persists_via_core() {
        // EditorScreen with query panel active query "#todo".
        // Triggering SaveSearchConfirmed{name:"t",query:"#todo"} writes to the vault.
        // (Set up an EditorScreen with a temp vault; mirror existing editor test harness.)
        let screen = make_editor_screen("#todo").await; // helper to build with a temp vault
        screen.persist_saved_search("t", "#todo").await.unwrap();
        let all = screen.vault().list_saved_searches().await.unwrap();
        assert_eq!(all.iter().find(|s| s.name == "t").unwrap().query, "#todo");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p kimun --lib save_current_query_persists`
Expected: FAIL.

- [ ] **Step 3: Implement**

- Add `SaveCurrentQuery` arm to the editor action match (near `ToggleBacklinks`/`SearchNotes` handling, editor.rs:614-690): take the query panel's `active_query()` (or, if a Ctrl+K modal is open and focused, the modal's current query text) and call `self.dialogs.open_save_search(query, self.focus_index()); self.set_focus(Focus::Dialog);`.
- Handle `AppEvent::SaveSearchConfirmed { name, query }` where app events are routed (find the editor's app-event handler): spawn `let v = self.vault.clone(); tokio::spawn(async move { v.save_search(&name, &query).await.ok(); });`. Add a thin `persist_saved_search` method wrapping that for the test.

- [ ] **Step 4: Run tests + build**

Run: `cargo test -p kimun --lib save_current_query && cargo build -p kimun`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(tui): SaveCurrentQuery action persists via core"
```

### Task 13: Saved Searches modal (list + filter + numeric quick-select)

**Files:**
- Create: `tui/src/components/saved_searches_modal.rs`
- Modify: `tui/src/components/mod.rs`
- Test: inline

- [ ] **Step 1: Write the failing test (ranking logic)**

```rust
    #[test]
    fn filter_ranks_exact_index_first() {
        // Entries indexed 1..=3; the virtual backlinks entry is pinned at top.
        let items = vec![
            SearchItem::saved(1, "todo", "#todo"),
            SearchItem::saved(2, "backlinks", ">{note}"),
            SearchItem::saved(3, "two-things", "#a"), // name contains "two"/"2"? no digit
        ];
        // Typing "2" surfaces index-2 entry first.
        let ranked = rank_items(&items, "2");
        assert_eq!(ranked[0].name, "backlinks");
        // Typing "tod" surfaces the name match.
        let ranked = rank_items(&items, "tod");
        assert_eq!(ranked[0].name, "todo");
    }

    #[test]
    fn virtual_backlinks_entry_present_and_not_deletable() {
        let model = SavedSearchesModel::new(vec![]); // no user searches
        assert!(model.items()[0].is_virtual);
        assert_eq!(model.items()[0].query, ">{note}");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p kimun --lib saved_searches_modal`
Expected: FAIL.

- [ ] **Step 3: Implement the model + ranking (pure, tested) then the widget**

Model:

```rust
pub struct SearchItem {
    pub index: Option<u8>, // 1..=9 for the first nine, else None
    pub name: String,
    pub query: String,
    pub is_virtual: bool, // the pinned "Backlinks (current note)" entry
}
```

- `SavedSearchesModel::new(user: Vec<SavedSearch>)`: prepend a virtual item `{ index: None, name: "Backlinks (current note)", query: ">{note}".into(), is_virtual: true }`, then user searches; assign `index = Some(1..=9)` to the first nine items overall (virtual included or excluded — pick: virtual is pinned and **not** numbered, user items numbered 1..=9; document the choice in a code comment).
- `rank_items(items, filter)`: match against `format!("{} {}", index_or_blank, name)`; an exact leading-index match (`filter` parses to a `u8` equal to an item's `index`) ranks that item first; otherwise substring-on-name, stable order. Return filtered+sorted refs.

Widget (mirror `NoteBrowserModal` structure, note_browser/mod.rs): a `SingleLineInput` filter box on top, a list below, `centered_rect` popup. Keys: typing edits filter; Up/Down navigate; Enter emits `AppEvent::SavedSearchSelected { query: item.query, name: item.name }` + close; `d`/Delete on a non-virtual selected item emits a delete-confirm (or directly `vault.delete_saved_search`); `r` triggers rename (reuse the name dialog). Loading user searches: `vault.list_saved_searches()` (async, like the provider pattern).

- [ ] **Step 4: Run tests + build**

Run: `cargo test -p kimun --lib saved_searches_modal && cargo build -p kimun`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(tui): saved searches modal — filter + numeric quick-select + virtual backlinks entry"
```

### Task 14: Wire the modal into the editor + apply selection to the panel

**Files:**
- Modify: `tui/src/app_screen/editor.rs` (store `saved_searches_modal: Option<SavedSearchesModal>`; handle `OpenSavedSearches` action; route input when focused; handle `SavedSearchSelected`)
- Test: inline editor test

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn selecting_saved_search_sets_panel_query_and_opens_panel() {
        let mut screen = make_editor_screen(">{note}").await;
        screen.apply_saved_search(">{note}".to_string(), "Backlinks".to_string());
        assert!(screen.query_panel_visible());
        assert_eq!(screen.query_panel_active_query(), ">{note}");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p kimun --lib selecting_saved_search`
Expected: FAIL.

- [ ] **Step 3: Implement**

- Add field `saved_searches_modal: Option<SavedSearchesModal>` (mirror `note_browser: Option<NoteBrowserModal>`, editor.rs:62) + a `Focus::SavedSearches` variant (extend the `Focus` enum at editor.rs:42, `focus_index`/`from_index` at 426/435, and the focus label match at ~1065).
- `OpenSavedSearches` action arm: construct the modal (`SavedSearchesModal::new(self.vault.clone(), ...)`, kicks off async load), `self.set_focus(Focus::SavedSearches)`.
- Route input/render/mouse to the modal when `Some` + focused, like `note_browser` (editor.rs:731-759, render path).
- Handle `AppEvent::SavedSearchSelected { query, name }` via an `apply_saved_search(query, name)` method: set `self.backlinks_visible = true` (panel visible), `self.query_panel.set_active_query(query)`, set its title source to `name`, run the query, `self.set_focus(Focus::Backlinks)` (the query-panel focus), and close the modal.
- Add test accessors `query_panel_visible`, `query_panel_active_query`.

- [ ] **Step 4: Run tests + full build + suite**

Run: `cargo test -p kimun && cargo build`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(tui): open saved searches modal; apply selection to query panel"
```

---

## Final verification

- [ ] **Run the full workspace suite**

Run: `cargo test`
Expected: all green.

- [ ] **Clippy + fmt**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt --all -- --check`
Expected: clean.

- [ ] **Manual smoke (use the `run` skill or `cargo run -p kimun`)**

Check: open a note → panel shows backlinks (title "Backlinks"); type `#todo` in the panel → live results, navigate away → results persist; type `>` → autocomplete offers `{note}` + note names; save a query → appears in the Saved Searches modal; numeric/filter select → applies to panel; rename/delete in modal persists to `.kimun/saved-searches.toml`.

---

## Self-Review notes (spec coverage)

- Framing A (backlinks = default query `>{note}`) → Tasks 8, 9 (default `active_query`, title).
- `{note}` resolved in TUI → Task 4; refresh-gated → Task 9.
- `>`/`->` autocomplete + `{note}` suggestion, panel + Ctrl+K → Tasks 5, 6.
- Generalised context preview → Task 7.
- In-vault storage, core-owned → Tasks 1–3.
- Create from panel + Ctrl+K, blank-name→query fallback, overwrite-confirm (upsert) → Tasks 11, 12 (upsert is in core Task 3).
- Single `OpenSavedSearches` binding, no per-search shortcuts, rename + migration → Task 10.
- Modal: filter + leading-index ranking, virtual backlinks entry, select opens/focuses panel showing template → Tasks 13, 14.

**Known follow-ups (deferred, not in this plan):** default key-combo choices (left to implementer in Task 10), saved-search reorder, manage-rename UI polish, CLI/MCP exposure of saved searches.
