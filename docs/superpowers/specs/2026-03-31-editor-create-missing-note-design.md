# Editor: Create Missing Note Dialog

**Goal:** When the user follows a wiki link or note link to a note that doesn't exist, show a confirmation dialog asking whether to create it, instead of silently showing a flash message or navigating away.

**Architecture:** A new `CreateNoteDialog` follows the existing dialog pattern in `dialogs/`. It is triggered from two places in `EditorScreen`: `follow_link()` when `open_or_search` returns no results, and `open_path()` when `get_note_text` returns `VaultError::FSError(FSError::VaultPathNotFound)`. On confirm, the dialog calls `load_or_create_note` and sends `AppEvent::OpenPath(path)`; the existing `OpenPath` handler in `handle_app_message` clears the dialog and loads the now-existing note. On cancel, sends `AppEvent::CloseDialog`.

**Tech stack:** Rust/Ratatui, existing `NoteVault::load_or_create_note`, `kimun_core::error::{VaultError, FSError}`.

---

## 1. `CreateNoteDialog` (`tui/src/components/dialogs/create_note_dialog.rs`)

New file, modelled on `delete_dialog.rs`.

```rust
pub struct CreateNoteDialog {
    pub path: VaultPath,
    pub vault: Arc<NoteVault>,
    pub filename: String,      // path.to_string() — full vault-relative path for display
    pub error: Option<String>,
}
```

### `new(path, vault)`

```rust
pub fn new(path: VaultPath, vault: Arc<NoteVault>) -> Self {
    let filename = path.to_string();
    Self { path, vault, filename, error: None }
}
```

### `handle_key(key, tx) -> EventState`

| Key | Action |
|---|---|
| `Enter` | Spawn async: `vault.load_or_create_note(&path, None).await` → on `Ok` send `AppEvent::OpenPath(path)`; on `Err` send `AppEvent::DialogError(e.to_string())`. Return `Consumed`. |
| `Esc` | Send `AppEvent::CloseDialog`. Return `Consumed`. |
| other | Return `NotConsumed`. |

### `render()` — fixed 52 × 9 popup (10 with error)

```
┌ Create note? ──────────────────────────────────────┐
│                                                    │
│  notes/my-note.md                                  │
│────────────────────────────────────────────────────│
│  Note doesn't exist.                               │
│                                                    │
│  [Enter] Create   [Esc] Cancel                     │
│                                                    │
└────────────────────────────────────────────────────┘
```

Row layout (inside border):
- Row 0: spacer
- Row 1: `render_path_row(filename)`
- Row 2: `render_separator`
- Row 3: `"  Note doesn't exist."` in `fg_muted`
- Row 4: spacer
- Row 5: `"  [Enter] Create   [Esc] Cancel"` in `fg_muted`
- Row 6: `render_error_row` (optional, only when `error.is_some()`)
- Row 7: remainder (`Min(0)`)

Border style uses the theme's default (no special colour — this is not a destructive action).

---

## 2. `dialogs/mod.rs` changes

Add to `ActiveDialog` enum:
```rust
CreateNote(CreateNoteDialog),
```

Add export at top:
```rust
pub use create_note_dialog::CreateNoteDialog;
```

Add module declaration:
```rust
pub mod create_note_dialog;
```

Add arm to `set_error`:
```rust
ActiveDialog::CreateNote(d) => d.error = Some(msg),
```

Add arms to `handle_input` and `render` dispatch blocks:
```rust
ActiveDialog::CreateNote(d) => d.handle_key(*key, tx),
ActiveDialog::CreateNote(d) => d.render(f, rect, theme, focused),
```

---

## 3. `EditorScreen` changes (`tui/src/app_screen/editor.rs`)

### Imports to add

```rust
use kimun_core::error::{VaultError, FSError};
use crate::components::dialogs::CreateNoteDialog;
```

### `follow_link()` — replace the "empty results" flash with a dialog

**Before:**
```rust
Ok(results) if results.is_empty() => {
    self.key_flash = Some((format!("Not found: {target}"), std::time::Instant::now()));
}
```

**After:**
```rust
Ok(results) if results.is_empty() => {
    self.pre_dialog_focus = Some(self.focus);
    self.active_dialog = Some(ActiveDialog::CreateNote(CreateNoteDialog::new(
        path,
        self.vault.clone(),
    )));
    self.focus = Focus::Dialog;
}
```

(`path` is the `VaultPath::note_path_from(target_clean)` already computed earlier in `follow_link`.)

### `open_path()` — replace blanket error navigation with dialog on VaultPathNotFound

**Before:**
```rust
Err(e) => {
    log::error!("Failed to read note {}: {e}", self.path);
    let parent = self.path.get_parent_path().0;
    tx.send(AppEvent::OpenScreen(ScreenEvent::OpenBrowse(self.vault.clone(), parent))).ok();
    return;
}
```

**After:**
```rust
Err(e) => {
    if matches!(e, VaultError::FSError(FSError::VaultPathNotFound { .. })) {
        self.pre_dialog_focus = Some(self.focus);
        self.active_dialog = Some(ActiveDialog::CreateNote(CreateNoteDialog::new(
            self.path.clone(),
            self.vault.clone(),
        )));
        self.focus = Focus::Dialog;
    } else {
        log::error!("Failed to read note {}: {e}", self.path);
        let parent = self.path.get_parent_path().0;
        tx.send(AppEvent::OpenScreen(ScreenEvent::OpenBrowse(
            self.vault.clone(),
            parent,
        ))).ok();
    }
    return;
}
```

### `handle_app_message()` — clear dialog before handling `OpenPath`

`OpenPath` is sent by the dialog on successful creation. Clearing the dialog first ensures the overlay is gone before the note loads.

**Before:**
```rust
AppEvent::OpenPath(path) => {
    if path.is_note() {
        self.open_path(path, tx).await;
        self.focus_editor();
    } else {
        self.navigate_sidebar(path, tx).await;
    }
    None
}
```

**After:**
```rust
AppEvent::OpenPath(path) => {
    self.restore_focus();  // dismiss CreateNote dialog (or any other active dialog)
    if path.is_note() {
        self.open_path(path, tx).await;
        self.focus_editor();
    } else {
        self.navigate_sidebar(path, tx).await;
    }
    None
}
```

---

## 4. No new `AppEvent` variants

The dialog reuses:
- `AppEvent::OpenPath(VaultPath)` — signals successful creation; handled by EditorScreen
- `AppEvent::CloseDialog` — signals cancel; handled by EditorScreen's existing arm
- `AppEvent::DialogError(String)` — signals creation failure; handled by `set_error`

---

## 5. Testing

Unit tests in `create_note_dialog.rs` following the `delete_dialog.rs` pattern:

- `esc_sends_close_dialog` — construct dialog (real vault via tempdir), send Esc, assert `CloseDialog` received and `EventState::Consumed` returned.
- `new_does_not_panic` — compile-time/channel-only smoke test (no real vault needed).
- `new_with_vault_does_not_panic` (ignored) — real vault construction, assert `error` is `None`.

No new tests in `editor.rs` — dialog rendering and focus paths are covered by compile-time checks and manual testing, matching existing practice.
