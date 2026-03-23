# File Operations Dialogs — Design Spec

**Date:** 2026-03-23
**Feature:** Delete, Rename, Move dialogs for sidebar and editor (`Ctrl+Shift+D/R/M`)

---

## Overview

Three keyboard-triggered modal dialogs that let the user delete, rename, or move any vault entry (notes, directories, attachments) directly from the sidebar or from the editor. Each operation has a confirmation dialog to prevent accidental data loss. The core library already implements `delete_note`, `delete_directory`, `rename_note`, and `rename_directory` — the TUI just needs the UI layer.

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
| `src/keys/action_shortcuts.rs` | Add `DeleteEntry`, `RenameEntry`, `MoveEntry` variants |
| `src/settings/mod.rs` | Add default bindings `Ctrl+Shift+D/R/M`; update `CONFIG_HEADER` examples |
| `src/components/events.rs` | Add `ShowDeleteDialog(VaultPath)`, `ShowRenameDialog(VaultPath)`, `ShowMoveDialog(VaultPath)`, `EntryDeleted(VaultPath)`, `EntryRenamed { from: VaultPath, to: VaultPath }`, `EntryMoved { from: VaultPath, to: VaultPath }`, `CloseDialog` |
| `src/components/sidebar.rs` | Intercept `Ctrl+Shift+D/R/M` on selected sidebar entry |
| `src/app_screen/editor.rs` | Intercept `Ctrl+Shift+D/R/M` when editor focused; own `active_dialog`; handle dialog AppEvents; post-op refresh |

---

## Triggering the Dialogs

### From the sidebar (`SidebarComponent::handle_input`)

When `Ctrl+Shift+D/R/M` is pressed and the sidebar has a selected entry:

```rust
if let Some(entry) = self.file_list.selected_entry() {
    let path = entry.path().clone();
    match action {
        ActionShortcuts::DeleteEntry => tx.send(AppEvent::ShowDeleteDialog(path)).ok(),
        ActionShortcuts::RenameEntry => tx.send(AppEvent::ShowRenameDialog(path)).ok(),
        ActionShortcuts::MoveEntry   => tx.send(AppEvent::ShowMoveDialog(path)).ok(),
    };
    return EventState::Consumed;
}
```

If no entry is selected (empty list), the event is silently ignored.

`Up` entries (`FileListEntry::Up`) are excluded — the shortcuts do nothing when `..` is selected.

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
// same pattern for ShowRenameDialog and ShowMoveDialog
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

Attachments that are not notes and not directories: use `delete_note` (the vault resolves it via filesystem path regardless of `.md` extension check). If `delete_note` returns an error for non-note paths, the error is shown inline.

> **Note:** `vault.delete_note` validates that the path ends with `.md`. For attachment deletion, a separate `vault.delete_attachment` may be needed in the core library, or the dialog can use direct filesystem removal via `std::fs::remove_file` after checking `vault.path_to_pathbuf`. This is an implementation-time decision.

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

### Pre-fill

`input` is pre-filled with `path.get_parent_path().1` (the filename component, e.g. `"kimun.md"`). Initial `ValidationState::Idle` — no check needed until the user types.

### Real-time validation

On each `Char` or `Backspace` keystroke, if the input differs from the original filename:

1. Abort previous `validation_task`
2. Build `candidate = parent_dir.append(&VaultPath::note_path_from(&input))`
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

```rust
let parent = self.path.get_parent_path().0;
let new_path = parent.append(&VaultPath::note_path_from(&self.input));
let vault = Arc::clone(&self.vault);
let tx = self.tx.clone();
let old_path = self.path.clone();
tokio::spawn(async move {
    match vault.rename_note(&old_path, &new_path).await {
        Ok(()) => { tx.send(AppEvent::EntryRenamed { from: old_path, to: new_path }).ok(); }
        Err(e) => { tx.send(AppEvent::DialogError(e.to_string())).ok(); }
    }
});
```

For directories: `vault.rename_directory(&old_path, &new_path)`.

---

## MoveDialog

**File:** `src/components/dialogs/move_dialog.rs`

```rust
pub struct MoveDialog {
    path: VaultPath,
    vault: Arc<NoteVault>,
    search_query: String,
    dirs_cache: Arc<tokio::sync::OnceCell<Vec<VaultPath>>>,
    load_task: Option<JoinHandle<()>>,
    load_rx: Option<Receiver<Vec<VaultPath>>>,
    results: Vec<VaultPath>,
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

1. Fetches all directories from the vault. Uses `vault.get_directories()` — or, if that method is synchronous, wraps it in `tokio::task::spawn_blocking`.
2. The root directory (`/`) is prepended to the list as the vault root option.
3. Results sent via `mpsc::channel`, polled in `render` → stored in `results`.

The dirs list is cached in `OnceCell` so re-opening the dialog on the same vault instance reuses it.

### Filtering

On each keystroke, run nucleo fuzzy-match against the cached dirs list (same `spawn_blocking` pattern as `FileFinderProvider`). `results` is updated with the filtered+ranked list.

With empty query: show all directories sorted alphabetically.

### Navigation

`↑` / `↓` move the `ListState` selection. `Enter` picks the highlighted directory.

### Confirmation

On `Enter` with a selected directory:

```rust
let dest_dir = &self.results[selected_idx];
let filename = self.path.get_parent_path().1;
let new_path = dest_dir.append(&VaultPath::note_path_from(&filename));
let vault = Arc::clone(&self.vault);
let tx = self.tx.clone();
let old_path = self.path.clone();
tokio::spawn(async move {
    match vault.rename_note(&old_path, &new_path).await {
        Ok(()) => { tx.send(AppEvent::EntryMoved { from: old_path, to: new_path }).ok(); }
        Err(e) => { tx.send(AppEvent::DialogError(e.to_string())).ok(); }
    }
});
```

For directories: `vault.rename_directory`. The destination for a directory move is `dest_dir.append(&VaultPath::new(&dirname))`.

---

## Post-Operation Behavior

`EditorScreen::handle_app_message` handles the result events:

```rust
AppEvent::EntryDeleted(path) |
AppEvent::EntryRenamed { from: path, .. } |
AppEvent::EntryMoved { from: path, .. } => {
    self.active_dialog = None;
    self.focus = Focus::Editor;

    if path == self.path {
        // The currently open note was affected — go to browse screen
        let parent = self.path.get_parent_path().0;
        tx.send(AppEvent::OpenScreen(ScreenEvent::OpenBrowse(
            self.vault.clone(), parent
        ))).ok();
    } else {
        // Refresh sidebar to reflect the change
        let dir = self.sidebar.current_dir.clone();
        self.navigate_sidebar(dir, tx).await;
    }
    None
}

AppEvent::DialogError(msg) => {
    // Forward to active dialog for inline display
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

For `EntryRenamed` and `EntryMoved`, the `path` matched against `self.path` is the `from` path.

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
- Rename dialog for attachments with extension validation (`.md` enforcement is note-specific; attachments keep their extension)
- Move dialog's destination creating new directories on the fly
- Drag-and-drop
