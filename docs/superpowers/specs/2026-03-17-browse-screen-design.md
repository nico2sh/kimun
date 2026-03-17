# BrowseScreen Design

## Goal

Add a `BrowseScreen` that shows the note browser (sidebar with search) as the primary view when no specific note is being opened, and whenever a directory path is navigated to.

## Architecture

Four files change:

### New file: `tui/src/app_screen/browse.rs`

```rust
pub struct BrowseScreen {
    vault: Arc<NoteVault>,
    sidebar: SidebarComponent,
    settings: AppSettings,
    theme: Theme,
    initial_path: VaultPath,
}
```

**`new(vault: Arc<NoteVault>, path: VaultPath, settings: AppSettings) -> Self`**
- Creates `SidebarComponent::new(settings.key_bindings.clone(), vault.clone())`
- Stores `path` as `initial_path` for use in `on_enter`

**`on_enter(&mut self, tx: &AppTx)`**
- Calls `self.navigate_to(self.initial_path.clone(), tx).await`
- `navigate_to` follows the same pattern as `EditorScreen::navigate_sidebar`:
  - Spawns a tokio task running `vault.browse_vault(options)` with non-recursive, full-validation options
  - Calls `sidebar.start_loading(rx, dir)` with the channel receiver
  - Sends `AppMessage::Redraw` on completion

**`handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState`**
- `q` or `Esc` → sends `AppMessage::Quit`, returns `Consumed`
- Settings keybinding (from `settings.key_bindings`) → sends `AppMessage::OpenSettings`, returns `Consumed`
- All other events → delegates to `self.sidebar.handle_event(event, tx)`

**`handle_app_message(&mut self, msg: AppMessage, tx: &AppTx) -> Option<AppMessage>`**
- `OpenPath(path)` where `!path.is_note()` → calls `navigate_to(path, tx).await`, returns `None` (consumed)
- Everything else → returns `Some(msg)` (forwarded to main loop)

**`render(&mut self, f: &mut Frame)`**
- Renders `sidebar` into the full frame area: `self.sidebar.render(f, f.area(), &self.theme, true)`
- `SidebarComponent` already renders its own directory header, search box, and file list

### Modified: `tui/src/components/app_message.rs`

Add one variant:

```rust
/// Navigate to the browse screen for the given vault and directory path.
OpenBrowse(NoteVault, VaultPath),
```

### Modified: `tui/src/app_screen/mod.rs`

Add `pub mod browse;`

### Modified: `tui/src/main.rs`

**`OpenPath` fallthrough** — currently does nothing when path is a directory. Change to:

```rust
if let Some(AppMessage::OpenPath(path)) = unhandled {
    if let Some(vault_path) = &app.settings.workspace_dir {
        let vault = NoteVault::new(vault_path).await.map_err(io::Error::other)?;
        if path.is_note() {
            tx.send(AppMessage::OpenEditor(vault, path)).ok();
        } else {
            tx.send(AppMessage::OpenBrowse(vault, path)).ok();
        }
    } else {
        tx.send(AppMessage::OpenSettings).ok();
    }
}
```

**New `OpenBrowse` arm** in the `try_recv` drain loop (alongside `OpenEditor`):

```rust
AppMessage::OpenBrowse(vault, path) => {
    let mut screen: Box<dyn AppScreen> =
        Box::new(BrowseScreen::new(Arc::new(vault), path, app.settings.clone()));
    screen.on_enter(&tx).await;
    app.current_screen = Some(screen);
}
```

## Navigation Flow

```
App starts
  → StartScreen::on_enter → OpenPath(last_path or VaultPath::root())
  → main loop: workspace set, path is directory
  → OpenBrowse(vault, path) → BrowseScreen mounted

User selects a directory entry in BrowseScreen
  → SidebarComponent sends OpenPath(dir)
  → main loop routes to BrowseScreen::handle_app_message
  → BrowseScreen navigates sidebar internally (consumed)

User selects a note entry in BrowseScreen
  → SidebarComponent sends OpenPath(note)
  → BrowseScreen::handle_app_message returns Some(msg) (forwarded)
  → main loop: path.is_note() → creates vault → OpenEditor → EditorScreen

User presses q or Esc in BrowseScreen
  → AppMessage::Quit → app exits

User presses settings keybinding in BrowseScreen
  → AppMessage::OpenSettings → SettingsScreen
  → On CloseSettings / SettingsSaved → StartScreen → re-routes to BrowseScreen
```

## Error Handling

- Vault creation failure in `main.rs` (`NoteVault::new`) propagates as `io::Error` (same as existing `OpenEditor` handling).
- `navigate_to` failures (vault browse errors) are silently ignored — the sidebar remains empty, consistent with `EditorScreen::navigate_sidebar` behavior.

## Testing

- `BrowseScreen::new` with a mock vault produces a screen with correct initial path
- `handle_event` with `q` and `Esc` sends `AppMessage::Quit`
- `handle_event` with the settings keybinding sends `AppMessage::OpenSettings`
- `handle_app_message(OpenPath(dir))` is consumed (returns `None`)
- `handle_app_message(OpenPath(note))` is forwarded (returns `Some`)
- `handle_app_message` for unrelated messages (e.g., `FocusEditor`) is forwarded
