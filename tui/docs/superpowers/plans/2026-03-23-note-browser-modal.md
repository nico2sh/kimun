# Note Browser Modal Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a telescope-style modal overlay to the editor screen that lets users search and open notes with a search box, filtered list, and live preview.

**Architecture:** A `NoteBrowserProvider` trait drives what appears in the list; `NoteBrowserModal` owns the search query, delegates all filtering to the provider, and uses `FileListComponent` purely for display and navigation. `SearchNotesProvider` supplies recent notes when the query is empty and uses `vault.search_notes()` when it is not.

**Tech Stack:** Rust, ratatui, tokio, kimun_core (`NoteVault`, `search_notes`, `get_all_notes`, `get_note_text`), async-trait, std::sync::mpsc

---

## Chunk 1: FileListEntry and FileListComponent Extensions

**Files:**
- Modify: `src/components/file_list.rs`

### Task 1: Add `FileListEntry::CreateNote` variant

- [ ] **Step 1: Add the variant to the enum**

  In `src/components/file_list.rs`, add to the `FileListEntry` enum after the `Attachment` variant:

  ```rust
  CreateNote {
      filename: String,
      path: VaultPath,
  },
  ```

- [ ] **Step 2: Add `path()` arm**

  In `impl FileListEntry`, the `path()` method:

  ```rust
  Self::CreateNote { path, .. } => path,
  ```

- [ ] **Step 3: Add `search_str()` arm**

  ```rust
  Self::CreateNote { filename, .. } => Some(filename.clone()),
  ```

- [ ] **Step 4: Add `sort_key()` arm**

  ```rust
  Self::CreateNote { filename, .. } => filename.to_lowercase(),
  ```

- [ ] **Step 5: Add `visual_height()` arm**

  ```rust
  Self::CreateNote { .. } => 1,
  ```

- [ ] **Step 6: Add `to_list_item()` arm**

  ```rust
  Self::CreateNote { filename, .. } => vec![Line::from(Span::styled(
      format!("+ Create: {}", filename),
      Style::default().fg(theme.accent.to_ratatui()),
  ))],
  ```

- [ ] **Step 7: Guard `CreateNote` in `push_entry`**

  The spec requires `CreateNote` entries to never be inserted via `push_entry` (they use `prepend_create_entry` instead). Extend the existing guard in `push_entry` to also reject `CreateNote`:

  ```rust
  pub fn push_entry(&mut self, entry: FileListEntry) {
      if matches!(entry, FileListEntry::Attachment { .. } | FileListEntry::CreateNote { .. }) {
          return;
      }
      // ... rest unchanged
  ```

- [ ] **Step 8: Verify the build compiles with no errors**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui && cargo build 2>&1 | head -30
  ```

  Expected: no errors (warnings about unused variant are acceptable).

### Task 2: Add `selected_entry()` to `FileListComponent`

- [ ] **Step 1: Write a failing test**

  In `src/components/file_list.rs` tests module, add:

  ```rust
  #[test]
  fn selected_entry_returns_highlighted_item() {
      let mut list = FileListComponent::new(
          crate::keys::KeyBindings::empty(),
          crate::settings::icons::Icons::new(true),
      );
      list.push_entry(make_note("a.md", "A"));
      list.push_entry(make_note("b.md", "B"));
      // Default selection is index 0
      let entry = list.selected_entry();
      assert!(entry.is_some());
      if let Some(FileListEntry::Note { filename, .. }) = entry {
          assert_eq!(filename, "a.md");
      } else {
          panic!("expected Note entry");
      }
  }

  #[test]
  fn selected_entry_returns_none_when_empty() {
      let list = FileListComponent::new(
          crate::keys::KeyBindings::empty(),
          crate::settings::icons::Icons::new(true),
      );
      assert!(list.selected_entry().is_none());
  }
  ```

- [ ] **Step 2: Run tests to verify they fail**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui && cargo test selected_entry 2>&1
  ```

  Expected: compile error — `selected_entry` does not exist yet.

- [ ] **Step 3: Implement `selected_entry()`**

  In `impl FileListComponent`, add after the `select_prev` method:

  ```rust
  pub fn selected_entry(&self) -> Option<&FileListEntry> {
      let display_idx = self.list_state.selected()?;
      let entry_idx = match &self.display_indices {
          None => display_idx,
          Some(v) => *v.get(display_idx)?,
      };
      self.entries.get(entry_idx)
  }
  ```

- [ ] **Step 4: Run tests to verify they pass**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui && cargo test selected_entry 2>&1
  ```

  Expected: 2 tests pass.

### Task 3: Add `prepend_create_entry()` to `FileListComponent`

- [ ] **Step 1: Write a failing test**

  In the tests module:

  ```rust
  #[test]
  fn prepend_create_entry_inserts_at_position_zero() {
      let mut list = FileListComponent::new(
          crate::keys::KeyBindings::empty(),
          crate::settings::icons::Icons::new(true),
      );
      list.push_entry(make_note("a.md", "A"));
      list.prepend_create_entry("new-note.md".to_string());
      assert!(matches!(
          &list.entries[0],
          FileListEntry::CreateNote { filename, .. } if filename == "new-note.md"
      ));
  }
  ```

- [ ] **Step 2: Run to verify it fails**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui && cargo test prepend_create_entry 2>&1
  ```

  Expected: compile error — method does not exist.

- [ ] **Step 3: Implement `prepend_create_entry()`**

  In `impl FileListComponent`, add after `add_up_entry`:

  ```rust
  pub fn prepend_create_entry(&mut self, filename: String) {
      let path = VaultPath::new(&filename);
      // Reset any active filter — inserting at 0 would shift all stored indices.
      self.display_indices = None;
      self.entries.insert(0, FileListEntry::CreateNote { filename, path });
      self.list_state.select(Some(0));
  }
  ```

- [ ] **Step 4: Run tests to verify they pass**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui && cargo test prepend_create_entry 2>&1
  ```

  Expected: 1 test passes.

- [ ] **Step 5: Run the full test suite to ensure nothing is broken**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui && cargo test 2>&1
  ```

  Expected: all existing tests pass.

- [ ] **Step 6: Commit**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui
  git add src/components/file_list.rs
  git commit -m "feat: extend FileListEntry and FileListComponent for note browser

  - Add CreateNote variant to FileListEntry
  - Add selected_entry() to read the highlighted item without triggering nucleo
  - Add prepend_create_entry() for FileFinderProvider extension point

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

## Chunk 2: Events, Provider Trait, and SearchNotesProvider

**Files:**
- Modify: `src/components/events.rs`
- Create: `src/components/note_browser/mod.rs`
- Create: `src/components/note_browser/search_provider.rs`
- Modify: `src/components/mod.rs`

### Task 4: Add `AppEvent::CloseNoteBrowser`

- [ ] **Step 1: Add the variant**

  In `src/components/events.rs`, add to the `AppEvent` enum after `OpenJournal`:

  ```rust
  /// Sent by NoteBrowserModal on Esc or after Enter+open.
  CloseNoteBrowser,
  ```

- [ ] **Step 2: Verify the build compiles**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui && cargo build 2>&1 | grep "^error" | head -20
  ```

  Expected: no errors (the new variant is just unhandled in some match arms — warnings only, fixed in Task 9).

### Task 5: Create `NoteBrowserProvider` trait and `NoteBrowserModal` struct

- [ ] **Step 1: Create the module file**

  Create `src/components/note_browser/mod.rs` with the full content:

  ```rust
  use std::sync::Arc;
  use std::sync::mpsc::Receiver;

  use async_trait::async_trait;
  use kimun_core::NoteVault;
  use kimun_core::nfs::VaultPath;
  use ratatui::Frame;
  use ratatui::layout::{Constraint, Direction, Layout, Rect};
  use ratatui::style::Style;
  use ratatui::text::Span;
  use ratatui::widgets::{Block, Borders, Clear, Paragraph};

  use crate::components::Component;
  use crate::components::event_state::EventState;
  use crate::components::events::{AppEvent, AppTx, InputEvent};
  use crate::components::file_list::{FileListComponent, FileListEntry};
  use crate::keys::KeyBindings;
  use crate::settings::icons::Icons;
  use crate::settings::themes::Theme;

  pub mod search_provider;

  // ---------------------------------------------------------------------------
  // NoteBrowserProvider trait
  // ---------------------------------------------------------------------------

  #[async_trait]
  pub trait NoteBrowserProvider: Send + Sync {
      /// Called on every query change. Empty string = initial/empty state (recent notes).
      async fn load(&self, query: &str) -> Vec<FileListEntry>;

      /// Whether to prepend a "Create: <query>" entry when query is non-empty.
      /// Defaults to false. Used by future FileFinderProvider.
      fn allows_create(&self) -> bool {
          false
      }
  }

  // ---------------------------------------------------------------------------
  // NoteBrowserModal
  // ---------------------------------------------------------------------------

  pub struct NoteBrowserModal {
      search_query: String,
      provider: Arc<dyn NoteBrowserProvider>,
      file_list: FileListComponent,
      preview_text: String,
      vault: Arc<NoteVault>,
      tx: AppTx,
      // List async loading
      load_task: Option<tokio::task::JoinHandle<()>>,
      load_rx: Option<Receiver<Vec<FileListEntry>>>,
      // Preview async loading
      preview_task: Option<tokio::task::JoinHandle<()>>,
      preview_rx: Option<Receiver<String>>,
  }

  impl NoteBrowserModal {
      pub fn new(
          provider: impl NoteBrowserProvider + 'static,
          vault: Arc<NoteVault>,
          key_bindings: KeyBindings,
          icons: Icons,
          tx: AppTx,
      ) -> Self {
          let file_list = FileListComponent::new(key_bindings, icons);
          let mut modal = Self {
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

      // ── Async list loading ─────────────────────────────────────────────────

      fn schedule_load(&mut self, tx: AppTx) {
          if let Some(handle) = self.load_task.take() {
              handle.abort();
          }
          let query = self.search_query.clone();
          let provider = Arc::clone(&self.provider);
          let (result_tx, result_rx) = std::sync::mpsc::channel();
          self.load_rx = Some(result_rx);

          let handle = tokio::spawn(async move {
              let entries = provider.load(&query).await;
              result_tx.send(entries).ok();
              tx.send(AppEvent::Redraw).ok();
          });
          self.load_task = Some(handle);
      }

      fn poll_load(&mut self) {
          let Some(rx) = &self.load_rx else { return };
          match rx.try_recv() {
              Ok(entries) => {
                  self.file_list.clear();
                  for entry in entries {
                      self.file_list.push_entry(entry);
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

      // ── Async preview loading ──────────────────────────────────────────────

      fn schedule_preview(&mut self, path: VaultPath) {
          if let Some(handle) = self.preview_task.take() {
              handle.abort();
          }
          let vault = Arc::clone(&self.vault);
          let tx = self.tx.clone();
          let (result_tx, result_rx) = std::sync::mpsc::channel();
          self.preview_rx = Some(result_rx);

          let handle = tokio::spawn(async move {
              let text = vault.get_note_text(&path).await.unwrap_or_default();
              result_tx.send(text).ok();
              tx.send(AppEvent::Redraw).ok();
          });
          self.preview_task = Some(handle);
      }

      fn poll_preview(&mut self) {
          let Some(rx) = &self.preview_rx else { return };
          match rx.try_recv() {
              Ok(text) => {
                  self.preview_text = text;
                  self.preview_rx = None;
                  self.preview_task = None;
              }
              Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                  self.preview_rx = None;
              }
              Err(std::sync::mpsc::TryRecvError::Empty) => {}
          }
      }

      /// Called after selection changes to kick off a preview load for the
      /// highlighted note, or clear the preview if a non-note entry is selected.
      fn refresh_preview(&mut self) {
          let maybe_path = self.file_list.selected_entry().and_then(|e| match e {
              FileListEntry::Note { path, .. } => Some(path.clone()),
              _ => None,
          });
          if let Some(path) = maybe_path {
              self.schedule_preview(path);
          } else {
              self.preview_text.clear();
              if let Some(h) = self.preview_task.take() {
                  h.abort();
              }
          }
      }
  }

  // ---------------------------------------------------------------------------
  // Component impl
  // ---------------------------------------------------------------------------

  impl Component for NoteBrowserModal {
      fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
          let InputEvent::Key(key) = event else {
              return EventState::NotConsumed;
          };
          use ratatui::crossterm::event::{KeyCode, KeyModifiers};
          match key.code {
              KeyCode::Esc => {
                  tx.send(AppEvent::CloseNoteBrowser).ok();
                  EventState::Consumed
              }
              KeyCode::Enter => {
                  if let Some(entry) = self.file_list.selected_entry() {
                      match entry {
                          FileListEntry::CreateNote { .. } => {
                              // Future: create note from query
                          }
                          _ => {
                              let path = entry.path().clone();
                              tx.send(AppEvent::OpenPath(path)).ok();
                              tx.send(AppEvent::CloseNoteBrowser).ok();
                          }
                      }
                  }
                  EventState::Consumed
              }
              KeyCode::Up => {
                  self.file_list.select_prev();
                  self.refresh_preview();
                  EventState::Consumed
              }
              KeyCode::Down => {
                  self.file_list.select_next();
                  self.refresh_preview();
                  EventState::Consumed
              }
              KeyCode::Char(c) => {
                  if key.modifiers.contains(KeyModifiers::SHIFT) {
                      self.search_query.push(c.to_ascii_uppercase());
                  } else {
                      self.search_query.push(c);
                  }
                  self.schedule_load(tx.clone());
                  EventState::Consumed
              }
              KeyCode::Backspace => {
                  self.search_query.pop();
                  self.schedule_load(tx.clone());
                  EventState::Consumed
              }
              _ => EventState::NotConsumed,
          }
      }

      fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, _focused: bool) {
          self.poll_load();
          self.poll_preview();

          let popup_rect = centered_rect(80, 75, area);

          // Clear the area behind the modal so the editor doesn't bleed through.
          f.render_widget(Clear, popup_rect);

          let outer_block = Block::default()
              .title(" Note Browser ")
              .borders(Borders::ALL)
              .border_style(theme.border_style(true))
              .style(theme.panel_style());
          let inner = outer_block.inner(popup_rect);
          f.render_widget(outer_block, popup_rect);

          let rows = Layout::default()
              .direction(Direction::Vertical)
              .constraints([
                  Constraint::Length(3),
                  Constraint::Min(0),
                  Constraint::Length(1),
              ])
              .split(inner);

          // ── Search box ────────────────────────────────────────────────────
          let search_block = Block::default()
              .title(" Search ")
              .borders(Borders::ALL)
              .border_style(theme.border_style(true))
              .style(theme.panel_style());
          let search_inner = search_block.inner(rows[0]);
          f.render_widget(search_block, rows[0]);
          f.render_widget(
              Paragraph::new(self.search_query.as_str()).style(
                  Style::default()
                      .fg(theme.fg.to_ratatui())
                      .bg(theme.bg_panel.to_ratatui()),
              ),
              search_inner,
          );
          // Cursor at end of search text
          let cursor_x = search_inner.x + self.search_query.chars().count() as u16;
          f.set_cursor_position((cursor_x, search_inner.y));

          // ── List + Preview ────────────────────────────────────────────────
          let columns = Layout::default()
              .direction(Direction::Horizontal)
              .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
              .split(rows[1]);

          self.file_list.render(f, columns[0], theme, false);

          let preview_block = Block::default()
              .title(" Preview ")
              .borders(Borders::ALL)
              .border_style(theme.border_style(false))
              .style(theme.panel_style());
          let preview_inner = preview_block.inner(columns[1]);
          f.render_widget(preview_block, columns[1]);
          f.render_widget(
              Paragraph::new(self.preview_text.as_str()).style(
                  Style::default()
                      .fg(theme.fg.to_ratatui())
                      .bg(theme.bg.to_ratatui()),
              ),
              preview_inner,
          );

          // ── Hint bar ──────────────────────────────────────────────────────
          f.render_widget(
              Paragraph::new("↑↓: navigate  |  Enter: open  |  Esc: close").style(
                  Style::default().fg(theme.fg_secondary.to_ratatui()),
              ),
              rows[2],
          );
      }

      fn hint_shortcuts(&self) -> Vec<(String, String)> {
          vec![
              ("↑↓".to_string(), "navigate".to_string()),
              ("Enter".to_string(), "open".to_string()),
              ("Esc".to_string(), "close".to_string()),
          ]
      }
  }

  // ---------------------------------------------------------------------------
  // Layout helper
  // ---------------------------------------------------------------------------

  fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
      let popup_height = area.height * percent_y / 100;
      let popup_width = area.width * percent_x / 100;
      Rect {
          x: area.x + (area.width.saturating_sub(popup_width)) / 2,
          y: area.y + (area.height.saturating_sub(popup_height)) / 2,
          width: popup_width,
          height: popup_height,
      }
  }

  // ---------------------------------------------------------------------------
  // Tests
  // ---------------------------------------------------------------------------

  #[cfg(test)]
  mod tests {
      use super::*;
      use crate::components::events::AppEvent;
      use tokio::sync::mpsc::unbounded_channel;

      fn make_tx() -> AppTx {
          let (tx, _rx) = unbounded_channel::<AppEvent>();
          tx
      }

      #[test]
      fn centered_rect_is_centered() {
          let area = Rect { x: 0, y: 0, width: 100, height: 40 };
          let r = centered_rect(80, 75, area);
          assert_eq!(r.width, 80);
          assert_eq!(r.height, 30);
          assert_eq!(r.x, 10); // (100 - 80) / 2
          assert_eq!(r.y, 5);  // (40 - 30) / 2
      }

      #[test]
      fn centered_rect_does_not_underflow() {
          // Very small area — must not panic.
          let area = Rect { x: 0, y: 0, width: 5, height: 5 };
          let _ = centered_rect(80, 75, area);
      }
  }
  ```

- [ ] **Step 2: Create `src/components/note_browser/search_provider.rs`**

  ```rust
  use std::sync::Arc;

  use async_trait::async_trait;
  use chrono::NaiveDate;
  use kimun_core::NoteVault;
  use kimun_core::nfs::NoteEntryData;
  use kimun_core::note::NoteContentData;

  use crate::components::file_list::FileListEntry;
  use super::NoteBrowserProvider;

  pub struct SearchNotesProvider {
      vault: Arc<NoteVault>,
  }

  impl SearchNotesProvider {
      pub fn new(vault: Arc<NoteVault>) -> Self {
          Self { vault }
      }

      fn into_entry(&self, entry: NoteEntryData, content: NoteContentData) -> FileListEntry {
          let filename = entry.path.get_parent_path().1;
          let title = if content.title.trim().is_empty() {
              "<no title>".to_string()
          } else {
              content.title
          };
          let journal_date = self.vault.journal_date(&entry.path).map(format_journal_date);
          FileListEntry::Note {
              path: entry.path,
              title,
              filename,
              journal_date,
          }
      }
  }

  #[async_trait]
  impl NoteBrowserProvider for SearchNotesProvider {
      async fn load(&self, query: &str) -> Vec<FileListEntry> {
          if query.is_empty() {
              let mut notes = self.vault.get_all_notes().await.unwrap_or_default();
              notes.sort_by(|(a, _), (b, _)| b.modified_secs.cmp(&a.modified_secs));
              notes.truncate(20);
              notes
                  .into_iter()
                  .map(|(entry, content)| self.into_entry(entry, content))
                  .collect()
          } else {
              self.vault
                  .search_notes(query)
                  .await
                  .unwrap_or_default()
                  .into_iter()
                  .map(|(entry, content)| self.into_entry(entry, content))
                  .collect()
          }
      }
  }

  fn format_journal_date(date: NaiveDate) -> String {
      date.format("%A, %B %-d, %Y").to_string()
  }
  ```

- [ ] **Step 3: Register the module in `src/components/mod.rs`**

  Add to the module list:

  ```rust
  pub mod note_browser;
  ```

- [ ] **Step 4: Verify the build compiles**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui && cargo build 2>&1 | grep "^error" | head -20
  ```

  Expected: no errors.

- [ ] **Step 5: Run tests**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui && cargo test note_browser 2>&1
  ```

  Expected: `centered_rect_is_centered` and `centered_rect_does_not_underflow` pass.

- [ ] **Step 6: Commit**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui
  git add src/components/note_browser/ src/components/mod.rs src/components/events.rs
  git commit -m "feat: add NoteBrowserModal, NoteBrowserProvider trait, SearchNotesProvider

  - AppEvent::CloseNoteBrowser for modal lifecycle
  - NoteBrowserProvider trait with async load() and allows_create() hook
  - NoteBrowserModal: search box, file list, live preview, async loading
  - SearchNotesProvider: recent notes (empty query) and vault search (non-empty)
  - centered_rect layout helper

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

## Chunk 3: EditorScreen Integration

**Files:**
- Modify: `src/app_screen/editor.rs`

### Task 6: Add `Focus::NoteBrowser` and `note_browser` field

- [ ] **Step 1: Add the new Focus variant**

  In `src/app_screen/editor.rs`, change the `Focus` enum from:

  ```rust
  enum Focus {
      Sidebar,
      Editor,
  }
  ```

  to:

  ```rust
  enum Focus {
      Sidebar,
      Editor,
      NoteBrowser,
  }
  ```

- [ ] **Step 2: Add the import for NoteBrowserModal and SearchNotesProvider**

  Add to the imports at the top of `editor.rs`:

  ```rust
  use crate::components::note_browser::NoteBrowserModal;
  use crate::components::note_browser::search_provider::SearchNotesProvider;
  ```

- [ ] **Step 3: Add `note_browser` field to `EditorScreen`**

  In the `EditorScreen` struct, add after `key_flash`:

  ```rust
  note_browser: Option<NoteBrowserModal>,
  ```

- [ ] **Step 4: Initialise the field to `None` in `EditorScreen::new()`**

  In the `Self { ... }` constructor block, add:

  ```rust
  note_browser: None,
  ```

- [ ] **Step 5: Verify the build compiles**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui && cargo build 2>&1 | grep "^error" | head -30
  ```

  Expected: errors about non-exhaustive match on `Focus` (in `handle_input` and `render`) — these are fixed in subsequent steps.

### Task 7: Wire `ToggleNoteBrowser` key binding

- [ ] **Step 1: Add the `ToggleNoteBrowser` arm to `handle_input`**

  In `EditorScreen::handle_input`, find the existing key binding match:

  ```rust
  match self.settings.key_bindings.get_action(&combo) {
      Some(ActionShortcuts::ToggleSidebar) => { ... }
      Some(ActionShortcuts::NewJournal) => { ... }
      _ => {}
  }
  ```

  Add before the `_ => {}` arm:

  ```rust
  Some(ActionShortcuts::ToggleNoteBrowser) => {
      if self.note_browser.is_some() {
          self.note_browser = None;
          self.focus = Focus::Editor;
      } else {
          let provider = SearchNotesProvider::new(self.vault.clone());
          self.note_browser = Some(NoteBrowserModal::new(
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

- [ ] **Step 2: Route keyboard events to the modal when focused**

  In the existing `match self.focus` at the end of `handle_input` (currently matches `Sidebar` and `Editor`), extend to:

  ```rust
  match self.focus {
      Focus::Sidebar => self.sidebar.handle_input(event, tx),
      Focus::Editor => self.editor.handle_input(event, tx),
      Focus::NoteBrowser => {
          if let Some(modal) = &mut self.note_browser {
              modal.handle_input(event, tx)
          } else {
              EventState::NotConsumed
          }
      }
  }
  ```

### Task 8: Handle `CloseNoteBrowser` in `handle_app_message`

- [ ] **Step 1: Add the arm**

  In `EditorScreen::handle_app_message`, before the `other => Some(other)` catch-all arm, add:

  ```rust
  AppEvent::CloseNoteBrowser => {
      self.note_browser = None;
      self.focus = Focus::Editor;
      None
  }
  ```

### Task 9: Update `render()` for `Focus::NoteBrowser`

- [ ] **Step 1: Fix `focus_label`**

  In `EditorScreen::render`, replace:

  ```rust
  let focus_label = if editor_focused { "EDITOR" } else { "SIDEBAR" };
  ```

  with:

  ```rust
  let focus_label = match self.focus {
      Focus::Editor => "EDITOR",
      Focus::Sidebar => "SIDEBAR",
      Focus::NoteBrowser => "NOTE BROWSER",
  };
  ```

  Also update the two boolean variables above it. They are currently:
  ```rust
  let editor_focused = matches!(self.focus, Focus::Editor);
  let sidebar_focused = matches!(self.focus, Focus::Sidebar);
  ```
  These remain unchanged — they are still used to pass focus state to the sidebar and editor renders. No change needed.

- [ ] **Step 2: Fix the hint shortcuts match**

  Replace:

  ```rust
  let hints = match self.focus {
      Focus::Editor => self.editor.hint_shortcuts(),
      Focus::Sidebar => self.sidebar.hint_shortcuts(),
  };
  ```

  with:

  ```rust
  let hints = match self.focus {
      Focus::Editor => self.editor.hint_shortcuts(),
      Focus::Sidebar => self.sidebar.hint_shortcuts(),
      Focus::NoteBrowser => self
          .note_browser
          .as_ref()
          .map(|m| m.hint_shortcuts())
          .unwrap_or_default(),
  };
  ```

- [ ] **Step 3: Render the modal overlay**

  At the end of `EditorScreen::render`, after `f.render_widget(...)` for the hints paragraph, add:

  ```rust
  // Modal overlay — rendered last so it appears on top of everything.
  if let Some(modal) = &mut self.note_browser {
      modal.render(f, f.area(), &self.theme, true);
  }
  ```

- [ ] **Step 4: Verify the build compiles with no errors**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui && cargo build 2>&1 | grep "^error" | head -30
  ```

  Expected: no errors.

- [ ] **Step 5: Run the full test suite**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui && cargo test 2>&1
  ```

  Expected: all tests pass.

- [ ] **Step 6: Smoke test the running app**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui && cargo run 2>&1 &
  ```

  Manual verification:
  - Open the editor
  - Press the key bound to `ToggleNoteBrowser` (check default keybindings — if unbound, add a temporary binding)
  - Verify the modal appears centered over the editor
  - Verify recent notes appear in the list
  - Type a query — list updates to search results
  - Use ↑/↓ to navigate — preview panel updates
  - Press Enter — selected note opens in editor, modal closes
  - Press the toggle key again — modal opens
  - Press Esc — modal closes

- [ ] **Step 7: Commit**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui
  git add src/app_screen/editor.rs
  git commit -m "feat: integrate NoteBrowserModal into EditorScreen

  - ToggleNoteBrowser action opens/closes the note picker modal
  - Focus::NoteBrowser routes key events to modal
  - CloseNoteBrowser event tears down modal and restores editor focus
  - Modal renders as overlay on top of editor+sidebar
  - Footer shows NOTE BROWSER label and modal hint shortcuts when focused

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

## Chunk 4: Default Keybinding

**Files:**
- Check: `src/settings/mod.rs` or wherever default keybindings are defined

### Task 10: Ensure `ToggleNoteBrowser` has a default keybinding

- [ ] **Step 1: Find where default keybindings are defined**

  ```bash
  grep -rn "ToggleNoteBrowser\|ToggleSidebar\|default.*binding\|keybinding" \
    /Users/nhormazabal/development/personal/kimun/tui/src/settings/ \
    --include="*.rs" | head -20
  ```

- [ ] **Step 2: Check if `ToggleNoteBrowser` already has a default binding**

  If it does: verify it's reachable from the editor screen and move on.

  If it does **not**: add a reasonable default. A common choice is `Ctrl+P` (matches VS Code / many TUI tools) or `Ctrl+F`. Add it alongside `ToggleSidebar` in the defaults map.

  Example (adapt to the actual data structure found in Step 1):
  ```rust
  ActionShortcuts::ToggleNoteBrowser => vec![KeyCombo::ctrl('p')],
  ```

- [ ] **Step 3: Build and verify**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui && cargo build 2>&1 | grep "^error"
  ```

  Expected: no errors.

- [ ] **Step 4: Commit if changes were made**

  ```bash
  cd /Users/nhormazabal/development/personal/kimun/tui
  git add src/settings/
  git commit -m "feat: add default keybinding for ToggleNoteBrowser (Ctrl+P)

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

  Skip this step if the binding already existed.
