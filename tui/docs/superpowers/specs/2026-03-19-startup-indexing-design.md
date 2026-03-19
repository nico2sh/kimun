# Startup Indexing — Design Spec

**Date:** 2026-03-19
**Status:** Approved

## Problem

When the app starts with a configured vault, the index may be stale or missing. Currently `init_and_validate` is never called from the TUI, so notes added or changed since the last settings-triggered reindex are invisible until the user manually triggers a reindex from settings.

## Goal

On every app start, if a vault path is configured, run `vault.init_and_validate()` once. Show the same progress dialog used in settings while it runs, then auto-dismiss and continue normal startup.

## Approach

Extend `StartScreen` with an optional overlay that reuses the existing `IndexingProgressState` + throbber pattern from `SettingsScreen`.

---

## Architecture

### Shared module: `components/indexing.rs`

Move `IndexingProgressState` and `spawn_running` out of `app_screen/settings.rs` into a new `components/indexing.rs`. Both `StartScreen` and `SettingsScreen` import from there.

```
components/
  indexing.rs   ← IndexingProgressState, spawn_running (moved from settings.rs)
  mod.rs        ← pub mod indexing
```

### `StartScreen` changes

New fields:
```rust
vault: Option<Arc<NoteVault>>,
overlay: Option<IndexingProgressState>,
```

`on_enter`:
- If `settings.workspace_dir` is `Some(path)`:
  - Construct `NoteVault::new(&path)`.
  - Spawn async task calling `vault.init_and_validate()`, sends `AppEvent::IndexingDone` on completion.
  - Set `overlay = Some(spawn_running(handle, tx))`.
  - Do **not** send `OpenPath` yet — wait for `IndexingDone`.
- If no vault path: send `OpenPath` immediately (existing behaviour, falls through to settings).

`handle_input`:
- If `overlay` is `Some(Running)`: return `EventState::Consumed` (block all input).
- Otherwise: existing `NotConsumed`.

`handle_app_message`:
- Intercept `AppEvent::IndexingDone(_)`:
  - Clear `overlay` (set to `None`).
  - Send `AppEvent::OpenPath(last_path)` to continue startup (same path logic as today).
  - Return `None` (consumed).
- All other messages: return `Some(msg)` (pass through).

`render`:
- If `overlay` is `Some(Running)`: render centered `IndexingProgress` dialog (throbber + "Initializing vault…").
- Otherwise: existing render.

---

## Event flow

```
App starts
  └─ StartScreen::on_enter
       ├─ no vault → OpenPath (→ OpenSettings)
       └─ vault configured
            ├─ spawn init_and_validate task
            ├─ overlay = Running
            └─ [throbber visible, input blocked]
                  ↓
            task sends IndexingDone(Ok | Err)
                  ↓
            StartScreen::handle_app_message(IndexingDone)
            ├─ overlay = None
            └─ send OpenPath(last_path)   →  normal browse/editor startup
```

---

## Error handling

A failed `init_and_validate` at startup is non-fatal. The overlay is cleared and startup continues with `OpenPath` regardless. The error is logged in debug builds. No error dialog is shown — the user did not trigger this action and can always reindex manually from settings.

---

## No new AppEvent variants

`AppEvent::IndexingDone(Result<Duration, String>)` already exists and already flows through `handle_app_message` in `main.rs`. No changes to the event enum.

---

## Render detail

The progress dialog matches settings visually:
- Same `fixed_centered_rect(44, 5, f.area())` popup.
- Title: `"Indexing"`.
- Body while running: throbber + label `"  Initializing vault…"`.
- No Done/Failed state is rendered (auto-dismissed on completion).

`fixed_centered_rect` and `centered_rect` helpers stay in `settings.rs` (only used there for other overlays). `StartScreen` uses its own inline rect or imports the helper — preference is to keep `fixed_centered_rect` in `settings.rs` and duplicate the tiny helper in `start.rs` to avoid cross-module coupling on a layout utility.

---

## Testing

| Test | Assertion |
|------|-----------|
| `on_enter` with no vault | no overlay, `OpenPath` sent immediately |
| `on_enter` with vault | overlay is `Running`, no `OpenPath` sent yet |
| `handle_app_message(IndexingDone(Ok))` | overlay cleared, `OpenPath` sent |
| `handle_app_message(IndexingDone(Err))` | overlay cleared, `OpenPath` sent |
| `handle_input` while `Running` | returns `Consumed`, no message sent |

---

## Files changed

| File | Change |
|------|--------|
| `src/components/indexing.rs` | New — `IndexingProgressState`, `spawn_running` |
| `src/components/mod.rs` | Add `pub mod indexing` |
| `src/app_screen/settings.rs` | Import `IndexingProgressState`, `spawn_running` from `components::indexing` |
| `src/app_screen/start.rs` | Add overlay field, `on_enter` indexing logic, `handle_app_message`, `handle_input` guard, render |
