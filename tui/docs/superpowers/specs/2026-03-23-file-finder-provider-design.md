# File Finder Provider — Design Spec

**Date:** 2026-03-23
**Feature:** FileFinderProvider — fuzzy note finder modal (Ctrl+O)

---

## Overview

A second use case for `NoteBrowserModal`: a telescope-style fuzzy file finder that searches all notes by filename and title. Triggered by `ActionShortcuts::OpenNote` (`Ctrl+O`). Supports creating a new note from the search query.

---

## Architecture

The feature reuses the existing `NoteBrowserModal` + `NoteBrowserProvider` infrastructure. A new `FileFinderProvider` implements `NoteBrowserProvider` and adds a one-time cached vault fetch plus in-memory nucleo filtering.

### Components touched

| Component | Change |
|-----------|--------|
| `src/components/note_browser/file_finder_provider.rs` | **New file** — `FileFinderProvider` |
| `src/components/note_browser/mod.rs` | Add `pub mod file_finder_provider;`; add `title` field; wire `CreateNote` split in `poll_load`; wire `CreateNote` Enter and mouse double-click handlers |
| `src/components/file_list.rs` | Update `prepend_create_entry` signature to accept `FileListEntry`; update the one existing call-site test |
| `src/app_screen/editor.rs` | Add import for `FileFinderProvider`; wire `ActionShortcuts::OpenNote`; update `ToggleNoteBrowser` call to pass title |

---

## FileFinderProvider

**File:** `src/components/note_browser/file_finder_provider.rs`

```rust
pub struct FileFinderProvider {
    vault: Arc<NoteVault>,
    current_dir: VaultPath,
    notes_cache: Arc<tokio::sync::OnceCell<Vec<(NoteEntryData, NoteContentData)>>>,
}

impl FileFinderProvider {
    pub fn new(vault: Arc<NoteVault>, current_dir: VaultPath) -> Self {
        Self {
            vault,
            current_dir,
            notes_cache: Arc::new(tokio::sync::OnceCell::new()),
        }
    }
}
```

Declare in `note_browser/mod.rs`: `pub mod file_finder_provider;`

### Caching strategy

`tokio::sync::OnceCell` — first call to `load()` triggers `get_or_init()` which fetches `vault.get_all_notes()`. All subsequent calls (including concurrent calls from rapid keystrokes) coalesce on the same future and reuse the cached result. No `Mutex`, no explicit `LoadState` enum.

### `load("")` — empty query

1. Fetch from cache using the pattern below. The async block must capture `vault` by clone because `get_or_init` requires a `'static` future:
   ```rust
   let vault = Arc::clone(&self.vault);
   let notes = self.notes_cache.get_or_init(async move {
       vault.get_all_notes().await.unwrap_or_default()
   }).await;
   ```
2. Sort cached notes by `NoteEntryData.modified_secs` descending (most recently modified first)
3. Map each to `FileListEntry::Note { path, title, filename, journal_date }` — populate `journal_date` via `self.vault.journal_date(&entry.path).map(format_journal_date)` where `format_journal_date` is moved to `note_browser/mod.rs` as `pub(super)` (see note in `format_journal_date` section below)
4. Return `Vec<FileListEntry>` — no `CreateNote` entry for empty query

### `load(q)` — non-empty query

1. Populate cache with the same `Arc::clone` + `async move` pattern (as above)
2. Build haystack: for each `(entry, content)`, the match string is `format!("{} {}", entry.path.get_parent_path().1, content.title)`
3. Run nucleo: `Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart).match_list(candidates, &mut matcher)` — this is synchronous but operates only on in-memory data (no I/O), so no `spawn_blocking` needed
4. Sort results by nucleo score descending
5. Map to `Vec<FileListEntry::Note>` (with `journal_date` populated as above)
6. **Prepend** one `FileListEntry::CreateNote` entry at index 0 (see below)
7. Return the combined vec

### `allows_create() -> bool`

Returns `true`. This signals to callers that the provider can produce `CreateNote` entries. The modal's `poll_load` uses the structural presence of a `CreateNote` entry in the returned `Vec` to trigger `prepend_create_entry` — it does not need to call `allows_create()` at runtime. `allows_create()` remains a trait-level hint for documentation and potential future use.

### CreateNote entry

When query is non-empty:
- Resolve full path: `self.current_dir.append(VaultPath::note_path_from(query)).flatten()`
- Entry: `FileListEntry::CreateNote { filename: resolved_path.to_string(), path: resolved_path }`
- Displayed as `"+ Create: <resolved_path>"` — user sees exactly where the note will be created

---

## NoteBrowserModal changes

### Module declaration

Add to `src/components/note_browser/mod.rs`:
```rust
pub mod file_finder_provider;
```

### Constructor: add `title` parameter

```rust
pub fn new(
    title: impl Into<String>,
    provider: impl NoteBrowserProvider + 'static,
    vault: Arc<NoteVault>,
    key_bindings: KeyBindings,
    icons: Icons,
    tx: AppTx,
) -> Self
```

Store `title: String` as a field. Render in the modal's outer block: `Block::default().title(format!(" {} ", self.title))`.

### `poll_load`: split `CreateNote` entries before `push_entry`

The `push_entry` method silently drops `CreateNote` variants. The split **must happen before any call to `push_entry`** — the guard is not a safety net, it is an active trap.

```rust
fn poll_load(&mut self) {
    let Some(rx) = &self.load_rx else { return };
    match rx.try_recv() {
        Ok(entries) => {
            self.file_list.clear();
            let mut create_entry: Option<FileListEntry> = None;
            for entry in entries {
                if matches!(entry, FileListEntry::CreateNote { .. }) {
                    create_entry = Some(entry);
                } else {
                    self.file_list.push_entry(entry);
                }
            }
            if let Some(entry) = create_entry {
                self.file_list.prepend_create_entry(entry);
            }
            self.load_rx = None;
            self.load_task = None;
            self.refresh_preview();
        }
        // ... error arms unchanged
    }
}
```

### `CreateNote` Enter handler

Replace the current stub `// Future: create note from query` in the `KeyCode::Enter` match arm:

```rust
FileListEntry::CreateNote { path, .. } => {
    let path = path.clone();
    let vault = Arc::clone(&self.vault);
    let tx = tx.clone();
    tokio::spawn(async move {
        vault.load_or_create_note(&path, None).await.ok();
        tx.send(AppEvent::OpenPath(path)).ok();
        tx.send(AppEvent::CloseNoteBrowser).ok();
    });
}
```

### `CreateNote` mouse double-click handler

The current double-click path in `handle_input` has a guard that skips `CreateNote`:
```rust
if !matches!(entry, FileListEntry::CreateNote { .. }) {
    let path = entry.path().clone();
    tx.send(AppEvent::OpenPath(path)).ok();
    tx.send(AppEvent::CloseNoteBrowser).ok();
}
```

Replace this `if` block with a `match` so `CreateNote` is handled explicitly:
```rust
match entry {
    FileListEntry::CreateNote { path, .. } => {
        let path = path.clone();
        let vault = Arc::clone(&self.vault);
        let tx = tx.clone();
        tokio::spawn(async move {
            vault.load_or_create_note(&path, None).await.ok();
            tx.send(AppEvent::OpenPath(path)).ok();
            tx.send(AppEvent::CloseNoteBrowser).ok();
        });
    }
    _ => {
        let path = entry.path().clone();
        tx.send(AppEvent::OpenPath(path)).ok();
        tx.send(AppEvent::CloseNoteBrowser).ok();
    }
}
```

---

## FileListComponent changes

### `prepend_create_entry` signature update

Change from:
```rust
pub fn prepend_create_entry(&mut self, filename: String)
```
To:
```rust
pub fn prepend_create_entry(&mut self, entry: FileListEntry)
```

The old implementation constructed a `FileListEntry::CreateNote` internally from the filename string and called `VaultPath::new`. The new implementation receives the pre-constructed entry directly. **Delete the internal `VaultPath::new` construction** — the entry is already built by the provider.

The method inserts the provided entry at index 0 of `self.entries`, resets `display_indices = None` (so all entries are visible), and resets the selection to 0.

### Update the existing test

The test at `#[cfg(test)]` in `file_list.rs` calls the old signature:
```rust
list.prepend_create_entry("new-note.md".to_string());
```
Update it to pass a full `FileListEntry::CreateNote`:
```rust
list.prepend_create_entry(FileListEntry::CreateNote {
    filename: "new-note.md".to_string(),
    path: VaultPath::new("new-note.md"),
});
```

---

## EditorScreen changes

### Add import

```rust
use crate::components::note_browser::file_finder_provider::FileFinderProvider;
```

### Wire `ActionShortcuts::OpenNote`

Add a new arm to the `match self.settings.key_bindings.get_action(&combo)` block:

```rust
Some(ActionShortcuts::OpenNote) => {
    if self.note_browser.is_some() {
        self.note_browser = None;
        if matches!(self.focus, Focus::NoteBrowser) {
            self.focus = Focus::Editor;
        }
    } else {
        let current_dir = self.path.get_parent_path().0;
        let provider = FileFinderProvider::new(self.vault.clone(), current_dir);
        self.note_browser = Some(NoteBrowserModal::new(
            "Find Note",
            provider,
            self.vault.clone(),
            self.settings.key_bindings.clone(),
            self.settings.icons(),
            tx.clone(),
        ));
        self.focus = Focus::NoteBrowser;
    }
    return EventState::Consumed;
}
```

`Ctrl+O` toggles the finder modal. If any modal is already open, it closes it. If closed, opens the finder.

### Update existing `ToggleNoteBrowser` call

The existing `ToggleNoteBrowser` arm constructs `NoteBrowserModal::new` without a title. Update it to pass `"Note Browser"` as the first argument:

```rust
self.note_browser = Some(NoteBrowserModal::new(
    "Note Browser",
    provider,
    self.vault.clone(),
    self.settings.key_bindings.clone(),
    self.settings.icons(),
    tx.clone(),
));
```

---

## Data flow

```
Ctrl+O pressed
  → EditorScreen creates FileFinderProvider(vault, current_dir)
  → NoteBrowserModal::new("Find Note", provider, ...)
  → modal.schedule_load() called immediately
    → tokio::spawn: provider.load("") → get_or_init fetches all notes → sort by modified_secs
    → result_tx.send(entries)   // no CreateNote in empty-query result
    → AppEvent::Redraw

User types query
  → modal aborts previous load_task, spawns new one
  → provider.load(query) → notes already cached → nucleo filter
    → returns [CreateNote, Note, Note, ...]  // CreateNote at index 0
  → result_tx.send(entries)

poll_load receives entries
  → file_list.clear()
  → split: collect CreateNote separately, push all Note entries via push_entry
  → call file_list.prepend_create_entry(create_entry) if present

User presses Enter on CreateNote
  → tokio::spawn: vault.load_or_create_note(&path).await
  → AppEvent::OpenPath(path)
  → AppEvent::CloseNoteBrowser

User double-clicks CreateNote row
  → same tokio::spawn sequence
```

---

## `format_journal_date` — move to shared location

`format_journal_date` is currently a private function in `search_provider.rs`. Both `SearchNotesProvider` and `FileFinderProvider` need it. Move it to `note_browser/mod.rs` with `pub(super)` visibility so both sibling modules can use it:

```rust
// in note_browser/mod.rs
pub(super) fn format_journal_date(date: chrono::NaiveDate) -> String {
    date.format("%A, %B %-d, %Y").to_string()
}
```

Remove it from `search_provider.rs` and update the `use` statement to reference the parent module function:
```rust
// in search_provider.rs
use super::format_journal_date;
```

---

## Required imports for `file_finder_provider.rs`

```rust
use std::sync::Arc;
use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::nfs::{NoteEntryData, VaultPath};
use kimun_core::note::NoteContentData;
use nucleo::{Matcher, Config};
use nucleo::pattern::{Pattern, CaseMatching, Normalization};
use crate::components::file_list::FileListEntry;
use super::{NoteBrowserProvider, format_journal_date};
```

---

## Nucleo usage

```rust
use nucleo::{Matcher, Config};
use nucleo::pattern::{Pattern, CaseMatching, Normalization};

let mut matcher = Matcher::new(Config::DEFAULT);
let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);

// candidates: Vec<(Utf8Str, original_item)> or similar — see existing usage in file_list.rs
let matches: Vec<_> = pattern.match_list(candidates, &mut matcher);
// results are sorted by score descending
```

The `match_list` call must run inside `tokio::task::spawn_blocking` — this is the established pattern in `file_list.rs` (lines 398–408). Even though the data is in-memory, `spawn_blocking` keeps the async executor unblocked. Since `provider.load()` is already called from within a `tokio::spawn`, wrap the nucleo work like this:

```rust
let matched = tokio::task::spawn_blocking(move || {
    let mut matcher = Matcher::new(nucleo::Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    pattern.match_list(candidates, &mut matcher)
})
.await
.unwrap_or_default();
```

Where `candidates` is a `Vec<MatchEntry>` (with `AsRef<str>` impl returning the haystack string) and `query` is moved into the closure. See `file_list.rs:382–408` for the exact pattern to follow.

---

## Out of scope

- Cache invalidation across multiple modal opens (cache is per-`FileFinderProvider` instance; each `Ctrl+O` creates a fresh provider with a fresh cache)
- Fuzzy matching against note body content (filename + title only)
- Creating notes in arbitrary directories (path is always relative to current note's parent)
