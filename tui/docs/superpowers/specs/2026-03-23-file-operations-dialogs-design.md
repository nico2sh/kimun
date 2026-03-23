# File Operations Dialogs — Design Spec

**Date:** 2026-03-23
**Feature:** Delete, Rename, Move dialogs for sidebar and editor (`Ctrl+Shift+D/R/M`)

---

## Overview

Three keyboard-triggered modal dialogs that let the user delete, rename, or move vault entries (notes and directories) directly from the sidebar or from the editor. Each operation has a confirmation dialog to prevent accidental data loss. The core library already implements `delete_note`, `delete_directory`, `rename_note`, and `rename_directory` — the TUI just needs the UI layer.

**Attachment scope:** `FileListComponent::push_entry` silently drops `FileListEntry::Attachment` entries, so attachments are never shown in the sidebar file list and cannot be selected. From the editor, `self.path` is always a note. Attachment paths therefore cannot reach any of the three dialogs in this feature. Attachment support is out of scope.

---

## Keyboard Shortcuts

| Action | Shortcut | Context |
|--------|----------|---------|
| Delete | `Ctrl+Shift+D` | Sidebar (selected entry) or Editor (current note) |
| Rename | `Ctrl+Shift+R` | Sidebar (selected entry) or Editor (current note) |
| Move   | `Ctrl+Shift+M` | Sidebar (selected entry) or Editor (current note) |

**Editor focus:** always operates on `self.path` (the currently open note), regardless of what is selected in the sidebar. These three shortcuts from the editor only apply to notes (not directories or attachments — those can only be selected from the sidebar).

---

## Architecture

### New files

| File | Responsibility |
|------|----------------|
| `src/components/dialogs/mod.rs` | Module declaration + shared `ActiveDialog` enum |
| `src/components/dialogs/delete_dialog.rs` | `DeleteConfirmDialog` component |
| `src/components/dialogs/rename_dialog.rs` | `RenameDialog` component |
| `src/components/dialogs/move_dialog.rs` | `MoveDialog` component |

### Modified files

| File | Change |
|------|--------|
| `src/keys/action_shortcuts.rs` | Add `DeleteEntry`, `RenameEntry`, `MoveEntry` variants with `Display` strings `"DeleteEntry"`, `"RenameEntry"`, `"MoveEntry"` and corresponding `TryFrom<String>` arms |
| `src/settings/mod.rs` | Add default bindings `Ctrl+Shift+D/R/M`; update `CONFIG_HEADER` examples |
| `src/components/events.rs` | Add `ShowDeleteDialog(VaultPath)`, `ShowRenameDialog(VaultPath)`, `ShowMoveDialog(VaultPath)`, `EntryDeleted(VaultPath)`, `EntryRenamed { from: VaultPath, to: VaultPath }`, `EntryMoved { from: VaultPath, to: VaultPath }`, `CloseDialog` |
| `src/components/file_list.rs` | Intercept `Ctrl+Shift+D/R/M` on selected entry (has `KeyBindings` already) |
| `src/components/sidebar.rs` | Add `pub fn current_dir(&self) -> &VaultPath` accessor |
| `src/app_screen/editor.rs` | Intercept `Ctrl+Shift+D/R/M` when editor focused; own `active_dialog`; handle dialog AppEvents; post-op refresh |

---

## Triggering the Dialogs

### From the sidebar (`FileListComponent::handle_input`)

`SidebarComponent::handle_input` delegates entirely to `self.file_list.handle_input(event, tx)`. `FileListComponent` already holds a `KeyBindings` field, so the new shortcuts are intercepted there.

When `Ctrl+Shift+D/R/M` is pressed inside `FileListComponent::handle_input` and there is a selected entry that is not `FileListEntry::Up`, the three new arms are inserted inside the existing `Some(action)` branch alongside the existing `ActionShortcuts` arms (which end with a `_ => {}` wildcard):

```rust
// Inside the existing Some(action) match in handle_input:
Some(ActionShortcuts::DeleteEntry) => {
    if let Some(entry) = self.selected_entry() {
        if !matches!(entry, FileListEntry::Up { .. }) {
            tx.send(AppEvent::ShowDeleteDialog(entry.path().clone())).ok();
            return EventState::Consumed;
        }
    }
    EventState::NotConsumed
}
Some(ActionShortcuts::RenameEntry) => {
    if let Some(entry) = self.selected_entry() {
        if !matches!(entry, FileListEntry::Up { .. }) {
            tx.send(AppEvent::ShowRenameDialog(entry.path().clone())).ok();
            return EventState::Consumed;
        }
    }
    EventState::NotConsumed
}
Some(ActionShortcuts::MoveEntry) => {
    if let Some(entry) = self.selected_entry() {
        if !matches!(entry, FileListEntry::Up { .. }) {
            tx.send(AppEvent::ShowMoveDialog(entry.path().clone())).ok();
            return EventState::Consumed;
        }
    }
    EventState::NotConsumed
}
```

If no entry is selected (empty list) or `..` is selected, `EventState::NotConsumed` is returned.

**Key ordering:** `FileListComponent::handle_input` runs the `key_bindings.get_action(&combo)` check first, before the `KeyCode::Char(c)` fallthrough arm that appends to `search_query`. The new shortcuts must be matched in the `get_action` branch so they are consumed before reaching the char arm. This works correctly as long as the bindings are registered — the implementer must ensure the three new `ActionShortcuts` variants are added to the default keybindings and to the `CONFIG_HEADER`.

### From the editor (`EditorScreen::handle_input`)

Handled in the existing global `match self.settings.key_bindings.get_action(&combo)` block, only when `Focus::Editor`:

```rust
Some(ActionShortcuts::DeleteEntry) if matches!(self.focus, Focus::Editor) => {
    tx.send(AppEvent::ShowDeleteDialog(self.path.clone())).ok();
    return EventState::Consumed;
}
Some(ActionShortcuts::RenameEntry) if matches!(self.focus, Focus::Editor) => {
    tx.send(AppEvent::ShowRenameDialog(self.path.clone())).ok();
    return EventState::Consumed;
}
Some(ActionShortcuts::MoveEntry) if matches!(self.focus, Focus::Editor) => {
    tx.send(AppEvent::ShowMoveDialog(self.path.clone())).ok();
    return EventState::Consumed;
}
```

`self.path` is the currently open note. This always produces a note path — no directory/attachment handling needed here.

---

## EditorScreen Dialog State

```rust
pub struct EditorScreen {
    // ... existing fields ...
    active_dialog: Option<ActiveDialog>,
}
```

`ActiveDialog` is an enum in `dialogs/mod.rs`:

```rust
pub enum ActiveDialog {
    Delete(DeleteConfirmDialog),
    Rename(RenameDialog),
    Move(MoveDialog),
}

impl ActiveDialog {
    pub fn set_error(&mut self, msg: String) {
        match self {
            ActiveDialog::Delete(d) => d.error = Some(msg),
            ActiveDialog::Rename(d) => d.error = Some(msg),
            ActiveDialog::Move(d)   => d.error = Some(msg),
        }
    }
}
```

`EditorScreen::handle_app_message` handles the show-dialog events:

```rust
AppEvent::ShowDeleteDialog(path) => {
    self.active_dialog = Some(ActiveDialog::Delete(
        DeleteConfirmDialog::new(path, self.vault.clone(), tx.clone())
    ));
    self.focus = Focus::Dialog;
    None
}
AppEvent::ShowRenameDialog(path) => {
    self.active_dialog = Some(ActiveDialog::Rename(
        RenameDialog::new(path, self.vault.clone(), tx.clone())
    ));
    self.focus = Focus::Dialog;
    None
}
AppEvent::ShowMoveDialog(path) => {
    // MoveDialog::new must call self.schedule_load() to start the directory fetch.
    self.active_dialog = Some(ActiveDialog::Move(
        MoveDialog::new(path, self.vault.clone(), tx.clone())
    ));
    self.focus = Focus::Dialog;
    None
}
```

`CloseDialog` and post-operation events clear `active_dialog` and restore `Focus::Editor`.

In `render`, the active dialog is drawn last (on top of everything) using the same `centered_rect` pattern as `NoteBrowserModal`:

```rust
if let Some(dialog) = &mut self.active_dialog {
    match dialog {
        ActiveDialog::Delete(d) => d.render(f, f.area(), &self.theme, true),
        ActiveDialog::Rename(d) => d.render(f, f.area(), &self.theme, true),
        ActiveDialog::Move(d)   => d.render(f, f.area(), &self.theme, true),
    }
}
```

Input is routed to the active dialog when `Focus::Dialog`:

```rust
Focus::Dialog => {
    if let Some(dialog) = &mut self.active_dialog {
        match dialog {
            ActiveDialog::Delete(d) => d.handle_input(event, tx),
            ActiveDialog::Rename(d) => d.handle_input(event, tx),
            ActiveDialog::Move(d)   => d.handle_input(event, tx),
        }
    } else {
        EventState::NotConsumed
    }
}
```

---

## DeleteConfirmDialog

**File:** `src/components/dialogs/delete_dialog.rs`

```rust
pub struct DeleteConfirmDialog {
    path: VaultPath,
    vault: Arc<NoteVault>,
    tx: AppTx,
    error: Option<String>,
}
```

### Layout

```
┌─ Delete ─────────────────────────────────────┐
│                                               │
│  Are you sure you want to delete:             │
│  ▌notes/projects/kimun.md                    │
│                                               │
│  This cannot be undone.                       │
│                                               │
│  [Enter: Delete]  [Esc: Cancel]               │
└───────────────────────────────────────────────┘
```

### Key handling

- `Enter` → spawn async delete task, close on success
- `Esc` → send `AppEvent::CloseDialog`

### Async delete

`vault.delete_note` hard-rejects non-`.md` paths and `vault.delete_directory` hard-rejects `.md` paths. Since attachment paths are out of scope (see Overview), the delete dialog only handles notes and directories. `VaultPath` has no `is_directory()` method — use `!path.is_note()` for the directory branch.

```rust
KeyCode::Enter => {
    let path = self.path.clone();
    let vault = Arc::clone(&self.vault);
    let tx = self.tx.clone();
    tokio::spawn(async move {
        let result = if path.is_note() {
            vault.delete_note(&path).await
        } else {
            vault.delete_directory(&path).await
        };
        match result {
            Ok(()) => { tx.send(AppEvent::EntryDeleted(path)).ok(); }
            Err(e) => { tx.send(AppEvent::DialogError(e.to_string())).ok(); }
        }
    });
    EventState::Consumed
}
```

### Error display

Add `AppEvent::DialogError(String)` to events. `EditorScreen::handle_app_message` forwards it to the active dialog to display inline rather than closing the dialog.

---

## RenameDialog

**File:** `src/components/dialogs/rename_dialog.rs`

```rust
pub struct RenameDialog {
    path: VaultPath,
    vault: Arc<NoteVault>,
    input: String,                          // current text in the field
    validation_state: ValidationState,
    validation_task: Option<JoinHandle<()>>,
    validation_rx: Option<Receiver<bool>>,  // true = available
    tx: AppTx,
    error: Option<String>,
}

enum ValidationState {
    Idle,       // no input yet / unchanged from original
    Pending,    // async check in flight
    Available,  // name is free
    Taken,      // name already exists
}
```

### Layout

```
┌─ Rename ──────────────────────────────────────┐
│                                               │
│  CURRENT PATH                                 │
│  notes/projects/kimun.md                      │
│                                               │
│  NEW NAME                                     │
│  [ kimun-tui.md                    | ]  ✓     │
│  Available                                    │
│                                               │
│  [Enter: Rename]  [Esc: Cancel]               │
└───────────────────────────────────────────────┘
```

Validation indicator:
- `⌛` — `ValidationState::Pending`
- `✓` green — `ValidationState::Available`
- `✗` red — `ValidationState::Taken`

`Enter` is disabled (greyed out, does nothing) unless `ValidationState::Available`.

`Esc` → send `AppEvent::CloseDialog` (same as `DeleteConfirmDialog`).

### Pre-fill

`input` is pre-filled with `path.get_parent_path().1` (the filename component, e.g. `"kimun.md"` for a note, `"projects"` for a directory). Initial `ValidationState::Idle` — no check needed until the user types.

`VaultPath::note_path_from` is idempotent with respect to the `.md` extension: passing `"kimun.md"` returns `"kimun.md"` (not `"kimun.md.md"`), so the full filename is safe to use as pre-fill for notes.

### Real-time validation

On each `Char` or `Backspace` keystroke, if the input differs from the original filename:

1. Abort previous `validation_task`: call `handle.abort()` on the old `JoinHandle` before dropping it (same pattern as `FileListComponent::schedule_filter`)
2. Build candidate path — construction differs by entry type:
   - Note: `parent_dir.append(&VaultPath::note_path_from(&input))` (appends `.md`)
   - Directory: `parent_dir.append(&VaultPath::new(&input))` (no extension added)
3. Spawn:
   ```rust
   let vault = Arc::clone(&self.vault);
   let (tx, rx) = std::sync::mpsc::channel();
   let handle = tokio::spawn(async move {
       let exists = vault.exists(&candidate).await.is_some();
       tx.send(!exists).ok(); // true = available
   });
   self.validation_task = Some(handle);
   self.validation_rx = Some(rx);
   self.validation_state = ValidationState::Pending;
   ```

`poll_validation()` is called in `render`:
```rust
fn poll_validation(&mut self) {
    let Some(rx) = &self.validation_rx else { return };
    if let Ok(available) = rx.try_recv() {
        self.validation_state = if available { ValidationState::Available } else { ValidationState::Taken };
        self.validation_rx = None;
        self.validation_task = None;
    }
}
```

### Confirmation

On `Enter` when `ValidationState::Available`:

Since attachment paths are out of scope, `self.path` is always a note or directory.

```rust
let parent = self.path.get_parent_path().0;
// Use note_path_from only for notes (appends .md); directories use VaultPath::new.
let new_path = if self.path.is_note() {
    parent.append(&VaultPath::note_path_from(&self.input))
} else {
    parent.append(&VaultPath::new(&self.input))
};
let vault = Arc::clone(&self.vault);
let tx = self.tx.clone();
let old_path = self.path.clone();
tokio::spawn(async move {
    let result = if old_path.is_note() {
        vault.rename_note(&old_path, &new_path).await
    } else {
        vault.rename_directory(&old_path, &new_path).await
    };
    match result {
        Ok(()) => { tx.send(AppEvent::EntryRenamed { from: old_path, to: new_path }).ok(); }
        Err(e) => { tx.send(AppEvent::DialogError(e.to_string())).ok(); }
    }
});
```

---

## MoveDialog

**File:** `src/components/dialogs/move_dialog.rs`

```rust
pub struct MoveDialog {
    path: VaultPath,
    vault: Arc<NoteVault>,
    search_query: String,
    all_dirs: Vec<VaultPath>,           // full list once loaded
    load_task: Option<JoinHandle<()>>,
    load_rx: Option<Receiver<Vec<VaultPath>>>,
    results: Vec<VaultPath>,            // filtered view shown in the list
    list_state: ListState,
    tx: AppTx,
    error: Option<String>,
}
```

### Layout

```
┌─ Move ────────────────────────────────────────┐
│                                               │
│  MOVING                                       │
│  notes/projects/kimun.md                      │
│                                               │
│  DESTINATION                                  │
│  [ arch                            |  ]       │
│  ┌─────────────────────────────────┐          │
│  │ ▶ 📁 archive                   │          │
│  │   📁 archive/old               │          │
│  │   📁 articles                  │          │
│  └─────────────────────────────────┘          │
│                                               │
│  [Enter: Move here]  [Esc: Cancel]            │
└───────────────────────────────────────────────┘
```

### Directory fetch

On construction, `schedule_load()` is called immediately. It spawns a task that:

1. Fetches all directories from the vault. `vault.get_directories(path, recursive)` is **synchronous** and returns `Result<Vec<DirectoryDetails>, VaultError>`. Call it with `(VaultPath::root(), true)` to get the full tree, wrapped in `tokio::task::spawn_blocking`. Map results to `Vec<VaultPath>` via `details.path`.
2. The root directory (`VaultPath::root()`) is prepended to the list as the vault root option.
3. Results sent via `mpsc::channel`, polled in `render` → stored in `results`.

```rust
fn schedule_load(&mut self) {
    let vault = Arc::clone(&self.vault);
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            vault.get_directories(&VaultPath::root(), true)
        })
        .await;
        if let Ok(Ok(dirs)) = result {
            let mut paths: Vec<VaultPath> = std::iter::once(VaultPath::root())
                .chain(dirs.into_iter().map(|d| d.path))
                .collect();
            paths.sort();
            tx.send(paths).ok();
        }
    });
    self.load_task = Some(handle);
    self.load_rx = Some(rx);
}
```

When `load_rx` delivers results in `render`, store them in `self.all_dirs` and initialize `self.results = self.all_dirs.clone()`. All subsequent filtering runs against `self.all_dirs`.

### Filtering

On each keystroke, run nucleo fuzzy-match against `self.all_dirs` (same `spawn_blocking` pattern as `FileFinderProvider`). `results` is updated with the filtered+ranked list. After delivering results from the async task, send `AppEvent::Redraw` so the dialog repaints (same pattern as `FileListComponent::schedule_filter`).

With empty query: `results = all_dirs.clone()` (already sorted alphabetically from `schedule_load`).

### Navigation and key handling

`↑` / `↓` move the `ListState` selection. `Enter` picks the highlighted directory. `Esc` sends `AppEvent::CloseDialog`.

### Confirmation

On `Enter` with a selected directory:

Since attachment paths are out of scope, `self.path` is always a note or directory.

```rust
let dest_dir = &self.results[selected_idx];
let filename = self.path.get_parent_path().1;
// Use note_path_from only for notes (appends .md); directories use VaultPath::new.
let new_path = if self.path.is_note() {
    dest_dir.append(&VaultPath::note_path_from(&filename))
} else {
    dest_dir.append(&VaultPath::new(&filename))
};
let vault = Arc::clone(&self.vault);
let tx = self.tx.clone();
let old_path = self.path.clone();
tokio::spawn(async move {
    let result = if old_path.is_note() {
        vault.rename_note(&old_path, &new_path).await
    } else {
        vault.rename_directory(&old_path, &new_path).await
    };
    match result {
        Ok(()) => { tx.send(AppEvent::EntryMoved { from: old_path, to: new_path }).ok(); }
        Err(e) => { tx.send(AppEvent::DialogError(e.to_string())).ok(); }
    }
});
```

---

## Post-Operation Behavior

`EditorScreen::handle_app_message` handles the result events. The three variants cannot share a single `|` match arm because they mix a tuple variant (`EntryDeleted(path)`) with struct variants (`EntryRenamed { from, .. }`, `EntryMoved { from, .. }`). Use three separate arms that each extract the `from` path and call shared logic:

```rust
AppEvent::EntryDeleted(path) => {
    self.on_entry_op(path, tx).await;
    None
}
AppEvent::EntryRenamed { from, .. } => {
    self.on_entry_op(from, tx).await;
    None
}
AppEvent::EntryMoved { from, .. } => {
    self.on_entry_op(from, tx).await;
    None
}
AppEvent::DialogError(msg) => {
    if let Some(dialog) = &mut self.active_dialog {
        dialog.set_error(msg);
    }
    None
}
AppEvent::CloseDialog => {
    self.active_dialog = None;
    self.focus = Focus::Editor;
    None
}
```

The shared `on_entry_op` helper on `EditorScreen`. Before navigating away, call `self.try_save().await` (existing signature, no arguments) to flush any unsaved editor content — the editor may have changes that would be silently discarded if the user deletes or moves the currently open note before saving.

```rust
async fn on_entry_op(&mut self, from: VaultPath, tx: &AppTx) {
    self.active_dialog = None;
    self.focus = Focus::Editor;
    if from == self.path {
        // Flush unsaved edits before navigating away
        self.try_save().await;
        // The currently open note was affected — navigate to parent browse screen
        let parent = self.path.get_parent_path().0;
        tx.send(AppEvent::OpenScreen(ScreenEvent::OpenBrowse(
            self.vault.clone(), parent
        ))).ok();
    } else {
        // Refresh sidebar to reflect the change.
        // Requires `pub fn current_dir(&self) -> &VaultPath` on SidebarComponent (see Modified Files).
        let dir = self.sidebar.current_dir().clone();
        self.navigate_sidebar(dir, tx).await;
    }
}
```

---

## Focus enum update

Add `Dialog` variant to the existing `Focus` enum in `editor.rs`:

```rust
enum Focus {
    Sidebar,
    Editor,
    NoteBrowser,
    Dialog,
}
```

`editor.rs` has exhaustive `match self.focus` expressions for `focus_label` (the label shown in the status bar) and `hints` (keyboard hint list). Add `Focus::Dialog` arms to both:

```rust
// focus_label
Focus::Dialog => "DIALOG",

// hints — return empty: the dialog itself renders its own key hints
Focus::Dialog => vec![],
```

**Mouse events when dialog is open:** `EditorScreen::handle_input` checks `matches!(self.focus, Focus::NoteBrowser)` for mouse routing. Add `Focus::Dialog` to that check so that mouse events are also consumed and not passed to the sidebar or editor while a dialog is open:

```rust
if matches!(self.focus, Focus::NoteBrowser | Focus::Dialog) {
    // swallow mouse events
    return EventState::Consumed;
}
```

---

## AppEvent additions

```rust
// Triggers
ShowDeleteDialog(VaultPath),
ShowRenameDialog(VaultPath),
ShowMoveDialog(VaultPath),

// Results
EntryDeleted(VaultPath),
EntryRenamed { from: VaultPath, to: VaultPath },
EntryMoved { from: VaultPath, to: VaultPath },

// Error feedback to active dialog
DialogError(String),

// Dismiss
CloseDialog,
```

---

## Out of scope

- Undo / undo history
- Multi-select (deleting/moving multiple entries at once)
- Attachment operations (attachments are not shown in the sidebar file list; see Overview)
- Move dialog's destination creating new directories on the fly
- Drag-and-drop
