# Autosave Design — TUI Editor

**Date:** 2026-03-18
**Scope:** `tui/` crate only
**Status:** Approved

---

## Overview

Add an autosave system to the TUI editor screen that periodically saves changed content, saves when navigating to a new note, saves before quitting, and shows an unsaved-changes indicator in the UI.

---

## Components

### 1. Settings (`tui/src/settings/mod.rs`)

Add `autosave_interval_secs: u64` to `AppSettings`:

```rust
#[serde(default = "default_autosave_interval")]
pub autosave_interval_secs: u64,
```

```rust
fn default_autosave_interval() -> u64 { 5 }
```

Serializes transparently to/from `config.toml`. Missing key defaults to `5`.

---

### 2. Dirty tracking (`tui/src/components/text_editor.rs`)

Add `last_saved_text: String` to `TextEditorComponent`.

New methods:
- `get_text() -> String` — joins `self.text_area.lines()` with `"\n"`
- `mark_saved(text: String)` — stores the snapshot as `last_saved_text`
- `is_dirty() -> bool` — returns `get_text() != self.last_saved_text`

`set_text()` calls `mark_saved` after loading so a freshly-opened note starts clean.

---

### 3. `AppMessage::Autosave` (`tui/src/components/app_message.rs`)

Add the `Autosave` variant to `AppMessage`. The main loop's `other =>` arm already forwards unknown messages to the current screen — no changes needed in `main.rs`.

---

### 4. Autosave timer task (`tui/src/app_screen/editor.rs`)

`EditorScreen` gains:

```rust
autosave_handle: Option<tokio::task::JoinHandle<()>>,
```

On `on_enter`, spawn:

```rust
let mut interval = tokio::time::interval(Duration::from_secs(n));
interval.tick().await; // skip the immediate first tick
loop {
    interval.tick().await;
    tx.send(AppMessage::Autosave).ok();
}
```

Store the handle. Abort it in `open_path` before spawning a fresh one (interval resets when a new note loads).

---

### 5. Save triggers (`tui/src/app_screen/editor.rs`)

Private method:

```rust
async fn try_save(&mut self) {
    if self.editor.is_dirty() {
        let text = self.editor.get_text();
        if self.vault.save_note(&self.path, &text).await.is_ok() {
            self.editor.mark_saved(text);
        }
    }
}
```

Called from three places:

| Trigger | Location |
|---------|----------|
| Periodic timer | `handle_app_message(AppMessage::Autosave)` |
| Open new note | `open_path()` — before loading new content |
| Quit | `handle_event` intercepts `ActionShortcuts::Quit`, calls `try_save().await`, then sends `AppMessage::Quit` |

Note: `handle_event` is currently `fn` (sync). The quit intercept requires it to become `async fn` or the save must be dispatched differently. Since the `AppScreen` trait declares `handle_event` as sync, the quit path will spawn a task that saves then sends `Quit`, rather than awaiting inline.

**Quit save pattern:**
```rust
// Inside handle_event, on Quit action:
let vault = self.vault.clone();
let path = self.path.clone();
let text = self.editor.get_text();
let dirty = self.editor.is_dirty();
let tx2 = tx.clone();
tokio::spawn(async move {
    if dirty {
        vault.save_note(&path, &text).await.ok();
    }
    tx2.send(AppMessage::Quit).ok();
});
return EventState::Consumed;
```

This replaces the global Ctrl+Q handler in `main.rs` for the editor screen — the editor intercepts Quit, saves, then re-sends Quit. The main loop's global handler must be removed or the editor must intercept before it reaches the global handler.

**Adjustment:** The global Quit handler in `main.rs` fires before the screen sees the event. To allow the editor to intercept, the global handler needs to forward Quit to the current screen first when on the editor screen, or the editor needs a different save-on-quit mechanism.

**Simpler approach for quit:** Use `handle_app_message` instead. The main loop sends `AppMessage::Quit` through the message bus — add a `Quit` arm to `EditorScreen::handle_app_message` that calls `try_save().await` then returns `Some(AppMessage::Quit)` to let the main loop handle it.

Since `handle_app_message` is async, this works cleanly without spawning a task.

---

### 6. Unsaved indicator (`tui/src/app_screen/editor.rs`)

In `render`, change the editor block title:

```rust
let title = if self.editor.is_dirty() { "Editor [+]" } else { "Editor" };
let editor_block = Block::default()
    .title(title)
    // ...
```

Appears and disappears reactively each frame.

---

## Data Flow

```
tokio::time::interval (N secs)
        │
        ▼
AppMessage::Autosave ──► EditorScreen::handle_app_message
                                │
                         is_dirty()?
                          yes │
                              ▼
                     vault.save_note(path, text)
                              │
                         mark_saved(text)

open_path(new_path)  ──► try_save() ──► vault.save_note (if dirty)
                     ──► load new content ──► mark_saved

AppMessage::Quit     ──► EditorScreen::handle_app_message
                     ──► try_save() ──► vault.save_note (if dirty)
                     ──► return Some(AppMessage::Quit)
```

---

## Error Handling

Save errors are silently ignored (`is_ok()` check). The dirty flag is only cleared on success, so a failed save will be retried on the next autosave tick.

---

## Files Modified

| File | Change |
|------|--------|
| `tui/src/settings/mod.rs` | Add `autosave_interval_secs` field + default fn |
| `tui/src/components/text_editor.rs` | Add dirty tracking fields and methods |
| `tui/src/components/app_message.rs` | Add `AppMessage::Autosave` variant |
| `tui/src/app_screen/editor.rs` | Autosave timer, `try_save`, quit intercept, dirty indicator |
