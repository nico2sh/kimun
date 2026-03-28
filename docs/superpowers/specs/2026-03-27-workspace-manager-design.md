# Workspace Manager Design

## Goal

Add workspace management to the kimun TUI: create, rename, and delete workspaces from a dedicated overlay, accessible via `Ctrl+W` from anywhere in the app.

---

## User Stories

1. **Switch workspace** — From the editor or browse screen, press `Ctrl+W` to open the workspace manager, select a different workspace, press `Enter` to switch. The app reindexes the new vault if needed and reloads.
2. **Add workspace** — Press `n` in the workspace manager list to start the create flow: pick a directory via file browser, type a name, confirm.
3. **Rename workspace** — Select a workspace, press `r`, edit the name inline, press `Enter` to confirm.
4. **Delete workspace** — Select a non-active workspace, press `d`, confirm deletion.

---

## Architecture

### New file

**`tui/src/components/workspace_manager.rs`**

Contains `WorkspaceManagerOverlay` — a self-contained struct that manages all state and rendering for the overlay. It is used in two places: as a variant of `Overlay` in `SettingsScreen`, and as a field in `EditorScreen` and `BrowseScreen`.

### Modified files

| File | Change |
|------|--------|
| `tui/src/settings/workspace_config.rs` | Add `remove_workspace(name: &str)` and `rename_workspace(old: &str, new: &str)` |
| `tui/src/components/events.rs` | Add `AppEvent::OpenWorkspaceManager` and `AppEvent::SwitchWorkspace(String)` |
| `tui/src/app_screen/settings.rs` | Add `Overlay::WorkspaceManager(WorkspaceManagerOverlay)`; handle `OpenWorkspaceManager` |
| `tui/src/app_screen/editor.rs` | Handle `Ctrl+W` → send `OpenWorkspaceManager`; handle overlay input; handle `SwitchWorkspace` |
| `tui/src/app_screen/browse.rs` | Same as editor |
| `tui/src/app.rs` | Handle `SwitchWorkspace`: update config, reload vault, trigger reindex |

---

## WorkspaceManagerOverlay

### Internal state machine

```
WorkspaceManagerMode::List
  → press n → WorkspaceManagerMode::Create(CreateStep::PickDir(FileBrowserState))
                → press c → WorkspaceManagerMode::Create(CreateStep::NameInput { dir, name_buf })
                              → press Enter → back to List (workspace added)
                              → press Esc  → back to List (cancelled)
              → press Esc  → back to List (cancelled)
  → press r → WorkspaceManagerMode::Rename { index, name_buf }
              → press Enter → back to List (renamed)
              → press Esc  → back to List (cancelled)
  → press d → WorkspaceManagerMode::ConfirmDelete { index, focused: Cancel|Confirm }
              → confirm    → back to List (deleted)
              → cancel/Esc → back to List
  → press Esc → overlay closes (send AppEvent::CloseWorkspaceManager or just set overlay to None)
  → press Enter → switch to selected workspace (send AppEvent::SwitchWorkspace(name))
```

### Struct

```rust
pub struct WorkspaceManagerOverlay {
    pub workspaces: Vec<(String, WorkspaceEntry)>, // sorted by name
    pub active_workspace: String,
    pub list_state: ListState,
    pub mode: WorkspaceManagerMode,
    pub error_msg: Option<String>,
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

`WorkspaceManagerOverlay::new(config: &WorkspaceConfig) -> Self`

Builds the sorted workspace list and sets the initial selection to the active workspace.

### Methods

- `handle_input(&mut self, event: &InputEvent, config: &mut WorkspaceConfig, tx: &AppTx) -> EventState`
  Drives all state transitions. Sends `AppEvent::SwitchWorkspace(name)` on Enter in List mode.
- `render(&mut self, frame: &mut Frame, area: Rect, theme: &Theme)`
  Renders the overlay as a centered popup. Each mode has a different inner widget.

### Rendering

The overlay renders as a centered popup (60% width, 70% height) with a border and title "Workspaces".

- **List mode**: `ratatui::List` showing all workspace names. Active workspace marked with `*`. Selected item highlighted. Footer line: `[n]ew  [r]ename  [d]elete  Enter=switch  Esc=close`.
- **Create / PickDir**: Reuses `FileBrowserState` rendering logic (same as `Overlay::FileBrowser`). Footer: `[c]=choose dir  Esc=back`.
- **Create / NameInput**: Single-line text input, pre-filled with folder name. Footer: `Enter=confirm  Esc=back`.
- **Rename**: Same as NameInput but label says "Rename workspace:".
- **ConfirmDelete**: Two-button dialog. Footer: `←/→ to choose  Enter=confirm`.
- **Error messages**: Shown in red below the list/input when `error_msg` is set. Cleared on any new input.

---

## Events

```rust
// In AppEvent:
OpenWorkspaceManager,
SwitchWorkspace(String),   // workspace name to switch to
```

---

## Workspace config changes

### `remove_workspace(name: &str) -> Result<(), WorkspaceConfigError>`

- Returns `Err` if `name` is the current workspace (`WorkspaceConfigError::CannotRemoveActive` or similar).
- Returns `Err` if `name` doesn't exist.
- Removes from `workspaces` map and saves.

### `rename_workspace(old: &str, new: &str) -> Result<(), WorkspaceConfigError>`

- Returns `Err` if `old` doesn't exist.
- Returns `Err` if `new` already exists.
- If `old` == `global.current_workspace`, updates `current_workspace` to `new`.
- Renames the key in the map.

---

## Workspace switching flow (in `app.rs`)

On `AppEvent::SwitchWorkspace(name)`:

1. Look up the workspace entry in `workspace_config`.
2. Validate the path exists on disk; if not, send an error event and return.
3. Update `workspace_config.global.current_workspace = name`.
4. Save config to disk.
5. Create new `NoteVault` for the new path.
6. Spawn background task: `vault.recreate_index()`.
7. Show `IndexingProgress` overlay (reuse existing mechanism).
8. On `IndexingDone(Ok(...))`: send `AppEvent::SettingsSaved(updated_settings)` to reload vault + browse.
9. On `IndexingDone(Err(e))`: show error, revert `current_workspace`.

---

## Error handling

| Scenario | Handling |
|----------|----------|
| Delete active workspace | Block: show `error_msg = "Cannot delete the active workspace"` |
| Rename to existing name | Block: show `error_msg = "A workspace with that name already exists"` |
| Create with duplicate name | Block: show same error |
| Create empty name | Block: show `error_msg = "Name cannot be empty"` |
| Switch to non-existent path | Show error overlay, do not switch |
| Reindex failure on switch | Show existing `IndexingDone(Err)` path |

---

## Testing

### Unit tests in `workspace_config.rs`

- `remove_workspace` removes a non-active workspace
- `remove_workspace` returns error on active workspace
- `remove_workspace` returns error on unknown name
- `rename_workspace` renames successfully, updates `current_workspace` if active
- `rename_workspace` returns error on duplicate name
- `rename_workspace` returns error on unknown name

### Unit tests in `workspace_manager.rs`

- List → Create(PickDir) on `n`
- Create(PickDir) → Create(NameInput) on `c`
- Create(NameInput) → List on `Enter` (workspace added to config)
- Create(NameInput) → List on `Esc` (no change)
- List → Rename on `r`
- Rename → List on `Enter` (name changed)
- List → ConfirmDelete on `d`
- ConfirmDelete → List on `Esc` (no change)
- ConfirmDelete → List on `Enter` with Confirm focused (workspace removed)
- Blocking: `d` on active workspace shows error, stays in List mode

---

## Settings screen: Vault section context

The existing `VaultSection` in Settings shows the vault path for the current workspace. It should display which workspace is being configured, so the user knows they're editing settings for "work" and not "personal".

**Change:** `VaultSection` receives the active workspace name and displays it as a label above the path field, e.g.:

```
Workspace: work
Path: /Users/me/notes/work
```

This is a read-only label — changing the workspace is done via the Workspace Manager overlay (`Ctrl+W`), not from this settings section.

**Modified files:**
- `tui/src/components/settings/vault_section.rs` — add `workspace_name: Option<String>` field, render label
- `tui/src/app_screen/settings.rs` — pass `workspace_name` to `VaultSection::new`

---

## Out of scope

- Importing/migrating notes between workspaces
- Workspace-level settings (theme per workspace, etc.)
- Ordering workspaces
