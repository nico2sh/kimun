use std::path::PathBuf;

use async_trait::async_trait;
use kimun_core::error::VaultError;
use kimun_core::{NoteVault, NotesValidation, VaultConfig};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use throbber_widgets_tui::ThrobberState;

use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::Component;
use crate::components::button_row::ButtonRow;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::hint_row::{HintRow, is_press_outside, list_index_at};
use crate::components::indexing::{
    IndexingProgressState, fixed_centered_rect, render_indexing_overlay, spawn_running,
};
use crate::components::preferences::appearance_section::AppearanceSection;
use crate::components::preferences::display_section::DisplaySection;
use crate::components::preferences::editor_section::EditorSection;
use crate::components::preferences::indexing_section::IndexingSection;
use crate::components::preferences::server_section::ServerSection;
use crate::components::preferences::sorting_section::SortingSection;
use crate::components::preferences::workspaces_section::{
    Mode as WorkspaceMode, WorkspacesSection,
};
use crate::settings::AppSettings;
use crate::settings::SharedSettings;
use crate::settings::config_migration::CURRENT_CONFIG_VERSION;
use crate::settings::themes::Theme;

use crate::components::dir_browser::FileBrowserState;

// ── Overlay types ─────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub enum ConfirmButton {
    Cancel,
    Confirm,
}

#[derive(Debug, PartialEq)]
pub enum SaveButton {
    Save,
    Discard,
}

pub enum Overlay {
    None,
    FileBrowser(FileBrowserState),
    ConfirmFullReindex {
        focused_button: ConfirmButton,
    },
    ConfirmSave {
        focused_button: SaveButton,
    },
    IndexingProgress(IndexingProgressState),
    /// Vault was rejected due to structural errors (e.g. case conflicts).
    /// Rendered like the other confirmation dialogs but with a single close button.
    VaultConflict(String),
}

// ── Section / Focus enums ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum PreferencesSection {
    Workspaces,
    Appearance,
    Display,
    Sorting,
    Indexing,
    Editor,
    Server,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PreferencesFocus {
    Sidebar,
    Content,
}

// ── PreferencesScreen ────────────────────────────────────────────────────────────

pub struct PreferencesScreen {
    pub settings: SharedSettings,
    pub initial_settings: AppSettings,
    pub theme: Theme,
    section: PreferencesSection,
    focus: PreferencesFocus,
    appearance_section: AppearanceSection,
    display_section: DisplaySection,
    sorting_section: SortingSection,
    workspaces_section: WorkspacesSection,
    pending_create_name: Option<String>,
    indexing_section: IndexingSection,
    editor_section: EditorSection,
    server_section: ServerSection,
    pub overlay: Overlay,
    pub pending_save_after_index: bool,
    throbber_state: ThrobberState,
    /// Last mouse position, for the hover highlight (`components::clickable`).
    pointer: Option<Position>,
    /// Section-list rows from the last render (inside its border).
    sidebar_rows: Rect,
    /// The section content area from the last render.
    content_rect: Rect,
    /// The footer's `[Esc] Save & Close  [Tab] …` chips.
    footer: HintRow,
    /// The open overlay's outer rect from the last render; a press outside
    /// it cancels, like Esc.
    overlay_rect: Rect,
    /// Drops the tail of the double-click that opened an overlay (see
    /// [`OpenGuard`](crate::components::clickable::OpenGuard)); `overlay_shown`
    /// tracks when one first draws, which is when it counts as opened.
    open_guard: crate::components::clickable::OpenGuard,
    overlay_shown: bool,
    /// The folder picker's list rows and chips from the last render.
    browser_rows: Rect,
    browser_hints: HintRow,
    /// The two buttons of the Full Reindex / Save confirmations.
    confirm_buttons: ButtonRow,
}

impl PreferencesScreen {
    pub fn new(settings: SharedSettings) -> Self {
        let s = settings.read().unwrap();
        // Build the theme list once and resolve the active theme from it;
        // `get_theme()` would rebuild the entire list (plus disk IO) again.
        let themes = s.theme_list();
        let theme = themes
            .iter()
            .find(|t| t.name == s.theme)
            .cloned()
            .unwrap_or_default()
            .adapt_to_terminal();
        let active_name = theme.name.clone();
        let vault_available = s
            .workspace_config
            .as_ref()
            .is_some_and(|wc| wc.get_current_workspace().is_some());
        let autosave_interval_secs = s.autosave_interval_secs;
        let editor_backend = s.editor_backend;
        let use_nerd_fonts = s.use_nerd_fonts;
        let update_check = s.update_check();
        let mouse = s.mouse();
        let leader_timeout_ms = s.leader_timeout_ms;
        let server_url = s
            .workspace_config
            .as_ref()
            .and_then(|wc| wc.global.kimun_server_url.clone());
        let initial_settings = s.clone();
        let workspaces_section = WorkspacesSection::new(&s);
        let sorting_section = SortingSection::new(
            s.default_sort_field,
            s.default_sort_order,
            s.journal_sort_field,
            s.journal_sort_order,
        );
        drop(s);
        Self {
            appearance_section: AppearanceSection::new(themes, &active_name),
            display_section: DisplaySection::new(
                use_nerd_fonts,
                update_check,
                mouse,
                leader_timeout_ms,
            ),
            sorting_section,
            workspaces_section,
            pending_create_name: None,
            indexing_section: IndexingSection::new(vault_available),
            editor_section: EditorSection::new(autosave_interval_secs, editor_backend),
            server_section: ServerSection::new(server_url),
            settings,
            initial_settings,
            theme,
            section: PreferencesSection::Workspaces,
            focus: PreferencesFocus::Sidebar,
            overlay: Overlay::None,
            pending_save_after_index: false,
            throbber_state: ThrobberState::default(),
            pointer: None,
            sidebar_rows: Rect::default(),
            content_rect: Rect::default(),
            footer: HintRow::new(&[
                (KeyCode::Esc, "Esc", "Save & Close"),
                (KeyCode::Tab, "Tab", "Switch sidebar/content"),
            ]),
            overlay_rect: Rect::default(),
            open_guard: Default::default(),
            overlay_shown: false,
            browser_rows: Rect::default(),
            browser_hints: HintRow::new(&[
                (KeyCode::Enter, "⏎", "Open"),
                (KeyCode::Char('c'), "c", "Choose this folder"),
                (KeyCode::Esc, "Esc", "Cancel"),
                (KeyCode::Null, "a-z", "Jump"),
            ])
            .with_indent(0),
            confirm_buttons: ButtonRow::new(&["Cancel", "Confirm"]),
        }
    }

    /// Creates a settings screen with a `Failed` error overlay pre-populated.
    /// Used when the vault was rejected due to structural conflicts.
    ///
    /// The `settings` passed in should already have the workspace cleared —
    /// this is handled by the `VaultConflict` branch in `app::handle_app_message`
    /// before calling `switch_screen`.
    pub fn new_with_error(settings: SharedSettings, error: String) -> Self {
        let mut s = Self::new(settings);
        s.overlay = Overlay::VaultConflict(error);
        s
    }

    fn do_save(&mut self, tx: &AppTx) {
        let s = self.settings.read().unwrap();
        let current_path = s.resolve_workspace_path();
        let initial_path = self.initial_settings.resolve_workspace_path();
        if current_path != initial_path {
            let Some(workspace) = current_path else {
                drop(s);
                tx.send(AppEvent::IndexingDone(Err("No workspace set".to_string())))
                    .ok();
                return;
            };
            let workspace_name = s
                .workspace_config
                .as_ref()
                .map(|wc| wc.global.current_workspace.clone())
                .filter(|n| !n.is_empty());
            let cache_path = workspace_name.as_ref().map(|n| s.index_for(n));
            drop(s);
            self.pending_save_after_index = true;
            let tx2 = tx.clone();
            let handle = tokio::spawn(async move {
                let mut config = VaultConfig::new(workspace);
                if let Some(path) = cache_path {
                    config = config.with_index(path);
                }
                let event = match NoteVault::new(config).await {
                    Err(e) => AppEvent::IndexingDone(Err(e.to_string())),
                    Ok(vault) => {
                        let result = vault.recreate_index().await;
                        // Throwaway vault with its own pool — closing it does
                        // not touch the app's vault, and leaving it to drop
                        // keeps a second handle on the cache file open.
                        vault.close().await;
                        match result {
                            Ok(r) => AppEvent::IndexingDone(Ok(r.duration)),
                            Err(e @ VaultError::CaseConflict { .. }) => {
                                AppEvent::VaultConflict(e.to_string())
                            }
                            Err(e) => AppEvent::IndexingDone(Err(e.to_string())),
                        }
                    }
                };
                tx2.send(event).ok();
            });
            self.overlay = Overlay::IndexingProgress(spawn_running(handle, tx));
        } else {
            s.save_to_disk().ok();
            drop(s);
            tx.send(AppEvent::PreferencesSaved).ok();
        }
    }

    /// Called when the file browser confirms a directory path (via 'c' or Ctrl+Enter).
    fn confirm_file_browser(&mut self, chosen: PathBuf, _tx: &AppTx) {
        use crate::settings::workspace_config::WorkspaceConfig;

        if let Some(name) = self.pending_create_name.take() {
            let name = name.to_lowercase();
            {
                let mut s = self.settings.write().unwrap();
                if s.workspace_config.is_none() {
                    s.workspace_config = Some(WorkspaceConfig::new_empty());
                    s.config_version = CURRENT_CONFIG_VERSION;
                }
                if let Some(ref mut wc) = s.workspace_config
                    && let Err(e) = wc.add_workspace(name.clone(), chosen)
                {
                    tracing::warn!("rejected workspace add: {}", e);
                }
            }
            self.workspaces_section
                .refresh(&self.settings.read().unwrap());
            self.indexing_section.set_vault_available(true);
        } else {
            // Browsing path for the selected workspace.
            {
                let mut s = self.settings.write().unwrap();
                // Repoints the selected entry and flags a reindex when that
                // actually moves the vault — one call, so the two cannot
                // disagree the way they did when the reindex flag hung off a
                // separate legacy field.
                if let Some(name) = self
                    .workspaces_section
                    .selected_name()
                    .map(|s| s.to_string())
                {
                    s.set_workspace_path(&name, chosen);
                }
            }
            self.workspaces_section
                .refresh(&self.settings.read().unwrap());
            self.indexing_section.set_vault_available(true);
        }
        self.overlay = Overlay::None;
    }
}

impl PreferencesScreen {
    /// Write the current section's edited values into the shared settings
    /// (and, for Appearance, preview the theme). One place for keys and
    /// clicks, so a click that only moves a selection — a theme row —
    /// previews exactly like the arrow key does. Workspaces and Indexing
    /// act through their own key flows and have nothing to sync here.
    fn sync_section(&mut self) {
        use crate::settings::workspace_config::WorkspaceConfig;
        match self.section {
            PreferencesSection::Appearance => {
                // Take the theme straight from the section's already-loaded
                // list: `get_theme()` would re-read custom themes from disk
                // and rebuild every built-in on each step.
                let theme = self.appearance_section.selected_theme().clone();
                if self.settings.read().unwrap().theme != theme.name {
                    self.settings.write().unwrap().set_theme(theme.name.clone());
                    self.theme = theme.adapt_to_terminal();
                }
            }
            PreferencesSection::Display => {
                let mut s = self.settings.write().unwrap();
                s.use_nerd_fonts = self.display_section.use_nerd_fonts;
                let global = &mut s
                    .workspace_config
                    .get_or_insert_with(WorkspaceConfig::new_empty)
                    .global;
                global.update_check = self.display_section.update_check;
                global.mouse = self.display_section.mouse;
                s.leader_timeout_ms = self.display_section.leader_timeout_ms;
            }
            PreferencesSection::Sorting => {
                let mut s = self.settings.write().unwrap();
                s.default_sort_field = self.sorting_section.default_sort_field;
                s.default_sort_order = self.sorting_section.default_sort_order;
                s.journal_sort_field = self.sorting_section.journal_sort_field;
                s.journal_sort_order = self.sorting_section.journal_sort_order;
            }
            PreferencesSection::Editor => {
                let mut s = self.settings.write().unwrap();
                s.autosave_interval_secs = self.editor_section.autosave_interval_secs;
                s.editor_backend = self.editor_section.editor_backend;
            }
            PreferencesSection::Server => {
                self.settings
                    .write()
                    .unwrap()
                    .workspace_config
                    .get_or_insert_with(WorkspaceConfig::new_empty)
                    .global
                    .kimun_server_url = self.server_section.server_url.clone();
            }
            PreferencesSection::Workspaces | PreferencesSection::Indexing => {}
        }
    }

    /// Run `code` through the key path, exactly as if it had been pressed.
    fn press(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        self.handle_input(&InputEvent::Key(key), tx)
    }

    /// Every click is turned into the key it stands for and run through
    /// [`AppScreen::handle_input`] — settings write-back and the workspace
    /// create/rename/delete flows included — so a click can never do
    /// something the key would not.
    fn handle_mouse(&mut self, m: &MouseEvent, tx: &AppTx) -> EventState {
        let press = matches!(m.kind, MouseEventKind::Down(_));
        if !matches!(self.overlay, Overlay::None) {
            // The second press of the double-click that opened the overlay
            // is not aimed at it.
            if self
                .open_guard
                .swallows(&InputEvent::Mouse(*m), std::time::Instant::now())
            {
                return EventState::Consumed;
            }
            return self.handle_overlay_mouse(m, tx);
        }
        let pos = Position::new(m.column, m.row);
        if let Some(key) = self.footer.hit(m) {
            return self.press(key, tx);
        }
        // The section list: a row picks that section.
        if press && self.sidebar_rows.contains(pos) {
            self.focus = PreferencesFocus::Sidebar;
            if let Some(idx) = list_index_at(m, self.sidebar_rows, 0, SECTIONS.len()) {
                self.section = SECTIONS[idx];
            }
            return EventState::Consumed;
        }
        if !self.content_rect.contains(pos) {
            return EventState::NotConsumed;
        }
        if press {
            self.focus = PreferencesFocus::Content;
        }
        let key = match self.section {
            PreferencesSection::Workspaces => self.workspaces_section.handle_mouse(m),
            PreferencesSection::Appearance => self.appearance_section.handle_mouse(m),
            PreferencesSection::Display => self.display_section.handle_mouse(m),
            PreferencesSection::Sorting => self.sorting_section.handle_mouse(m),
            PreferencesSection::Indexing => self.indexing_section.handle_mouse(m),
            PreferencesSection::Editor => self.editor_section.handle_mouse(m),
            // A click on the field just focuses it; typing edits it.
            PreferencesSection::Server => None,
        };
        // A key runs through the key path, which syncs settings itself. The
        // one selection-only click that changes a setting is a theme row
        // (the live preview); every other section's selection is just a
        // cursor, and motion must never write settings.
        match key {
            Some(key) => self.press(key, tx),
            None => {
                if press && self.section == PreferencesSection::Appearance {
                    self.sync_section();
                }
                EventState::Consumed
            }
        }
    }

    fn handle_overlay_mouse(&mut self, m: &MouseEvent, tx: &AppTx) -> EventState {
        let esc = KeyEvent::from(KeyCode::Esc);
        let enter = KeyEvent::from(KeyCode::Enter);
        let press = matches!(m.kind, MouseEventKind::Down(_));
        match &mut self.overlay {
            Overlay::None => EventState::NotConsumed,
            // Nothing to click while the reindex runs.
            Overlay::IndexingProgress(IndexingProgressState::Running { .. }) => {
                EventState::Consumed
            }
            // One button (`[ OK ]`): any press dismisses.
            Overlay::IndexingProgress(_) | Overlay::VaultConflict(_) => {
                if press {
                    return self.press(enter, tx);
                }
                EventState::Consumed
            }
            Overlay::FileBrowser(fb) => {
                if is_press_outside(m, self.overlay_rect) {
                    return self.press(esc, tx);
                }
                if let Some(key) = self.browser_hints.hit(m) {
                    return self.press(key, tx);
                }
                let total = fb.entries.len() + usize::from(fb.has_parent);
                match m.kind {
                    MouseEventKind::ScrollUp => self.press(KeyEvent::from(KeyCode::Up), tx),
                    MouseEventKind::ScrollDown => self.press(KeyEvent::from(KeyCode::Down), tx),
                    _ => {
                        if let Some(idx) =
                            list_index_at(m, self.browser_rows, fb.list_state.offset(), total)
                        {
                            // Click selects; clicking the selected folder
                            // opens it, like Enter.
                            if fb.list_state.selected() == Some(idx) {
                                return self.press(enter, tx);
                            }
                            fb.list_state.select(Some(idx));
                        }
                        EventState::Consumed
                    }
                }
            }
            Overlay::ConfirmFullReindex { focused_button } => {
                if is_press_outside(m, self.overlay_rect) {
                    return self.press(esc, tx);
                }
                match self.confirm_buttons.hit(m.column, m.row) {
                    Some(idx) if press => {
                        *focused_button = if idx == 0 {
                            ConfirmButton::Cancel
                        } else {
                            ConfirmButton::Confirm
                        };
                        self.press(enter, tx)
                    }
                    _ => EventState::Consumed,
                }
            }
            Overlay::ConfirmSave { focused_button } => {
                if is_press_outside(m, self.overlay_rect) {
                    return self.press(esc, tx);
                }
                match self.confirm_buttons.hit(m.column, m.row) {
                    Some(idx) if press => {
                        *focused_button = if idx == 0 {
                            SaveButton::Save
                        } else {
                            SaveButton::Discard
                        };
                        self.press(enter, tx)
                    }
                    _ => EventState::Consumed,
                }
            }
        }
    }
}

/// The button line of a confirmation dialog: the fourth inner row (blank,
/// message, blank, buttons), clipped to the dialog.
fn button_line(inner: Rect) -> Rect {
    if inner.height < 4 {
        return Rect::default();
    }
    Rect::new(inner.x + 1, inner.y + 3, inner.width.saturating_sub(1), 1)
}

/// The sections in sidebar order.
const SECTIONS: [PreferencesSection; 7] = [
    PreferencesSection::Workspaces,
    PreferencesSection::Appearance,
    PreferencesSection::Display,
    PreferencesSection::Sorting,
    PreferencesSection::Indexing,
    PreferencesSection::Editor,
    PreferencesSection::Server,
];

// ── AppScreen impl ────────────────────────────────────────────────────────────

#[async_trait]
impl AppScreen for PreferencesScreen {
    fn get_kind(&self) -> ScreenKind {
        ScreenKind::Preferences
    }

    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        self.open_guard.observe_input(event);
        if let InputEvent::Mouse(m) = event {
            self.pointer = Some(Position::new(m.column, m.row));
            return self.handle_mouse(m, tx);
        }
        // Route to active overlay first.
        match &mut self.overlay {
            Overlay::None => {}

            Overlay::FileBrowser(fb) => {
                let InputEvent::Key(key) = event else {
                    return EventState::NotConsumed;
                };
                let offset = if fb.has_parent { 1 } else { 0 };
                let total = fb.entries.len() + offset;
                match key.code {
                    KeyCode::Esc => {
                        self.overlay = Overlay::None;
                    }
                    KeyCode::Up if total > 0 => {
                        let cur = fb.list_state.selected().unwrap_or(0);
                        fb.list_state.select(Some((cur + total - 1) % total));
                    }
                    KeyCode::Down if total > 0 => {
                        let cur = fb.list_state.selected().unwrap_or(0);
                        fb.list_state.select(Some((cur + 1) % total));
                    }
                    KeyCode::Left => {
                        fb.go_up();
                    }
                    KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        // Ctrl+Enter confirms the current directory.
                        let chosen = fb.current_path.clone();
                        self.confirm_file_browser(chosen, tx);
                    }
                    KeyCode::Right | KeyCode::Enter => {
                        if let Some(idx) = fb.list_state.selected() {
                            if fb.has_parent && idx == 0 {
                                fb.go_up();
                            } else if let Some(entry) = fb.entries.get(idx - offset).cloned() {
                                fb.navigate_into(entry);
                            }
                        }
                    }
                    KeyCode::Char('c') if key.modifiers.is_empty() => {
                        let chosen = fb.current_path.clone();
                        self.confirm_file_browser(chosen, tx);
                    }
                    // Shift is fine (uppercase jump targets); Ctrl/Alt chords
                    // must not be mistaken for plain letters.
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        fb.jump_to_char(c);
                    }
                    _ => {}
                }
                return EventState::Consumed;
            }

            Overlay::ConfirmFullReindex { focused_button } => {
                let InputEvent::Key(key) = event else {
                    return EventState::NotConsumed;
                };
                match key.code {
                    KeyCode::Esc => {
                        self.overlay = Overlay::None;
                    }
                    KeyCode::Left | KeyCode::Char('h') => {
                        *focused_button = ConfirmButton::Cancel;
                    }
                    KeyCode::Right | KeyCode::Char('l') => {
                        *focused_button = ConfirmButton::Confirm;
                    }
                    KeyCode::Enter => {
                        if *focused_button == ConfirmButton::Confirm {
                            let s = self.settings.read().unwrap();
                            let Some(workspace) = s.resolve_workspace_path() else {
                                drop(s);
                                self.overlay = Overlay::None;
                                return EventState::Consumed;
                            };
                            let workspace_name = s
                                .workspace_config
                                .as_ref()
                                .map(|wc| wc.global.current_workspace.clone())
                                .filter(|n| !n.is_empty());
                            let cache_path = workspace_name.as_ref().map(|n| s.index_for(n));
                            drop(s);
                            let tx2 = tx.clone();
                            let handle = tokio::spawn(async move {
                                let mut config = VaultConfig::new(workspace);
                                if let Some(path) = cache_path {
                                    config = config.with_index(path);
                                }
                                let event = match NoteVault::new(config).await {
                                    Err(e) => AppEvent::IndexingDone(Err(e.to_string())),
                                    Ok(vault) => {
                                        let result = vault.recreate_index().await;
                                        // Throwaway vault with its own pool —
                                        // see the reindex task above.
                                        vault.close().await;
                                        match result {
                                            Ok(r) => AppEvent::IndexingDone(Ok(r.duration)),
                                            Err(e @ VaultError::CaseConflict { .. }) => {
                                                AppEvent::VaultConflict(e.to_string())
                                            }
                                            Err(e) => AppEvent::IndexingDone(Err(e.to_string())),
                                        }
                                    }
                                };
                                tx2.send(event).ok();
                            });
                            self.overlay = Overlay::IndexingProgress(spawn_running(handle, tx));
                        } else {
                            self.overlay = Overlay::None;
                        }
                    }
                    _ => {}
                }
                return EventState::Consumed;
            }

            Overlay::ConfirmSave { focused_button } => {
                let InputEvent::Key(key) = event else {
                    return EventState::NotConsumed;
                };
                match key.code {
                    KeyCode::Esc => {
                        self.overlay = Overlay::None;
                    }
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
                            // Discard: section edits were written live into
                            // the shared settings (theme preview, autosave,
                            // editor_backend) — roll them back so a later
                            // save_to_disk can't persist discarded choices.
                            *self.settings.write().unwrap() = self.initial_settings.clone();
                            tx.send(AppEvent::ClosePreferences).ok();
                        }
                    }
                    _ => {}
                }
                return EventState::Consumed;
            }

            Overlay::IndexingProgress(state) => {
                match state {
                    IndexingProgressState::Running { .. } => {
                        return EventState::Consumed; // block all input while running
                    }
                    IndexingProgressState::Done(_) | IndexingProgressState::Failed(_) => {
                        let InputEvent::Key(key) = event else {
                            return EventState::NotConsumed;
                        };
                        if key.code == KeyCode::Enter || key.code == KeyCode::Esc {
                            self.overlay = Overlay::None;
                        }
                        return EventState::Consumed;
                    }
                }
            }

            Overlay::VaultConflict(_) => {
                let InputEvent::Key(key) = event else {
                    return EventState::NotConsumed;
                };
                if key.code == KeyCode::Enter || key.code == KeyCode::Esc {
                    self.overlay = Overlay::None;
                }
                return EventState::Consumed;
            }
        }

        // No active overlay — handle global keys.
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        match key.code {
            KeyCode::Esc => {
                let changed = *self.settings.read().unwrap() != self.initial_settings;
                if !changed {
                    tx.send(AppEvent::ClosePreferences).ok();
                } else {
                    self.overlay = Overlay::ConfirmSave {
                        focused_button: SaveButton::Save,
                    };
                }
                EventState::Consumed
            }
            KeyCode::Tab => {
                self.focus = match self.focus {
                    PreferencesFocus::Sidebar => PreferencesFocus::Content,
                    PreferencesFocus::Content => PreferencesFocus::Sidebar,
                };
                EventState::Consumed
            }
            _ => match self.focus {
                PreferencesFocus::Sidebar => match key.code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.section = match self.section {
                            PreferencesSection::Workspaces => PreferencesSection::Appearance,
                            PreferencesSection::Appearance => PreferencesSection::Display,
                            PreferencesSection::Display => PreferencesSection::Sorting,
                            PreferencesSection::Sorting => PreferencesSection::Indexing,
                            PreferencesSection::Indexing => PreferencesSection::Editor,
                            PreferencesSection::Editor => PreferencesSection::Server,
                            PreferencesSection::Server => PreferencesSection::Workspaces,
                        };
                        EventState::Consumed
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.section = match self.section {
                            PreferencesSection::Workspaces => PreferencesSection::Server,
                            PreferencesSection::Appearance => PreferencesSection::Workspaces,
                            PreferencesSection::Display => PreferencesSection::Appearance,
                            PreferencesSection::Sorting => PreferencesSection::Display,
                            PreferencesSection::Indexing => PreferencesSection::Sorting,
                            PreferencesSection::Editor => PreferencesSection::Indexing,
                            PreferencesSection::Server => PreferencesSection::Editor,
                        };
                        EventState::Consumed
                    }
                    KeyCode::Enter => {
                        self.focus = PreferencesFocus::Content;
                        EventState::Consumed
                    }
                    _ => EventState::NotConsumed,
                },
                PreferencesFocus::Content => {
                    let app_event = InputEvent::Key(*key);
                    match self.section {
                        PreferencesSection::Appearance => {
                            let r = self.appearance_section.handle_input(&app_event, tx);
                            // Live theme preview on every navigation step.
                            if r.is_consumed() {
                                self.sync_section();
                            }
                            r
                        }
                        PreferencesSection::Display => {
                            let r = self.display_section.handle_input(&app_event, tx);
                            self.sync_section();
                            r
                        }
                        PreferencesSection::Sorting => {
                            let r = self.sorting_section.handle_input(&app_event, tx);
                            self.sync_section();
                            r
                        }
                        PreferencesSection::Workspaces => {
                            // Capture pre-action state for rename/delete
                            let pre_mode = self.workspaces_section.mode().clone();
                            let pre_selected = self
                                .workspaces_section
                                .selected_name()
                                .map(|s| s.to_string());

                            let r = self.workspaces_section.handle_input(&app_event, tx);

                            let post_mode = self.workspaces_section.mode().clone();

                            // Creating: section collected a name and sent OpenFileBrowser.
                            // The section stays in Creating mode after Enter; store the name
                            // for when the file browser confirms a path, then reset.
                            if pre_mode == WorkspaceMode::Creating
                                && post_mode == WorkspaceMode::Creating
                                && key.code == KeyCode::Enter
                            {
                                let name = self.workspaces_section.input().trim().to_string();
                                if !name.is_empty() {
                                    // Check for duplicate name.
                                    let exists = self
                                        .settings
                                        .read()
                                        .unwrap()
                                        .workspace_config
                                        .as_ref()
                                        .is_some_and(|wc| wc.workspaces.contains_key(&name));
                                    if exists {
                                        self.workspaces_section.set_error(format!(
                                            "Workspace '{}' already exists.",
                                            name
                                        ));
                                    } else {
                                        self.pending_create_name = Some(name);
                                        self.workspaces_section.reset_mode();
                                    }
                                }
                            }

                            // Renaming: section was in Renaming, Enter pressed — apply rename.
                            if pre_mode == WorkspaceMode::Renaming
                                && post_mode == WorkspaceMode::Renaming
                                && key.code == KeyCode::Enter
                            {
                                let new_name = self.workspaces_section.input().trim().to_string();
                                // Check for duplicate name.
                                let duplicate = !new_name.is_empty()
                                    && pre_selected.as_deref() != Some(&new_name)
                                    && self
                                        .settings
                                        .read()
                                        .unwrap()
                                        .workspace_config
                                        .as_ref()
                                        .is_some_and(|wc| wc.workspaces.contains_key(&new_name));
                                if duplicate {
                                    self.workspaces_section.set_error(format!(
                                        "Workspace '{}' already exists.",
                                        new_name
                                    ));
                                } else if let Some(old_name) = pre_selected.as_deref()
                                    && !new_name.is_empty()
                                    && new_name != old_name
                                {
                                    let mut s = self.settings.write().unwrap();
                                    if let Some(ref mut wc) = s.workspace_config {
                                        // Through `rename_workspace`, never by
                                        // re-keying the map here: it also pins
                                        // what the index and history files are
                                        // called, and a rename that skips that
                                        // step points the workspace at files
                                        // that were never created — an empty
                                        // index and a lost history, silently.
                                        wc.rename_workspace(old_name, new_name.clone());
                                    }
                                }
                                self.workspaces_section.reset_mode();
                                self.workspaces_section
                                    .refresh(&self.settings.read().unwrap());
                            }

                            // Delete confirmation: section stays in ConfirmDelete after 'y'.
                            if pre_mode == WorkspaceMode::ConfirmDelete
                                && post_mode == WorkspaceMode::ConfirmDelete
                                && key.code == KeyCode::Char('y')
                            {
                                if let Some(name) = pre_selected.as_deref() {
                                    // Artifacts are read while the entry still
                                    // exists — both file names come from its
                                    // `file_key` — and deleted after the lock
                                    // is released, since a delete can wait on a
                                    // handle another process holds. This path
                                    // used to drop the entry and leave the
                                    // index and history behind under a key
                                    // nothing could attribute to a workspace.
                                    let artifacts = {
                                        let mut s = self.settings.write().unwrap();
                                        let removable =
                                            s.workspace_config.as_ref().is_some_and(|wc| {
                                                name != wc.global.current_workspace
                                                    && wc.workspaces.contains_key(name)
                                            });
                                        removable.then(|| {
                                            let artifacts = s.workspace_artifacts(name);
                                            if let Some(ref mut wc) = s.workspace_config {
                                                wc.workspaces.remove(name);
                                            }
                                            artifacts
                                        })
                                    };
                                    if let Some((index, history)) = artifacts {
                                        let leftovers =
                                            crate::settings::delete_artifacts(&index, &history);
                                        if !leftovers.is_empty() {
                                            tracing::warn!(
                                                "workspace '{name}' removed, but these files \
                                                 stayed behind:\n{}",
                                                leftovers.join("\n")
                                            );
                                        }
                                    }
                                }
                                self.workspaces_section.reset_mode();
                                self.workspaces_section
                                    .refresh(&self.settings.read().unwrap());
                            }

                            r
                        }
                        PreferencesSection::Indexing => {
                            self.indexing_section.handle_input(&app_event, tx)
                        }
                        PreferencesSection::Editor => {
                            let r = self.editor_section.handle_input(&app_event, tx);
                            self.sync_section();
                            r
                        }
                        PreferencesSection::Server => {
                            let r = self.server_section.handle_input(&app_event, tx);
                            if r.is_consumed() {
                                self.sync_section();
                            }
                            r
                        }
                    }
                }
            },
        }
    }

    async fn handle_app_message(&mut self, msg: AppEvent, tx: &AppTx) {
        match msg {
            AppEvent::OpenFileBrowser => {
                let starting_dir = self
                    .settings
                    .read()
                    .unwrap()
                    .resolve_workspace_path()
                    .or_else(|| kimun_core::system::browse_root().ok())
                    .map(|p| p.into_path_buf())
                    .unwrap_or_else(|| PathBuf::from("/"));
                self.overlay = Overlay::FileBrowser(FileBrowserState::load(starting_dir));
            }
            AppEvent::TriggerFastReindex => {
                // Fast reindex starts immediately (no confirmation overlay) — it is a
                // low-cost incremental operation unlike full reindex.
                let s = self.settings.read().unwrap();
                let Some(workspace) = s.resolve_workspace_path() else {
                    drop(s);
                    tx.send(AppEvent::IndexingDone(Err("No workspace set".to_string())))
                        .ok();
                    return;
                };
                let workspace_name = s
                    .workspace_config
                    .as_ref()
                    .map(|wc| wc.global.current_workspace.clone())
                    .filter(|n| !n.is_empty());
                let cache_path = workspace_name.as_ref().map(|n| s.index_for(n));
                drop(s);
                let tx2 = tx.clone();
                let handle = tokio::spawn(async move {
                    let result = async {
                        let mut config = VaultConfig::new(workspace);
                        if let Some(path) = cache_path {
                            config = config.with_index(path);
                        }
                        let vault = NoteVault::new(config).await.map_err(|e| e.to_string())?;
                        let result = vault.index_notes(NotesValidation::Fast).await;
                        // Throwaway vault with its own pool — see the reindex
                        // task above.
                        vault.close().await;
                        result.map_err(|e| e.to_string()).map(|r| r.duration)
                    }
                    .await;
                    tx2.send(AppEvent::IndexingDone(result)).ok();
                });
                self.overlay = Overlay::IndexingProgress(spawn_running(handle, tx));
            }
            AppEvent::TriggerFullReindex => {
                self.overlay = Overlay::ConfirmFullReindex {
                    focused_button: ConfirmButton::Cancel,
                };
            }
            AppEvent::IndexingDone(result) => match result {
                Ok(duration) => {
                    self.settings.write().unwrap().report_indexed();
                    if self.pending_save_after_index {
                        self.pending_save_after_index = false;
                        self.settings.read().unwrap().save_to_disk().ok();
                        tx.send(AppEvent::PreferencesSaved).ok();
                    } else {
                        self.overlay =
                            Overlay::IndexingProgress(IndexingProgressState::Done(duration));
                    }
                }
                Err(msg) => {
                    self.pending_save_after_index = false;
                    self.overlay = Overlay::IndexingProgress(IndexingProgressState::Failed(msg));
                }
            },
            _ => {}
        }
    }

    fn render(&mut self, f: &mut Frame) {
        // Each screen starts its frame's click targets empty.
        crate::components::clickable::clear();
        let theme = self.theme.clone();
        f.render_widget(Block::default().style(theme.base_style()), f.area());

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(f.area());

        let header = Block::default()
            .title("Preferences")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border_dim.to_ratatui()))
            .style(theme.base_style())
            .title_style(Style::default().fg(theme.accent.to_ratatui()));
        f.render_widget(header, rows[0]);

        // Footer hint — clickable chips.
        self.footer.render(
            f,
            rows[2],
            Style::default()
                .fg(theme.gray.to_ratatui())
                .bg(theme.bg.to_ratatui()),
            &theme,
        );

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(20), Constraint::Min(0)])
            .split(rows[1]);

        // Sidebar navigation
        let sidebar_focused = self.focus == PreferencesFocus::Sidebar;
        let active_idx = match self.section {
            PreferencesSection::Workspaces => 0,
            PreferencesSection::Appearance => 1,
            PreferencesSection::Display => 2,
            PreferencesSection::Sorting => 3,
            PreferencesSection::Indexing => 4,
            PreferencesSection::Editor => 5,
            PreferencesSection::Server => 6,
        };
        let items: Vec<ListItem> = [
            "Workspaces",
            "Appearance",
            "Display",
            "Sorting",
            "Indexing",
            "Editor",
            "Server",
        ]
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let prefix = if i == active_idx { "> " } else { "  " };
            let fg = if i == active_idx {
                theme.accent.to_ratatui()
            } else {
                theme.fg.to_ratatui()
            };
            ListItem::new(format!("{}{}", prefix, name))
                .style(Style::default().fg(fg).bg(theme.bg_panel.to_ratatui()))
        })
        .collect();
        let sidebar_block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border_style(sidebar_focused))
            .style(theme.panel_style());
        self.sidebar_rows = sidebar_block.inner(cols[0]);
        let sidebar_list = List::new(items).block(sidebar_block);
        f.render_widget(sidebar_list, cols[0]);
        self.content_rect = cols[1];

        // Content panel
        let content_focused = self.focus == PreferencesFocus::Content;
        match self.section {
            PreferencesSection::Appearance => {
                self.appearance_section
                    .render(f, cols[1], &theme, content_focused)
            }
            PreferencesSection::Display => {
                self.display_section
                    .render(f, cols[1], &theme, content_focused)
            }
            PreferencesSection::Sorting => {
                self.sorting_section
                    .render(f, cols[1], &theme, content_focused)
            }
            PreferencesSection::Workspaces => {
                self.workspaces_section
                    .render(f, cols[1], &theme, content_focused)
            }
            PreferencesSection::Indexing => {
                self.indexing_section
                    .render(f, cols[1], &theme, content_focused)
            }
            PreferencesSection::Editor => {
                self.editor_section
                    .render(f, cols[1], &theme, content_focused)
            }
            PreferencesSection::Server => {
                self.server_section
                    .render(f, cols[1], &theme, content_focused)
            }
        }

        // An overlay is modal: what it covers is not clickable, so it
        // must not light up on hover either.
        if matches!(self.overlay, Overlay::None) {
            if std::mem::take(&mut self.overlay_shown) {
                self.open_guard.closed();
            }
        } else {
            crate::components::clickable::clear();
            if !std::mem::replace(&mut self.overlay_shown, true) {
                self.open_guard.opened(std::time::Instant::now());
            }
        }
        self.render_overlay(f, &theme);
        crate::components::clickable::apply_hover(f.buffer_mut(), self.pointer, &theme);
    }
}

impl PreferencesScreen {
    fn render_overlay(&mut self, f: &mut Frame, theme: &Theme) {
        match &mut self.overlay {
            Overlay::None => {}

            Overlay::FileBrowser(fb) => {
                let area = crate::components::centered_rect(60, 80, f.area());
                self.overlay_rect = area;
                f.render_widget(Clear, area);
                let block = Block::default()
                    .title("Select Vault Directory")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.accent.to_ratatui()))
                    .style(theme.base_style());
                let inner = block.inner(area);
                f.render_widget(block, area);

                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Min(0),
                        Constraint::Length(1),
                    ])
                    .split(inner);

                let path_str = fb.current_path.to_string_lossy().into_owned();
                f.render_widget(Paragraph::new(path_str).style(theme.base_style()), rows[0]);

                let mut items: Vec<ListItem> = Vec::new();
                if fb.has_parent {
                    items.push(
                        ListItem::new("  ../").style(
                            Style::default()
                                .fg(theme.fg_secondary.to_ratatui())
                                .bg(theme.bg.to_ratatui()),
                        ),
                    );
                }
                for e in &fb.entries {
                    let name = e.file_name().unwrap_or_default().to_string_lossy();
                    items.push(
                        ListItem::new(format!("  {}/", name)).style(
                            Style::default()
                                .fg(theme.fg.to_ratatui())
                                .bg(theme.bg.to_ratatui()),
                        ),
                    );
                }
                let list = List::new(items)
                    .highlight_symbol("▶ ")
                    .highlight_style(Style::default().add_modifier(Modifier::BOLD));
                f.render_stateful_widget(list, rows[1], &mut fb.list_state);
                self.browser_rows = rows[1];
                self.browser_hints
                    .render(f, rows[2], theme.base_style(), theme);
            }

            Overlay::ConfirmFullReindex { focused_button } => {
                let area = fixed_centered_rect(44, 6, f.area());
                self.overlay_rect = area;
                f.render_widget(Clear, area);
                let block = Block::default()
                    .title("Full Reindex")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.accent.to_ratatui()))
                    .style(theme.base_style());
                let inner = block.inner(area);
                f.render_widget(block, area);
                f.render_widget(
                    Paragraph::new("\n  This may take a while.").style(theme.base_style()),
                    inner,
                );
                let focused = match focused_button {
                    ConfirmButton::Cancel => 0,
                    ConfirmButton::Confirm => 1,
                };
                self.confirm_buttons = ButtonRow::new(&["Cancel", "Confirm"]);
                self.confirm_buttons.set_focused(Some(focused));
                self.confirm_buttons.render(f, button_line(inner), theme);
            }

            Overlay::ConfirmSave { focused_button } => {
                let area = fixed_centered_rect(44, 6, f.area());
                self.overlay_rect = area;
                f.render_widget(Clear, area);
                let block = Block::default()
                    .title("Save Preferences?")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.accent.to_ratatui()))
                    .style(theme.base_style());
                let inner = block.inner(area);
                f.render_widget(block, area);
                f.render_widget(
                    Paragraph::new("\n  You have unsaved changes.").style(theme.base_style()),
                    inner,
                );
                let focused = match focused_button {
                    SaveButton::Save => 0,
                    SaveButton::Discard => 1,
                };
                self.confirm_buttons = ButtonRow::new(&["Save", "Discard"]);
                self.confirm_buttons.set_focused(Some(focused));
                self.confirm_buttons.render(f, button_line(inner), theme);
            }

            Overlay::IndexingProgress(state) => {
                render_indexing_overlay(
                    f,
                    state,
                    &mut self.throbber_state,
                    theme,
                    "Reindex in progress…",
                );
            }

            Overlay::VaultConflict(msg) => {
                let area = fixed_centered_rect(60, 9, f.area());
                f.render_widget(Clear, area);
                let block = Block::default()
                    .title("Vault Error")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.accent.to_ratatui()))
                    .style(theme.base_style());
                let inner = block.inner(area);
                f.render_widget(block, area);
                f.render_widget(
                    Paragraph::new(format!("\n  {}\n\n  [ OK ]", msg))
                        .style(theme.base_style())
                        .wrap(Wrap { trim: false }),
                    inner,
                );
            }
        }
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
        let names: Vec<_> = state
            .entries
            .iter()
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
        // has_parent is true for a temp dir, so the ".." entry exists → selected = Some(0)
        assert!(state.has_parent);
        assert_eq!(state.list_state.selected(), Some(0));
    }

    #[test]
    fn load_root_has_no_parent_entry() {
        let state = FileBrowserState::load(PathBuf::from("/"));
        assert!(!state.has_parent);
        // Only real entries; if none, selection is None
        if state.entries.is_empty() {
            assert_eq!(state.list_state.selected(), None);
        } else {
            assert_eq!(state.list_state.selected(), Some(0));
        }
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

#[cfg(test)]
mod settings_screen_tests {
    use std::sync::{Arc, RwLock};
    use std::time::Duration;

    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use tokio::sync::mpsc::unbounded_channel;

    fn key(code: KeyCode) -> InputEvent {
        InputEvent::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn shared_defaults() -> SharedSettings {
        Arc::new(RwLock::new(AppSettings::default()))
    }

    /// A host-absolute workspace path from a `/`-separated literal.
    ///
    /// `do_save` compares workspaces through `resolve_workspace_path`, which
    /// drops anything `SystemPath` cannot make absolute — and `/original/path`
    /// carries no drive prefix, so on Windows *both* sides resolve to `None`,
    /// the save sees no change, and the test asserts on a branch it never
    /// entered.
    fn abs(unix_style: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!(
                "C:\\{}",
                unix_style.trim_start_matches('/').replace('/', "\\")
            ))
        } else {
            PathBuf::from(unix_style)
        }
    }

    fn make_screen() -> PreferencesScreen {
        PreferencesScreen::new(shared_defaults())
    }

    #[test]
    fn esc_sends_close_settings_when_no_changes() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.handle_input(&key(KeyCode::Esc), &tx);
        let msg = rx.try_recv().expect("expected message");
        assert!(matches!(msg, AppEvent::ClosePreferences));
    }

    #[test]
    fn esc_shows_confirm_save_when_settings_changed() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen
            .settings
            .write()
            .unwrap()
            .set_theme("Gruvbox Light".to_string());
        screen.handle_input(&key(KeyCode::Esc), &tx);
        assert!(rx.try_recv().is_err(), "no message should be sent yet");
        assert!(matches!(screen.overlay, Overlay::ConfirmSave { .. }));
    }

    #[test]
    fn confirm_save_discard_sends_close_settings() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen
            .settings
            .write()
            .unwrap()
            .set_theme("Gruvbox Light".to_string());
        screen.overlay = Overlay::ConfirmSave {
            focused_button: SaveButton::Discard,
        };
        screen.handle_input(&key(KeyCode::Enter), &tx);
        let msg = rx.try_recv().expect("expected message");
        assert!(matches!(msg, AppEvent::ClosePreferences));
    }

    #[test]
    fn confirm_save_save_vault_unchanged_sends_settings_saved() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen
            .settings
            .write()
            .unwrap()
            .set_theme("Gruvbox Light".to_string());
        screen.overlay = Overlay::ConfirmSave {
            focused_button: SaveButton::Save,
        };
        screen.handle_input(&key(KeyCode::Enter), &tx);
        let msg = rx.try_recv().expect("expected message");
        assert!(matches!(msg, AppEvent::PreferencesSaved));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn confirm_save_vault_changed_sets_pending_and_shows_progress() {
        let (tx, _rx) = unbounded_channel();
        let mut settings = AppSettings::default();
        let mut wc = crate::settings::workspace_config::WorkspaceConfig::new_empty();
        wc.add_workspace("main".to_string(), abs("/original/path"))
            .unwrap();
        settings.workspace_config = Some(wc);
        let shared = Arc::new(RwLock::new(settings));
        let mut screen = PreferencesScreen::new(shared);
        screen
            .settings
            .write()
            .unwrap()
            .set_workspace_path("main", abs("/new/path"));
        screen.overlay = Overlay::ConfirmSave {
            focused_button: SaveButton::Save,
        };
        screen.handle_input(&key(KeyCode::Enter), &tx);
        assert!(screen.pending_save_after_index);
        assert!(matches!(
            screen.overlay,
            Overlay::IndexingProgress(IndexingProgressState::Running { .. })
        ));
    }

    #[tokio::test]
    async fn indexing_done_ok_with_pending_auto_closes() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.pending_save_after_index = true;
        screen.overlay = Overlay::IndexingProgress(IndexingProgressState::Running {
            work: tokio::spawn(async {}),
            ticker: tokio::spawn(async {}),
        });
        screen
            .handle_app_message(AppEvent::IndexingDone(Ok(Duration::from_secs(1))), &tx)
            .await;
        let msg = rx.try_recv().expect("expected PreferencesSaved");
        assert!(matches!(msg, AppEvent::PreferencesSaved));
        assert!(!screen.pending_save_after_index);
    }

    #[tokio::test]
    async fn indexing_done_err_with_pending_shows_failed_no_save() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.pending_save_after_index = true;
        screen.overlay = Overlay::IndexingProgress(IndexingProgressState::Running {
            work: tokio::spawn(async {}),
            ticker: tokio::spawn(async {}),
        });
        screen
            .handle_app_message(AppEvent::IndexingDone(Err("disk error".to_string())), &tx)
            .await;
        assert!(
            rx.try_recv().is_err(),
            "no PreferencesSaved when index failed"
        );
        assert!(!screen.pending_save_after_index);
        assert!(matches!(
            screen.overlay,
            Overlay::IndexingProgress(IndexingProgressState::Failed(_))
        ));
    }

    #[tokio::test]
    async fn indexing_done_ok_without_pending_shows_done() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.pending_save_after_index = false;
        screen.overlay = Overlay::IndexingProgress(IndexingProgressState::Running {
            work: tokio::spawn(async {}),
            ticker: tokio::spawn(async {}),
        });
        screen
            .handle_app_message(AppEvent::IndexingDone(Ok(Duration::from_secs(2))), &tx)
            .await;
        assert!(
            rx.try_recv().is_err(),
            "no auto-close when pending is false"
        );
        assert!(matches!(
            screen.overlay,
            Overlay::IndexingProgress(IndexingProgressState::Done(_))
        ));
    }

    #[test]
    fn esc_blocked_while_indexing_running() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.overlay = Overlay::IndexingProgress(IndexingProgressState::Running {
            work: rt.spawn(async {}),
            ticker: rt.spawn(async {}),
        });
        screen.handle_input(&key(KeyCode::Esc), &tx);
        assert!(rx.try_recv().is_err(), "Esc must be blocked while indexing");
    }

    #[tokio::test]
    async fn confirm_full_reindex_esc_closes_overlay() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.overlay = Overlay::ConfirmFullReindex {
            focused_button: ConfirmButton::Cancel,
        };
        screen.handle_input(&key(KeyCode::Esc), &tx);
        assert!(matches!(screen.overlay, Overlay::None));
    }

    #[test]
    fn new_with_error_sets_vault_conflict_overlay_with_message() {
        let screen =
            PreferencesScreen::new_with_error(shared_defaults(), "test error msg".to_string());
        match screen.overlay {
            Overlay::VaultConflict(ref msg) => {
                assert_eq!(msg, "test error msg");
            }
            _ => panic!("expected Overlay::VaultConflict(...)"),
        }
    }
}

#[cfg(test)]
mod mouse_tests {
    use std::sync::{Arc, RwLock};

    use super::*;
    use crate::components::events::AppEvent;
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    fn screen() -> PreferencesScreen {
        PreferencesScreen::new(Arc::new(RwLock::new(AppSettings::default())))
    }

    fn draw(s: &mut PreferencesScreen) -> ratatui::buffer::Buffer {
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        t.draw(|f| s.render(f)).unwrap();
        t.backend().buffer().clone()
    }

    /// Render, then left-click the first cell of `text` shifted by `dx`.
    fn click(s: &mut PreferencesScreen, text: &str, dx: u16) -> UnboundedReceiver<AppEvent> {
        let buf = draw(s);
        let (x, y) = crate::test_support::find_text(&buf, text)
            .unwrap_or_else(|| panic!("{text:?} not drawn"));
        let (tx, rx) = unbounded_channel();
        s.handle_input(&crate::test_support::mouse_down_at(x + dx, y), &tx);
        rx
    }

    /// Let an overlay that just opened accept clicks (past the guard that
    /// drops the tail of the gesture which opened it).
    fn settle_overlay(s: &mut PreferencesScreen) {
        draw(s);
        s.open_guard.closed();
    }

    #[test]
    fn clicking_a_section_switches_to_it() {
        let mut s = screen();
        click(&mut s, "Display", 0);
        assert_eq!(s.section, PreferencesSection::Display);
        assert_eq!(s.focus, PreferencesFocus::Sidebar);
    }

    #[test]
    fn display_checkbox_and_stepper_write_settings() {
        let mut s = screen();
        s.section = PreferencesSection::Display;
        let before = s.settings.read().unwrap().use_nerd_fonts;
        // `  Use Nerd Fonts  [x]` — the checkbox sits 16 cells after the label.
        click(&mut s, "Use Nerd Fonts", 16);
        assert_eq!(s.settings.read().unwrap().use_nerd_fonts, !before);
        assert_eq!(s.focus, PreferencesFocus::Content);

        let delay = s.settings.read().unwrap().leader_timeout_ms;
        click(&mut s, "▶", 0);
        assert_eq!(s.settings.read().unwrap().leader_timeout_ms, delay + 50);
    }

    #[test]
    fn clicking_a_theme_previews_it() {
        let mut s = screen();
        s.section = PreferencesSection::Appearance;
        let current = s.settings.read().unwrap().theme.clone();
        let other = s
            .settings
            .read()
            .unwrap()
            .theme_list()
            .into_iter()
            .map(|t| t.name)
            .find(|n| *n != current)
            .unwrap();
        click(&mut s, &other, 0);
        assert_eq!(s.settings.read().unwrap().theme, other);
    }

    #[test]
    fn sorting_value_cycles_on_click() {
        let mut s = screen();
        s.section = PreferencesSection::Sorting;
        let before = s.settings.read().unwrap().default_sort_field;
        click(&mut s, "[Name]", 1);
        assert_ne!(s.settings.read().unwrap().default_sort_field, before);
    }

    #[test]
    fn editor_backend_steps_on_click() {
        let mut s = screen();
        s.section = PreferencesSection::Editor;
        let before = s.settings.read().unwrap().editor_backend;
        // The second `▶` on screen is the backend's.
        let buf = draw(&mut s);
        let (_, y) = crate::test_support::find_text(&buf, "Editor Backend").unwrap();
        let col = (0..100u16)
            .find(|&c| buf[(c, y + 1)].symbol() == "▶")
            .unwrap();
        let (tx, _rx) = unbounded_channel();
        s.handle_input(&crate::test_support::mouse_down_at(col, y + 1), &tx);
        assert_ne!(s.settings.read().unwrap().editor_backend, before);
    }

    #[test]
    fn reindex_buttons_run_their_reindex() {
        let mut s = screen();
        s.section = PreferencesSection::Indexing;
        s.indexing_section = IndexingSection::new(true);
        let mut rx = click(&mut s, "Full Reindex", 0);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::TriggerFullReindex)));
        let mut rx = click(&mut s, "Fast Reindex", 0);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::TriggerFastReindex)));
    }

    /// The folder picker: a click selects a folder, clicking it again opens
    /// it, and `[c] Choose this folder` picks the folder being shown.
    #[test]
    fn folder_picker_rows_and_choose_chip() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("inner")).unwrap();
        let mut s = screen();
        s.overlay = Overlay::FileBrowser(FileBrowserState::load(dir.path().to_path_buf()));
        settle_overlay(&mut s);
        click(&mut s, "inner/", 0);
        click(&mut s, "inner/", 0);
        let Overlay::FileBrowser(fb) = &s.overlay else {
            panic!("picker still open")
        };
        assert!(fb.current_path.ends_with("inner"), "re-click opened it");

        click(&mut s, "[c] Choose this folder", 0);
        assert!(
            matches!(s.overlay, Overlay::None),
            "choosing closes the picker"
        );
    }

    /// Moving the pointer over a section is not an edit: settings stay
    /// untouched, so Esc does not ask to save.
    #[test]
    fn hovering_sections_changes_no_settings() {
        let mut s = screen();
        for section in SECTIONS {
            s.section = section;
            draw(&mut s);
            let (tx, _rx) = unbounded_channel();
            for y in (0..30u16).step_by(3) {
                for x in (0..100u16).step_by(7) {
                    s.handle_input(
                        &InputEvent::Mouse(ratatui::crossterm::event::MouseEvent {
                            kind: MouseEventKind::Moved,
                            column: x,
                            row: y,
                            modifiers: KeyModifiers::NONE,
                        }),
                        &tx,
                    );
                }
            }
        }
        assert_eq!(*s.settings.read().unwrap(), s.initial_settings);
    }

    #[test]
    fn workspace_new_chip_starts_creating() {
        let mut s = screen();
        click(&mut s, "[n] New", 0);
        assert_eq!(*s.workspaces_section.mode(), WorkspaceMode::Creating);
    }

    /// The footer's `[Esc]` closes when nothing changed; with a change the
    /// save prompt opens, and its `Discard` button rolls the change back.
    #[test]
    fn footer_esc_and_discard_button() {
        let mut s = screen();
        let mut rx = click(&mut s, "[Esc] Save & Close", 0);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::ClosePreferences)));

        s.section = PreferencesSection::Display;
        click(&mut s, "Use Nerd Fonts", 16);
        click(&mut s, "[Esc] Save & Close", 0);
        assert!(matches!(s.overlay, Overlay::ConfirmSave { .. }));
        settle_overlay(&mut s);
        let mut rx = click(&mut s, "Discard", 0);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::ClosePreferences)));
        assert_eq!(*s.settings.read().unwrap(), s.initial_settings);
    }

    /// The second press of the double-click that opened an overlay must not
    /// land on it; a press outside a settled overlay cancels it.
    #[test]
    fn overlay_guard_and_outside_press() {
        let mut s = screen();
        // A click opened it (the guard arms only for mouse opens).
        s.open_guard
            .observe_input(&crate::test_support::mouse_down_at(5, 5));
        s.overlay = Overlay::ConfirmSave {
            focused_button: SaveButton::Save,
        };
        let mut rx = click(&mut s, "Discard", 0);
        assert!(
            rx.try_recv().is_err(),
            "the opening gesture's tail is dropped"
        );
        assert!(matches!(s.overlay, Overlay::ConfirmSave { .. }));

        settle_overlay(&mut s);
        let (tx, _rx) = unbounded_channel();
        s.handle_input(&crate::test_support::mouse_down_at(0, 0), &tx);
        assert!(matches!(s.overlay, Overlay::None), "outside press cancels");
    }

    #[test]
    fn hovering_a_control_highlights_it() {
        let mut s = screen();
        let buf = draw(&mut s);
        let (x, y) = crate::test_support::find_text(&buf, "[n] New").unwrap();
        let (tx, _rx) = unbounded_channel();
        s.handle_input(
            &InputEvent::Mouse(ratatui::crossterm::event::MouseEvent {
                kind: MouseEventKind::Moved,
                column: x + 1,
                row: y,
                modifiers: KeyModifiers::NONE,
            }),
            &tx,
        );
        let buf = draw(&mut s);
        assert_eq!(buf[(x, y)].bg, s.theme.hover().bg.unwrap());
    }
}
