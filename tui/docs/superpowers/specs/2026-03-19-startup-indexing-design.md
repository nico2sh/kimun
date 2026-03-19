# Startup Indexing — Design Spec

**Date:** 2026-03-19
**Status:** Approved (rev 3)

## Problem

When the app starts with a configured vault, the index may be stale or missing. Currently `init_and_validate` is never called from the TUI, so notes added or changed since the last settings-triggered reindex are invisible until the user manually triggers a reindex from settings.

## Goal

On every app start, if a vault path is configured, run `vault.init_and_validate()` once. Show the same progress dialog used in settings while it runs, then auto-dismiss and continue normal startup.

## Approach

Extend `StartScreen` with an optional overlay that reuses the existing `IndexingProgressState` + throbber pattern from `SettingsScreen`.

---

## Architecture

### Shared module: `components/indexing.rs`

Move the following out of `app_screen/settings.rs` into a new `components/indexing.rs`:
- `IndexingProgressState` (enum + `Drop` impl)
- `spawn_running` (fn)
- `fixed_centered_rect` (layout helper, also needed in `start.rs`)

Both `StartScreen` and `SettingsScreen` import from `components::indexing`.

```
components/
  indexing.rs   ← IndexingProgressState, spawn_running, fixed_centered_rect
  mod.rs        ← pub mod indexing
```

### `StartScreen` changes

**Constructor signature change:**

```rust
pub fn new(settings: AppSettings, vault: Option<Arc<NoteVault>>) -> Self
```

The vault parameter is `Some` only on initial app startup — it is the same `Arc<NoteVault>` already constructed in `App::new`. `Arc` cloning is safe here; both `App` and `StartScreen` hold a reference-counted pointer to the same instance. When `StartScreen` is re-created on `SettingsSaved` / `CloseSettings` transitions via `switch_screen`, the vault parameter is `None`, so no startup indexing runs on those re-entries.

**New fields:**
```rust
vault: Option<Arc<NoteVault>>,
overlay: Option<IndexingProgressState>,
throbber_state: ThrobberState,
```

**`on_enter`:**
- If `self.vault` is `Some(vault)`:
  - Clone the vault, spawn async task: `vault.init_and_validate()` → sends `AppEvent::IndexingDone` on completion.
  - Set `self.overlay = Some(spawn_running(handle, tx))`.
  - Do **not** send `OpenPath` yet — defer until `IndexingDone` is received.
- If `self.vault` is `None`: send `OpenPath(last_path)` immediately (existing behaviour — vault is `None` when no workspace is configured, which `App::new` handles by setting `vault = None` when `NoteVault::new` fails or no path is set).

**`handle_input`:**
- If `self.overlay` is `Some(Running{..})`: return `EventState::Consumed` (block input).
- Otherwise: return `EventState::NotConsumed`.

Note: the global `Ctrl+,` shortcut in `main.rs` bypasses `handle_input` entirely. If the user opens settings while startup indexing is running, `switch_screen` calls `on_exit` (a no-op) then overwrites `app.current_screen = Some(new_screen)`, which **drops** the old `StartScreen` and with it `self.overlay`. `IndexingProgressState::Drop` calls `work.abort()` and `ticker.abort()`. Tokio's `abort()` is cooperative — it posts cancellation but does not guarantee the task stops before `abort()` returns. In the unlikely event that the task completes and sends `IndexingDone` before cancellation takes effect, the event will reach `SettingsScreen`. `SettingsScreen::handle_app_message` already handles `IndexingDone` in all cases (it transitions to `Done`/`Failed` state, dismissible by Enter/Esc), so no corruption occurs. This edge case is benign.

**`on_exit`:**
No explicit action needed. The cleanup mechanism is `Drop` on `IndexingProgressState`, not `on_exit`. In `switch_screen` (main.rs), the sequence is: `on_exit` → `on_enter` of new screen → `app.current_screen = Some(screen)`. The old `StartScreen` is dropped at the assignment on the last line — **after** `on_enter` of the new screen has already run. This means the `StartScreen` and its running tasks remain alive throughout `SettingsScreen::on_enter`. `IndexingProgressState::Drop` calls `work.abort()` / `ticker.abort()` only after `switch_screen` returns. This is acceptable: the benign abort-race discussed in `handle_input` applies in the same way. Note: if a future change explicitly drops the old screen before calling `on_enter` (e.g. to avoid both screens being alive simultaneously), the abort would then fire earlier — but the current behaviour already provides the safety guarantee described above.

**`handle_app_message`:**
- Intercept `AppEvent::IndexingDone(_)`:
  - Set `self.overlay = None`.
  - Compute path: `self.settings.last_paths.last().map_or_else(VaultPath::root, |p| p.to_owned())`.
  - Send `AppEvent::OpenPath(path)`.
  - Return `None` (consumed).
- All other messages: return `Some(msg)` (pass through).

`self.settings` is safe to read here. It is a snapshot frozen at `StartScreen` construction. The only event that mutates `app.settings` is `SettingsSaved`, which triggers an immediate `OpenScreen(Start)` — replacing `StartScreen` before any `IndexingDone` from the current run could arrive. The single-threaded `run_app` event loop processes one event at a time, so no interleaving is possible.

**`render`:**
- If `self.overlay` is `Some(Running{..})`:
  - Call `self.throbber_state.calc_next()` on every render tick (required to animate the throbber).
  - Render centered dialog: `fixed_centered_rect(44, 5, f.area())`, throbber + label `"  Initializing vault…"`.
- No Done/Failed state rendered (overlay is cleared to `None` before the next render after `IndexingDone`).
- Otherwise: existing render.

### `App::new` change

Pass vault to `StartScreen`:
```rust
current_screen: Some(Box::new(StartScreen::new(settings.clone(), vault.clone()))),
```

### `switch_screen` change (main.rs)

Pass `None` vault when re-creating `StartScreen` on settings transitions:
```rust
ScreenEvent::Start => Box::new(StartScreen::new(app.settings.clone(), None)),
```

---

## Event flow

```
App starts (App::new)
  └─ vault = NoteVault::new(workspace)  [or None if not configured / construction failed]
  └─ StartScreen::new(settings, vault)

run_app
  └─ StartScreen::on_enter
       ├─ vault = None → OpenPath (→ OpenSettings or OpenBrowse)
       └─ vault = Some(v)
            ├─ spawn init_and_validate task
            ├─ overlay = Running
            └─ [throbber visible, input blocked]
                  ↓
            task sends IndexingDone(Ok | Err)
                  ↓
            StartScreen::handle_app_message(IndexingDone)
            ├─ overlay = None
            └─ send OpenPath(last_path)  →  normal browse/editor startup

User presses Ctrl+, during startup indexing
  └─ switch_screen called in main loop
       └─ StartScreen::on_exit (no-op)
       └─ SettingsScreen::on_enter  ← old StartScreen still alive here
       └─ app.current_screen = Some(SettingsScreen) — drops StartScreen
            └─ IndexingProgressState::Drop: work.abort(), ticker.abort()
  └─ IndexingDone may arrive at SettingsScreen in rare abort-race case
       └─ SettingsScreen handles it gracefully (Done/Failed overlay, dismissible)
```

---

## Error handling

A failed `init_and_validate` at startup is non-fatal. On `IndexingDone(Err(_))`, the overlay is cleared and startup continues with `OpenPath` regardless. `NoteVault::new` failure is handled upstream in `App::new` — if it fails, `app.vault` is `None` and `StartScreen` receives `None`, so no indexing is attempted.

---

## No new AppEvent variants

`AppEvent::IndexingDone(Result<Duration, String>)` already exists and flows through `handle_app_message` in `main.rs`. No changes to the event enum.

---

## Testing

| Test | Assertion |
|------|-----------|
| `on_enter` with `vault = None` | overlay remains `None`; `OpenPath` sent on tx |
| `on_enter` with `vault = Some(v)` | overlay is `Some(Running{..})`; tx is **empty** (no `OpenPath` sent yet) |
| `handle_app_message(IndexingDone(Ok(...)))` | overlay set to `None`; `OpenPath` sent on tx |
| `handle_app_message(IndexingDone(Err(...)))` | overlay set to `None`; `OpenPath` sent on tx |
| `handle_input` while `overlay = Some(Running)` | returns `Consumed`; tx empty |
| `handle_input` while `overlay = None` | returns `NotConsumed` |
| `IndexingProgressState::Drop` aborts handles | construct `Running` with a never-completing task; drop it; yield once; assert `JoinHandle::is_finished()` |

---

## Files changed

| File | Change |
|------|--------|
| `src/components/indexing.rs` | New — `IndexingProgressState`, `spawn_running`, `fixed_centered_rect` |
| `src/components/mod.rs` | Add `pub mod indexing` |
| `src/app_screen/settings.rs` | Import from `components::indexing`; remove `IndexingProgressState`, `spawn_running`, `fixed_centered_rect` |
| `src/app_screen/start.rs` | New constructor signature, `vault`/`overlay`/`throbber_state` fields, `on_enter`, `handle_app_message`, `handle_input` guard, render with `calc_next()` |
| `src/app.rs` | Pass `vault.clone()` to `StartScreen::new` |
| `src/main.rs` | Pass `None` vault in `switch_screen` for `ScreenEvent::Start` |
