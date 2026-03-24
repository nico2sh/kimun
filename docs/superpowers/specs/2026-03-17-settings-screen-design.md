# Settings Screen Design

**Date:** 2026-03-17
**Status:** Approved
**Scope:** TUI settings screen (`tui/src/app_screen/settings.rs` and related components)

---

## Overview

Replace the placeholder `SettingsScreen` with a fully functional settings screen. The screen uses a left-sidebar navigation + right content panel layout. It exposes three settings sections: Theme, Vault Path, and Indexing.

**Save behavior:** Settings are saved only when the user explicitly chooses Save. If settings have not changed, Esc closes without any dialog. If settings have changed, Esc shows a "Save / Discard" confirmation dialog. On Save, if the vault path was changed, a full reindex is run automatically before closing (same progress overlay, auto-closes when done).

---

## Layout

```
┌─────────────────────────────────────────────────────────────────────┐
│ Settings                                             ESC: Back       │
├──────────────────┬──────────────────────────────────────────────────┤
│                  │                                                   │
│ > Theme          │  (content panel for selected section)            │
│   Vault          │                                                   │
│   Indexing       │                                                   │
│                  │                                                   │
└──────────────────┴──────────────────────────────────────────────────┘
```

Tab switches focus between the left sidebar and the right content panel. Up/Down (or `j`/`k`) navigate sections in the sidebar. Esc closes settings — with a Save/Discard confirmation if settings have changed, or immediately without saving if nothing changed.

---

## Architecture

### Approach

Component-based, matching the existing `EditorScreen` + `SidebarComponent` + `TextEditorComponent` pattern.

### `SettingsScreen`

**File:** `tui/src/app_screen/settings.rs`

```rust
pub struct SettingsScreen {
    settings: AppSettings,           // mutable local clone
    initial_settings: AppSettings,   // snapshot at open, for change detection
    theme: Theme,                    // current active theme; updated live as user changes theme
    section: SettingsSection,        // currently highlighted sidebar item
    focus: SettingsFocus,            // Sidebar | Content
    theme_picker: ThemePicker,
    vault_section: VaultSection,
    indexing_section: IndexingSection,
    overlay: Overlay,
    pending_save_after_index: bool,  // true when vault path changed; auto-close on IndexingDone(Ok)
}

enum SettingsSection { Theme, Vault, Indexing }
enum SettingsFocus { Sidebar, Content }
```

**`AppSettings` must derive `PartialEq`** — used to detect whether settings have changed (`settings != initial_settings`). `KeyBindings` must also derive `PartialEq`. Add derives if not already present.

**Construction:**
```rust
pub fn new(settings: AppSettings) -> Self
```
Receives a clone of `App::settings`. Stores it both as `settings` (mutable) and `initial_settings` (immutable snapshot). `theme` is populated via `settings.get_theme()`. After every `ThemePicker` event, `SettingsScreen` must call `self.theme = self.settings.get_theme()` so the live preview updates immediately.

`App::settings` is `Send` (`AppSettings` derives no non-Send types: `Vec<VaultPath>`, `Option<PathBuf>`, `String`, `KeyBindings` are all Send-safe).

**Three call sites** must be updated when the constructor signature changes:
1. `main.rs` line 80: `SettingsScreen::new()` → `SettingsScreen::new(app.settings.clone())`
2. `app_screen/mod.rs` test at line 43: add `AppSettings::default()` argument
3. `app_screen/mod.rs` test at line 50: add `AppSettings::default()` argument

**Esc behavior (no overlay active):**
- If `settings == initial_settings`: send `AppMessage::CloseSettings` (no save, no dialog).
- If `settings != initial_settings`: set `overlay = Overlay::ConfirmSave { focused_button: SaveButton::Save }`.

**ConfirmSave — Save path:**
1. If `settings.workspace_dir != initial_settings.workspace_dir`: set `pending_save_after_index = true`, set `overlay = Overlay::IndexingProgress(Running(handle))`, spawn full reindex task (same as `TriggerFullReindex`). This spawn happens inside the synchronous `handle_event` — `tx` must be cloned before the closure: `let tx = tx.clone();` (same requirement as the `TriggerFastReindex`/`TriggerFullReindex` paths). On `IndexingDone(Ok)` with `pending_save_after_index == true`: call `settings.report_indexed()`, then `settings.save_to_disk().ok()` (error is non-fatal — log it if a logger is available, then proceed), send `AppMessage::SettingsSaved(settings)`, auto-close (no OK button).
2. Otherwise: call `settings.save_to_disk().ok()` (non-fatal — same error policy), send `AppMessage::SettingsSaved(settings)`.

**`save_to_disk` error policy:** Treat as non-fatal. Call `.ok()` to discard the `eyre::Result`. The settings are still applied to `App::settings` via `SettingsSaved` for the current session; they simply will not persist across restarts. Proper error display (e.g., a toast or error overlay) is out of scope for this spec.

**ConfirmSave — Discard path:** send `AppMessage::CloseSettings`. All in-memory mutations to `settings` are discarded — including any `report_indexed()` calls from user-triggered reindexes during this session. `App::settings` is not updated.

`AppMessage::SettingsSaved(settings)` — main loop updates `App::settings` and navigates back.
`AppMessage::CloseSettings` — main loop navigates back without updating `App::settings`.
Both navigate back by sending `OpenPath` with `last_paths.last()` or `VaultPath::root()` (pattern used in `start.rs`).

**`handle_app_message` routing:** `SettingsScreen::handle_app_message` must include a fallback `other => Some(other)` arm to pass unrecognized variants back to the main loop. This preserves the existing behavior of `FocusEditor` / `FocusSidebar` variants (routed via the main loop's `other =>` catch-all) and ensures future variants do not silently disappear.

**Event routing:** When `focus == Content`, key events are forwarded to the active section component. When `focus == Sidebar`, Up/Down change the selected section; Tab moves focus to the content panel.

**After forwarding to a sub-component**, `SettingsScreen` reads back any state changes (see ThemePicker and IndexingSection sections below) and syncs them into `self.settings`.

---

## Sub-Components

All three implement the `Component` trait:
- `fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState`
- `fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool)`

They live under a new `tui/src/components/settings/` submodule with a `mod.rs`.

### `ThemePicker`

**File:** `tui/src/components/settings/theme_picker.rs`

Renders the full list from a `Vec<Theme>` passed at construction time (`ThemePicker::new(themes: Vec<Theme>, active_name: &str)`). Up/Down moves `ListState` selection — **no Enter required**. The highlighted theme is applied as a live preview on every Up/Down keystroke.

**Live preview:** After every `handle_event` call on `ThemePicker`, `SettingsScreen` reads `theme_picker.selected_theme_name()` and immediately calls `self.settings.set_theme(name.to_string())` then `self.theme = self.settings.get_theme()`. Because `self.theme` is passed to every sub-component's `render` call, the entire settings screen re-renders with the new colors on the next draw cycle — borders, text, highlights, all change in real time as the user moves through the list.

**Change detection:** `self.settings.theme` is mutated on every navigation step. If the user browses back to the original theme name, `settings == initial_settings` will be `true` again and Esc will close without a dialog. If they land on a different theme, the ConfirmSave dialog appears as normal.

**`ThemePicker` does NOT hold a reference to `AppSettings`.** It exposes `pub fn selected_theme_name(&self) -> &str`. The parent (`SettingsScreen`) owns all settings mutations — same "parent reads child state" pattern as `EditorScreen`. Use `self.settings.set_theme()`, not direct field assignment.

```
┌─ Theme ──────────────────────────────────────────────┐
│  ● Gruvbox Dark                                      │
│    Gruvbox Light                                     │
│    Catppuccin Mocha                                  │
│    Tokyo Night                                       │
│    Nord                                              │
└──────────────────────────────────────────────────────┘
```

**State:** `list_state: ListState`, `themes: Vec<Theme>` (loaded once at construction).

### `VaultSection`

**File:** `tui/src/components/settings/vault_section.rs`

Displays the current vault path. Enter or `b` sends `AppMessage::OpenFileBrowser` via `tx`. `SettingsScreen::handle_app_message` intercepts this message (returns `None`) and sets `self.overlay = Overlay::FileBrowser(...)`.

**Ownership model:** `VaultSection` stores its own `current_path: Option<PathBuf>` copy. `SettingsScreen` calls `vault_section.set_path(Option<PathBuf>)` whenever `settings.workspace_dir` changes (on file browser confirm) before the next render.

```
┌─ Vault Path ─────────────────────────────────────────┐
│  /Users/me/notes                  [Enter: Browse]    │
│                                                      │
│  (no vault set)                   [Enter: Browse]    │
└──────────────────────────────────────────────────────┘
```

### `IndexingSection`

**File:** `tui/src/components/settings/indexing_section.rs`

Two navigable actions: `[Fast Reindex]` and `[Full Reindex]`. Left/Right or `h`/`l` cycles between them. Disabled (greyed, no interaction) when `vault_available == false` (a bool passed to `IndexingSection::new` and updatable via `set_vault_available(bool)`).

On Enter: sends `AppMessage::TriggerFastReindex` or `AppMessage::TriggerFullReindex` via `tx`. `SettingsScreen::handle_app_message` intercepts these and acts:
- `TriggerFastReindex` → sets `overlay = Overlay::IndexingProgress(Running(...))`, spawns task
- `TriggerFullReindex` → sets `overlay = Overlay::ConfirmFullReindex`

```
┌─ Reindex ────────────────────────────────────────────┐
│                                                      │
│    [ Fast Reindex ]    [ Full Reindex ]              │
│                                                      │
│  Fast: checks file sizes                             │
│  Full: compares content hashes (slower)              │
└──────────────────────────────────────────────────────┘
```

**State:** `selected: IndexAction` (`Fast | Full`), `vault_available: bool`.

---

## Overlays

Managed as an enum on `SettingsScreen`. Only one overlay is active at a time.

```rust
enum Overlay {
    None,
    FileBrowser(FileBrowserState),
    ConfirmFullReindex { focused_button: ConfirmButton },  // Cancel | Confirm
    ConfirmSave { focused_button: SaveButton },            // Save | Discard
    IndexingProgress(IndexingProgressState),
}

enum ConfirmButton { Cancel, Confirm }
enum SaveButton { Save, Discard }
```

### File Browser Overlay

A centered popup drawn on top of the settings screen. Shows directories only (files are skipped). Navigation:

- Up/Down — move selection within the directory list
- Right or Enter when a directory row is selected — navigate into it (calls `fs::read_dir` synchronously — fast for local FS, acceptable blocking for a TUI)
- Left — go up one level (`current_path.parent()`)
- Ctrl+Enter (or `c`) — **confirm** current directory as vault path, close overlay
- Esc — cancel, discard changes

The header line showing the current path is **rendered separately** (not part of `ListState`). `ListState` index 0 always refers to the first directory entry. The header is not selectable. The "confirm" action always confirms `current_path` regardless of which list item is selected, and is triggered via Ctrl+Enter or `c` (not plain Enter, which is reserved for navigating into the selected directory).

```
┌──────────────────────────────────────────────────┐
│  Select Vault Directory                          │
│  /Users/me/                                      │
├──────────────────────────────────────────────────┤
│  ▶ notes/                                        │
│    projects/                                     │
│    documents/                                    │
├──────────────────────────────────────────────────┤
│  Enter: open  Ctrl+Enter/c: confirm  Esc: cancel │
└──────────────────────────────────────────────────┘
```

**Starting directory:** `settings.workspace_dir` if `Some`, otherwise `$HOME` (via `std::env::var("HOME")`; fallback to `/`).

```rust
struct FileBrowserState {
    current_path: PathBuf,
    entries: Vec<PathBuf>,       // directories only, sorted alphabetically
    list_state: ListState,
}
```

`FileBrowserState::load(path: PathBuf)` reads the directory with `std::fs::read_dir`, filters to directories only, sorts alphabetically, and returns the state. Called synchronously — directory listing on local FS is fast enough for interactive navigation.

On confirm: `SettingsScreen` calls `self.settings.set_workspace(&chosen_path)`, calls `vault_section.set_path(Some(chosen_path))`, calls `indexing_section.set_vault_available(true)`.

### Full Reindex Confirmation Overlay

Simple two-button dialog. Left/Right selects Cancel or Confirm. Enter activates; Esc cancels (returns to `Overlay::None`).

```
┌──────────────────────────────────────────────────┐
│  Full Reindex                                    │
│                                                  │
│  This may take a while on large vaults.          │
│                                                  │
│       [ Cancel ]        [ Confirm ]              │
└──────────────────────────────────────────────────┘
```

### Save Confirmation Overlay

Shown when the user presses Esc and settings have changed. Left/Right selects Save or Discard. Enter activates; Esc cancels (returns to `Overlay::None`, stays in settings).

```
┌──────────────────────────────────────────────────┐
│  Save Settings?                                  │
│                                                  │
│  You have unsaved changes.                       │
│                                                  │
│       [ Save ]        [ Discard ]                │
└──────────────────────────────────────────────────┘
```

- **Save**: if vault path changed → run full reindex with `pending_save_after_index = true` (auto-close on done); otherwise save immediately and close.
- **Discard**: send `AppMessage::CloseSettings`, close without saving.

### Indexing Progress Overlay

Uses `throbber-widgets-tui` (version `"0.10"`, compatible with ratatui 0.29+/0.30; verified against ATAC project usage). Added to `tui/Cargo.toml`.

The indexing task spawn location depends on what triggered it:
- **User-triggered** (`TriggerFastReindex` / `TriggerFullReindex` + confirm): spawn happens inside `SettingsScreen::handle_app_message` (which is `async`).
- **Vault-path-change on Save** (ConfirmSave → Save, vault path changed): spawn happens inside `SettingsScreen::handle_event` (synchronous). `tokio::spawn` is still valid here because the Tokio runtime is active.

In both cases, `tokio::spawn` requires a `'static` future, so `tx` (a `&AppTx` reference) **must be cloned** before the spawn: `let tx = tx.clone();`. The owned `tx` is then moved into the closure. The task:
1. Constructs `NoteVault::new(workspace_dir).await.map_err(|e| e.to_string())` — `VaultError` is not `Send` (it wraps `sqlx::Error`), so the error **must** be converted to `String` immediately at this call site. Do not use `?` with `VaultError` inside `tokio::spawn` — the future will not satisfy the `Send` bound.
2. Calls the appropriate indexing method; similarly converts `VaultError` to `String` via `.map_err(|e| e.to_string())`
3. Sends `AppMessage::IndexingDone(Ok(duration))` or `AppMessage::IndexingDone(Err(String))`
4. Sends periodic `AppMessage::Redraw` (~100ms via `tokio::time::sleep`) so the throbber animates while running

```rust
enum IndexingProgressState {
    Running(tokio::task::JoinHandle<()>),
    Done(Duration),
    Failed(String),
}
```

**Esc while Running:** Esc is blocked — the user must wait for the indexing task to complete. The UI shows the throbber with no dismiss option until `Done` or `Failed`.

**On Done:** `SettingsScreen` calls `self.settings.report_indexed()`.

**On `IndexingDone(Ok)` with `pending_save_after_index == true`:** auto-close — call `settings.report_indexed()`, then `settings.save_to_disk().ok()` (non-fatal, same policy as above), send `AppMessage::SettingsSaved(settings)`. No OK button is shown; the screen closes immediately. `pending_save_after_index` is reset to `false`.

**On `IndexingDone(Err)` with `pending_save_after_index == true`:** show the Failed state with an OK button. The user must acknowledge. On dismiss, save is **not** triggered (index failed; user stays in settings with `pending_save_after_index = false`).

**On dismiss (Enter or Esc on Done/Failed, `pending_save_after_index == false`):** sets `overlay = Overlay::None`. The `JoinHandle` stored in `Running` is dropped when the state transitions away — since it has already completed by then, this is safe. If Esc were ever allowed during `Running`, the `JoinHandle` must be explicitly `.abort()`'ed first.

```
Running:
┌──────────────────────────────────────────────────┐
│  Indexing                                        │
│     ⣾  Fast reindex in progress…                │
└──────────────────────────────────────────────────┘

Done (normal — user triggered):
┌──────────────────────────────────────────────────┐
│  Indexing                                        │
│     ✓  Done in 3 seconds                        │
│            [ OK ]                                │
└──────────────────────────────────────────────────┘

Done (pending_save_after_index — auto-closes, no UI shown):
(screen closes immediately)

Failed:
┌──────────────────────────────────────────────────┐
│  Indexing                                        │
│     ✗  Error: <message>                         │
│            [ OK ]                                │
└──────────────────────────────────────────────────┘
```

---

## AppMessage Changes

New variants added to `AppMessage`:

```rust
// Sent by SettingsScreen when user confirms Save (or saves + vault path unchanged).
// Main loop updates App::settings and navigates back.
AppMessage::SettingsSaved(AppSettings)

// Sent by SettingsScreen when user discards changes (or closes with no changes).
// Main loop navigates back without updating App::settings.
AppMessage::CloseSettings

// Sent by VaultSection component when user presses Enter/b to open the file browser.
// SettingsScreen::handle_app_message intercepts this (returns None).
AppMessage::OpenFileBrowser

// Sent by IndexingSection when user activates Fast or Full reindex buttons.
// SettingsScreen::handle_app_message intercepts both (returns None).
// NOTE: TriggerFullReindex does NOT start indexing directly — it opens the
// ConfirmFullReindex overlay first. Only after user confirms does indexing start.
AppMessage::TriggerFastReindex
AppMessage::TriggerFullReindex

// Sent by the indexing tokio task on completion.
// SettingsScreen::handle_app_message intercepts this (returns None).
AppMessage::IndexingDone(Result<Duration, String>)
```

All new variants contain only `Send`-safe types (`AppSettings`, `Duration`, `String` are all `Send`).

**Main loop drain — complete updated match block** (the `while let Ok(msg) = rx.try_recv()` loop in `main.rs`):

```rust
AppMessage::Quit => return Ok(()),
AppMessage::Redraw => {}
AppMessage::OpenSettings => {
    let mut screen: Box<dyn AppScreen> = Box::new(SettingsScreen::new(app.settings.clone()));
    screen.on_enter(&tx).await;
    app.current_screen = Some(screen);
}
AppMessage::OpenEditor(vault, path) => {
    // Copy verbatim from main.rs lines 84–92:
    // Box::new(EditorScreen::new(Arc::new(vault), path, app.settings.clone()))
    // screen.on_enter(&tx).await; app.current_screen = Some(screen)
}
AppMessage::OpenPath(path) => {
    // This arm body is UNCHANGED from the existing main.rs implementation.
    // Must be preserved verbatim — it handles vault creation and the OpenSettings
    // fallback when no workspace_dir is configured:
    //   let unhandled = current_screen.handle_app_message(OpenPath(path)).await;
    //   if unhandled == OpenPath(path):
    //     if path.is_note() && workspace_dir is Some:
    //       NoteVault::new(workspace_dir) → OpenEditor(vault, path)
    //     else:
    //       OpenSettings
}
AppMessage::SettingsSaved(settings) => {
    app.settings = settings;
    let path = app.settings.last_paths.last()
        .cloned()
        .unwrap_or_else(VaultPath::root);
    tx.send(AppMessage::OpenPath(path)).ok();
}
AppMessage::CloseSettings => {
    // Navigate back without updating app.settings (user discarded changes).
    let path = app.settings.last_paths.last()
        .cloned()
        .unwrap_or_else(VaultPath::root);
    tx.send(AppMessage::OpenPath(path)).ok();
}
// All remaining variants are screen-internal: route to the active screen.
// This arm MUST be last — it is a catch-all.
// Covered variants: FocusEditor, FocusSidebar, OpenFileBrowser,
//   TriggerFastReindex, TriggerFullReindex, IndexingDone.
// Note: SettingsSaved and CloseSettings are handled above and never reach here.
// Return value is discarded: these messages are scoped to the screen that
// sent them. If the active screen returns Some (didn't handle it), the
// message is intentionally dropped — acceptable for screen-internal signals.
other => {
    if let Some(screen) = app.current_screen.as_mut() {
        screen.handle_app_message(other, &tx).await;
    }
}
```

The existing explicit `FocusEditor | FocusSidebar` arm is **removed** — it is now covered by the `other` catch-all, which has identical behaviour for those variants.

---

## Data Flow

```
User presses Esc (no overlay active, settings unchanged)
  → SettingsScreen::handle_event
    → tx.send(AppMessage::CloseSettings)
      → main loop sends OpenPath(last_path_or_root)
        → main loop creates EditorScreen with original settings

User presses Esc (no overlay active, settings changed)
  → SettingsScreen::handle_event
    → overlay = ConfirmSave { focused_button: Save }

User confirms Save (vault path unchanged)
  → SettingsScreen::handle_event (ConfirmSave overlay, Enter on Save)
    → settings.save_to_disk()
    → tx.send(AppMessage::SettingsSaved(settings))
      → main loop updates App::settings
      → main loop sends OpenPath(last_path_or_root)
        → main loop creates EditorScreen with updated settings

User confirms Save (vault path changed)
  → SettingsScreen::handle_event (ConfirmSave overlay, Enter on Save)
    → pending_save_after_index = true
    → overlay = IndexingProgress(Running(handle))
    → tokio::spawn full reindex task
    → on IndexingDone(Ok):
        settings.save_to_disk()
        tx.send(AppMessage::SettingsSaved(settings))  [auto-close, no OK button]
    → on IndexingDone(Err):
        overlay = IndexingProgress(Failed(msg))  [user must dismiss OK, stays in settings]

User selects Discard in ConfirmSave
  → tx.send(AppMessage::CloseSettings)
      → main loop sends OpenPath(last_path_or_root)
```

```
User presses Enter on [Fast Reindex] (vault path set)
  → IndexingSection::handle_event → tx.send(AppMessage::TriggerFastReindex)
  → main loop routes to SettingsScreen::handle_app_message
    → overlay = IndexingProgress(Running(handle))
    → tokio::spawn:
        NoteVault::new(workspace_dir).await
        vault.index_notes(NotesValidation::Fast).await
        loop { sleep(100ms); tx.send(Redraw) } until done
        tx.send(AppMessage::IndexingDone(Ok(duration) | Err(msg)))
  → main loop routes IndexingDone to SettingsScreen::handle_app_message
    → overlay = IndexingProgress(Done(duration) | Failed(msg))
    → settings.report_indexed() on success
```

```
User presses Enter on vault path row (Browse)
  → VaultSection::handle_event → tx.send(AppMessage::OpenFileBrowser)
  → main loop routes to SettingsScreen::handle_app_message
    → overlay = FileBrowser(FileBrowserState::load(starting_dir))
  → User navigates, presses Enter to confirm
    → SettingsScreen::handle_event (overlay active)
      → settings.set_workspace(&chosen_path)
      → vault_section.set_path(Some(chosen_path))
      → indexing_section.set_vault_available(true)
      → overlay = Overlay::None
```

---

## File Structure

```
tui/src/
  app_screen/
    settings.rs              ← SettingsScreen (full implementation)
  components/
    settings/
      mod.rs                 ← pub mod theme_picker; vault_section; indexing_section;
      theme_picker.rs
      vault_section.rs
      indexing_section.rs
```

`tui/src/components/mod.rs` gets `pub mod settings;`.

---

## Dependencies

Add to `tui/Cargo.toml`:
```toml
throbber-widgets-tui = "0.10"
```

Version 0.10 is compatible with ratatui 0.29+/0.30 (verified via ATAC project which uses the same dependency combination).

---

## Testing

All tests are `#[cfg(test)]` inline modules.

| Component / Unit | Tests |
|-----------------|-------|
| `ThemePicker` | `selected_theme_name()` returns initial theme; Up/Down changes selection and wraps; renders without panic (`TestBackend`) |
| `VaultSection` | Renders `(no vault set)` when path is `None`; renders path string when `Some` |
| `IndexingSection` | `handle_event` returns `NotConsumed` when `vault_available == false`; Left/Right cycles `Fast ↔ Full`; Enter sends correct `AppMessage` |
| `FileBrowserState::load` | Returns only directories; sorted alphabetically; handles empty directory |
| `FileBrowserState` navigation | Navigate-into updates `current_path` and reloads entries; go-up updates path to parent |
| Overlay state machine | `ConfirmFullReindex` + Esc → `Overlay::None`; `IndexingDone(Ok)` → `Done` + `report_indexed()` called; `IndexingDone(Err)` → `Failed`; Esc blocked while `Running` |
| Save confirmation | Settings unchanged + Esc → `CloseSettings` sent (no dialog); Settings changed + Esc → `ConfirmSave` overlay shown |
| `ConfirmSave` — Save, vault unchanged | `SettingsSaved` sent; `CloseSettings` not sent |
| `ConfirmSave` — Save, vault changed | `pending_save_after_index = true`; `IndexingProgress(Running)` overlay shown; on `IndexingDone(Ok)` → `SettingsSaved` sent (auto-close, no OK button); on `IndexingDone(Err)` → `Failed` shown, settings not saved |
| `ConfirmSave` — Discard | `CloseSettings` sent; `SettingsSaved` not sent |
| `AppSettings` `PartialEq` | `settings == initial_settings` true when no changes; false after any change |
| `app_screen::tests` | Update two existing tests to call `SettingsScreen::new(AppSettings::default())` |

We do not test `NoteVault::index_notes` / `recreate_index` — those are covered by core library tests.
