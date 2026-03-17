# BrowseScreen Design

## Goal

Add a `BrowseScreen` that shows the note browser (sidebar with search) as the primary view when no specific note is being opened, and whenever a directory path is navigated to.

## Architecture

Four files change:

- `tui/src/app_screen/browse.rs` — new file, `BrowseScreen`
- `tui/src/app_screen/mod.rs` — add `pub mod browse`
- `tui/src/components/app_message.rs` — add `OpenBrowse(NoteVault, VaultPath)`
- `tui/src/main.rs` — handle `OpenBrowse`; update `OpenPath` fallthrough for directories

---

## `BrowseScreen`

```rust
pub struct BrowseScreen {
    vault: Arc<NoteVault>,
    sidebar: SidebarComponent,
    settings: AppSettings,
    theme: Theme,
    path: VaultPath,   // current directory; updated on navigation
}
```

### `new(vault: Arc<NoteVault>, path: VaultPath, settings: AppSettings) -> Self`

- `theme = settings.get_theme()`
- `sidebar = SidebarComponent::new(settings.key_bindings.clone(), vault.clone())`
- Stores `path` as the initial directory to browse on `on_enter`

### `on_enter(&mut self, tx: &AppTx)`

Calls `self.navigate_sidebar(self.path.clone(), tx).await`.

If the user returns from `SettingsScreen` (which routes through `StartScreen` → `OpenPath` → `OpenBrowse`), a fresh `BrowseScreen` is created, so re-entry is not a concern.

### `navigate_sidebar(&mut self, dir: VaultPath, tx: &AppTx)` (private async)

Follows the same pattern as `EditorScreen::navigate_sidebar` (which is `pub`; here it is private since no caller outside `BrowseScreen` needs it):

```rust
async fn navigate_sidebar(&mut self, dir: VaultPath, tx: &AppTx) {
    let (options, rx) = VaultBrowseOptionsBuilder::new(&dir)
        .non_recursive()
        .full_validation()
        .build();
    self.path = dir.clone(); // updated synchronously before spawn
    let vault = self.vault.clone();
    let tx2 = tx.clone();
    tokio::spawn(async move {
        vault.browse_vault(options).await.ok();
        tx2.send(AppMessage::Redraw).ok();
    });
    self.sidebar.start_loading(rx, dir);
}
```

Errors from `browse_vault` are silently ignored — the sidebar remains empty, consistent with `EditorScreen`.

### `handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState`

**Important**: `Esc` is the only quit shortcut. The `q` key is NOT intercepted here because it would prevent typing `q` in the sidebar's search box.

The checks run in this exact order:

1. **Settings keybinding** — check via `key_event_to_combo` before anything else:
   ```rust
   if let AppEvent::Key(key) = event {
       // key_event_to_combo returns Option<KeyCombo>
       if let Some(combo) = key_event_to_combo(key) {
           if self.settings.key_bindings.get_action(&combo) == Some(ActionShortcuts::OpenSettings) {
               tx.send(AppMessage::OpenSettings).ok();
               return EventState::Consumed;
           }
       }
   }
   ```
2. **`Esc`** (`KeyCode::Esc`) — sends `AppMessage::Quit`, returns `Consumed`. This is checked after the settings keybinding so that if the user were to bind `OpenSettings` to `Esc`, settings takes priority. In practice `Esc` is never bound to `OpenSettings`, so this ordering has no visible effect.
3. **Everything else** — delegates to `self.sidebar.handle_event(event, tx)`

### `async fn handle_app_message(&mut self, msg: AppMessage, tx: &AppTx) -> Option<AppMessage>`

- `OpenPath(path)` where `!path.is_note()` → calls `navigate_sidebar(path, tx).await`, returns `None` (consumed)
- Everything else → returns `Some(msg)` (forwarded to main loop)

When the user selects a note, the sidebar sends `OpenPath(note_path)`. `BrowseScreen` forwards it. The main loop's `OpenPath` fallthrough creates a fresh `NoteVault` and sends `OpenEditor` — this is the same vault-reconstruction pattern used throughout the main loop and is intentional (vault creation is cheap).

### `render(&mut self, f: &mut Frame)`

```rust
self.sidebar.render(f, f.area(), &self.theme, true);
```

`SidebarComponent` already renders its own directory header, search box, and file list.

---

## `AppMessage` change

Add alongside `OpenEditor`:

```rust
/// Navigate to the browse screen for the given vault root and directory path.
/// Follows the same convention as OpenEditor — NoteVault is passed directly
/// (accepted deviation from the "keep data simple" comment, same as OpenEditor).
OpenBrowse(NoteVault, VaultPath),
```

Also update the doc-comment at the top of the `AppMessage` enum to remove the "no vault handles" restriction, since `OpenEditor` and `OpenBrowse` both carry `NoteVault` directly. The `OpenEditor` line-level doc-comment should be updated similarly for consistency.

`OpenBrowse` is only ever produced by the named `OpenPath` fallthrough arm in `main.rs` (which runs before any screen is mounted). It is never sent while `BrowseScreen` is already the active screen, so there is no need to intercept it in `BrowseScreen::handle_app_message` or in the main loop's `other` arm.

---

## `main.rs` changes

### `OpenPath` fallthrough

Currently does nothing when path is a directory. Change to:

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

Note: vault construction (`NoteVault::new`) now happens before the `is_note()` branch, so a vault failure is fatal (propagated via `?`) for both note and directory navigation. This is the same behaviour as the existing `OpenEditor` arm — vault construction failure has always been fatal — and is intentional.

### New `OpenBrowse` arm (alongside `OpenEditor`)

```rust
AppMessage::OpenBrowse(vault, path) => {
    let mut screen: Box<dyn AppScreen> =
        Box::new(BrowseScreen::new(Arc::new(vault), path, app.settings.clone()));
    screen.on_enter(&tx).await;
    app.current_screen = Some(screen);
}
```

---

## Navigation Flow

```
App starts
  → StartScreen::on_enter → OpenPath(last_path or VaultPath::root())
  → main loop: workspace set, path is directory
  → OpenBrowse(vault, path) → BrowseScreen mounted

User selects a directory entry in BrowseScreen
  → SidebarComponent sends OpenPath(dir)
  → main loop routes to BrowseScreen::handle_app_message
  → BrowseScreen calls navigate_sidebar internally (consumed)

User selects a note entry in BrowseScreen
  → SidebarComponent sends OpenPath(note)
  → BrowseScreen::handle_app_message returns Some(msg) (forwarded)
  → main loop: path.is_note() → creates vault → OpenEditor → EditorScreen
  → BrowseScreen is dropped (no on_exit hook; app.current_screen is simply replaced)

User presses Esc in BrowseScreen
  → AppMessage::Quit → app exits

User presses settings keybinding in BrowseScreen
  → AppMessage::OpenSettings → SettingsScreen
  → On CloseSettings / SettingsSaved → StartScreen → OpenPath → OpenBrowse → fresh BrowseScreen
```

---

## Tests

- `new` produces a screen with correct `path`
- `handle_event(Esc)` sends `AppMessage::Quit`
- `handle_event(settings_key)` sends `AppMessage::OpenSettings`
- `handle_app_message(OpenPath(dir))` is consumed (returns `None`) and updates `self.path`
- `handle_app_message(OpenPath(note))` is forwarded (returns `Some`)
- `handle_app_message(FocusEditor)` is forwarded (unrelated message)
