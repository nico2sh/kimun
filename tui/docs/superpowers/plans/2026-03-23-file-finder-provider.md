# File Finder Provider Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a fuzzy file-finder modal (Ctrl+O) to the Kimun TUI that searches all notes by filename/title, caches vault data on first load, and can create notes from the search query.

**Architecture:** A new `FileFinderProvider` implements the existing `NoteBrowserProvider` trait, using `tokio::sync::OnceCell` to cache all notes on first `load()` call and nucleo for in-memory fuzzy filtering on subsequent calls. The existing `NoteBrowserModal` gains a configurable title and wired `CreateNote` handling; `FileListComponent.prepend_create_entry` is updated to accept a pre-built entry rather than a filename string.

**Tech Stack:** Rust, ratatui, tokio async, nucleo fuzzy matching, kimun_core NoteVault

---

## Chunk 1: Foundation — shared `format_journal_date` + `prepend_create_entry` signature

### Task 1: Move `format_journal_date` to `note_browser/mod.rs`

**Files:**
- Modify: `src/components/note_browser/mod.rs`
- Modify: `src/components/note_browser/search_provider.rs`

**Context:** `format_journal_date` is currently private to `search_provider.rs`. `FileFinderProvider` (a sibling module) needs it too. Move it to the parent module with `pub(super)` visibility.

- [ ] **Step 1: Add `format_journal_date` to `note_browser/mod.rs`**

  At the bottom of `src/components/note_browser/mod.rs` (after the `tests` module), add:

  ```rust
  // ---------------------------------------------------------------------------
  // Shared helpers
  // ---------------------------------------------------------------------------

  pub(super) fn format_journal_date(date: chrono::NaiveDate) -> String {
      date.format("%A, %B %-d, %Y").to_string()
  }
  ```

  Also add `use chrono::NaiveDate;` to the imports at the top of `mod.rs`.

- [ ] **Step 2: Update `search_provider.rs` to use the parent module's function**

  In `src/components/note_browser/search_provider.rs`:

  1. Remove the private `fn format_journal_date` at the bottom of the file (lines 70–72)
  2. Remove `use chrono::NaiveDate;` import (it's no longer needed directly)
  3. Add `use super::format_journal_date;` — the function is now in the parent module

- [ ] **Step 3: Build to confirm it compiles**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui && cargo build 2>&1 | head -30
  ```

  Expected: clean build (no errors about `format_journal_date`)

- [ ] **Step 4: Run tests**

  ```bash
  cargo test -p kimun-tui 2>&1 | tail -20
  ```

  Expected: all tests pass

- [ ] **Step 5: Commit**

  ```bash
  git add src/components/note_browser/mod.rs src/components/note_browser/search_provider.rs
  git commit -m "refactor: move format_journal_date to note_browser/mod.rs as pub(super)"
  ```

---

### Task 2: Update `prepend_create_entry` to accept a `FileListEntry`

**Files:**
- Modify: `src/components/file_list.rs:322-328` (method body)
- Modify: `src/components/file_list.rs:862-873` (test)

**Context:** The current signature `prepend_create_entry(&mut self, filename: String)` constructs a `CreateNote` entry internally. The new signature accepts a pre-built `FileListEntry` so providers can pass the full resolved path.

- [ ] **Step 1: Update the test first (TDD — expect compile failure)**

  In `src/components/file_list.rs`, find the test `prepend_create_entry_inserts_at_position_zero` (around line 861) and update the call:

  ```rust
  // BEFORE:
  list.prepend_create_entry("new-note.md".to_string());

  // AFTER:
  list.prepend_create_entry(FileListEntry::CreateNote {
      filename: "new-note.md".to_string(),
      path: VaultPath::new("new-note.md"),
  });
  ```

- [ ] **Step 2: Verify the test fails to compile**

  ```bash
  cargo test -p kimun-tui prepend_create_entry 2>&1 | head -20
  ```

  Expected: compile error — wrong argument type

- [ ] **Step 3: Update `prepend_create_entry` method signature and body**

  In `src/components/file_list.rs`, replace the method (lines 322–328):

  ```rust
  // BEFORE:
  pub fn prepend_create_entry(&mut self, filename: String) {
      let path = VaultPath::new(&filename);
      // Reset any active filter — inserting at 0 would shift all stored indices.
      self.display_indices = None;
      self.entries.insert(0, FileListEntry::CreateNote { filename, path });
      self.list_state.select(Some(0));
  }

  // AFTER:
  pub fn prepend_create_entry(&mut self, entry: FileListEntry) {
      // Reset any active filter — inserting at 0 would shift all stored indices.
      self.display_indices = None;
      self.entries.insert(0, entry);
      self.list_state.select(Some(0));
  }
  ```

- [ ] **Step 4: Run tests**

  ```bash
  cargo test -p kimun-tui 2>&1 | tail -20
  ```

  Expected: all tests pass

- [ ] **Step 5: Commit**

  ```bash
  git add src/components/file_list.rs
  git commit -m "refactor: prepend_create_entry accepts FileListEntry instead of filename string"
  ```

---

## Chunk 2: NoteBrowserModal — title field, poll_load wiring, CreateNote handlers

### Task 3: Add `title` field and update constructor

**Files:**
- Modify: `src/components/note_browser/mod.rs`

**Context:** `NoteBrowserModal` needs a configurable title (e.g., "Note Browser" vs "Find Note") displayed in the modal's outer border. Add a `title: String` field and update the `new()` constructor.

- [ ] **Step 1: Add `title` field to `NoteBrowserModal` struct**

  In `src/components/note_browser/mod.rs`, find the `NoteBrowserModal` struct (around line 42) and add `title: String` as the first field:

  ```rust
  pub struct NoteBrowserModal {
      title: String,          // <-- add this
      search_query: String,
      provider: Arc<dyn NoteBrowserProvider>,
      // ... rest unchanged
  }
  ```

- [ ] **Step 2: Update `new()` constructor signature and initialization**

  Change the `new()` signature (line 58) to accept `title` as the first parameter:

  ```rust
  pub fn new(
      title: impl Into<String>,      // <-- add as first param
      provider: impl NoteBrowserProvider + 'static,
      vault: Arc<NoteVault>,
      key_bindings: KeyBindings,
      icons: Icons,
      tx: AppTx,
  ) -> Self {
      let file_list = FileListComponent::new(key_bindings, icons);
      let mut modal = Self {
          title: title.into(),        // <-- add to struct init
          search_query: String::new(),
          provider: Arc::new(provider),
          file_list,
          preview_text: String::new(),
          vault,
          tx: tx.clone(),
          load_task: None,
          load_rx: None,
          preview_task: None,
          preview_rx: None,
      };
      modal.schedule_load(tx);
      modal
  }
  ```

- [ ] **Step 3: Use `title` in render — update the outer block title**

  In the `render()` method (around line 283), find:
  ```rust
  let outer_block = Block::default()
      .title(" Note Browser ")
  ```
  Replace with:
  ```rust
  let outer_block = Block::default()
      .title(format!(" {} ", self.title))
  ```

- [ ] **Step 4: Fix the only existing call-site in `editor.rs` (atomically with the signature change)**

  The constructor signature now requires `title` as the first argument. The existing call in
  `src/app_screen/editor.rs` (inside the `ToggleNoteBrowser` arm, around line 227) must be
  updated before we commit, or the crate will not compile.

  Find:
  ```rust
  self.note_browser = Some(NoteBrowserModal::new(
      provider,
      self.vault.clone(),
      self.settings.key_bindings.clone(),
      self.settings.icons(),
      tx.clone(),
  ));
  ```
  Replace with:
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

- [ ] **Step 5: Build to confirm clean compile**

  ```bash
  cargo build 2>&1 | grep "error\[" | head -10
  ```

  Expected: clean build — no errors

- [ ] **Step 6: Run tests**

  ```bash
  cargo test -p kimun-tui 2>&1 | tail -20
  ```

  Expected: all tests pass

- [ ] **Step 7: Commit**

  ```bash
  git add src/components/note_browser/mod.rs src/app_screen/editor.rs
  git commit -m "feat: add title field to NoteBrowserModal constructor"
  ```

---

### Task 4: Update `poll_load` to split `CreateNote` entries

**Files:**
- Modify: `src/components/note_browser/mod.rs:101-118` (poll_load)

**Context:** `push_entry` silently drops `CreateNote` variants. `poll_load` must split entries before calling `push_entry` — extract any `CreateNote` entry and route it to `prepend_create_entry` instead.

- [ ] **Step 1: Replace `poll_load` body**

  In `src/components/note_browser/mod.rs`, replace the `poll_load` method (lines 101–118) with:

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
          Err(std::sync::mpsc::TryRecvError::Disconnected) => {
              self.load_rx = None;
          }
          Err(std::sync::mpsc::TryRecvError::Empty) => {}
      }
  }
  ```

  Add `use crate::components::file_list::FileListEntry;` to the imports at the top of `mod.rs` if not already present.

- [ ] **Step 2: Build**

  ```bash
  cargo build 2>&1 | grep "error\[" | head -10
  ```

  Expected: clean build — no errors

- [ ] **Step 3: Run tests**

  ```bash
  cargo test -p kimun-tui 2>&1 | tail -20
  ```

  Expected: all tests pass

- [ ] **Step 4: Commit**

  ```bash
  git add src/components/note_browser/mod.rs
  git commit -m "feat: poll_load splits CreateNote entries to prepend_create_entry"
  ```

---

### Task 5: Wire `CreateNote` Enter and mouse double-click handlers

**Files:**
- Modify: `src/components/note_browser/mod.rs` (handle_input)

**Context:** The Enter handler for `CreateNote` currently has a `// Future: create note from query` stub. The mouse double-click handler skips `CreateNote` entries. Both need to spawn an async task that calls `vault.load_or_create_note` then sends `OpenPath` + `CloseNoteBrowser`.

- [ ] **Step 1: Replace the `CreateNote` Enter stub**

  In `handle_input`, find the `KeyCode::Enter` arm. The inner `match entry` currently has:
  ```rust
  FileListEntry::CreateNote { .. } => {
      // Future: create note from query
  }
  ```
  Replace with:
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

- [ ] **Step 2: Replace the mouse double-click guard with a full match**

  In `handle_input`, find the `MouseEventKind::Down(_)` arm. The double-click path currently has:
  ```rust
  if !matches!(entry, FileListEntry::CreateNote { .. }) {
      let path = entry.path().clone();
      tx.send(AppEvent::OpenPath(path)).ok();
      tx.send(AppEvent::CloseNoteBrowser).ok();
  }
  ```
  Replace with:
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

- [ ] **Step 3: Build**

  ```bash
  cargo build 2>&1 | grep "error\[" | head -10
  ```

  Expected: clean build — no errors

- [ ] **Step 4: Run tests**

  ```bash
  cargo test -p kimun-tui 2>&1 | tail -20
  ```

  Expected: all tests pass

- [ ] **Step 5: Commit**

  ```bash
  git add src/components/note_browser/mod.rs
  git commit -m "feat: wire CreateNote Enter and mouse double-click to create note"
  ```

---

## Chunk 3: FileFinderProvider + EditorScreen wiring

### Task 6: Create `FileFinderProvider`

**Files:**
- Create: `src/components/note_browser/file_finder_provider.rs`
- Modify: `src/components/note_browser/mod.rs` (add `pub mod file_finder_provider;`)

**Context:** A new provider that loads all vault notes once via `OnceCell`, then runs nucleo fuzzy filtering in-memory for each query. Returns `CreateNote` as the first entry when query is non-empty.

- [ ] **Step 1: Add module declaration to `note_browser/mod.rs`**

  Add after `pub mod search_provider;` (line 20):
  ```rust
  pub mod file_finder_provider;
  ```

- [ ] **Step 2: Create `file_finder_provider.rs` with struct and constructor**

  Create `src/components/note_browser/file_finder_provider.rs`:

  ```rust
  use std::sync::Arc;

  use async_trait::async_trait;
  use kimun_core::NoteVault;
  use kimun_core::nfs::{NoteEntryData, VaultPath};
  use kimun_core::note::NoteContentData;
  use nucleo::Matcher;
  use nucleo::pattern::{CaseMatching, Normalization, Pattern};

  use crate::components::file_list::FileListEntry;
  use super::{NoteBrowserProvider, format_journal_date};

  // ---------------------------------------------------------------------------
  // MatchEntry — adapts (index, haystack_str) for nucleo match_list
  // ---------------------------------------------------------------------------

  #[derive(Clone)]
  struct MatchEntry {
      idx: usize,
      text: String,
  }

  impl AsRef<str> for MatchEntry {
      fn as_ref(&self) -> &str {
          &self.text
      }
  }

  // ---------------------------------------------------------------------------
  // FileFinderProvider
  // ---------------------------------------------------------------------------

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

      fn into_entry(&self, entry: &NoteEntryData, content: &NoteContentData) -> FileListEntry {
          let filename = entry.path.get_parent_path().1;
          let title = if content.title.trim().is_empty() {
              "<no title>".to_string()
          } else {
              content.title.clone()
          };
          let journal_date = self.vault.journal_date(&entry.path).map(format_journal_date);
          FileListEntry::Note {
              path: entry.path.clone(),
              title,
              filename,
              journal_date,
          }
      }
  }
  ```

- [ ] **Step 3: Implement `NoteBrowserProvider` — empty-query path**

  Append to `file_finder_provider.rs`:

  ```rust
  #[async_trait]
  impl NoteBrowserProvider for FileFinderProvider {
      async fn load(&self, query: &str) -> Vec<FileListEntry> {
          let vault = Arc::clone(&self.vault);
          let notes = self
              .notes_cache
              .get_or_init(async move {
                  vault.get_all_notes().await.unwrap_or_default()
              })
              .await;

          if query.is_empty() {
              let mut sorted = notes.clone();
              sorted.sort_by(|(a, _), (b, _)| b.modified_secs.cmp(&a.modified_secs));
              return sorted
                  .iter()
                  .map(|(entry, content)| self.into_entry(entry, content))
                  .collect();
          }

          // Non-empty query: nucleo fuzzy filter
          let candidates: Vec<MatchEntry> = notes
              .iter()
              .enumerate()
              .map(|(i, (entry, content))| {
                  let filename = entry.path.get_parent_path().1;
                  let text = format!("{} {}", filename, content.title);
                  MatchEntry { idx: i, text }
              })
              .collect();

          let query_str = query.to_string();
          let matched = tokio::task::spawn_blocking(move || {
              let mut matcher = Matcher::new(nucleo::Config::DEFAULT);
              let pattern = Pattern::parse(&query_str, CaseMatching::Ignore, Normalization::Smart);
              pattern.match_list(candidates, &mut matcher)
          })
          .await
          .unwrap_or_default();

          let mut result: Vec<FileListEntry> = matched
              .into_iter()
              .map(|(e, _score)| self.into_entry(&notes[e.idx].0, &notes[e.idx].1))
              .collect();

          // Prepend CreateNote entry so the user can create a note with this query as the path.
          let resolved = self
              .current_dir
              .append(VaultPath::note_path_from(query))
              .flatten();
          result.insert(
              0,
              FileListEntry::CreateNote {
                  filename: resolved.to_string(),
                  path: resolved,
              },
          );

          result
      }

      fn allows_create(&self) -> bool {
          true
      }
  }
  ```

- [ ] **Step 4: Build**

  ```bash
  cargo build 2>&1 | grep "error\[" | head -20
  ```

  Expected: clean build — no errors (`editor.rs` was already fixed in Task 3)

- [ ] **Step 5: Commit**

  ```bash
  git add src/components/note_browser/file_finder_provider.rs src/components/note_browser/mod.rs
  git commit -m "feat: add FileFinderProvider with OnceCell cache and nucleo fuzzy filter"
  ```

---

### Task 7: Wire `editor.rs` — `OpenNote` action

**Files:**
- Modify: `src/app_screen/editor.rs`

**Context:** Add `FileFinderProvider` import and wire `ActionShortcuts::OpenNote` to open the Find Note modal. The `ToggleNoteBrowser` title fix was already done in Task 3.

- [ ] **Step 1: Add `FileFinderProvider` import**

  In `src/app_screen/editor.rs`, find the existing imports for note browser (around line 16):
  ```rust
  use crate::components::note_browser::NoteBrowserModal;
  use crate::components::note_browser::search_provider::SearchNotesProvider;
  ```
  Add:
  ```rust
  use crate::components::note_browser::file_finder_provider::FileFinderProvider;
  ```

- [ ] **Step 2: Wire `ActionShortcuts::OpenNote`**

  In `handle_input`, add a new arm to the `match self.settings.key_bindings.get_action(&combo)` block, just before `_ => {}`:

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

- [ ] **Step 3: Build**

  ```bash
  cargo build 2>&1 | grep "error\[" | head -10
  ```

  Expected: clean build

- [ ] **Step 4: Run all tests**

  ```bash
  cargo test -p kimun-tui 2>&1 | tail -20
  ```

  Expected: all tests pass

- [ ] **Step 5: Commit**

  ```bash
  git add src/app_screen/editor.rs
  git commit -m "feat: wire OpenNote action to FileFinderProvider modal (Ctrl+O)"
  ```

---

## Chunk 4: Integration smoke test

### Task 8: Manual smoke test checklist

**Files:** none — verification only

This task has no automated tests because the feature requires a running TUI. Verify these manually after running `cargo run` (or however the TUI is launched):

- [ ] **Step 1: Build release**

  ```bash
  cargo build 2>&1 | tail -5
  ```

  Expected: clean build, no warnings about unused imports or dead code from our changes

- [ ] **Step 2: Smoke test — Note Browser (Ctrl+F)**

  - Press `Ctrl+F` → modal opens titled `" Note Browser "`
  - Press `Ctrl+F` again → modal closes
  - Type in search → list filters
  - Press `Esc` → modal closes

- [ ] **Step 3: Smoke test — Find Note empty query (Ctrl+O)**

  - Press `Ctrl+O` → modal opens titled `" Find Note "`
  - List shows notes sorted by most-recently-modified first
  - No `+ Create:` entry when query is empty
  - Arrow keys navigate; Enter opens the selected note
  - Mouse scroll and click work as expected

- [ ] **Step 4: Smoke test — Find Note with query (Ctrl+O + type)**

  - Press `Ctrl+O`, then type a partial filename or title
  - First entry is `+ Create: /path/to/resolved-note.md` showing the full resolved path
  - Remaining entries are fuzzy-matched notes sorted by score
  - Pressing `Enter` on the `+ Create:` entry creates and opens the note

- [ ] **Step 5: Smoke test — relative path resolution**

  - With a note open at `/dir/subdir/note.md`, press `Ctrl+O`
  - Type `../new-note` → Create entry shows `/dir/new-note.md`
  - Press Enter → note is created at `/dir/new-note.md` and opened

- [ ] **Step 6: Final commit if all smoke tests pass**

  ```bash
  git log --oneline -8
  ```

  Review the commit history looks clean, then the branch is ready for review/merge.
