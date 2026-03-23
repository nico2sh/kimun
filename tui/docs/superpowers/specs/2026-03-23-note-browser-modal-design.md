# Note Browser Modal — Design Spec

**Date:** 2026-03-23
**Status:** Approved
**Feature:** Telescope-style note picker modal for the TUI editor screen

---

## Overview

Add a keyboard-triggered modal overlay to the editor screen that lets the user search and open notes without leaving the editor. The modal displays a search box, a filtered note list, and a live preview of the highlighted note. The design is built around an extensible `NoteBrowserProvider` trait so future browsers (e.g. a fuzzy file-finder) can reuse the same modal shell with different data sources.

---

## Layout

The modal renders as a centered overlay at 80% terminal width × 75% terminal height, drawn on top of the editor screen. Internal layout (Option A):

```
┌─ Note Browser ──────────────────────────────────────────────────┐
│ ┌─ Search ─────────────────────────────────────────────────────┐│
│ │ > query_                                                     ││
│ └──────────────────────────────────────────────────────────────┘│
│ ┌─ Notes (50%) ────────────────┐ ┌─ Preview (50%) ─────────────┐│
│ │  ...file list...             │ │  ...raw note text...        ││
│ └──────────────────────────────┘ └─────────────────────────────┘│
│  ↑↓: navigate  Enter: open  Esc: close                           │
└─────────────────────────────────────────────────────────────────┘
```

- **Search box:** `Constraint::Length(3)` — full width, cursor shown when modal is focused
- **List + preview:** `Constraint::Min(0)` — split 50/50 horizontally
- **Hint bar:** `Constraint::Length(1)` — inside the outer block at the bottom
- Preview shows raw note text (same as the editor)

---

## Component Structure

```
EditorScreen
├── focus: Focus { Sidebar | Editor | NoteBrowser }   ← new variant
├── note_browser: Option<NoteBrowserModal>             ← new field
├── sidebar: SidebarComponent
└── editor: TextEditorComponent

NoteBrowserModal
├── search_query: String
├── provider: Box<dyn NoteBrowserProvider>
├── file_list: FileListComponent      (display + navigation only — see Key Event Handling)
├── preview_text: String
├── load_task: Option<JoinHandle<()>>
├── load_rx: Option<Receiver<Vec<FileListEntry>>>
├── preview_task: Option<JoinHandle<()>>
└── preview_rx: Option<Receiver<String>>
```

---

## Provider Trait

```rust
#[async_trait]
pub trait NoteBrowserProvider: Send + Sync {
    /// Called on every query change. Empty string = initial/empty state.
    async fn load(&self, query: &str) -> Vec<FileListEntry>;

    /// Whether to prepend a "Create: <query>" entry when query is non-empty.
    /// Defaults to false. Used by future FileFinderProvider.
    fn allows_create(&self) -> bool { false }
}
```

### SearchNotesProvider (implemented in this spec)

| State | Behavior |
|-------|----------|
| `query = ""` | `vault.get_all_notes()` → sort by `modified_secs` descending in the provider → take top 20 |
| `query = non-empty` | `vault.search_notes(query)` — uses vault's full search syntax |

The sort and truncation for the empty-query case happen inside `SearchNotesProvider::load`, not in SQL. `get_all_notes()` returns unsorted rows; the provider sorts the `Vec<(NoteEntryData, NoteContentData)>` by `entry.modified_secs` descending and takes the first 20 before converting to `FileListEntry`.

Converts `(NoteEntryData, NoteContentData)` tuples into `FileListEntry::Note` values directly (no `SearchResult` wrapper needed). Respects `vault.journal_date()` to populate `journal_date` on journal entries.

### FileFinderProvider (future — not in scope for this spec)

- `load("")` → all notes via `vault.get_all_notes()`, sorted by `modified_secs`
- `load(q)` → run nucleo fuzzy match internally, return filtered `Vec<FileListEntry>`
- `allows_create() = true` → prepends `FileListEntry::CreateNote { filename: query }` when `query` is non-empty

---

## FileListEntry Extension

Add one new variant to `FileListEntry` now, so `FileFinderProvider` can use it later without touching the enum again:

```rust
pub enum FileListEntry {
    // existing variants...
    CreateNote { filename: String, path: VaultPath },
}
```

- `to_list_item`: renders as `"+ Create: <filename>"` in accent color
- `path()`: returns the stored `VaultPath` (a sentinel path constructed from the filename)
- `search_str()`: returns `Some(filename.clone())`

`SearchNotesProvider` never returns this variant. The modal identifies `CreateNote` entries by pattern-matching the `FileListEntry` variant directly before taking any action — it never calls `activate_selected()` on `FileListComponent` (see Key Event Handling below).

`CreateNote` entries are **never** inserted via `push_entry()`. When `FileFinderProvider` prepends a create entry, it calls a new `FileListComponent::prepend_create_entry(filename: String)` method (mirrors the existing `add_up_entry` pattern) that does `self.entries.insert(0, FileListEntry::CreateNote { ... })` directly. This ensures `push_entry`'s drop-filter logic for `Attachment` entries is never involved.

---

## Key Event Handling

The modal intercepts **all** key events itself. It **never** calls `FileListComponent::handle_input()`, because `FileListComponent`'s input handler unconditionally updates its own `search_query` and triggers its internal nucleo filter — both of which must be bypassed here. The modal calls `file_list.select_prev()` / `file_list.select_next()` directly for navigation, and reads the selected entry via `file_list.selected_entry()` (a new pub method to add) for activation.

| Key | Action |
|-----|--------|
| `Char(c)` / `Backspace` | Update `search_query`, abort previous `load_task`, spawn new `provider.load(query)` |
| `↑` / `↓` | `file_list.select_prev()` / `select_next()`, abort previous `preview_task`, spawn preview load |
| `Enter` | Read selected entry: if `CreateNote` → (future) create note; else → `tx.send(AppEvent::OpenPath(path))` + `tx.send(AppEvent::CloseNoteBrowser)` |
| `Esc` | `tx.send(AppEvent::CloseNoteBrowser)` |

**`selected_entry()` helper:** Add `pub fn selected_entry(&self) -> Option<&FileListEntry>` to `FileListComponent`. It reads `self.list_state.selected()`, resolves through `self.display_indices` if set, and returns `Some(&self.entries[idx])`.

---

## Async Loading Pattern

**List loading** (same cancel-and-respawn pattern as `FileListComponent::schedule_filter`):
1. Abort `load_task` if present
2. Spawn tokio task: call `provider.load(query).await`, send result + `AppEvent::Redraw` through channels. The `Redraw` event is sent from within the spawned task (same as `schedule_filter` lines 391–393 in `file_list.rs`) — not from `render()` or `poll_load()`.
3. `poll_load()` called in `render()` — drains channel, calls `file_list.clear()`, then pushes new entries with `file_list.push_entry()`, which auto-selects index 0

**Preview loading:**
1. On selection change after `select_prev`/`select_next`, abort `preview_task` if present
2. Spawn tokio task: call `vault.get_note_text(path).await`, send result + `AppEvent::Redraw` from within the task
3. `poll_preview()` called in `render()` — updates `self.preview_text`

---

## EditorScreen Integration

### Trigger action

Use `ActionShortcuts::ToggleNoteBrowser` to open/close the modal. This action is already defined in `action_shortcuts.rs` and is semantically correct for this feature. `ActionShortcuts::SearchNotes` is a distinct action (reserved for future in-sidebar or RAG search) and must not be used here.

### New items in `editor.rs`

```rust
enum Focus {
    Sidebar,
    Editor,
    NoteBrowser,   // new
}

pub struct EditorScreen {
    // existing fields...
    note_browser: Option<NoteBrowserModal>,   // new
}
```

### Opening the modal

Wire `ActionShortcuts::ToggleNoteBrowser` in `EditorScreen::handle_input`:

```rust
Some(ActionShortcuts::ToggleNoteBrowser) => {
    if self.note_browser.is_some() {
        self.note_browser = None;
        self.focus = Focus::Editor;
    } else {
        let provider = Box::new(SearchNotesProvider::new(self.vault.clone()));
        self.note_browser = Some(NoteBrowserModal::new(
            provider,
            self.vault.clone(),
            self.settings.key_bindings.clone(),
            self.settings.icons(),
            tx.clone(),   // needed to trigger initial load
        ));
        self.focus = Focus::NoteBrowser;
    }
    EventState::Consumed
}
```

The modal's `new()` immediately schedules an initial `provider.load("")` call so recent notes are loaded before the first render.

### Input routing

- `Focus::NoteBrowser` → all keyboard input goes to `NoteBrowserModal::handle_input`
- Mouse events → check modal bounds first (same pattern as sidebar vs editor)

### Rendering

`centered_rect` is a private free function defined inside `src/components/note_browser/mod.rs`. It is called from `NoteBrowserModal::render` (not from `EditorScreen::render`). `EditorScreen` simply calls:

```rust
if let Some(modal) = &mut self.note_browser {
    modal.render(f, f.area(), theme, true);
}
```

`NoteBrowserModal::render` computes its own centered popup rect internally using `centered_rect(80, 75, area)` before rendering its content.

`EditorScreen::render` has two places that switch on `Focus` and must be updated to handle `Focus::NoteBrowser`:

1. **`focus_label`** — currently `if editor_focused { "EDITOR" } else { "SIDEBAR" }`. Change to a full match:
   ```rust
   let focus_label = match self.focus {
       Focus::Editor => "EDITOR",
       Focus::Sidebar => "SIDEBAR",
       Focus::NoteBrowser => "NOTE BROWSER",
   };
   ```

2. **Hint shortcuts** — currently `match self.focus { Editor => ..., Sidebar => ... }`. Add:
   ```rust
   Focus::NoteBrowser => self.note_browser
       .as_ref()
       .map(|m| m.hint_shortcuts())
       .unwrap_or_default(),
   ```
   `NoteBrowserModal` implements `hint_shortcuts()` returning `vec![("↑↓", "navigate"), ("Enter", "open"), ("Esc", "close")]`.

### Closing the modal

`EditorScreen::handle_app_message` intercepts `AppEvent::CloseNoteBrowser`:

```rust
AppEvent::CloseNoteBrowser => {
    self.note_browser = None;
    self.focus = Focus::Editor;
    None
}
```

---

## Events

Add to `events.rs`:

```rust
pub enum AppEvent {
    // existing...
    CloseNoteBrowser,
}
```

`OpenPath` sent from the modal already flows through the existing handler in `EditorScreen` and opens the note in the editor. No other new events required.

---

## New / Modified Files

| File | Change |
|------|--------|
| `src/components/note_browser/mod.rs` | New — `NoteBrowserModal`, `NoteBrowserProvider` trait, `centered_rect` helper |
| `src/components/note_browser/search_provider.rs` | New — `SearchNotesProvider` |
| `src/components/mod.rs` | Add `pub mod note_browser` |
| `src/components/file_list.rs` | Add `FileListEntry::CreateNote` variant; add `FileListComponent::selected_entry()` and `prepend_create_entry()` methods |
| `src/components/events.rs` | Add `AppEvent::CloseNoteBrowser` |
| `src/app_screen/editor.rs` | Add `Focus::NoteBrowser`, `note_browser` field, wire `ToggleNoteBrowser` action, render modal overlay |

---

## Out of Scope

- `FileFinderProvider` (future spec) — the `CreateNote` variant and `allows_create` hook are added now as extension points only
- Rendered/styled markdown in the preview pane — raw text is sufficient
- Mouse interaction within the modal
- Configurable split ratio between list and preview
