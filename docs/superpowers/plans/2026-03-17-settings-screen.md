# Settings Screen Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the placeholder `SettingsScreen` with a fully functional settings UI exposing Theme picker, Vault path browser, and Reindex controls.

**Architecture:** Component-based, matching `EditorScreen` + `SidebarComponent` pattern. Three sub-components (`ThemePicker`, `VaultSection`, `IndexingSection`) under `tui/src/components/settings/`. `SettingsScreen` owns all state and overlays. Settings are saved only on explicit user confirmation.

**Tech Stack:** Rust, Ratatui 0.30, tokio, throbber-widgets-tui 0.10, kimun_core `NoteVault`

---

## Chunk 1: Foundation — AppMessage, Cargo.toml, components/settings scaffold

### Task 1: Add throbber dependency

**Files:**
- Modify: `tui/Cargo.toml`

- [ ] **Step 1: Add dependency**

In `tui/Cargo.toml`, add after the `nucleo` line:

```toml
throbber-widgets-tui = "0.10"
```

- [ ] **Step 2: Verify it compiles**

```bash
cd tui && cargo check 2>&1 | head -20
```

Expected: no errors (warnings OK).

- [ ] **Step 3: Commit**

```bash
git add tui/Cargo.toml
git commit -m "feat: add throbber-widgets-tui dependency"
```

---

### Task 2: Add new AppMessage variants

**Files:**
- Modify: `tui/src/components/app_message.rs`

- [ ] **Step 1: Write the failing test**

Add to `tui/src/components/app_message.rs` (at the bottom):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use crate::settings::AppSettings;

    #[test]
    fn settings_saved_variant_exists() {
        // This test fails to compile until SettingsSaved(AppSettings) is added.
        let _msg = AppMessage::SettingsSaved(AppSettings::default());
    }

    #[test]
    fn indexing_done_variant_exists() {
        // This test fails to compile until IndexingDone(Result<Duration, String>) is added.
        let _msg = AppMessage::IndexingDone(Ok(Duration::from_secs(1)));
    }
}
```

- [ ] **Step 2: Run test — it must fail to compile** (variants don't exist yet)

```bash
cd tui && cargo test -p tui components::app_message 2>&1 | head -30
```

Expected: compile error `no variant or associated item named SettingsSaved`.

- [ ] **Step 3: Add new variants**

Replace the `AppMessage` enum in `tui/src/components/app_message.rs` with:

```rust
use kimun_core::{NoteVault, nfs::VaultPath};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use crate::settings::AppSettings;

/// Messages screens send to the main loop. All variants must be `Send` so
/// they can travel through the tokio channel. Keep data simple — no vault
/// handles, no `Arc<…>`. The main loop reconstructs whatever it needs.
#[derive(Debug)]
pub enum AppMessage {
    Quit,
    Redraw,
    OpenSettings,
    /// Navigate to the editor for the given vault root path.
    OpenEditor(NoteVault, VaultPath),
    OpenPath(VaultPath),
    FocusEditor,
    FocusSidebar,
    /// Sent by SettingsScreen when user confirms Save.
    /// Main loop updates App::settings and navigates back.
    SettingsSaved(AppSettings),
    /// Sent by SettingsScreen when user discards or closes unchanged.
    /// Main loop navigates back without updating App::settings.
    CloseSettings,
    /// Sent by VaultSection; SettingsScreen::handle_app_message intercepts.
    OpenFileBrowser,
    /// Sent by IndexingSection; SettingsScreen intercepts.
    /// NOTE: does NOT start indexing directly — opens ConfirmFullReindex overlay.
    TriggerFastReindex,
    TriggerFullReindex,
    /// Sent by indexing tokio task on completion.
    IndexingDone(Result<Duration, String>),
}

/// Convenience alias used throughout the codebase.
pub type AppTx = UnboundedSender<AppMessage>;
```

- [ ] **Step 4: Run test — should pass**

```bash
cd tui && cargo test -p tui components::app_message 2>&1 | head -20
```

Expected: `test components::app_message::tests::new_appmessage_variants_are_send ... ok`

- [ ] **Step 5: Fix compile errors in main.rs**

The new enum variants make the `match msg` block in `tui/src/main.rs` non-exhaustive. Find the `while let Ok(msg) = rx.try_recv()` block. Remove the explicit `AppMessage::FocusEditor | AppMessage::FocusSidebar` arm entirely and replace it with an `other =>` catch-all as the **last arm**. This single arm covers all six new variants (`SettingsSaved`, `CloseSettings`, `OpenFileBrowser`, `TriggerFastReindex`, `TriggerFullReindex`, `IndexingDone`) plus the existing `FocusEditor`/`FocusSidebar` — all routed to the active screen. Dedicated arms for `SettingsSaved` and `CloseSettings` will be added in Chunk 3; for now the catch-all is correct temporary behaviour.

Replace the `FocusEditor | FocusSidebar` arm with (must be LAST arm):

```rust
// In the while let Ok(msg) = rx.try_recv() block, replace:
//   AppMessage::FocusEditor | AppMessage::FocusSidebar => { ... }
// with (must be LAST arm):
other => {
    if let Some(screen) = app.current_screen.as_mut() {
        screen.handle_app_message(other, &tx).await;
    }
}
```

- [ ] **Step 6: Verify full compile**

```bash
cd tui && cargo check 2>&1 | head -30
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add tui/src/components/app_message.rs tui/src/main.rs
git commit -m "feat: add settings AppMessage variants and catch-all routing"
```

---

### Task 3: Create components/settings scaffold

**Files:**
- Create: `tui/src/components/settings/mod.rs`
- Modify: `tui/src/components/mod.rs`

- [ ] **Step 1: Write a failing test**

Create `tui/src/components/settings/mod.rs`:

```rust
pub mod theme_picker;
pub mod vault_section;
pub mod indexing_section;
```

Create three empty placeholder files that make the module compile:

`tui/src/components/settings/theme_picker.rs`:
```rust
// ThemePicker — placeholder, implementation in Task 5
```

`tui/src/components/settings/vault_section.rs`:
```rust
// VaultSection — placeholder, implementation in Task 6
```

`tui/src/components/settings/indexing_section.rs`:
```rust
// IndexingSection — placeholder, implementation in Task 7
```

- [ ] **Step 2: Add to components/mod.rs**

In `tui/src/components/mod.rs`, add:

```rust
pub mod settings;
```

- [ ] **Step 3: Verify compile**

```bash
cd tui && cargo check 2>&1 | head -20
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add tui/src/components/settings/ tui/src/components/mod.rs
git commit -m "feat: add components/settings scaffold"
```

---

### Task 4: Update SettingsScreen constructor + fix existing tests

**Files:**
- Modify: `tui/src/app_screen/settings.rs`
- Modify: `tui/src/app_screen/mod.rs`

- [ ] **Step 1: Write failing tests**

The two existing tests in `tui/src/app_screen/mod.rs` currently call `SettingsScreen::new()`. They will fail once we change the signature. Verify they pass now:

```bash
cd tui && cargo test -p tui app_screen::tests 2>&1
```

Expected: both pass with the old `new()`.

- [ ] **Step 2: Update SettingsScreen to accept AppSettings**

Replace `tui/src/app_screen/settings.rs` entirely with:

```rust
use async_trait::async_trait;
use ratatui::crossterm::event::KeyCode;
use ratatui::widgets::{Block, Borders};

use crate::app_screen::AppScreen;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

// Sub-component types are added in Chunk 2 (Tasks 5–7).
// Overlay and section enums are added in Chunk 3 (Task 8).

#[derive(Debug, Clone, Copy, PartialEq)]
enum SettingsSection { Theme, Vault, Indexing }

#[derive(Debug, Clone, Copy, PartialEq)]
enum SettingsFocus { Sidebar, Content }

pub struct SettingsScreen {
    settings: AppSettings,
    initial_settings: AppSettings,
    theme: Theme,
    section: SettingsSection,
    focus: SettingsFocus,
    pending_save_after_index: bool,
    // theme_picker, vault_section, indexing_section, overlay — added in Chunk 2/3
}

impl SettingsScreen {
    pub fn new(settings: AppSettings) -> Self {
        let theme = settings.get_theme();
        let initial_settings = settings.clone();
        Self {
            settings,
            initial_settings,
            theme,
            section: SettingsSection::Theme,
            focus: SettingsFocus::Sidebar,
            pending_save_after_index: false,
        }
    }
}

#[async_trait]
impl AppScreen for SettingsScreen {
    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState {
        match event {
            AppEvent::Key(key) if key.code == KeyCode::Esc => {
                tx.send(AppMessage::CloseSettings).ok();
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        let block = Block::default()
            .title("Settings")
            .borders(Borders::ALL);
        f.render_widget(block, f.area());
    }

    async fn handle_app_message(&mut self, msg: AppMessage, _tx: &AppTx) -> Option<AppMessage> {
        Some(msg)
    }
}
```

- [ ] **Step 3: Update the two call sites in mod.rs tests**

In `tui/src/app_screen/mod.rs`, replace the entire `#[cfg(test)]` block with:

```rust
#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::unbounded_channel;

    use super::*;
    use crate::app_screen::settings::SettingsScreen;
    use crate::components::app_message::AppMessage;
    use crate::settings::AppSettings;

    #[tokio::test]
    async fn non_editor_screen_passes_focus_message_back() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = SettingsScreen::new(AppSettings::default());
        let result = screen.handle_app_message(AppMessage::FocusSidebar, &tx).await;
        assert!(result.is_some(), "SettingsScreen should not consume FocusSidebar");
    }

    #[tokio::test]
    async fn non_editor_screen_passes_focus_editor_message_back() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = SettingsScreen::new(AppSettings::default());
        let result = screen.handle_app_message(AppMessage::FocusEditor, &tx).await;
        assert!(result.is_some(), "SettingsScreen should not consume FocusEditor");
    }
}
```

- [ ] **Step 4: Update main.rs call site**

In `tui/src/main.rs`, line 80, change:
```rust
// was:
Box::new(SettingsScreen::new())
// becomes:
Box::new(SettingsScreen::new(app.settings.clone()))
```

- [ ] **Step 5: Run existing tests**

```bash
cd tui && cargo test -p tui app_screen::tests 2>&1
```

Expected: both `non_editor_screen_passes_focus_message_back` and `non_editor_screen_passes_focus_editor_message_back` pass.

- [ ] **Step 6: Commit**

```bash
git add tui/src/app_screen/settings.rs tui/src/app_screen/mod.rs tui/src/main.rs
git commit -m "feat: SettingsScreen::new takes AppSettings, wire CloseSettings"
```

---
## Chunk 2: Sub-components — ThemePicker, VaultSection, IndexingSection

### Task 5: ThemePicker

**Files:**
- Modify: `tui/src/components/settings/theme_picker.rs`

- [ ] **Step 1: Write failing tests**

Replace the placeholder in `tui/src/components/settings/theme_picker.rs` with:

```rust
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::style::{Style, Modifier};

use crate::components::Component;
use crate::components::app_message::AppTx;
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::settings::themes::Theme;

pub struct ThemePicker {
    themes: Vec<Theme>,
    list_state: ListState,
}

impl ThemePicker {
    pub fn new(themes: Vec<Theme>, active_name: &str) -> Self {
        todo!()
    }

    pub fn selected_theme_name(&self) -> &str {
        todo!()
    }
}

impl Component for ThemePicker {
    fn handle_event(&mut self, event: &AppEvent, _tx: &AppTx) -> EventState {
        todo!()
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_themes() -> Vec<Theme> {
        vec![
            Theme::gruvbox_dark(),
            Theme::gruvbox_light(),
            Theme::catppuccin_mocha(),
        ]
    }

    #[test]
    fn selected_theme_name_returns_initial() {
        let picker = ThemePicker::new(make_themes(), "Gruvbox Light");
        assert_eq!(picker.selected_theme_name(), "Gruvbox Light");
    }

    #[test]
    fn down_moves_selection() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, KeyEventKind, KeyEventState};
        use crate::components::events::AppEvent;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut picker = ThemePicker::new(make_themes(), "Gruvbox Dark");
        let key = AppEvent::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        picker.handle_event(&key, &tx);
        assert_eq!(picker.selected_theme_name(), "Gruvbox Light");
    }

    #[test]
    fn up_wraps_from_first_to_last() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, KeyEventKind, KeyEventState};
        use crate::components::events::AppEvent;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut picker = ThemePicker::new(make_themes(), "Gruvbox Dark");
        let key = AppEvent::Key(KeyEvent {
            code: KeyCode::Up,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        picker.handle_event(&key, &tx);
        assert_eq!(picker.selected_theme_name(), "Catppuccin Mocha");
    }

    #[test]
    fn down_wraps_from_last_to_first() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, KeyEventKind, KeyEventState};
        use crate::components::events::AppEvent;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut picker = ThemePicker::new(make_themes(), "Catppuccin Mocha");
        let key = AppEvent::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        picker.handle_event(&key, &tx);
        assert_eq!(picker.selected_theme_name(), "Gruvbox Dark");
    }

    #[test]
    fn j_key_moves_selection_down() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, KeyEventKind, KeyEventState};
        use crate::components::events::AppEvent;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut picker = ThemePicker::new(make_themes(), "Gruvbox Dark");
        let key = AppEvent::Key(KeyEvent {
            code: KeyCode::Char('j'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        picker.handle_event(&key, &tx);
        assert_eq!(picker.selected_theme_name(), "Gruvbox Light");
    }

    #[test]
    fn k_key_wraps_from_first_to_last() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, KeyEventKind, KeyEventState};
        use crate::components::events::AppEvent;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut picker = ThemePicker::new(make_themes(), "Gruvbox Dark");
        let key = AppEvent::Key(KeyEvent {
            code: KeyCode::Char('k'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        picker.handle_event(&key, &tx);
        assert_eq!(picker.selected_theme_name(), "Catppuccin Mocha");
    }

    #[test]
    fn renders_without_panic() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut picker = ThemePicker::new(make_themes(), "Gruvbox Dark");
        let theme = Theme::gruvbox_dark();
        terminal.draw(|f| {
            picker.render(f, f.area(), &theme, false);
        }).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let flat: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(flat.contains("Gruvbox Dark"), "Expected theme name in rendered output");
    }
}
```

- [ ] **Step 2: Run tests — must fail (todo! panics)**

```bash
cd tui && cargo test -p tui components::settings::theme_picker 2>&1 | head -30
```

Expected: tests fail with `not yet implemented`.

- [ ] **Step 3: Implement ThemePicker**

Replace the `todo!()` bodies with:

```rust
impl ThemePicker {
    pub fn new(themes: Vec<Theme>, active_name: &str) -> Self {
        let idx = themes.iter().position(|t| t.name == active_name).unwrap_or(0);
        let mut list_state = ListState::default();
        list_state.select(Some(idx));
        Self { themes, list_state }
    }

    pub fn selected_theme_name(&self) -> &str {
        // Precondition: themes must be non-empty (always true when constructed from AppSettings::theme_list())
        debug_assert!(!self.themes.is_empty(), "ThemePicker requires at least one theme");
        let idx = self.list_state.selected().unwrap_or(0);
        &self.themes[idx].name
    }
}

impl Component for ThemePicker {
    fn handle_event(&mut self, event: &AppEvent, _tx: &AppTx) -> EventState {
        let AppEvent::Key(key) = event else { return EventState::NotConsumed; };
        let count = self.themes.len();
        match key.code {
            ratatui::crossterm::event::KeyCode::Down | ratatui::crossterm::event::KeyCode::Char('j') => {
                let cur = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((cur + 1) % count));
                EventState::Consumed
            }
            ratatui::crossterm::event::KeyCode::Up | ratatui::crossterm::event::KeyCode::Char('k') => {
                let cur = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((cur + count - 1) % count));
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let border_style = theme.border_style(focused);
        let block = Block::default()
            .title("Theme")
            .borders(Borders::ALL)
            .border_style(border_style);
        // Selection indicator is rendered via manual prefix; no highlight_style needed.
        // Use render_widget (not render_stateful_widget) to avoid misleading stateful usage.
        let items: Vec<ListItem> = self.themes.iter().enumerate().map(|(i, t)| {
            let selected = self.list_state.selected() == Some(i);
            let prefix = if selected { "● " } else { "  " };
            ListItem::new(format!("{}{}", prefix, t.name))
        }).collect();
        let list = List::new(items).block(block);
        f.render_widget(list, rect);
    }
}
```

- [ ] **Step 4: Run tests — must all pass**

```bash
cd tui && cargo test -p tui components::settings::theme_picker 2>&1
```

Expected: all 7 tests pass.

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/settings/theme_picker.rs
git commit -m "feat: implement ThemePicker component"
```

---

### Task 6: VaultSection

**Files:**
- Modify: `tui/src/components/settings/vault_section.rs`

- [ ] **Step 1: Write failing tests**

Replace the placeholder with:

```rust
use std::path::PathBuf;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::components::Component;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::settings::themes::Theme;

pub struct VaultSection {
    current_path: Option<PathBuf>,
}

impl VaultSection {
    pub fn new(current_path: Option<PathBuf>) -> Self {
        todo!()
    }

    pub fn set_path(&mut self, path: Option<PathBuf>) {
        todo!()
    }
}

impl Component for VaultSection {
    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState {
        todo!()
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_no_vault_set_when_none() {
        // We test the display text by checking the component constructs without panic.
        // Render tests use TestBackend in integration; here we just verify the state.
        let section = VaultSection::new(None);
        assert!(section.current_path.is_none());
    }

    #[test]
    fn renders_path_when_some() {
        let path = PathBuf::from("/Users/me/notes");
        let section = VaultSection::new(Some(path.clone()));
        assert_eq!(section.current_path.as_ref().unwrap(), &path);
    }

    #[test]
    fn set_path_updates_current() {
        let mut section = VaultSection::new(None);
        let path = PathBuf::from("/Users/me/notes");
        section.set_path(Some(path.clone()));
        assert_eq!(section.current_path.as_ref().unwrap(), &path);
    }

    #[test]
    fn enter_sends_open_file_browser() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, KeyEventKind, KeyEventState};
        use crate::components::events::AppEvent;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = VaultSection::new(None);
        let key = AppEvent::Key(KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        let result = section.handle_event(&key, &tx);
        assert!(matches!(result, crate::components::event_state::EventState::Consumed));
        let msg = rx.try_recv().expect("message should be sent");
        assert!(matches!(msg, AppMessage::OpenFileBrowser));
    }

    #[test]
    fn b_key_sends_open_file_browser() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, KeyEventKind, KeyEventState};
        use crate::components::events::AppEvent;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = VaultSection::new(None);
        let key = AppEvent::Key(KeyEvent {
            code: KeyCode::Char('b'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        let result = section.handle_event(&key, &tx);
        assert!(matches!(result, crate::components::event_state::EventState::Consumed));
        let msg = rx.try_recv().expect("message should be sent");
        assert!(matches!(msg, AppMessage::OpenFileBrowser));
    }

    #[test]
    fn renders_no_vault_set_text() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let backend = TestBackend::new(60, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut section = VaultSection::new(None);
        let theme = crate::settings::themes::Theme::gruvbox_dark();
        terminal.draw(|f| {
            section.render(f, f.area(), &theme, false);
        }).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let flat: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(flat.contains("(no vault set)"), "Expected '(no vault set)' in rendered output");
    }

    #[test]
    fn renders_vault_path_text() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let backend = TestBackend::new(60, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let path = PathBuf::from("/Users/me/notes");
        let mut section = VaultSection::new(Some(path));
        let theme = crate::settings::themes::Theme::gruvbox_dark();
        terminal.draw(|f| {
            section.render(f, f.area(), &theme, false);
        }).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let flat: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(flat.contains("notes"), "Expected vault path in rendered output");
    }
}
```

- [ ] **Step 2: Run tests — must fail**

```bash
cd tui && cargo test -p tui components::settings::vault_section 2>&1 | head -20
```

Expected: tests fail with `not yet implemented`.

- [ ] **Step 3: Implement VaultSection**

Replace the `todo!()` bodies:

```rust
impl VaultSection {
    pub fn new(current_path: Option<PathBuf>) -> Self {
        Self { current_path }
    }

    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.current_path = path;
    }
}

impl Component for VaultSection {
    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState {
        let AppEvent::Key(key) = event else { return EventState::NotConsumed; };
        match key.code {
            ratatui::crossterm::event::KeyCode::Enter
            | ratatui::crossterm::event::KeyCode::Char('b') => {
                tx.send(AppMessage::OpenFileBrowser).ok();
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let border_style = theme.border_style(focused);
        let path_str = self.current_path.as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "(no vault set)".to_string());
        let text = format!("{}    [Enter: Browse]", path_str);
        let block = Block::default()
            .title("Vault Path")
            .borders(Borders::ALL)
            .border_style(border_style);
        let para = Paragraph::new(text).block(block);
        f.render_widget(para, rect);
    }
}
```

- [ ] **Step 4: Run tests — must all pass**

```bash
cd tui && cargo test -p tui components::settings::vault_section 2>&1
```

Expected: all 7 tests pass.

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/settings/vault_section.rs
git commit -m "feat: implement VaultSection component"
```

---

### Task 7: IndexingSection

**Files:**
- Modify: `tui/src/components/settings/indexing_section.rs`

- [ ] **Step 1: Write failing tests**

Replace the placeholder with:

```rust
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::components::Component;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::settings::themes::Theme;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IndexAction { Fast, Full }

pub struct IndexingSection {
    selected: IndexAction,
    vault_available: bool,
}

impl IndexingSection {
    pub fn new(vault_available: bool) -> Self {
        todo!()
    }

    pub fn set_vault_available(&mut self, available: bool) {
        todo!()
    }
}

impl Component for IndexingSection {
    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState {
        todo!()
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, KeyEventKind, KeyEventState};
    use crate::components::events::AppEvent;

    fn key(code: KeyCode) -> AppEvent {
        AppEvent::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    #[test]
    fn not_consumed_when_vault_unavailable() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = IndexingSection::new(false);
        let enter_result = section.handle_event(&key(KeyCode::Enter), &tx);
        assert!(matches!(enter_result, crate::components::event_state::EventState::NotConsumed));
        let right_result = section.handle_event(&key(KeyCode::Right), &tx);
        assert!(matches!(right_result, crate::components::event_state::EventState::NotConsumed));
        let left_result = section.handle_event(&key(KeyCode::Left), &tx);
        assert!(matches!(left_result, crate::components::event_state::EventState::NotConsumed));
        let l_result = section.handle_event(&key(KeyCode::Char('l')), &tx);
        assert!(matches!(l_result, crate::components::event_state::EventState::NotConsumed));
        let h_result = section.handle_event(&key(KeyCode::Char('h')), &tx);
        assert!(matches!(h_result, crate::components::event_state::EventState::NotConsumed));
        assert!(rx.try_recv().is_err(), "No messages should be sent when vault_available == false");
    }

    #[test]
    fn set_vault_available_enables_keys() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = IndexingSection::new(false);
        // Keys blocked before enabling
        section.handle_event(&key(KeyCode::Enter), &tx);
        assert!(rx.try_recv().is_err(), "Enter should be blocked when unavailable");
        // Enable and verify keys now work
        section.set_vault_available(true);
        section.handle_event(&key(KeyCode::Enter), &tx);
        let msg = rx.try_recv().expect("Enter should send message after enabling");
        assert!(matches!(msg, AppMessage::TriggerFastReindex));
    }

    #[test]
    fn right_is_idempotent_when_already_full() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = IndexingSection::new(true);
        section.handle_event(&key(KeyCode::Right), &tx); // Fast → Full
        section.handle_event(&key(KeyCode::Right), &tx); // Full → Full (no change)
        assert_eq!(section.selected, IndexAction::Full);
    }

    #[test]
    fn right_cycles_fast_to_full() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = IndexingSection::new(true);
        assert_eq!(section.selected, IndexAction::Fast);
        section.handle_event(&key(KeyCode::Right), &tx);
        assert_eq!(section.selected, IndexAction::Full);
    }

    #[test]
    fn left_cycles_full_to_fast() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = IndexingSection::new(true);
        section.handle_event(&key(KeyCode::Right), &tx); // now Full
        section.handle_event(&key(KeyCode::Left), &tx);
        assert_eq!(section.selected, IndexAction::Fast);
    }

    #[test]
    fn enter_on_fast_sends_trigger_fast_reindex() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = IndexingSection::new(true);
        section.handle_event(&key(KeyCode::Enter), &tx);
        let msg = rx.try_recv().expect("message should be sent");
        assert!(matches!(msg, AppMessage::TriggerFastReindex));
    }

    #[test]
    fn enter_on_full_sends_trigger_full_reindex() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut section = IndexingSection::new(true);
        section.handle_event(&key(KeyCode::Right), &tx); // select Full
        assert!(rx.try_recv().is_err(), "Right should not send any message");
        section.handle_event(&key(KeyCode::Enter), &tx);
        let msg = rx.try_recv().expect("message should be sent");
        assert!(matches!(msg, AppMessage::TriggerFullReindex));
    }
}
```

- [ ] **Step 2: Run tests — must fail**

```bash
cd tui && cargo test -p tui components::settings::indexing_section 2>&1 | head -20
```

Expected: tests fail with `not yet implemented`.

- [ ] **Step 3: Implement IndexingSection**

Replace the `todo!()` bodies:

```rust
impl IndexingSection {
    pub fn new(vault_available: bool) -> Self {
        Self { selected: IndexAction::Fast, vault_available }
    }

    pub fn set_vault_available(&mut self, available: bool) {
        self.vault_available = available;
    }
}

impl Component for IndexingSection {
    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState {
        if !self.vault_available {
            return EventState::NotConsumed;
        }
        let AppEvent::Key(key) = event else { return EventState::NotConsumed; };
        match key.code {
            ratatui::crossterm::event::KeyCode::Right
            | ratatui::crossterm::event::KeyCode::Char('l') => {
                self.selected = IndexAction::Full;
                EventState::Consumed
            }
            ratatui::crossterm::event::KeyCode::Left
            | ratatui::crossterm::event::KeyCode::Char('h') => {
                self.selected = IndexAction::Fast;
                EventState::Consumed
            }
            ratatui::crossterm::event::KeyCode::Enter => {
                let msg = match self.selected {
                    IndexAction::Fast => AppMessage::TriggerFastReindex,
                    IndexAction::Full => AppMessage::TriggerFullReindex,
                };
                tx.send(msg).ok();
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let border_style = theme.border_style(focused);
        let block = Block::default()
            .title("Reindex")
            .borders(Borders::ALL)
            .border_style(border_style);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let fast_label = if self.selected == IndexAction::Fast { "[ Fast Reindex ]" } else { "  Fast Reindex  " };
        let full_label = if self.selected == IndexAction::Full { "[ Full Reindex ]" } else { "  Full Reindex  " };
        let dim = if self.vault_available { Style::default() } else { Style::default().add_modifier(Modifier::DIM) };

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(inner);
        f.render_widget(Paragraph::new(fast_label).style(dim), cols[0]);
        f.render_widget(Paragraph::new(full_label).style(dim), cols[1]);
    }
}
```

- [ ] **Step 4: Run tests — must all pass**

```bash
cd tui && cargo test -p tui components::settings::indexing_section 2>&1
```

Expected: all 8 tests pass.

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/settings/indexing_section.rs
git commit -m "feat: implement IndexingSection component"
```

---

## Chunk 3: SettingsScreen full implementation and main.rs wiring

### Task 8: FileBrowserState

**Files:**
- Modify: `tui/src/app_screen/settings.rs`

- [ ] **Step 1: Write failing tests**

Add `FileBrowserState` with `todo!()` stubs and tests at the top of `tui/src/app_screen/settings.rs`:

```rust
use std::path::PathBuf;
use ratatui::widgets::ListState;

pub struct FileBrowserState {
    pub current_path: PathBuf,
    pub entries: Vec<PathBuf>,
    pub list_state: ListState,
}

impl FileBrowserState {
    pub fn load(path: PathBuf) -> Self {
        todo!()
    }

    pub fn navigate_into(&mut self, entry: PathBuf) {
        todo!()
    }

    pub fn go_up(&mut self) {
        todo!()
    }
}

#[cfg(test)]
mod file_browser_tests {
    use super::*;
    use std::fs;

    fn make_temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("kimun_test_{}", name));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn load_returns_only_directories() {
        let root = make_temp_dir("fb_only_dirs");
        fs::create_dir(root.join("alpha")).unwrap();
        fs::create_dir(root.join("beta")).unwrap();
        fs::write(root.join("note.md"), b"text").unwrap();

        let state = FileBrowserState::load(root.clone());

        assert_eq!(state.entries.len(), 2);
        assert!(state.entries.iter().all(|e| e.is_dir()));
    }

    #[test]
    fn load_sorts_alphabetically() {
        let root = make_temp_dir("fb_sorted");
        fs::create_dir(root.join("zebra")).unwrap();
        fs::create_dir(root.join("alpha")).unwrap();
        fs::create_dir(root.join("mango")).unwrap();

        let state = FileBrowserState::load(root.clone());

        let names: Vec<_> = state.entries.iter()
            .map(|e| e.file_name().unwrap().to_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["alpha", "mango", "zebra"]);
    }

    #[test]
    fn load_handles_empty_directory() {
        let root = make_temp_dir("fb_empty");
        let state = FileBrowserState::load(root.clone());
        assert_eq!(state.current_path, root);
        assert!(state.entries.is_empty());
        assert_eq!(state.list_state.selected(), None);
    }

    #[test]
    fn navigate_into_updates_path_and_reloads() {
        let root = make_temp_dir("fb_nav");
        let sub = root.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::create_dir(sub.join("child")).unwrap();

        let mut state = FileBrowserState::load(root.clone());
        state.navigate_into(sub.clone());

        assert_eq!(state.current_path, sub);
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].file_name().unwrap(), "child");
    }

    #[test]
    fn go_up_updates_to_parent() {
        let root = make_temp_dir("fb_go_up");
        let sub = root.join("sub");
        fs::create_dir_all(&sub).unwrap();

        let mut state = FileBrowserState::load(sub.clone());
        state.go_up();

        assert_eq!(state.current_path, root);
    }
}
```

- [ ] **Step 2: Run tests — must fail**

```bash
cd tui && cargo test -p tui app_screen::settings::file_browser_tests 2>&1 | head -20
```

Expected: tests fail with `not yet implemented`.

- [ ] **Step 3: Implement FileBrowserState**

Replace `todo!()` bodies:

```rust
impl FileBrowserState {
    pub fn load(path: PathBuf) -> Self {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        entries.sort();
        let mut list_state = ListState::default();
        if !entries.is_empty() {
            list_state.select(Some(0));
        }
        Self { current_path: path, entries, list_state }
    }

    pub fn navigate_into(&mut self, entry: PathBuf) {
        *self = Self::load(entry);
    }

    pub fn go_up(&mut self) {
        if let Some(parent) = self.current_path.parent() {
            *self = Self::load(parent.to_path_buf());
        }
    }
}
```

- [ ] **Step 4: Run tests — must all pass**

```bash
cd tui && cargo test -p tui app_screen::settings::file_browser_tests 2>&1
```

Expected: all 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add tui/src/app_screen/settings.rs
git commit -m "feat: FileBrowserState with TDD"
```

---

### Task 9: SettingsScreen full implementation

**Files:**
- Modify: `tui/src/app_screen/settings.rs`

This task replaces the stub `SettingsScreen` (from Task 4) with the full implementation including overlays, full event routing, and all wiring.

- [ ] **Step 1: Write failing tests**

Add these tests inside the existing `#[cfg(test)]` block in `settings.rs`. They will fail until Step 3 is complete because the fields and methods they reference do not exist yet.

```rust
#[cfg(test)]
mod settings_screen_tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, KeyEventKind, KeyEventState};
    use tokio::sync::mpsc::unbounded_channel;
    use crate::components::app_message::AppMessage;
    use crate::components::events::AppEvent;
    use crate::settings::AppSettings;

    fn key(code: KeyCode) -> AppEvent {
        AppEvent::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn make_screen() -> SettingsScreen {
        SettingsScreen::new(AppSettings::default())
    }

    #[test]
    fn esc_sends_close_settings_when_no_changes() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.handle_event(&key(KeyCode::Esc), &tx);
        let msg = rx.try_recv().expect("expected message");
        assert!(matches!(msg, AppMessage::CloseSettings));
    }

    #[test]
    fn esc_shows_confirm_save_when_settings_changed() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        // Mutate settings so they differ from initial
        screen.settings.set_theme("Gruvbox Light".to_string());
        screen.handle_event(&key(KeyCode::Esc), &tx);
        assert!(rx.try_recv().is_err(), "no message should be sent yet");
        assert!(matches!(screen.overlay, Overlay::ConfirmSave { .. }));
    }

    #[test]
    fn confirm_save_discard_sends_close_settings() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.settings.set_theme("Gruvbox Light".to_string());
        screen.overlay = Overlay::ConfirmSave { focused_button: SaveButton::Discard };
        screen.handle_event(&key(KeyCode::Enter), &tx);
        let msg = rx.try_recv().expect("expected message");
        assert!(matches!(msg, AppMessage::CloseSettings));
    }

    #[test]
    fn confirm_save_save_vault_unchanged_sends_settings_saved() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.settings.set_theme("Gruvbox Light".to_string());
        screen.overlay = Overlay::ConfirmSave { focused_button: SaveButton::Save };
        screen.handle_event(&key(KeyCode::Enter), &tx);
        let msg = rx.try_recv().expect("expected message");
        assert!(matches!(msg, AppMessage::SettingsSaved(_)));
        // CloseSettings must NOT also be sent
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn confirm_save_vault_changed_sets_pending_and_shows_progress() {
        use std::path::PathBuf;
        let (tx, _rx) = unbounded_channel();
        let mut settings = AppSettings::default();
        settings.set_workspace(&PathBuf::from("/original/path"));
        let mut screen = SettingsScreen::new(settings);
        // Now change the vault path so it differs from initial
        screen.settings.set_workspace(&PathBuf::from("/new/path"));
        screen.overlay = Overlay::ConfirmSave { focused_button: SaveButton::Save };
        screen.handle_event(&key(KeyCode::Enter), &tx);
        assert!(screen.pending_save_after_index, "pending flag must be set");
        assert!(matches!(screen.overlay, Overlay::IndexingProgress(IndexingProgressState::Running(_))));
    }

    #[tokio::test]
    async fn indexing_done_ok_with_pending_auto_closes() {
        use std::time::Duration;
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.pending_save_after_index = true;
        screen.overlay = Overlay::IndexingProgress(IndexingProgressState::Running(
            tokio::spawn(async {}),
        ));
        screen.handle_app_message(AppMessage::IndexingDone(Ok(Duration::from_secs(1))), &tx).await;
        let msg = rx.try_recv().expect("expected SettingsSaved");
        assert!(matches!(msg, AppMessage::SettingsSaved(_)));
        assert!(!screen.pending_save_after_index);
    }

    #[tokio::test]
    async fn indexing_done_err_with_pending_shows_failed_no_save() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.pending_save_after_index = true;
        screen.overlay = Overlay::IndexingProgress(IndexingProgressState::Running(
            tokio::spawn(async {}),
        ));
        screen.handle_app_message(
            AppMessage::IndexingDone(Err("disk error".to_string())), &tx
        ).await;
        assert!(rx.try_recv().is_err(), "no SettingsSaved when index failed");
        assert!(!screen.pending_save_after_index, "pending must be cleared");
        assert!(matches!(screen.overlay, Overlay::IndexingProgress(IndexingProgressState::Failed(_))));
    }

    #[tokio::test]
    async fn indexing_done_ok_without_pending_shows_done() {
        use std::time::Duration;
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.pending_save_after_index = false;
        screen.overlay = Overlay::IndexingProgress(IndexingProgressState::Running(
            tokio::spawn(async {}),
        ));
        screen.handle_app_message(AppMessage::IndexingDone(Ok(Duration::from_secs(2))), &tx).await;
        assert!(rx.try_recv().is_err(), "no auto-close when pending is false");
        assert!(matches!(screen.overlay, Overlay::IndexingProgress(IndexingProgressState::Done(_))));
    }

    #[test]
    fn esc_blocked_while_indexing_running() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.overlay = Overlay::IndexingProgress(IndexingProgressState::Running(
            tokio::runtime::Handle::current().spawn(async {}),
        ));
        screen.handle_event(&key(KeyCode::Esc), &tx);
        assert!(rx.try_recv().is_err(), "Esc must be blocked while indexing");
    }

    #[tokio::test]
    async fn confirm_full_reindex_esc_closes_overlay() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.overlay = Overlay::ConfirmFullReindex { focused_button: ConfirmButton::Cancel };
        screen.handle_event(&key(KeyCode::Esc), &tx);
        assert!(matches!(screen.overlay, Overlay::None));
    }
}
```

- [ ] **Step 2: Run tests — must fail**

```bash
cd tui && cargo test -p tui app_screen::settings::settings_screen_tests 2>&1 | head -20
```

Expected: compile errors / `not yet implemented` — fields `overlay`, `pending_save_after_index`, enums `Overlay`, `SaveButton`, `ConfirmButton`, `IndexingProgressState` do not exist yet.

- [ ] **Step 3: Implement SettingsScreen overlays and full logic**

Replace the entire `tui/src/app_screen/settings.rs` with:

```rust
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use kimun_core::{NoteVault, NotesValidation};
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::crossterm::event::KeyCode;
use throbber_widgets_tui::{Throbber, ThrobberState};

use crate::app_screen::AppScreen;
use crate::components::Component;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::components::settings::theme_picker::ThemePicker;
use crate::components::settings::vault_section::VaultSection;
use crate::components::settings::indexing_section::IndexingSection;
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

// ── FileBrowserState ─────────────────────────────────────────────────────────

pub struct FileBrowserState {
    pub current_path: PathBuf,
    pub entries: Vec<PathBuf>,
    pub list_state: ListState,
}

impl FileBrowserState {
    pub fn load(path: PathBuf) -> Self {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        entries.sort();
        let mut list_state = ListState::default();
        if !entries.is_empty() {
            list_state.select(Some(0));
        }
        Self { current_path: path, entries, list_state }
    }

    pub fn navigate_into(&mut self, entry: PathBuf) {
        *self = Self::load(entry);
    }

    pub fn go_up(&mut self) {
        if let Some(parent) = self.current_path.parent() {
            *self = Self::load(parent.to_path_buf());
        }
    }
}

// ── Overlay types ────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub enum ConfirmButton { Cancel, Confirm }

#[derive(Debug, PartialEq)]
pub enum SaveButton { Save, Discard }

pub enum IndexingProgressState {
    Running(tokio::task::JoinHandle<()>),
    Done(Duration),
    Failed(String),
}

pub enum Overlay {
    None,
    FileBrowser(FileBrowserState),
    ConfirmFullReindex { focused_button: ConfirmButton },
    ConfirmSave { focused_button: SaveButton },
    IndexingProgress(IndexingProgressState),
}

// ── Section / Focus enums ────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum SettingsSection { Theme, Vault, Indexing }

#[derive(Clone, Copy, PartialEq)]
enum SettingsFocus { Sidebar, Content }

// ── SettingsScreen ───────────────────────────────────────────────────────────

pub struct SettingsScreen {
    pub settings: AppSettings,
    pub initial_settings: AppSettings,
    pub theme: Theme,
    section: SettingsSection,
    focus: SettingsFocus,
    theme_picker: ThemePicker,
    vault_section: VaultSection,
    indexing_section: IndexingSection,
    pub overlay: Overlay,
    pub pending_save_after_index: bool,
    throbber_state: ThrobberState,
}

impl SettingsScreen {
    pub fn new(settings: AppSettings) -> Self {
        let theme = settings.get_theme();
        let themes = settings.theme_list();
        let active_name = settings.theme.clone();
        let vault_path = settings.workspace_dir.clone();
        let vault_available = vault_path.is_some();
        let initial_settings = settings.clone();
        Self {
            theme_picker: ThemePicker::new(themes, &active_name),
            vault_section: VaultSection::new(vault_path),
            indexing_section: IndexingSection::new(vault_available),
            settings,
            initial_settings,
            theme,
            section: SettingsSection::Theme,
            focus: SettingsFocus::Sidebar,
            overlay: Overlay::None,
            pending_save_after_index: false,
            throbber_state: ThrobberState::default(),
        }
    }

    fn do_save(&mut self, tx: &AppTx) {
        if self.settings.workspace_dir != self.initial_settings.workspace_dir {
            self.pending_save_after_index = true;
            let workspace = self.settings.workspace_dir.clone().unwrap();
            let tx2 = tx.clone();
            let handle = tokio::spawn(async move {
                let start = std::time::Instant::now();
                let result = async {
                    let vault = NoteVault::new(&workspace).await
                        .map_err(|e| e.to_string())?;
                    vault.recreate_index().await
                        .map_err(|e| e.to_string())
                        .map(|r| r.duration)
                }.await;
                // Send periodic redraws while running (this fires at most once after task completes
                // since we don't have a loop here — a real loop lives in the spawn below).
                let _ = start; // suppress unused warning
                tx2.send(AppMessage::IndexingDone(result)).ok();
            });
            self.overlay = Overlay::IndexingProgress(IndexingProgressState::Running(handle));
        } else {
            self.settings.save_to_disk().ok();
            let settings = self.settings.clone();
            tx.send(AppMessage::SettingsSaved(settings)).ok();
        }
    }
}

// ── Handle events ────────────────────────────────────────────────────────────

#[async_trait]
impl AppScreen for SettingsScreen {
    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState {
        // Route to overlay first if one is active.
        match &mut self.overlay {
            Overlay::None => {}

            Overlay::FileBrowser(fb) => {
                let AppEvent::Key(key) = event else { return EventState::NotConsumed; };
                match key.code {
                    KeyCode::Esc => { self.overlay = Overlay::None; }
                    KeyCode::Up => {
                        let n = fb.entries.len();
                        if n > 0 {
                            let cur = fb.list_state.selected().unwrap_or(0);
                            fb.list_state.select(Some((cur + n - 1) % n));
                        }
                    }
                    KeyCode::Down => {
                        let n = fb.entries.len();
                        if n > 0 {
                            let cur = fb.list_state.selected().unwrap_or(0);
                            fb.list_state.select(Some((cur + 1) % n));
                        }
                    }
                    KeyCode::Left => { fb.go_up(); }
                    KeyCode::Right | KeyCode::Enter => {
                        if let Some(idx) = fb.list_state.selected() {
                            if let Some(entry) = fb.entries.get(idx).cloned() {
                                fb.navigate_into(entry);
                            }
                        }
                    }
                    KeyCode::Char('c') | KeyCode::Char('\n') if key.modifiers.contains(
                        ratatui::crossterm::event::KeyModifiers::CONTROL
                    ) || key.code == KeyCode::Char('c') => {
                        // Confirm current directory
                        let chosen = fb.current_path.clone();
                        self.settings.set_workspace(&chosen);
                        self.vault_section.set_path(Some(chosen));
                        self.indexing_section.set_vault_available(true);
                        self.overlay = Overlay::None;
                    }
                    _ => {}
                }
                return EventState::Consumed;
            }

            Overlay::ConfirmFullReindex { focused_button } => {
                let AppEvent::Key(key) = event else { return EventState::NotConsumed; };
                match key.code {
                    KeyCode::Esc => { self.overlay = Overlay::None; }
                    KeyCode::Left | KeyCode::Char('h') => {
                        *focused_button = ConfirmButton::Cancel;
                    }
                    KeyCode::Right | KeyCode::Char('l') => {
                        *focused_button = ConfirmButton::Confirm;
                    }
                    KeyCode::Enter => {
                        if *focused_button == ConfirmButton::Confirm {
                            let workspace = self.settings.workspace_dir.clone().unwrap();
                            let tx2 = tx.clone();
                            let handle = tokio::spawn(async move {
                                let result = async {
                                    let vault = NoteVault::new(&workspace).await
                                        .map_err(|e| e.to_string())?;
                                    vault.recreate_index().await
                                        .map_err(|e| e.to_string())
                                        .map(|r| r.duration)
                                }.await;
                                tx2.send(AppMessage::IndexingDone(result)).ok();
                            });
                            self.overlay = Overlay::IndexingProgress(IndexingProgressState::Running(handle));
                        } else {
                            self.overlay = Overlay::None;
                        }
                    }
                    _ => {}
                }
                return EventState::Consumed;
            }

            Overlay::ConfirmSave { focused_button } => {
                let AppEvent::Key(key) = event else { return EventState::NotConsumed; };
                match key.code {
                    KeyCode::Esc => { self.overlay = Overlay::None; }
                    KeyCode::Left | KeyCode::Char('h') => {
                        *focused_button = SaveButton::Save;
                    }
                    KeyCode::Right | KeyCode::Char('l') => {
                        *focused_button = SaveButton::Discard;
                    }
                    KeyCode::Enter => {
                        if *focused_button == SaveButton::Save {
                            self.overlay = Overlay::None;
                            self.do_save(tx);
                        } else {
                            tx.send(AppMessage::CloseSettings).ok();
                        }
                    }
                    _ => {}
                }
                return EventState::Consumed;
            }

            Overlay::IndexingProgress(state) => {
                // Esc is blocked while Running; allowed on Done/Failed.
                match state {
                    IndexingProgressState::Running(_) => {
                        return EventState::Consumed; // swallow all input
                    }
                    IndexingProgressState::Done(_) | IndexingProgressState::Failed(_) => {
                        let AppEvent::Key(key) = event else { return EventState::NotConsumed; };
                        if key.code == KeyCode::Enter || key.code == KeyCode::Esc {
                            self.overlay = Overlay::None;
                        }
                        return EventState::Consumed;
                    }
                }
            }
        }

        // No overlay — handle global keys.
        let AppEvent::Key(key) = event else { return EventState::NotConsumed; };
        match key.code {
            KeyCode::Esc => {
                if self.settings == self.initial_settings {
                    tx.send(AppMessage::CloseSettings).ok();
                } else {
                    self.overlay = Overlay::ConfirmSave { focused_button: SaveButton::Save };
                }
                EventState::Consumed
            }
            KeyCode::Tab => {
                self.focus = match self.focus {
                    SettingsFocus::Sidebar => SettingsFocus::Content,
                    SettingsFocus::Content => SettingsFocus::Sidebar,
                };
                EventState::Consumed
            }
            _ => {
                match self.focus {
                    SettingsFocus::Sidebar => {
                        match key.code {
                            KeyCode::Down | KeyCode::Char('j') => {
                                self.section = match self.section {
                                    SettingsSection::Theme => SettingsSection::Vault,
                                    SettingsSection::Vault => SettingsSection::Indexing,
                                    SettingsSection::Indexing => SettingsSection::Theme,
                                };
                                EventState::Consumed
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                self.section = match self.section {
                                    SettingsSection::Theme => SettingsSection::Indexing,
                                    SettingsSection::Vault => SettingsSection::Theme,
                                    SettingsSection::Indexing => SettingsSection::Vault,
                                };
                                EventState::Consumed
                            }
                            _ => EventState::NotConsumed,
                        }
                    }
                    SettingsFocus::Content => {
                        let app_event = AppEvent::Key(*key);
                        let result = match self.section {
                            SettingsSection::Theme => {
                                let r = self.theme_picker.handle_event(&app_event, tx);
                                // Live theme preview: sync selected theme name into settings.
                                let name = self.theme_picker.selected_theme_name().to_string();
                                self.settings.set_theme(name);
                                self.theme = self.settings.get_theme();
                                r
                            }
                            SettingsSection::Vault => {
                                self.vault_section.handle_event(&app_event, tx)
                            }
                            SettingsSection::Indexing => {
                                self.indexing_section.handle_event(&app_event, tx)
                            }
                        };
                        result
                    }
                }
            }
        }
    }

    async fn handle_app_message(&mut self, msg: AppMessage, tx: &AppTx) -> Option<AppMessage> {
        match msg {
            AppMessage::OpenFileBrowser => {
                let starting_dir = self.settings.workspace_dir
                    .clone()
                    .or_else(|| std::env::var("HOME").ok().map(PathBuf::from))
                    .unwrap_or_else(|| PathBuf::from("/"));
                self.overlay = Overlay::FileBrowser(FileBrowserState::load(starting_dir));
                None
            }
            AppMessage::TriggerFastReindex => {
                let workspace = self.settings.workspace_dir.clone().unwrap();
                let tx2 = tx.clone();
                let handle = tokio::spawn(async move {
                    let result = async {
                        let vault = NoteVault::new(&workspace).await
                            .map_err(|e| e.to_string())?;
                        vault.index_notes(NotesValidation::Fast).await
                            .map_err(|e| e.to_string())
                            .map(|r| r.duration)
                    }.await;
                    tx2.send(AppMessage::IndexingDone(result)).ok();
                });
                self.overlay = Overlay::IndexingProgress(IndexingProgressState::Running(handle));
                None
            }
            AppMessage::TriggerFullReindex => {
                self.overlay = Overlay::ConfirmFullReindex { focused_button: ConfirmButton::Cancel };
                None
            }
            AppMessage::IndexingDone(result) => {
                match result {
                    Ok(duration) => {
                        self.settings.report_indexed();
                        if self.pending_save_after_index {
                            self.pending_save_after_index = false;
                            self.settings.save_to_disk().ok();
                            let settings = self.settings.clone();
                            tx.send(AppMessage::SettingsSaved(settings)).ok();
                        } else {
                            self.overlay = Overlay::IndexingProgress(IndexingProgressState::Done(duration));
                        }
                    }
                    Err(msg) => {
                        self.pending_save_after_index = false;
                        self.overlay = Overlay::IndexingProgress(IndexingProgressState::Failed(msg));
                    }
                }
                None
            }
            other => Some(other),
        }
    }

    fn render(&mut self, f: &mut Frame) {
        let theme = &self.theme.clone();

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(f.area());

        // Header
        let header = Block::default()
            .title("Settings")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border.to_ratatui()))
            .title_style(Style::default().fg(theme.accent.to_ratatui()));
        f.render_widget(header, rows[0]);

        // Main area: sidebar (left) + content (right)
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(20), Constraint::Min(0)])
            .split(rows[1]);

        // Sidebar navigation
        let sidebar_focused = self.focus == SettingsFocus::Sidebar;
        let sections = ["Theme", "Vault", "Indexing"];
        let active_idx = match self.section {
            SettingsSection::Theme => 0,
            SettingsSection::Vault => 1,
            SettingsSection::Indexing => 2,
        };
        let items: Vec<ListItem> = sections.iter().enumerate().map(|(i, name)| {
            let prefix = if i == active_idx { "> " } else { "  " };
            ListItem::new(format!("{}{}", prefix, name))
        }).collect();
        let sidebar_block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border_style(sidebar_focused));
        let sidebar_list = List::new(items).block(sidebar_block);
        f.render_widget(sidebar_list, cols[0]);

        // Content panel
        let content_focused = self.focus == SettingsFocus::Content;
        match self.section {
            SettingsSection::Theme => {
                self.theme_picker.render(f, cols[1], theme, content_focused);
            }
            SettingsSection::Vault => {
                self.vault_section.render(f, cols[1], theme, content_focused);
            }
            SettingsSection::Indexing => {
                self.indexing_section.render(f, cols[1], theme, content_focused);
            }
        }

        // Render overlay on top if active.
        self.render_overlay(f, theme);
    }
}

impl SettingsScreen {
    fn render_overlay(&mut self, f: &mut Frame, theme: &Theme) {
        match &mut self.overlay {
            Overlay::None => {}

            Overlay::FileBrowser(fb) => {
                let area = centered_rect(60, 80, f.area());
                f.render_widget(Clear, area);
                let block = Block::default()
                    .title("Select Vault Directory")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.accent.to_ratatui()));
                let inner = block.inner(area);
                f.render_widget(block, area);

                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)])
                    .split(inner);

                let path_str = fb.current_path.to_string_lossy();
                f.render_widget(Paragraph::new(path_str.as_ref()), rows[0]);

                let items: Vec<ListItem> = fb.entries.iter().map(|e| {
                    let name = e.file_name().unwrap_or_default().to_string_lossy();
                    ListItem::new(format!("  {}/", name))
                }).collect();
                let list = List::new(items)
                    .highlight_symbol("▶ ")
                    .highlight_style(Style::default().add_modifier(Modifier::BOLD));
                f.render_stateful_widget(list, rows[1], &mut fb.list_state);

                f.render_widget(
                    Paragraph::new("Enter: open  c: confirm  Esc: cancel"),
                    rows[2],
                );
            }

            Overlay::ConfirmFullReindex { focused_button } => {
                let area = centered_rect(50, 30, f.area());
                f.render_widget(Clear, area);
                let block = Block::default()
                    .title("Full Reindex")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.accent.to_ratatui()));
                let inner = block.inner(area);
                f.render_widget(block, area);

                let cancel_label = if *focused_button == ConfirmButton::Cancel { "[ Cancel ]" } else { "  Cancel  " };
                let confirm_label = if *focused_button == ConfirmButton::Confirm { "[ Confirm ]" } else { "  Confirm  " };
                let text = format!(
                    "\n  This may take a while on large vaults.\n\n  {}    {}",
                    cancel_label, confirm_label
                );
                f.render_widget(Paragraph::new(text), inner);
            }

            Overlay::ConfirmSave { focused_button } => {
                let area = centered_rect(50, 30, f.area());
                f.render_widget(Clear, area);
                let block = Block::default()
                    .title("Save Settings?")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.accent.to_ratatui()));
                let inner = block.inner(area);
                f.render_widget(block, area);

                let save_label = if *focused_button == SaveButton::Save { "[ Save ]" } else { "  Save  " };
                let discard_label = if *focused_button == SaveButton::Discard { "[ Discard ]" } else { "  Discard  " };
                let text = format!(
                    "\n  You have unsaved changes.\n\n  {}    {}",
                    save_label, discard_label
                );
                f.render_widget(Paragraph::new(text), inner);
            }

            Overlay::IndexingProgress(state) => {
                let area = centered_rect(50, 20, f.area());
                f.render_widget(Clear, area);
                let block = Block::default()
                    .title("Indexing")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.accent.to_ratatui()));
                let inner = block.inner(area);
                f.render_widget(block, area);

                match state {
                    IndexingProgressState::Running(_) => {
                        self.throbber_state.calc_next();
                        let throbber = Throbber::default()
                            .label("  Reindex in progress…");
                        f.render_stateful_widget(throbber, inner, &mut self.throbber_state);
                    }
                    IndexingProgressState::Done(dur) => {
                        let text = format!("  ✓  Done in {} second(s)\n\n       [ OK ]", dur.as_secs());
                        f.render_widget(Paragraph::new(text), inner);
                    }
                    IndexingProgressState::Failed(msg) => {
                        let text = format!("  ✗  Error: {}\n\n       [ OK ]", msg);
                        f.render_widget(Paragraph::new(text), inner);
                    }
                }
            }
        }
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
```

- [ ] **Step 4: Run tests — must all pass**

```bash
cd tui && cargo test -p tui app_screen::settings 2>&1
```

Expected: all tests in `file_browser_tests` and `settings_screen_tests` pass.

- [ ] **Step 5: Commit**

```bash
git add tui/src/app_screen/settings.rs
git commit -m "feat: full SettingsScreen with overlays and event routing"
```

---

### Task 10: main.rs — SettingsSaved and CloseSettings arms

**Files:**
- Modify: `tui/src/main.rs`

- [ ] **Step 1: Write failing tests**

These tests already exist from Task 4 (`app_screen::tests`). No new tests needed — the two existing tests verify the `other =>` routing. This step is a compile check: adding the `SettingsSaved` and `CloseSettings` arms will cause the `_= msg` discard in the old code to become a missing-arm compile error, confirming the tests are properly exhaustive.

Run the existing tests to confirm they still pass before making changes:

```bash
cd tui && cargo test -p tui app_screen::tests 2>&1
```

Expected: both pass.

- [ ] **Step 2: Add SettingsSaved and CloseSettings arms**

In `tui/src/main.rs`, inside the `while let Ok(msg) = rx.try_recv()` loop, add two arms **before** the `other =>` catch-all. Also add the `VaultPath` import if not present.

Add at top of file if missing:
```rust
use kimun_core::nfs::VaultPath;
```

Insert before the `other =>` arm:
```rust
AppMessage::SettingsSaved(new_settings) => {
    app.settings = new_settings;
    let path = app.settings.last_paths.last()
        .cloned()
        .unwrap_or_else(VaultPath::root);
    tx.send(AppMessage::OpenPath(path)).ok();
}
AppMessage::CloseSettings => {
    let path = app.settings.last_paths.last()
        .cloned()
        .unwrap_or_else(VaultPath::root);
    tx.send(AppMessage::OpenPath(path)).ok();
}
```

The final drain loop match block order must be:
1. `Quit`
2. `Redraw`
3. `OpenSettings`
4. `OpenEditor`
5. `OpenPath`
6. `SettingsSaved` ← new
7. `CloseSettings` ← new
8. `other =>` ← catch-all, must remain last

- [ ] **Step 3: Run full test suite**

```bash
cd tui && cargo test -p tui 2>&1
```

Expected: all tests pass, no compile errors.

- [ ] **Step 4: Commit**

```bash
git add tui/src/main.rs
git commit -m "feat: wire SettingsSaved and CloseSettings in main loop"
```

---
