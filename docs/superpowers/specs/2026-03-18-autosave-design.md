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
- `get_text() -> String` — joins `self.text_area.lines()` with `"\n"` (no trailing newline)
- `mark_saved(text: String)` — stores the snapshot as `last_saved_text`
- `is_dirty() -> bool` — returns `get_text() != self.last_saved_text`

**Correctness invariant:** `set_text()` MUST call `self.mark_saved(self.get_text())` — using the *reconstructed* form, not the original string — so that `last_saved_text` and future `get_text()` calls always use the same representation. Storing the original `text` parameter in `mark_saved` would cause `is_dirty()` to return true immediately after load if the original note had a trailing newline (which `text.lines()` strips).

---

### 3. `AppMessage::Autosave` (`tui/src/components/app_message.rs`)

Add the `Autosave` variant to `AppMessage`.

Routing: when `Autosave` arrives via the `select!` arm's `other => tx.send(other)` re-queue path, the drain loop's `other =>` arm forwards it to `screen.handle_app_message`. One frame of latency; harmless for autosave. No additional changes needed in `main.rs`.

---

### 4. `AppScreen::on_exit` trait method (`tui/src/app_screen/mod.rs`)

Add a lifecycle hook:

```rust
async fn on_exit(&mut self, _tx: &AppTx) {}
```

Default implementation is a no-op. `EditorScreen` overrides it to call `try_save().await`.

---

### 5. `main.rs` quit handling (`tui/src/main.rs`)

`AppMessage::Quit` is currently handled in **two** places:

1. **Drain loop** (top of loop): `AppMessage::Quit => return Ok(())`
2. **`select!` arm**: `AppMessage::Quit => return Ok(())`

Both must call `on_exit` before returning. The cleanest fix is to **remove** the explicit `Quit` match from the `select!` arm entirely so it falls into `other => tx.send(other).ok()` and is re-queued for the drain loop. The drain loop then becomes the single authoritative exit path:

```rust
// drain loop — single exit point for Quit
AppMessage::Quit => {
    if let Some(screen) = app.current_screen.as_mut() {
        screen.on_exit(&tx).await;
    }
    return Ok(());
}
```

`AppMessage::Quit` in a screen's `handle_app_message` is still unreachable; `on_exit` is the correct quit-time hook.

---

### 6. Autosave timer task (`tui/src/app_screen/editor.rs`)

`EditorScreen` gains:

```rust
autosave_handle: Option<tokio::task::JoinHandle<()>>,
```

The timer is spawned **at the end of `open_path`** (after content is loaded), not in `on_enter`. Since `on_enter` delegates to `open_path`, this covers both the initial load and note switching with a single spawn site. Before spawning, any existing handle is aborted:

```rust
if let Some(h) = self.autosave_handle.take() {
    h.abort();
}
let interval_secs = self.settings.autosave_interval_secs;
let tx2 = tx.clone();
self.autosave_handle = Some(tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
    interval.tick().await; // skip immediate first tick
    loop {
        interval.tick().await;
        if tx2.send(AppMessage::Autosave).is_err() {
            break;
        }
    }
}));
```

**Task cleanup:** Implement `Drop for EditorScreen` to abort the timer when the screen is replaced by a different screen:

```rust
impl Drop for EditorScreen {
    fn drop(&mut self) {
        if let Some(handle) = self.autosave_handle.take() {
            handle.abort();
        }
    }
}
```

---

### 7. Save triggers (`tui/src/app_screen/editor.rs`)

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
| Open new note | `open_path()` — before loading new content, before spawning new timer |
| Quit | `on_exit()` — called by `main.rs` drain loop before `return Ok(())` |

---

### 8. Unsaved indicator (`tui/src/app_screen/editor.rs`)

In `render`, change the editor block title:

```rust
let title = if self.editor.is_dirty() { "Editor [+]" } else { "Editor" };
```

Appears and disappears reactively each frame.

---

## Data Flow

```
tokio::time::interval (N secs)
        │
        ▼
AppMessage::Autosave ──► select! other arm re-queues ──► drain loop other arm
                                                               │
                                                      handle_app_message
                                                               │
                                                         is_dirty()?
                                                          yes │
                                                              ▼
                                                     vault.save_note(path, text)
                                                              │
                                                         mark_saved(text)

open_path(new_path)  ──► try_save() ──► vault.save_note (if dirty)
                     ──► load new content
                     ──► set_text() ──► mark_saved(get_text())   [clean state]
                     ──► spawn new timer

AppMessage::Quit     ──► select! other arm re-queues ──► drain loop Quit arm
                     ──► screen.on_exit() ──► try_save() ──► vault.save_note (if dirty)
                     ──► return Ok(())
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
| `tui/src/app_screen/mod.rs` | Add `on_exit` hook to `AppScreen` trait |
| `tui/src/main.rs` | Remove Quit from `select!` arm; call `on_exit` in drain loop before returning |
| `tui/src/app_screen/editor.rs` | Autosave timer, `Drop`, `try_save`, `on_exit`, dirty indicator |
