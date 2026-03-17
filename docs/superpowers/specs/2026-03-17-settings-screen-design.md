# Settings Screen Design

**Date:** 2026-03-17
**Status:** Approved
**Scope:** TUI settings screen (`tui/src/app_screen/settings.rs` and related components)

---

## Overview

Replace the placeholder `SettingsScreen` with a fully functional settings screen. The screen uses a left-sidebar navigation + right content panel layout. It exposes three settings sections: Theme, Vault Path, and Indexing. Settings are saved to disk when the user presses Esc to return to the editor.

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

Tab switches focus between the left sidebar and the right content panel. Up/Down (or `j`/`k`) navigate sections in the sidebar. Esc saves settings and returns to the editor.

---

## Architecture

### Approach

Component-based, matching the existing `EditorScreen` + `SidebarComponent` + `TextEditorComponent` pattern.

### `SettingsScreen`

**File:** `tui/src/app_screen/settings.rs`

```rust
pub struct SettingsScreen {
    settings: AppSettings,         // mutable local clone
    section: SettingsSection,      // currently highlighted sidebar item
    focus: SettingsFocus,          // Sidebar | Content
    theme_picker: ThemePicker,
    vault_section: VaultSection,
    indexing_section: IndexingSection,
    overlay: Overlay,
}

enum SettingsSection { Theme, Vault, Indexing }
enum SettingsFocus { Sidebar, Content }
```

**Construction:** `SettingsScreen::new(settings: AppSettings)` — receives a clone of `App::settings`.

**Esc behavior:** calls `settings.save_to_disk()` and sends `AppMessage::SettingsSaved(settings)`. The main loop updates `App::settings` and navigates back (sends `OpenPath` with last path, same as `StartScreen`).

**Event routing:** When `focus == Content`, key events are forwarded to the active section component. When `focus == Sidebar`, Up/Down change the selected section, Tab moves focus to the content panel.

---

## Sub-Components

All three components implement the `Component` trait:
`fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState`
`fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool)`

They live under a new `tui/src/components/settings/` submodule.

### `ThemePicker`

**File:** `tui/src/components/settings/theme_picker.rs`

Renders the full list from `AppSettings::theme_list()`. The currently active theme is highlighted. Up/Down moves selection; each move immediately updates `settings.theme`. No async work.

```
┌─ Theme ──────────────────────────────────────────────┐
│  ● Gruvbox Dark                                      │
│    Gruvbox Light                                     │
│    Catppuccin Mocha                                  │
│    Tokyo Night                                       │
│    Nord                                              │
└──────────────────────────────────────────────────────┘
```

**State:** `list_state: ListState`, `themes: Vec<Theme>` (loaded once on construction).

### `VaultSection`

**File:** `tui/src/components/settings/vault_section.rs`

Displays the current vault path (or `(no vault set)` if `None`). Enter or `b` triggers the file browser overlay (handled by `SettingsScreen`, not this component — it sends an internal signal via `EventState` or a dedicated `AppMessage`).

```
┌─ Vault Path ─────────────────────────────────────────┐
│  /Users/me/notes                  [Enter: Browse]    │
│                                                      │
│  (no vault set)                   [Enter: Browse]    │
└──────────────────────────────────────────────────────┘
```

**State:** holds a reference to the current `Option<PathBuf>` (passed in at render time from `SettingsScreen`).

### `IndexingSection`

**File:** `tui/src/components/settings/indexing_section.rs`

Two navigable actions: `[Fast Reindex]` and `[Full Reindex]`. Left/Right or `h`/`l` moves between them. Both are greyed out and non-interactive if `settings.workspace_dir` is `None`. Enter on Fast starts indexing immediately. Enter on Full triggers the confirmation overlay.

```
┌─ Reindex ────────────────────────────────────────────┐
│                                                      │
│    [ Fast Reindex ]    [ Full Reindex ]              │
│                                                      │
│  Fast: checks file sizes                             │
│  Full: compares content hashes (slower)              │
└──────────────────────────────────────────────────────┘
```

**State:** `selected: IndexAction` (`Fast | Full`).

---

## Overlays

Managed as an enum on `SettingsScreen`. Only one overlay is active at a time.

```rust
enum Overlay {
    None,
    FileBrowser(FileBrowserState),
    ConfirmFullReindex,
    IndexingProgress(IndexingProgressState),
}
```

### File Browser Overlay

A centered popup drawn on top of the settings screen. Shows directories only (files are greyed out). Navigation:

- Up/Down — move selection
- Right or Enter — enter selected directory
- Left — go up one level
- Enter on current directory header — confirm and set vault path
- Esc — cancel, discard changes

```
┌──────────────────────────────────────────────────┐
│  Select Vault Directory                          │
│  /Users/me/                                      │
├──────────────────────────────────────────────────┤
│  ▶ notes/                                        │
│    projects/                                     │
│    documents/                                    │
├──────────────────────────────────────────────────┤
│  Enter: select    ←: up    Esc: cancel           │
└──────────────────────────────────────────────────┘
```

**Starting directory:** current `workspace_dir` if set, otherwise `$HOME`.

```rust
struct FileBrowserState {
    current_path: PathBuf,
    entries: Vec<PathBuf>,       // directories only, sorted
    list_state: ListState,
}
```

### Full Reindex Confirmation Overlay

Simple two-button dialog. Left/Right selects Cancel or Confirm. Enter activates, Esc cancels.

```
┌──────────────────────────────────────────────────┐
│  Full Reindex                                    │
│                                                  │
│  This may take a while on large vaults.          │
│                                                  │
│       [ Cancel ]        [ Confirm ]              │
└──────────────────────────────────────────────────┘
```

### Indexing Progress Overlay

Uses `throbber-widgets-tui` crate (added to `Cargo.toml`). The indexing task runs as a `tokio::spawn` from `SettingsScreen`. The task sends `AppMessage::IndexingDone(Result<Duration, String>)` when finished and sends periodic `AppMessage::Redraw` (~100ms interval) so the throbber animates.

```rust
enum IndexingProgressState {
    Running(tokio::task::JoinHandle<()>),
    Done(Duration),
    Failed(String),
}
```

```
Running:
┌──────────────────────────────────────────────────┐
│  Indexing                                        │
│     ⣾  Fast reindex in progress…                │
└──────────────────────────────────────────────────┘

Done:
┌──────────────────────────────────────────────────┐
│  Indexing                                        │
│     ✓  Done in 3 seconds                        │
│            [ OK ]                                │
└──────────────────────────────────────────────────┘

Failed:
┌──────────────────────────────────────────────────┐
│  Indexing                                        │
│     ✗  Error: <message>                         │
│            [ OK ]                                │
└──────────────────────────────────────────────────┘
```

On Done: calls `settings.report_indexed()`. Esc or Enter dismisses the overlay.

---

## AppMessage Changes

Two new variants added to `AppMessage`:

```rust
AppMessage::SettingsSaved(AppSettings)
// Sent by SettingsScreen on Esc. Main loop updates App::settings
// and navigates back (OpenPath with last_paths.last() or root).

AppMessage::IndexingDone(Result<Duration, String>)
// Sent by the indexing tokio task on completion.
// Handled by SettingsScreen::handle_app_message to update overlay state.
```

**Main loop handling of `SettingsSaved`:**
```rust
AppMessage::SettingsSaved(settings) => {
    app.settings = settings;
    let path = app.settings.last_paths.last()
        .cloned()
        .unwrap_or_else(VaultPath::root);
    tx.send(AppMessage::OpenPath(path)).ok();
}
```

---

## Data Flow

```
User presses Esc
  → SettingsScreen::handle_event
    → settings.save_to_disk()
    → tx.send(AppMessage::SettingsSaved(settings))
      → main loop updates App::settings
      → main loop sends OpenPath(last_path)
        → main loop creates EditorScreen with updated settings
```

```
User triggers Fast Reindex
  → IndexingSection::handle_event → EventState::Consumed (screen detects action)
  → SettingsScreen sets overlay = IndexingProgress(Running(_))
  → tokio::spawn(async { vault.index_notes(Fast).await; tx.send(IndexingDone(...)) })
  → task sends Redraw every ~100ms
  → SettingsScreen::handle_app_message receives IndexingDone
    → overlay = IndexingProgress(Done(duration) | Failed(msg))
    → settings.report_indexed() on success
```

---

## File Structure

```
tui/src/
  app_screen/
    settings.rs              ← SettingsScreen (replace placeholder)
  components/
    settings/
      mod.rs                 ← pub mod theme_picker; vault_section; indexing_section;
      theme_picker.rs
      vault_section.rs
      indexing_section.rs
```

---

## Dependencies

Add to `tui/Cargo.toml`:
```toml
throbber-widgets-tui = "0.7"
```

---

## Testing

All tests are `#[cfg(test)]` inline modules.

| Component | Tests |
|-----------|-------|
| `ThemePicker` | Selection updates on Up/Down; wraps around; renders without panic |
| `VaultSection` | Renders `(no vault set)` when `None`; renders path when `Some` |
| `IndexingSection` | Actions disabled when no vault; Left/Right cycles selection |
| `FileBrowserState` | Loads only directories; Enter navigates in; Left goes up |
| Overlay state machine | `ConfirmFullReindex` Esc → `None`; `IndexingDone(Ok)` → `Done`; `IndexingDone(Err)` → `Failed` |

We do not test `NoteVault::index_notes` / `recreate_index` — those are covered by core library tests.
