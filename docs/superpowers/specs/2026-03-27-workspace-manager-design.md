# Workspace Manager Design

## Goal

Add workspace management to the kimun TUI: create, rename, and delete workspaces from a dedicated overlay, accessible via `Ctrl+W` from anywhere in the app. The settings Vault section shows which workspace is being configured.

---

## User Stories

1. **Switch workspace** — Press `Ctrl+W` from anywhere, select a different workspace, press `Enter` to switch. The app reindexes the new vault, then navigates to the Start screen.
2. **Add workspace** — Press `n` in the workspace manager, pick a directory via file browser, type a name, confirm.
3. **Rename workspace** — Select a workspace, press `r`, edit the name inline, press `Enter` to confirm.
4. **Delete workspace** — Select a non-active workspace, press `d`, confirm deletion.
5. **Vault section context** — The Settings > Vault section shows the active workspace name above the path field.

---

## Architecture

### New files

| File | Purpose |
|------|---------|
| `tui/src/components/file_browser.rs` | `FileBrowserState` struct — moved from `settings.rs`, now shared |
| `tui/src/components/workspace_manager.rs` | `WorkspaceManagerOverlay` struct |

### Modified files

| File | Change |
|------|--------|
| `tui/src/settings/workspace_config.rs` | Add `remove_workspace(name)` and `rename_workspace(old, new)` |
| `tui/src/components/events.rs` | Add `AppEvent::OpenWorkspaceManager` and `AppEvent::SwitchWorkspace(String)` |
| `tui/src/keys/action_shortcuts.rs` | Add `ActionShortcuts::OpenWorkspaceManager` variant |
| `tui/src/app_screen/settings.rs` | Add `Overlay::WorkspaceManager(WorkspaceManagerOverlay)`; import `FileBrowserState` from new location; handle `OpenWorkspaceManager` in `handle_app_message`; pass workspace name to `VaultSection::new` |
| `tui/src/app_screen/editor.rs` | Add `workspace_manager: Option<WorkspaceManagerOverlay>` field; handle `OpenWorkspaceManager` and `SwitchWorkspace` in `handle_app_message`; route overlay input in `handle_input` |
| `tui/src/app_screen/browse.rs` | Same as editor |
| `tui/src/main.rs` | Add `OpenWorkspaceManager` global shortcut handler in `run_app`; add `SwitchWorkspace` handler in `handle_app_message` |
| `tui/src/components/settings/vault_section.rs` | Add `workspace_name: Option<String>` field, render label |

---

## Global Shortcut: `Ctrl+W`

`Ctrl+W` is a global shortcut, handled in `run_app` before any screen gets the event — same pattern as `Ctrl+P` for OpenSettings.

**Changes:**

1. Add `ActionShortcuts::OpenWorkspaceManager` to `action_shortcuts.rs` (with `Display`/`TryFrom` impls).
2. Register default binding `Ctrl+W → OpenWorkspaceManager` in `AppSettings::default_key_bindings`.
3. In `run_app`'s global shortcut block, handle `ActionShortcuts::OpenWorkspaceManager`:
   ```rust
   Some(ActionShortcuts::OpenWorkspaceManager) => {
       tx.send(AppEvent::OpenWorkspaceManager).ok();
       continue;
   }
   ```
4. In `handle_app_message` in `main.rs`, route `OpenWorkspaceManager` to the current screen:
   ```rust
   AppEvent::OpenWorkspaceManager => {
       if let Some(screen) = app.current_screen.as_mut() {
           screen.handle_app_message(AppEvent::OpenWorkspaceManager, tx).await;
       }
   }
   ```
5. Each screen that can show the overlay (`EditorScreen`, `BrowseScreen`, `SettingsScreen`) handles `OpenWorkspaceManager` in its `handle_app_message` by creating and showing the overlay.

---

## FileBrowserState: move to shared module

`FileBrowserState` is currently defined in `tui/src/app_screen/settings.rs`. The `workspace_manager.rs` component (in `tui/src/components/`) cannot import from `app_screen/settings.rs` without creating an inversion.

**Fix:** Move `FileBrowserState` to `tui/src/components/file_browser.rs`. Update `settings.rs` to import from there.

`FileBrowserState` struct and its `impl` block move unchanged. The only diff in `settings.rs` is replacing the definition with `use crate::components::file_browser::FileBrowserState;`.

---

## WorkspaceManagerOverlay

### Struct

```rust
pub struct WorkspaceManagerOverlay {
    workspaces: Vec<(String, WorkspaceEntry)>, // sorted by name, refreshed on each state transition back to List
    active_workspace: String,
    list_state: ListState,
    mode: WorkspaceManagerMode,
    error_msg: Option<String>,
}

pub enum WorkspaceManagerMode {
    List,
    Create(CreateStep),
    Rename { index: usize, name_buf: String },
    ConfirmDelete { index: usize, focused: ConfirmButton },
}

pub enum CreateStep {
    PickDir(FileBrowserState),
    NameInput { dir: PathBuf, name_buf: String },
}
```

### Constructor

```rust
pub fn new(config: &WorkspaceConfig) -> Self
```

Builds `workspaces` sorted by name, sets `list_state` selection to the active workspace index.

### Key method

```rust
pub fn handle_input(
    &mut self,
    event: &InputEvent,
    config: &mut WorkspaceConfig,
    tx: &AppTx,
) -> EventState
```

Drives all state transitions. On successful workspace switch (`Enter` in List mode), sends `AppEvent::SwitchWorkspace(name)`. Any transition back to `List` mode must refresh `self.workspaces` from `config`.

### Render

```rust
pub fn render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme)
```

Renders as a centered popup (~60% width, ~70% height). Each mode renders a different inner widget:

- **List**: `ratatui::List`, active workspace marked with `*`. Footer: `[n]ew  [r]ename  [d]elete  Enter=switch  Esc=close`
- **Create/PickDir**: Same file browser rendering as current `Overlay::FileBrowser`. Footer: `[c]=choose dir  Esc=back`
- **Create/NameInput**: Single-line text input pre-filled with folder name. Label: "Workspace name:". Footer: `Enter=confirm  Esc=back`
- **Rename**: Single-line text input pre-filled with current name. Label: "Rename workspace:". Footer: `Enter=confirm  Esc=back`
- **ConfirmDelete**: Two-button dialog. Footer: `←/→ to choose  Enter=confirm`
- Error messages rendered in red below main content when `error_msg.is_some()`. Cleared on any input that changes state.

---

## State machine

```
List
  n → Create(PickDir)
        c → Create(NameInput { dir, name = folder_name })
              Enter → add_workspace(config), refresh list, → List
              Esc   → → List
        Esc → → List

  r (selected item) → Rename { index, name_buf = current_name }
        Enter → rename_workspace(config), refresh list, → List
        Esc   → → List

  d (non-active item) → ConfirmDelete { index, focused = Cancel }
        d (on active) → set error_msg, stay in List
        Enter (Cancel focused) → → List
        Enter (Confirm focused) → remove_workspace(config), refresh list, → List
        Esc → → List

  Enter (selected != active) → send SwitchWorkspace(name), close overlay
  Enter (selected == active) → close overlay (already on this workspace)
  Esc → close overlay
```

---

## SwitchWorkspace flow (in `main.rs` → `handle_app_message`)

Add a new `AppEvent::SwitchWorkspace(name)` arm:

```rust
AppEvent::SwitchWorkspace(name) => {
    // 1. Look up workspace entry
    let Some(config) = app.settings.workspace_config.as_mut() else { return Ok(()); };
    let entry = match config.get_workspace(&name) {
        Some(e) => e.clone(),
        None => { /* log error, return */ return Ok(()); }
    };

    // 2. Validate path exists
    if !entry.path.exists() {
        // send error via tx or log — do not switch
        return Ok(());
    }

    // 3. Update config: current_workspace + AppSettings.workspace_dir
    config.set_current_workspace(&name);
    app.settings.workspace_dir = Some(entry.path.clone());
    app.settings.save_to_disk().ok();

    // 4. Create new vault
    let vault = match kimun_core::NoteVault::new(&entry.path).await {
        Ok(v) => std::sync::Arc::new(v),
        Err(e) => { /* log, return */ return Ok(()); }
    };
    app.vault = Some(vault.clone());

    // 5. Spawn reindex with progress overlay (reuse existing mechanism)
    // Sends IndexingDone when done
    let tx2 = tx.clone();
    let handle = tokio::spawn(async move {
        let result = vault.recreate_index().await
            .map_err(|e| e.to_string())
            .map(|r| r.duration);
        tx2.send(AppEvent::IndexingDone(result)).ok();
    });
    // Show progress on current screen
    if let Some(screen) = app.current_screen.as_mut() {
        screen.handle_app_message(
            AppEvent::IndexingProgress(spawn_running(handle, tx)),
            tx
        ).await;
    }
}
```

On `AppEvent::IndexingDone(Ok(_))`: the existing handler sends `AppEvent::SettingsSaved(updated_settings)`. Pass `app.settings.clone()` — by this point `app.settings` already has both `workspace_dir` and `workspace_config` updated (step 3 above), so cloning it produces the correct `AppSettings`. This navigates to StartScreen — **intentional**: switching workspaces is a significant context change.

On `AppEvent::IndexingDone(Err(e))`: revert `current_workspace` to previous, show error.

**Note on IndexingProgress:** If the current screen doesn't support showing an indexing overlay directly, `SwitchWorkspace` can show a simple blocking progress overlay at the app level (full-screen spinner) rather than delegating to the screen. Implementer should check whether `EditorScreen` and `BrowseScreen` already handle `IndexingProgress` messages before choosing approach.

---

## workspace_config.rs additions

### `remove_workspace`

```rust
pub fn remove_workspace(&mut self, name: &str) -> Result<(), WorkspaceConfigError>
```

- Returns `Err(CannotRemoveActive)` if `name == self.global.current_workspace`
- Returns `Err(NotFound(name))` if name doesn't exist
- Removes from `workspaces` map

### `rename_workspace`

```rust
pub fn rename_workspace(&mut self, old: &str, new: &str) -> Result<(), WorkspaceConfigError>
```

- Returns `Err(NotFound(old))` if old doesn't exist
- Returns `Err(AlreadyExists(new))` if new already exists
- Renames the key in the map
- If `old == self.global.current_workspace`, updates `current_workspace` to `new`

### `set_current_workspace`

```rust
pub fn set_current_workspace(&mut self, name: &str)
```

Sets `global.current_workspace = name.to_string()`. (Used by SwitchWorkspace flow.)

---

## Settings: Vault section context

`VaultSection` displays the active workspace name above the path field:

```
Workspace: work
Path: /Users/me/notes/work   [Browse]
```

The workspace name is read-only in this section. Changing workspace is done via `Ctrl+W`.

**Changes:**
- `VaultSection::new` gains `workspace_name: Option<String>` parameter
- Renders a "Workspace: {name}" label (or "(none)" if no workspace configured) above the path row
- `SettingsScreen::new` passes `settings.workspace_config.as_ref().and_then(|c| Some(c.global.current_workspace.clone()))` as `workspace_name`

---

## Error handling

| Scenario | Handling |
|----------|----------|
| Delete active workspace | `error_msg = "Cannot delete the active workspace"` in overlay |
| Rename to existing name | `error_msg = "A workspace with that name already exists"` |
| Create with duplicate name | Same as above |
| Create with empty name | `error_msg = "Name cannot be empty"` |
| Switch to non-existent path | Log error, do not switch, show error (future: status bar message) |
| Reindex failure on switch | Revert `current_workspace`, show `IndexingDone(Err)` error path |
| Rename active workspace | Allowed — `rename_workspace` updates `current_workspace` |

---

## Testing

### Unit tests in `workspace_config.rs`

- `remove_workspace` removes a non-active workspace
- `remove_workspace` returns `CannotRemoveActive` on active workspace
- `remove_workspace` returns `NotFound` on unknown name
- `rename_workspace` renames successfully
- `rename_workspace` updates `current_workspace` when renaming active workspace
- `rename_workspace` returns `AlreadyExists` on duplicate name
- `rename_workspace` returns `NotFound` on unknown name

### Unit tests in `workspace_manager.rs`

- List → Create(PickDir) on `n`
- Create(PickDir) → Create(NameInput) on `c`
- Create(NameInput) → List on `Enter` (workspace added to config, list refreshed)
- Create(NameInput) → List on `Esc` (no change, list refreshed)
- List → Rename on `r`
- Rename → List on `Enter` (name changed, list refreshed)
- Rename → List on `Esc` (no change)
- List → ConfirmDelete on `d` for non-active workspace
- `d` on active workspace sets `error_msg`, stays in List mode
- ConfirmDelete → List on `Esc` (no change)
- ConfirmDelete → List on `Enter` with Confirm (workspace removed, list refreshed)
- `Enter` on active workspace closes overlay without sending `SwitchWorkspace`
- `Enter` on non-active workspace sends `SwitchWorkspace(name)`

### Integration test in `main.rs` tests or separate test file

- `SwitchWorkspace` with valid path: config updated, vault created, reindex triggered
- `SwitchWorkspace` with non-existent path: no state change

### Unit test in `action_shortcuts.rs`

- `OpenWorkspaceManager` roundtrip (Display + TryFrom)

---

## Out of scope

- Importing/migrating notes between workspaces
- Workspace-level settings (theme per workspace, etc.)
- Ordering workspaces
- Ctrl+W blocked while in text editing mode (it's a global shortcut like Ctrl+P; acceptable)
