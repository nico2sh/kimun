# Browse Screen: Create New Note — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users create a note from the browse screen by typing its name in the search box and pressing Enter on the "Create…" entry, with a persistent hint bar explaining the gesture.

**Architecture:** A separate `create_entry: Option<FileListEntry>` slot is added to `FileListComponent` — always rendered first, invisible to the filter machinery. `SidebarComponent` populates it from the search query and intercepts Enter to create the note via `vault.load_or_create_note`. `BrowseScreen` gains a one-line hint row at the bottom.

**Tech Stack:** Rust, Ratatui, `kimun_core::NoteVault::load_or_create_note`, `VaultPath::note_path_from`.

---

## File map

| File | Change |
|---|---|
| `tui/src/components/file_list.rs` | Add `create_entry` slot + update 6 methods |
| `tui/src/components/sidebar.rs` | Add `sync_create_entry`, update `handle_input` + `start_loading` |
| `tui/src/app_screen/browse.rs` | Add hint bar row to render |

---

## Task 1: `FileListComponent` — add `create_entry` slot

**Files:**
- Modify: `tui/src/components/file_list.rs`

The `create_entry` field is a separate slot that always renders at virtual display-index 0 when set. It is never stored in `entries` and never touched by the filter.

- [ ] **Step 1: Write three failing tests**

Add to the `#[cfg(test)]` block at the bottom of `tui/src/components/file_list.rs`, after the existing `make_tx` helper:

```rust
fn make_list() -> FileListComponent {
    FileListComponent::new(
        crate::keys::KeyBindings::empty(),
        crate::settings::icons::Icons::new(true),
    )
}

#[test]
fn set_create_entry_shows_at_virtual_index_zero() {
    let mut list = make_list();
    list.push_entry(make_note("a.md", "A"));
    list.push_entry(make_note("b.md", "B"));
    list.set_create_entry(Some(FileListEntry::CreateNote {
        filename: "new.md".to_string(),
        path: VaultPath::new("new.md"),
    }));
    // 2 notes + 1 virtual create entry
    assert_eq!(list.display_len(), 3);
    // Selection resets to 0, which is the CreateNote
    assert!(matches!(
        list.selected_entry(),
        Some(FileListEntry::CreateNote { filename, .. }) if filename == "new.md"
    ));
}

#[test]
fn set_create_entry_none_hides_it() {
    let mut list = make_list();
    list.push_entry(make_note("a.md", "A"));
    list.set_create_entry(Some(FileListEntry::CreateNote {
        filename: "new.md".to_string(),
        path: VaultPath::new("new.md"),
    }));
    list.set_create_entry(None);
    assert_eq!(list.display_len(), 1);
    assert!(matches!(list.selected_entry(), Some(FileListEntry::Note { .. })));
}

#[test]
fn clear_removes_create_entry() {
    let mut list = make_list();
    list.set_create_entry(Some(FileListEntry::CreateNote {
        filename: "new.md".to_string(),
        path: VaultPath::new("new.md"),
    }));
    list.clear();
    assert!(list.create_entry.is_none());
    assert_eq!(list.display_len(), 0);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p kimun-notes set_create_entry 2>&1 | tail -20
cargo test -p kimun-notes clear_removes_create_entry 2>&1 | tail -10
```

Expected: compile error — `create_entry` field and `set_create_entry` method don't exist yet, `display_len` is private.

- [ ] **Step 3: Add the `create_entry` field to `FileListComponent`**

In `tui/src/components/file_list.rs`, update the struct definition (currently around line 269):

```rust
pub struct FileListComponent {
    pub entries: Vec<FileListEntry>,
    pub loading: bool,
    display_indices: Option<Vec<usize>>,
    list_state: ListState,
    rendered_rect: Rect,
    // Search
    pub search_query: String,
    filter_rx: Option<Receiver<Vec<usize>>>,
    filter_task: Option<tokio::task::JoinHandle<()>>,
    // Sort
    pub sort_field: SortField,
    pub sort_order: SortOrder,
    // Always-visible pinned entry shown above all others (used for "Create…").
    // Not stored in `entries`; not touched by the filter.
    create_entry: Option<FileListEntry>,
    // Keybindings
    key_bindings: KeyBindings,
    // Icons resolved once at construction
    icons: Icons,
}
```

Update `new()` to initialise it:

```rust
pub fn new(key_bindings: KeyBindings, icons: Icons) -> Self {
    Self {
        entries: Vec::new(),
        loading: false,
        display_indices: None,
        list_state: ListState::default(),
        rendered_rect: Rect::default(),
        search_query: String::new(),
        filter_rx: None,
        filter_task: None,
        sort_field: SortField::Name,
        sort_order: SortOrder::Ascending,
        create_entry: None,
        key_bindings,
        icons,
    }
}
```

Add the public setter immediately after `new()`:

```rust
pub fn set_create_entry(&mut self, entry: Option<FileListEntry>) {
    self.create_entry = entry;
    self.reset_selection();
}
```

Make `display_len` public (change `fn display_len` to `pub fn display_len`) and update it to count the virtual slot:

```rust
pub fn display_len(&self) -> usize {
    let base = match &self.display_indices {
        None => self.entries.len(),
        Some(v) => v.len(),
    };
    base + usize::from(self.create_entry.is_some())
}
```

- [ ] **Step 4: Update `clear()` to wipe the create entry**

```rust
pub fn clear(&mut self) {
    if let Some(handle) = self.filter_task.take() {
        handle.abort();
    }
    self.entries.clear();
    self.display_indices = None;
    self.filter_rx = None;
    self.search_query.clear();
    self.create_entry = None;
    self.list_state.select(None);
    self.loading = false;
}
```

- [ ] **Step 5: Run tests — the three new tests should now pass**

```bash
cargo test -p kimun-notes set_create_entry 2>&1 | tail -20
cargo test -p kimun-notes clear_removes_create_entry 2>&1 | tail -10
```

Expected: all three pass.

- [ ] **Step 6: Update `selected_entry()` to resolve the virtual slot**

```rust
pub fn selected_entry(&self) -> Option<&FileListEntry> {
    let display_idx = self.list_state.selected()?;
    if self.create_entry.is_some() {
        if display_idx == 0 {
            return self.create_entry.as_ref();
        }
        let adjusted = display_idx - 1;
        let entry_idx = match &self.display_indices {
            None => adjusted,
            Some(v) => *v.get(adjusted)?,
        };
        return self.entries.get(entry_idx);
    }
    let entry_idx = match &self.display_indices {
        None => display_idx,
        Some(v) => *v.get(display_idx)?,
    };
    self.entries.get(entry_idx)
}
```

- [ ] **Step 7: Update `activate_selected()` to resolve the virtual slot**

```rust
fn activate_selected(&self, tx: &AppTx) {
    let Some(display_idx) = self.list_state.selected() else {
        return;
    };
    if self.create_entry.is_some() && display_idx == 0 {
        if let Some(entry) = &self.create_entry {
            tx.send(AppEvent::OpenPath(entry.path().clone())).ok();
        }
        return;
    }
    let adjusted = if self.create_entry.is_some() {
        display_idx - 1
    } else {
        display_idx
    };
    let entry_idx = match &self.display_indices {
        None => adjusted,
        Some(v) => v[adjusted],
    };
    tx.send(AppEvent::OpenPath(self.entries[entry_idx].path().clone()))
        .ok();
}
```

- [ ] **Step 8: Update `display_idx_at_row()` to resolve the virtual slot**

```rust
fn display_idx_at_row(&self, row: u16) -> Option<usize> {
    let offset = self.list_state.offset();
    let len = self.display_len();
    let mut y = 0u16;
    for display_idx in offset..len {
        let h = if self.create_entry.is_some() && display_idx == 0 {
            self.create_entry.as_ref().map(|e| e.visual_height()).unwrap_or(1)
        } else {
            let adjusted = if self.create_entry.is_some() {
                display_idx - 1
            } else {
                display_idx
            };
            let entry_idx = match &self.display_indices {
                None => adjusted,
                Some(v) => v[adjusted],
            };
            self.entries.get(entry_idx).map(|e| e.visual_height()).unwrap_or(1)
        };
        if row < y + h {
            return Some(display_idx);
        }
        y += h;
    }
    None
}
```

- [ ] **Step 9: Update `render()` to prepend the create entry**

Replace the `entry_iter` / `items` block (currently around lines 699–710):

```rust
let entry_iter: Box<dyn Iterator<Item = &FileListEntry>> = match &self.display_indices {
    None => Box::new(self.entries.iter()),
    Some(indices) => Box::new(indices.iter().map(|&i| &self.entries[i])),
};
let create_iter: Box<dyn Iterator<Item = &FileListEntry>> = match &self.create_entry {
    Some(e) => Box::new(std::iter::once(e)),
    None => Box::new(std::iter::empty()),
};
let items: Vec<ListItem> = create_iter
    .chain(entry_iter)
    .enumerate()
    .map(|(i, e)| {
        let bg = if i % 2 == 0 { bg_even } else { bg_odd };
        e.to_list_item(theme, &self.icons)
            .style(Style::default().bg(bg))
    })
    .collect();
```

- [ ] **Step 10: Run all tests**

```bash
cargo test 2>&1 | tail -20
```

Expected: all pass, no regressions.

- [ ] **Step 11: Commit**

```bash
git add tui/src/components/file_list.rs
git commit -m "feat(file-list): add create_entry virtual slot

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 2: `SidebarComponent` — sync create entry + intercept Enter

**Files:**
- Modify: `tui/src/components/sidebar.rs`

- [ ] **Step 1: Add missing imports**

At the top of `tui/src/components/sidebar.rs`, update the `use` lines:

```rust
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use crate::settings::themes::Theme;
use chrono::NaiveDate;
use kimun_core::SearchResult;
use kimun_core::nfs::VaultPath;
use kimun_core::{NoteVault, ResultType};
use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::file_list::{FileListComponent, FileListEntry, SortField, SortOrder};
use crate::keys::KeyBindings;
use crate::settings::icons::Icons;
use crate::settings::AppSettings;
```

- [ ] **Step 2: Add `sync_create_entry` helper method**

Add this private method to `impl SidebarComponent`, just before the `poll_loading` method:

```rust
fn sync_create_entry(&mut self) {
    if self.file_list.search_query.is_empty() {
        self.file_list.set_create_entry(None);
    } else {
        let path = self
            .current_dir
            .append(&VaultPath::note_path_from(&self.file_list.search_query))
            .flatten();
        let filename = path.get_parent_path().1;
        self.file_list
            .set_create_entry(Some(FileListEntry::CreateNote { filename, path }));
    }
}
```

- [ ] **Step 3: Call `sync_create_entry` from `start_loading`**

In `start_loading`, add a call at the end so clearing the file list also clears the create entry:

```rust
pub fn start_loading(&mut self, rx: Receiver<SearchResult>, current_dir: VaultPath) {
    self.current_dir = current_dir.clone();
    self.file_list.clear();
    self.file_list.loading = true;

    if &current_dir == self.vault.journal_path() {
        self.file_list.sort_field = self.journal_sort_field;
        self.file_list.sort_order = self.journal_sort_order;
    } else {
        self.file_list.sort_field = self.default_sort_field;
        self.file_list.sort_order = self.default_sort_order;
    }

    if !current_dir.is_root_or_empty() {
        let parent = current_dir.get_parent_path().0;
        self.file_list.add_up_entry(parent);
    }

    self.pending_rx = Some(rx);
    self.sync_create_entry(); // keep create entry consistent after directory change
}
```

- [ ] **Step 4: Update `handle_input` to intercept Enter and sync after typing**

Replace the entire `handle_input` implementation:

```rust
fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
    // Intercept Enter when the selected entry is a CreateNote.
    // The sidebar owns the vault, so it creates the note here before
    // forwarding OpenPath — mirroring the note browser modal pattern.
    if let InputEvent::Key(key) = event {
        if key.code == KeyCode::Enter {
            if let Some(FileListEntry::CreateNote { path, .. }) =
                self.file_list.selected_entry()
            {
                let path = path.clone();
                let vault = Arc::clone(&self.vault);
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = vault.load_or_create_note(&path, None).await {
                        log::warn!("create note failed for {path}: {e}");
                        return;
                    }
                    tx2.send(AppEvent::OpenPath(path)).ok();
                });
                return EventState::Consumed;
            }
        }
    }

    let result = self.file_list.handle_input(event, tx);

    // After a key that modifies the search query, keep the create entry in sync.
    if let InputEvent::Key(key) = event {
        if matches!(key.code, KeyCode::Char(_) | KeyCode::Backspace) {
            self.sync_create_entry();
        }
    }

    result
}
```

- [ ] **Step 5: Build to check for errors**

```bash
cargo build 2>&1 | grep -E 'error|warning: unused' | head -20
```

Expected: clean build (possibly one or two pre-existing unused-variable warnings, none new).

- [ ] **Step 6: Run all tests**

```bash
cargo test 2>&1 | tail -10
```

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add tui/src/components/sidebar.rs
git commit -m "feat(sidebar): show 'Create…' entry when search query is non-empty

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 3: `BrowseScreen` — hint bar

**Files:**
- Modify: `tui/src/app_screen/browse.rs`

- [ ] **Step 1: Add missing imports**

Update the import block in `tui/src/app_screen/browse.rs`:

```rust
use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::nfs::VaultPath;
use kimun_core::{NoteVault, VaultBrowseOptionsBuilder};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::widgets::{Block, Paragraph};

use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::sidebar::SidebarComponent;
use crate::settings::AppSettings;
use crate::settings::themes::Theme;
```

- [ ] **Step 2: Update `render` to add the hint bar**

Replace the `render` method:

```rust
fn render(&mut self, f: &mut Frame) {
    f.render_widget(
        Block::default().style(self.theme.base_style()),
        f.area(),
    );

    // Split into content area + one-line hint bar at the bottom.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(60),
            Constraint::Min(0),
        ])
        .split(rows[0]);

    self.sidebar.render(f, cols[1], &self.theme, true);

    f.render_widget(
        Paragraph::new(
            " Type to filter  ·  Enter to open  ·  Type + Enter to create a new note",
        )
        .style(
            Style::default()
                .fg(self.theme.fg_muted.to_ratatui())
                .bg(self.theme.bg.to_ratatui()),
        ),
        rows[1],
    );
}
```

- [ ] **Step 3: Build**

```bash
cargo build 2>&1 | grep 'error' | head -10
```

Expected: clean.

- [ ] **Step 4: Run all tests**

```bash
cargo test 2>&1 | tail -10
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add tui/src/app_screen/browse.rs
git commit -m "feat(browse): add persistent hint bar for create-note gesture

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```
