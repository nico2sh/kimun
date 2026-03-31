# Browse Screen: Create New Note

**Goal:** Let users create a note directly from the browse screen by typing its name in the search box and pressing Enter on the "Create…" entry — mirroring the file browser modal, with a persistent hint bar explaining the gesture.

**Architecture:** A separate `create_entry` slot is added to `FileListComponent` that always renders as the first list item, completely outside the filter/index machinery. The sidebar populates it from the search query and intercepts Enter to create the note. The browse screen gains a thin hint row at the bottom.

**Tech stack:** Rust/Ratatui, existing `NoteVault::load_or_create_note`, `VaultPath::note_path_from`.

---

## 1. `FileListComponent` changes (`tui/src/components/file_list.rs`)

Add one field:

```rust
create_entry: Option<FileListEntry>,
```

Initialised to `None`. The entry is always displayed as virtual display-index 0 when present, ahead of everything else — it is never stored in `entries` and never touched by the filter.

### New method

```rust
pub fn set_create_entry(&mut self, entry: Option<FileListEntry>) {
    self.create_entry = entry;
    self.reset_selection();
}
```

### Methods that must account for the virtual slot

| Method | Change |
|---|---|
| `display_len()` | `+1` when `create_entry.is_some()` |
| `selected_entry()` | If display idx 0 and `create_entry.is_some()`, return it; else subtract 1 from idx before indexing entries/display_indices |
| `activate_selected()` | Same offset logic as `selected_entry()` |
| `render()` | Prepend `create_entry.to_list_item()` to the items vec |
| `select_at_visual_row` / `display_idx_at_row` | Account for the extra visual row at the top |
| `clear()` | Set `create_entry = None` |

No changes to `prepend_create_entry`, `poll_filter`, or `schedule_filter`.

---

## 2. `SidebarComponent` changes (`tui/src/components/sidebar.rs`)

### After each search-modifying keystroke

After delegating a `Char` or `Backspace` key to `file_list.handle_input`, call a new private helper:

```rust
fn sync_create_entry(&mut self) {
    if self.file_list.search_query.is_empty() {
        self.file_list.set_create_entry(None);
    } else {
        let path = self.current_dir
            .append(&VaultPath::note_path_from(&self.file_list.search_query))
            .flatten();
        self.file_list.set_create_entry(Some(FileListEntry::CreateNote {
            filename: path.get_parent_path().1,
            path,
        }));
    }
}
```

`sync_create_entry` is also called after `start_loading` clears the list (to reset the create entry).

### Enter interception

In `handle_input`, before delegating to `file_list`:

```rust
if let InputEvent::Key(key) = event {
    if key.code == KeyCode::Enter {
        if let Some(FileListEntry::CreateNote { path, .. }) = self.file_list.selected_entry() {
            let path = path.clone();
            let vault = Arc::clone(&self.vault);
            let tx2 = tx.clone();
            tokio::spawn(async move {
                vault.load_or_create_note(&path, None).await.ok();
                tx2.send(AppEvent::OpenPath(path)).ok();
            });
            return EventState::Consumed;
        }
    }
}
```

---

## 3. `BrowseScreen` changes (`tui/src/app_screen/browse.rs`)

Add a fixed-height hint row at the bottom of the vertical layout (height 1):

```
Type to filter  ·  Enter to open  ·  Type + Enter to create a new note
```

Layout changes from `[Min(0)]` (sidebar only) to `[Min(0), Length(1)]`. The hint is rendered as a `Paragraph` with the theme's muted foreground style, left-aligned and padded with one space.

---

## Error handling

`load_or_create_note` failures are silently dropped (logged at warn level). If the note creation fails, no `OpenPath` is sent and the user stays on the browse screen. This matches the existing pattern in the note browser modal.

---

## Testing

- `FileListComponent`: unit test that `set_create_entry` shows the entry at virtual index 0, `selected_entry()` returns it when selected, and `clear()` removes it.
- `SidebarComponent`: no new unit tests (render path; covered by manual testing).
- `BrowseScreen`: no new unit tests.
