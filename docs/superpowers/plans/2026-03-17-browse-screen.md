# BrowseScreen Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `BrowseScreen` that shows the full-width note browser when the app starts with no note to open, and whenever a directory path is navigated to.

**Architecture:** A new `BrowseScreen` wraps the existing `SidebarComponent` full-width. An `OpenBrowse(NoteVault, VaultPath)` message is added to `AppMessage`; the main loop creates it when `OpenPath` resolves to a directory. Directory navigation is handled internally by `BrowseScreen`; note selection is forwarded to the main loop, which opens `EditorScreen`.

**Tech Stack:** Rust, Ratatui 0.30, Tokio, `SidebarComponent`, `VaultBrowseOptionsBuilder`, `key_event_to_combo`

---

## Chunk 1: AppMessage + BrowseScreen

### Task 1: Add `AppMessage::OpenBrowse` variant

**Files:**
- Modify: `tui/src/components/app_message.rs`

- [ ] **Step 1: Write the failing compile test**

Add to the `#[cfg(test)] mod tests` block in `app_message.rs`:

```rust
#[test]
fn open_browse_variant_exists() {
    // Fails to compile until OpenBrowse(NoteVault, VaultPath) is added.
    // NoteVault requires a real path at runtime, so we just verify the type compiles.
    let _: fn(NoteVault, VaultPath) -> AppMessage = AppMessage::OpenBrowse;
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd tui && cargo test -p kimun-tui app_message::tests::open_browse_variant_exists 2>&1 | head -30
```

Expected: compile error — `AppMessage::OpenBrowse` not found.

- [ ] **Step 3: Add the variant**

In `tui/src/components/app_message.rs`, update the enum and its doc-comment:

```rust
/// Messages screens send to the main loop. All variants must be `Send` so
/// they can travel through the tokio channel.
/// Note: `OpenEditor` and `OpenBrowse` carry `NoteVault` directly as an
/// accepted deviation from keeping data simple — vault construction is cheap.
#[derive(Debug)]
pub enum AppMessage {
    Quit,
    Redraw,
    OpenSettings,
    /// Navigate to the editor for the given vault root path.
    /// Accepted deviation: carries NoteVault directly (same as OpenBrowse).
    OpenEditor(NoteVault, VaultPath),
    /// Navigate to the browse screen for the given vault root and directory path.
    /// Accepted deviation: carries NoteVault directly (same as OpenEditor).
    OpenBrowse(NoteVault, VaultPath),
    OpenPath(VaultPath),
    FocusEditor,
    FocusSidebar,
    /// Sent by SettingsScreen when user confirms Save.
    SettingsSaved(AppSettings),
    /// Sent by SettingsScreen when user discards or closes unchanged.
    CloseSettings,
    /// Sent by VaultSection; SettingsScreen::handle_app_message intercepts.
    OpenFileBrowser,
    /// Sent by IndexingSection; SettingsScreen intercepts.
    TriggerFastReindex,
    TriggerFullReindex,
    /// Sent by indexing tokio task on completion.
    IndexingDone(Result<Duration, String>),
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cd tui && cargo test -p kimun-tui app_message::tests::open_browse_variant_exists 2>&1
```

Expected: PASS. Also run all app_message tests:

```bash
cd tui && cargo test -p kimun-tui app_message 2>&1
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
cd tui && git add src/components/app_message.rs
git commit -m "feat(app_message): add OpenBrowse(NoteVault, VaultPath) variant"
```

---

### Task 2: Create `BrowseScreen`

**Files:**
- Create: `tui/src/app_screen/browse.rs`
- Modify: `tui/src/app_screen/mod.rs` (add `pub mod browse;`)

- [ ] **Step 1: Write all failing tests first**

Create `tui/src/app_screen/browse.rs` with the module declaration and tests only (no `BrowseScreen` struct yet):

```rust
use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::{NoteVault, VaultBrowseOptionsBuilder};
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout};

use crate::app_screen::AppScreen;
use crate::components::Component;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::components::sidebar::SidebarComponent;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_event_to_combo;
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

pub struct BrowseScreen {
    vault: Arc<NoteVault>,
    sidebar: SidebarComponent,
    settings: AppSettings,
    theme: Theme,
    path: VaultPath,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    fn make_settings_with_defaults() -> AppSettings {
        AppSettings::default()
    }

    // Helper to create a dummy vault. NoteVault::new requires an actual path on disk.
    // We use a temp dir so the vault is real but isolated from the user's workspace.
    async fn make_vault() -> Arc<NoteVault> {
        let dir = std::env::temp_dir().join("kimun_browse_test_vault");
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(NoteVault::new(&dir).await.unwrap())
    }

    fn key_event(code: KeyCode) -> AppEvent {
        AppEvent::Key(ratatui::crossterm::event::KeyEvent {
            code,
            modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
            kind: ratatui::crossterm::event::KeyEventKind::Press,
            state: ratatui::crossterm::event::KeyEventState::NONE,
        })
    }

    // --- Test 1: new stores the path ---
    #[tokio::test]
    async fn new_stores_path() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        let path = VaultPath::root();
        let screen = BrowseScreen::new(vault, path.clone(), settings);
        assert_eq!(screen.path, path);
    }

    // --- Test 2: Esc sends Quit ---
    #[tokio::test]
    async fn esc_sends_quit() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        let (tx, mut rx) = unbounded_channel();
        let mut screen = BrowseScreen::new(vault, VaultPath::root(), settings);
        screen.handle_event(&key_event(KeyCode::Esc), &tx);
        let msg = rx.try_recv().expect("should have sent a message");
        assert!(matches!(msg, AppMessage::Quit));
    }

    // --- Test 3: settings keybinding sends OpenSettings ---
    #[tokio::test]
    async fn settings_keybinding_sends_open_settings() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        // Default OpenSettings binding: Cmd+Comma on macOS, Ctrl+Comma elsewhere.
        // key_event_to_combo maps SUPER|META → cmd=true, CONTROL → ctrl=true.
        // We construct the crossterm event directly to avoid non-existent Into<KeyCode> conversions.
        #[cfg(target_os = "macos")]
        let mods = ratatui::crossterm::event::KeyModifiers::SUPER;
        #[cfg(not(target_os = "macos"))]
        let mods = ratatui::crossterm::event::KeyModifiers::CONTROL;

        let event = AppEvent::Key(ratatui::crossterm::event::KeyEvent {
            code: ratatui::crossterm::event::KeyCode::Char(','),
            modifiers: mods,
            kind: ratatui::crossterm::event::KeyEventKind::Press,
            state: ratatui::crossterm::event::KeyEventState::NONE,
        });

        let (tx, mut rx) = unbounded_channel();
        let mut screen = BrowseScreen::new(vault, VaultPath::root(), settings);
        screen.handle_event(&event, &tx);
        let msg = rx.try_recv().expect("should have sent a message");
        assert!(matches!(msg, AppMessage::OpenSettings));
    }

    // --- Test 4: OpenPath(dir) is consumed and updates self.path ---
    #[tokio::test]
    async fn handle_app_message_open_path_dir_is_consumed() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        let (tx, _rx) = unbounded_channel();
        let dir = VaultPath::new("subdir");
        let mut screen = BrowseScreen::new(vault, VaultPath::root(), settings);
        let result = screen.handle_app_message(AppMessage::OpenPath(dir.clone()), &tx).await;
        assert!(result.is_none(), "OpenPath(dir) should be consumed");
        assert_eq!(screen.path, dir, "path should be updated");
    }

    // --- Test 5: OpenPath(note) is forwarded ---
    #[tokio::test]
    async fn handle_app_message_open_path_note_is_forwarded() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        let (tx, _rx) = unbounded_channel();
        let note = VaultPath::note_path_from("test.md");
        let mut screen = BrowseScreen::new(vault, VaultPath::root(), settings);
        let result = screen.handle_app_message(AppMessage::OpenPath(note.clone()), &tx).await;
        assert!(result.is_some(), "OpenPath(note) should be forwarded");
        assert!(matches!(result.unwrap(), AppMessage::OpenPath(_)));
    }

    // --- Test 6: Unrelated messages are forwarded ---
    #[tokio::test]
    async fn handle_app_message_unrelated_is_forwarded() {
        let vault = make_vault().await;
        let settings = make_settings_with_defaults();
        let (tx, _rx) = unbounded_channel();
        let mut screen = BrowseScreen::new(vault, VaultPath::root(), settings);
        let result = screen.handle_app_message(AppMessage::FocusEditor, &tx).await;
        assert!(result.is_some(), "FocusEditor should be forwarded");
    }
}
```

- [ ] **Step 2: Add `pub mod browse` to mod.rs**

In `tui/src/app_screen/mod.rs`, add after the existing module declarations:

```rust
pub mod browse;
pub mod editor;
pub mod settings;
pub mod start;
```

- [ ] **Step 3: Run tests to confirm they fail**

```bash
cd tui && cargo test -p kimun-tui browse 2>&1 | head -50
```

Expected: compile errors — `BrowseScreen` struct exists but has no `impl AppScreen`, and methods are missing.

- [ ] **Step 4: Implement `BrowseScreen`**

Add these impls to `tui/src/app_screen/browse.rs` after the struct definition (before the `#[cfg(test)]` block):

```rust
impl BrowseScreen {
    pub fn new(vault: Arc<NoteVault>, path: VaultPath, settings: AppSettings) -> Self {
        let kb = settings.key_bindings.clone();
        let theme = settings.get_theme();
        Self {
            sidebar: SidebarComponent::new(kb, vault.clone()),
            vault,
            settings,
            theme,
            path,
        }
    }

    async fn navigate_sidebar(&mut self, dir: VaultPath, tx: &AppTx) {
        let (options, rx) = VaultBrowseOptionsBuilder::new(&dir)
            .non_recursive()
            .full_validation()
            .build();
        self.path = dir.clone();
        let vault = self.vault.clone();
        let tx2 = tx.clone();
        tokio::spawn(async move {
            vault.browse_vault(options).await.ok();
            tx2.send(AppMessage::Redraw).ok();
        });
        self.sidebar.start_loading(rx, dir);
    }
}

#[async_trait]
impl AppScreen for BrowseScreen {
    async fn on_enter(&mut self, tx: &AppTx) {
        self.navigate_sidebar(self.path.clone(), tx).await;
    }

    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState {
        if let AppEvent::Key(key) = event {
            if let Some(combo) = key_event_to_combo(key) {
                if self.settings.key_bindings.get_action(&combo) == Some(ActionShortcuts::OpenSettings) {
                    tx.send(AppMessage::OpenSettings).ok();
                    return EventState::Consumed;
                }
            }
            if key.code == KeyCode::Esc {
                tx.send(AppMessage::Quit).ok();
                return EventState::Consumed;
            }
        }
        self.sidebar.handle_event(event, tx)
    }

    fn render(&mut self, f: &mut Frame) {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(60), Constraint::Min(0)])
            .split(f.area());
        self.sidebar.render(f, cols[1], &self.theme, true);
    }

    async fn handle_app_message(&mut self, msg: AppMessage, tx: &AppTx) -> Option<AppMessage> {
        if let AppMessage::OpenPath(path) = &msg {
            if !path.is_note() {
                let dir = path.clone();
                self.navigate_sidebar(dir, tx).await;
                return None;
            }
        }
        Some(msg)
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cd tui && cargo test -p kimun-tui browse 2>&1
```

Expected: all 6 tests pass. Also verify whole suite compiles:

```bash
cd tui && cargo build -p kimun-tui 2>&1 | head -30
```

- [ ] **Step 6: Commit**

```bash
cd tui && git add src/app_screen/browse.rs src/app_screen/mod.rs
git commit -m "feat(browse): add BrowseScreen with full-width sidebar"
```

---

## Chunk 2: main.rs wiring

### Task 3: Wire `main.rs` — `OpenBrowse` arm + updated `OpenPath` fallthrough

**Files:**
- Modify: `tui/src/main.rs`

> **TDD note:** The main event loop runs in a tokio task and owns the terminal; unit-testing the routing arms directly would require a substantial harness. The `AppMessage::OpenBrowse` variant is already compile-tested in Task 1, and the `BrowseScreen` is fully unit-tested in Task 2. For this task we verify correctness by (a) ensuring the code compiles without warnings and (b) running the full test suite, which provides adequate confidence for a single pattern-match routing change.

- [ ] **Step 1: Confirm compile test passes before changes**

```bash
cd tui && cargo build -p kimun-tui 2>&1 | head -20
```

Expected: clean build (the `other =>` arm currently catches `OpenBrowse`; we'll replace that behaviour).

- [ ] **Step 2: Add `OpenBrowse` arm and update `OpenPath` fallthrough in `main.rs`**

Add the import at the top of `main.rs` alongside the existing screen imports:

```rust
use crate::app_screen::browse::BrowseScreen;
```

Find the `AppMessage::OpenEditor(vault, path) => { ... }` arm in the `while let Ok(msg) = rx.try_recv()` drain loop and add the `OpenBrowse` arm immediately after it:

```rust
AppMessage::OpenBrowse(vault, path) => {
    let mut screen: Box<dyn AppScreen> =
        Box::new(BrowseScreen::new(Arc::new(vault), path, app.settings.clone()));
    screen.on_enter(&tx).await;
    app.current_screen = Some(screen);
}
```

Then find the `AppMessage::OpenPath(path) => { ... }` arm and update the fallthrough (the `if let Some(AppMessage::OpenPath(path)) = unhandled` block) to send `OpenBrowse` for directories:

```rust
AppMessage::OpenPath(path) => {
    let unhandled = if let Some(screen) = app.current_screen.as_mut() {
        screen
            .handle_app_message(AppMessage::OpenPath(path), &tx)
            .await
    } else {
        Some(AppMessage::OpenPath(path))
    };
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
}
```

- [ ] **Step 3: Build to verify no compile errors**

```bash
cd tui && cargo build -p kimun-tui 2>&1
```

Expected: clean build (0 errors).

- [ ] **Step 4: Run the full test suite**

```bash
cd tui && cargo test -p kimun-tui 2>&1
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
cd tui && git add src/main.rs
git commit -m "feat(main): wire OpenBrowse arm and send OpenBrowse for directory paths"
```
