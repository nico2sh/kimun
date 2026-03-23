# File Operations Dialogs Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Delete (Ctrl+Shift+D), Rename (Ctrl+Shift+R), and Move (Ctrl+Shift+M) modal dialogs for vault notes and directories, triggered from the sidebar file list and the editor.

**Architecture:** Three dialog components (`DeleteConfirmDialog`, `RenameDialog`, `MoveDialog`) owned by `EditorScreen` via an `ActiveDialog` enum. Dialogs communicate results back via `AppEvent` messages. The sidebar intercepts shortcuts in `FileListComponent` (which already holds `KeyBindings`); the editor intercepts shortcuts when `Focus::Editor`.

**Tech Stack:** Rust, ratatui, tokio async runtime, crossterm, nucleo (fuzzy matching), kimun_core vault API

---

## Chunk 1: Foundation (Tasks 1–4)

### Task 1: ActionShortcuts + keybindings

**Goal:** Add `DeleteEntry`, `RenameEntry`, `MoveEntry` action variants and register their default key bindings.

**Files:**
- `src/keys/action_shortcuts.rs` — add three new variants to the `ActionShortcuts` enum, extend `Display` and `TryFrom<String>` impls
- `src/settings/mod.rs` — add default `Ctrl+Shift+D/R/M` bindings in `default_keybindings()`, update `CONFIG_HEADER`

**Steps:**
- [ ] Write test in `src/keys/action_shortcuts.rs` (inside `#[cfg(test)]` mod) verifying roundtrip for all three new variants: `ActionShortcuts::DeleteEntry.to_string()` == `"DeleteEntry"` and `ActionShortcuts::try_from("DeleteEntry".to_string())` == `Ok(ActionShortcuts::DeleteEntry)` (repeat for `RenameEntry`, `MoveEntry`)
- [ ] Verify test fails: `cd /Users/nhormazabal/development/personal/kimun/tui && cargo test`
- [ ] Implement: add `DeleteEntry`, `RenameEntry`, `MoveEntry` variants to the `ActionShortcuts` enum; add `"DeleteEntry"`, `"RenameEntry"`, `"MoveEntry"` arms to `Display` and `TryFrom<String>`; add the batch_add chain in `default_keybindings()`:
  ```rust
  kb.batch_add()
      .with_ctrl()
      .with_shift()
      .add(KeyStrike::KeyD, ActionShortcuts::DeleteEntry)
      .add(KeyStrike::KeyR, ActionShortcuts::RenameEntry)
      .add(KeyStrike::KeyM, ActionShortcuts::MoveEntry);
  ```
  Update `CONFIG_HEADER` to add example lines for `DeleteEntry`, `RenameEntry`, `MoveEntry` (e.g. `#   DeleteEntry  = ["ctrl+shift&D"]  # Ctrl+Shift+D`)
- [ ] Verify tests pass: `cd /Users/nhormazabal/development/personal/kimun/tui && cargo test`
- [ ] Commit: `git commit -m "feat: add DeleteEntry/RenameEntry/MoveEntry action shortcuts"`

---

### Task 2: AppEvent variants

**Goal:** Add all new `AppEvent` variants needed by the dialog system.

**Files:**
- `src/components/events.rs` — add seven new variants to `AppEvent`

**Steps:**
- [ ] Implement: add the following variants to the `AppEvent` enum:
  ```rust
  ShowDeleteDialog(VaultPath),
  ShowRenameDialog(VaultPath),
  ShowMoveDialog(VaultPath),
  EntryDeleted(VaultPath),
  EntryRenamed { from: VaultPath, to: VaultPath },
  EntryMoved { from: VaultPath, to: VaultPath },
  DialogError(String),
  CloseDialog,
  ```
- [ ] Verify builds: `cd /Users/nhormazabal/development/personal/kimun/tui && cargo check`
- [ ] Run full test suite: `cd /Users/nhormazabal/development/personal/kimun/tui && cargo test`
- [ ] Commit: `git commit -m "feat: add dialog AppEvent variants"`

---

### Task 3: Dialog module scaffold

**Goal:** Create the `src/components/dialogs/` module with the `ActiveDialog` enum and register it in `mod.rs`.

**Files:**
- `src/components/dialogs/mod.rs` — new file: `ActiveDialog` enum, `set_error` impl, `centered_rect` private helper (copied from `note_browser/mod.rs`), re-exports and submodule declarations
- `src/components/mod.rs` — add `pub mod dialogs;`

**Steps:**
- [ ] Implement `src/components/dialogs/mod.rs` with:
  ```rust
  pub use delete_dialog::DeleteConfirmDialog;
  pub use rename_dialog::RenameDialog;
  pub use move_dialog::MoveDialog;

  pub mod delete_dialog;
  pub mod rename_dialog;
  pub mod move_dialog;

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
  Also copy `centered_rect` from `src/components/note_browser/mod.rs` as a private `fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect` helper in this module. Create stub files `delete_dialog.rs`, `rename_dialog.rs`, `move_dialog.rs` (empty structs with `pub error: Option<String>` field so `set_error` compiles). Register `pub mod dialogs;` in `src/components/mod.rs`.
- [ ] Verify builds: `cd /Users/nhormazabal/development/personal/kimun/tui && cargo check`
- [ ] Run full test suite: `cd /Users/nhormazabal/development/personal/kimun/tui && cargo test`
- [ ] Commit: `git commit -m "feat: scaffold dialogs module with ActiveDialog enum"`

---

### Task 4: DeleteConfirmDialog

**Goal:** Implement the delete confirmation dialog with async vault operation and inline error display.

**Files:**
- `src/components/dialogs/delete_dialog.rs` — full implementation replacing the stub

**Struct:**
```rust
pub struct DeleteConfirmDialog {
    pub path: VaultPath,
    pub vault: Arc<NoteVault>,
    pub tx: AppTx,
    pub error: Option<String>,
}
```

**Layout** (60% x, 40% y via `centered_rect`):
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
If `error` is `Some`, render an extra red line below the hint row.

**Key handling:**
- `Enter` — spawn async delete task (see snippet below), return `EventState::Consumed`
- `Esc` — send `AppEvent::CloseDialog`, return `EventState::Consumed`

**Async delete:**
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

**Steps:**
- [ ] Write test: `DeleteConfirmDialog::new(VaultPath::root(), vault_arc, tx)` does not panic (minimal smoke test — requires a fake `Arc<NoteVault>` or integration test setup; if a real vault is not available, gate behind `#[ignore]` and document)
- [ ] Verify test fails (or is ignored): `cd /Users/nhormazabal/development/personal/kimun/tui && cargo test`
- [ ] Implement full `DeleteConfirmDialog` in `src/components/dialogs/delete_dialog.rs`: `new()`, `handle_input()`, `render()` using `super::centered_rect(60, 40, area)` with a `Clear` widget backdrop, `Block` with "Delete" title, path display, confirmation text, hint line, and optional error line in red
- [ ] Verify tests pass: `cd /Users/nhormazabal/development/personal/kimun/tui && cargo test`
- [ ] Commit: `git commit -m "feat: implement DeleteConfirmDialog"`

---

## Chunk 2: Dialogs and Wiring (Tasks 5–8)

### Task 5: RenameDialog

**Goal:** Implement the rename dialog with pre-filled input, real-time async name availability validation, and confirmation logic.

**Files:**
- `src/components/dialogs/rename_dialog.rs` — full implementation replacing the stub

**Struct:**
```rust
pub struct RenameDialog {
    pub path: VaultPath,
    pub vault: Arc<NoteVault>,
    pub input: String,
    pub validation_state: ValidationState,
    pub validation_task: Option<JoinHandle<()>>,
    pub validation_rx: Option<Receiver<bool>>,
    pub tx: AppTx,
    pub error: Option<String>,
}

pub enum ValidationState {
    Idle,
    Pending,
    Available,
    Taken,
}
```

**Pre-fill:** `input` is initialized with `path.get_parent_path().1` (the filename component). Initial state is `ValidationState::Idle`.

**Layout** (60% x, 50% y via `centered_rect`):
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
Validation indicator: `⌛` for `Pending`, `✓` (green) for `Available`, `✗` (red) for `Taken`. `Enter` hint is greyed out (not actionable) unless `ValidationState::Available`.

**Key handling:**
- `Char(c)` / `Backspace` — update `input`, abort previous `validation_task` (`handle.abort()`), spawn new validation task, set `ValidationState::Pending`
- `Enter` when `ValidationState::Available` — spawn rename task (see confirmation snippet below)
- `Enter` when not `Available` — return `EventState::Consumed` (do nothing, hint is greyed)
- `Esc` — send `AppEvent::CloseDialog`

**Validation spawn pattern:**
```rust
let vault = Arc::clone(&self.vault);
let (vtx, vrx) = std::sync::mpsc::channel();
let handle = tokio::spawn(async move {
    let exists = vault.exists(&candidate).await.is_some();
    vtx.send(!exists).ok(); // true = available
});
self.validation_task = Some(handle);
self.validation_rx = Some(vrx);
self.validation_state = ValidationState::Pending;
```

**`poll_validation()`** called at start of `render()`:
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

**Confirmation on Enter:**
```rust
let parent = self.path.get_parent_path().0;
let new_path = if self.path.is_note() {
    parent.append(&VaultPath::note_path_from(&self.input))
} else {
    parent.append(&VaultPath::new(&self.input))
};
// spawn rename_note or rename_directory, send EntryRenamed or DialogError
```

**Steps:**
- [ ] Write test: `RenameDialog::new(some_note_path, vault_arc, tx)` pre-fills `input` with the filename component and does not panic
- [ ] Verify test fails (or is ignored): `cd /Users/nhormazabal/development/personal/kimun/tui && cargo test`
- [ ] Implement full `RenameDialog` in `src/components/dialogs/rename_dialog.rs`: `new()`, `handle_input()`, `poll_validation()`, `render()`
- [ ] Verify tests pass: `cd /Users/nhormazabal/development/personal/kimun/tui && cargo test`
- [ ] Commit: `git commit -m "feat: implement RenameDialog with live validation"`

---

### Task 6: MoveDialog

**Goal:** Implement the move dialog with async directory listing, nucleo fuzzy filtering, and keyboard-navigable destination list.

**Files:**
- `src/components/dialogs/move_dialog.rs` — full implementation replacing the stub

**Struct:**
```rust
pub struct MoveDialog {
    pub path: VaultPath,
    pub vault: Arc<NoteVault>,
    pub search_query: String,
    pub all_dirs: Vec<VaultPath>,
    pub load_task: Option<JoinHandle<()>>,
    pub load_rx: Option<Receiver<Vec<VaultPath>>>,
    pub results: Vec<VaultPath>,
    pub list_state: ListState,
    pub tx: AppTx,
    pub error: Option<String>,
}
```

**Layout** (70% x, 60% y via `centered_rect`):
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

**`new()` calls `schedule_load()` immediately.**

**`schedule_load()`:**
```rust
fn schedule_load(&mut self) {
    let vault = Arc::clone(&self.vault);
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            vault.get_directories(&VaultPath::root(), true)
        }).await;
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

When `load_rx` delivers results in `render()`, store them in `self.all_dirs` and set `self.results = self.all_dirs.clone()`.

**Filtering** — same `spawn_blocking` + nucleo pattern as `FileListComponent::schedule_filter` (abort previous task, spawn new, send `AppEvent::Redraw` after delivering results). With empty query: `results = all_dirs.clone()`.

**Key handling:**
- `Up` / `Down` — move `list_state` selection
- `Char(c)` / `Backspace` — update `search_query`, call `schedule_filter(tx.clone())`
- `Enter` with a selected result — spawn move task (see confirmation below), return `EventState::Consumed`
- `Esc` — send `AppEvent::CloseDialog`

**Confirmation on Enter:**
```rust
let dest_dir = &self.results[selected_idx];
let filename = self.path.get_parent_path().1;
let new_path = if self.path.is_note() {
    dest_dir.append(&VaultPath::note_path_from(&filename))
} else {
    dest_dir.append(&VaultPath::new(&filename))
};
// spawn rename_note or rename_directory, send EntryMoved or DialogError
```

**Steps:**
- [ ] Write test: `MoveDialog::new(some_path, vault_arc, tx)` initializes `results` as empty (load not yet complete) and does not panic
- [ ] Verify test fails (or is ignored): `cd /Users/nhormazabal/development/personal/kimun/tui && cargo test`
- [ ] Implement full `MoveDialog` in `src/components/dialogs/move_dialog.rs`: `new()`, `schedule_load()`, `schedule_filter()`, `handle_input()`, `render()`
- [ ] Verify tests pass: `cd /Users/nhormazabal/development/personal/kimun/tui && cargo test`
- [ ] Commit: `git commit -m "feat: implement MoveDialog with fuzzy directory picker"`

---

### Task 7: Sidebar wiring

**Goal:** Expose `current_dir` from `SidebarComponent` and intercept the three new shortcuts in `FileListComponent::handle_input`.

**Files:**
- `src/components/sidebar.rs` — add `pub fn current_dir(&self) -> &VaultPath`
- `src/components/file_list.rs` — add three new `ActionShortcuts` arms inside the existing `Some(action)` match block, before the `_ => {}` wildcard

**`current_dir` accessor** (field already exists as `self.current_dir: VaultPath`):
```rust
pub fn current_dir(&self) -> &VaultPath {
    &self.current_dir
}
```

**New arms in `FileListComponent::handle_input`** (inside `match self.key_bindings.get_action(&combo)`, before `_ => {}`):
```rust
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

**Steps:**
- [ ] Write test in `src/components/file_list.rs` (or a dedicated test module): construct a `FileListComponent` with a keybindings map that maps `Ctrl+Shift+D` to `DeleteEntry`, push a note entry, select it, call `handle_input` with the Ctrl+Shift+D key event, and assert that `ShowDeleteDialog` was sent on the channel
- [ ] Verify test fails: `cd /Users/nhormazabal/development/personal/kimun/tui && cargo test`
- [ ] Implement `pub fn current_dir` on `SidebarComponent`; add the three new `ActionShortcuts` arms to `FileListComponent::handle_input`
- [ ] Verify tests pass: `cd /Users/nhormazabal/development/personal/kimun/tui && cargo test`
- [ ] Commit: `git commit -m "feat: wire delete/rename/move shortcuts in sidebar file list"`

---

### Task 8: EditorScreen wiring

**Goal:** Add `Focus::Dialog` variant, `active_dialog` field, shortcut interception, AppEvent handling, `on_entry_op` helper, and dialog overlay rendering in `EditorScreen`.

**Files:**
- `src/app_screen/editor.rs` — multiple additions throughout the file

**Changes in detail:**

1. **`Focus` enum** — add `Dialog` variant:
   ```rust
   enum Focus {
       Sidebar,
       Editor,
       NoteBrowser,
       Dialog,
   }
   ```

2. **`EditorScreen` struct** — add field:
   ```rust
   active_dialog: Option<ActiveDialog>,
   ```
   Initialize as `None` in `EditorScreen::new()`.

3. **Shortcut interception in `handle_input`** — add three arms in the existing `match self.settings.key_bindings.get_action(&combo)` block (alongside the other `Some(ActionShortcuts::...)` arms):
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

4. **Focus routing in `handle_input`** — add `Focus::Dialog` arm to the final `match self.focus` block:
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

5. **Mouse swallowing** — change:
   ```rust
   if matches!(self.focus, Focus::NoteBrowser) {
   ```
   to:
   ```rust
   if matches!(self.focus, Focus::NoteBrowser | Focus::Dialog) {
   ```

6. **`handle_app_message`** — add these arms before the `other => Some(other)` fallthrough:
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
       self.active_dialog = Some(ActiveDialog::Move(
           MoveDialog::new(path, self.vault.clone(), tx.clone())
       ));
       self.focus = Focus::Dialog;
       None
   }
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

7. **`on_entry_op` helper** — new `async fn` on `EditorScreen`:
   ```rust
   async fn on_entry_op(&mut self, from: VaultPath, tx: &AppTx) {
       self.active_dialog = None;
       self.focus = Focus::Editor;
       if from == self.path {
           self.try_save().await;
           let parent = self.path.get_parent_path().0;
           tx.send(AppEvent::OpenScreen(ScreenEvent::OpenBrowse(self.vault.clone(), parent))).ok();
       } else {
           let dir = self.sidebar.current_dir().clone();
           self.navigate_sidebar(dir, tx).await;
       }
   }
   ```

8. **`render()`** — update exhaustive `match self.focus` expressions:
   - `focus_label`: add `Focus::Dialog => "DIALOG"`
   - `hints`: add `Focus::Dialog => vec![]`
   - After the existing `note_browser` modal overlay block, add:
     ```rust
     if let Some(dialog) = &mut self.active_dialog {
         match dialog {
             ActiveDialog::Delete(d) => d.render(f, f.area(), &self.theme, true),
             ActiveDialog::Rename(d) => d.render(f, f.area(), &self.theme, true),
             ActiveDialog::Move(d)   => d.render(f, f.area(), &self.theme, true),
         }
     }
     ```

**Steps:**
- [ ] Implement all changes listed above in `src/app_screen/editor.rs`; add necessary `use` imports for `ActiveDialog`, `DeleteConfirmDialog`, `RenameDialog`, `MoveDialog`
- [ ] Verify builds: `cd /Users/nhormazabal/development/personal/kimun/tui && cargo check`
- [ ] Run full test suite: `cd /Users/nhormazabal/development/personal/kimun/tui && cargo test`
- [ ] Commit: `git commit -m "feat: wire dialog system into EditorScreen"`
