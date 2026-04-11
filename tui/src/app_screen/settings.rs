use std::path::PathBuf;

use async_trait::async_trait;
use kimun_core::{NoteVault, NotesValidation};
use kimun_core::error::VaultError;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use throbber_widgets_tui::ThrobberState;

use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::settings::appearance_section::AppearanceSection;
use crate::components::settings::display_section::DisplaySection;
use crate::components::settings::editor_section::EditorSection;
use crate::components::settings::indexing_section::IndexingSection;
use crate::components::settings::sorting_section::SortingSection;
use crate::components::settings::workspaces_section::{WorkspacesSection, Mode as WorkspaceMode};
use crate::components::indexing::{
    IndexingProgressState, fixed_centered_rect, render_indexing_overlay, spawn_running,
};
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

// ── FileBrowserState ─────────────────────────────────────────────────────────

pub struct FileBrowserState {
    pub current_path: PathBuf,
    pub entries: Vec<PathBuf>,
    pub list_state: ListState,
    pub has_parent: bool,
    last_jump_char: Option<char>,
}

impl FileBrowserState {
    pub fn load(path: PathBuf) -> Self {
        let has_parent = path.parent().is_some();
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        entries.sort();
        let total = entries.len() + if has_parent { 1 } else { 0 };
        let mut list_state = ListState::default();
        if total > 0 {
            list_state.select(Some(0));
        }
        Self {
            current_path: path,
            entries,
            list_state,
            has_parent,
            last_jump_char: None,
        }
    }

    pub fn navigate_into(&mut self, entry: PathBuf) {
        *self = Self::load(entry);
    }

    pub fn go_up(&mut self) {
        if let Some(parent) = self.current_path.parent() {
            *self = Self::load(parent.to_path_buf());
        }
    }

    pub fn jump_to_char(&mut self, c: char) {
        let c_lower = c.to_lowercase().next().unwrap_or(c);
        let offset = if self.has_parent { 1 } else { 0 };
        let total = self.entries.len();
        if total == 0 {
            return;
        }

        // If same char as last jump, cycle to next match.
        let start = if self.last_jump_char == Some(c_lower) {
            let cur = self.list_state.selected().unwrap_or(0);
            if cur >= offset { cur - offset + 1 } else { 0 }
        } else {
            0
        };

        // Search from start, wrapping around.
        for i in 0..total {
            let idx = (start + i) % total;
            if let Some(name) = self.entries[idx].file_name().and_then(|n| n.to_str())
                && name.to_lowercase().starts_with(c_lower)
            {
                self.list_state.select(Some(idx + offset));
                self.last_jump_char = Some(c_lower);
                return;
            }
        }
        self.last_jump_char = None;
    }
}

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
    ConfirmFullReindex { focused_button: ConfirmButton },
    ConfirmSave { focused_button: SaveButton },
    IndexingProgress(IndexingProgressState),
    /// Vault was rejected due to structural errors (e.g. case conflicts).
    /// Rendered like the other confirmation dialogs but with a single close button.
    VaultConflict(String),
}

// ── Section / Focus enums ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum SettingsSection {
    Workspaces,
    Appearance,
    Display,
    Sorting,
    Indexing,
    Editor,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SettingsFocus {
    Sidebar,
    Content,
}

// ── SettingsScreen ────────────────────────────────────────────────────────────

pub struct SettingsScreen {
    pub settings: AppSettings,
    pub initial_settings: AppSettings,
    pub theme: Theme,
    section: SettingsSection,
    focus: SettingsFocus,
    appearance_section: AppearanceSection,
    display_section: DisplaySection,
    sorting_section: SortingSection,
    workspaces_section: WorkspacesSection,
    pending_create_name: Option<String>,
    indexing_section: IndexingSection,
    editor_section: EditorSection,
    pub overlay: Overlay,
    pub pending_save_after_index: bool,
    throbber_state: ThrobberState,
}

impl SettingsScreen {
    pub fn new(settings: AppSettings) -> Self {
        let theme = settings.get_theme();
        let themes = settings.theme_list();
        let active_name = settings.get_theme().name.clone();
        let vault_available = settings.workspace_dir.is_some();
        let autosave_interval_secs = settings.autosave_interval_secs;
        let use_nerd_fonts = settings.use_nerd_fonts;
        let initial_settings = settings.clone();
        Self {
            appearance_section: AppearanceSection::new(themes, &active_name),
            display_section: DisplaySection::new(use_nerd_fonts),
            sorting_section: SortingSection::new(
                settings.default_sort_field,
                settings.default_sort_order,
                settings.journal_sort_field,
                settings.journal_sort_order,
            ),
            workspaces_section: WorkspacesSection::new(&settings),
            pending_create_name: None,
            indexing_section: IndexingSection::new(vault_available),
            editor_section: EditorSection::new(autosave_interval_secs),
            settings,
            initial_settings,
            theme,
            section: SettingsSection::Workspaces,
            focus: SettingsFocus::Sidebar,
            overlay: Overlay::None,
            pending_save_after_index: false,
            throbber_state: ThrobberState::default(),
        }
    }

    /// Creates a settings screen with a `Failed` error overlay pre-populated.
    /// Used when the vault was rejected due to structural conflicts.
    ///
    /// The `settings` passed in should already have the workspace cleared —
    /// this is handled by the `VaultConflict` branch in `handle_app_message` (`main.rs`)
    /// before calling `switch_screen`.
    pub fn new_with_error(settings: AppSettings, error: String) -> Self {
        let mut s = Self::new(settings);
        s.overlay = Overlay::VaultConflict(error);
        s
    }

    fn do_save(&mut self, tx: &AppTx) {
        if self.settings.workspace_dir != self.initial_settings.workspace_dir {
            let Some(workspace) = self.settings.workspace_dir.clone() else {
                tx.send(AppEvent::IndexingDone(Err("No workspace set".to_string())))
                    .ok();
                return;
            };
            self.pending_save_after_index = true;
            let tx2 = tx.clone();
            let handle = tokio::spawn(async move {
                let event = match NoteVault::new(&workspace).await {
                    Err(e) => AppEvent::IndexingDone(Err(e.to_string())),
                    Ok(vault) => match vault.recreate_index().await {
                        Ok(r) => AppEvent::IndexingDone(Ok(r.duration)),
                        Err(e @ VaultError::CaseConflict { .. }) => {
                            AppEvent::VaultConflict(e.to_string())
                        }
                        Err(e) => AppEvent::IndexingDone(Err(e.to_string())),
                    },
                };
                tx2.send(event).ok();
            });
            self.overlay = Overlay::IndexingProgress(spawn_running(handle, tx));
        } else {
            self.settings.save_to_disk().ok();
            tx.send(AppEvent::SettingsSaved).ok();
        }
    }

    /// Called when the file browser confirms a directory path (via 'c' or Ctrl+Enter).
    fn confirm_file_browser(&mut self, chosen: PathBuf, _tx: &AppTx) {
        use crate::settings::workspace_config::{WorkspaceConfig, WorkspaceEntry};

        if let Some(name) = self.pending_create_name.take() {
            // Creating a new workspace with the chosen path.
            let entry = WorkspaceEntry {
                path: chosen,
                last_paths: Vec::new(),
                created: chrono::Utc::now(),
                quick_note_path: None,
                inbox_path: None,
            };
            if self.settings.workspace_config.is_none() {
                // Migrate Phase 1 legacy config to Phase 2 before adding.
                if let Some(ref legacy_path) = self.settings.workspace_dir {
                    let wc = WorkspaceConfig::from_phase1_migration(
                        legacy_path.clone(),
                        self.settings.last_paths.iter().map(|p| p.to_string()).collect(),
                    );
                    self.settings.workspace_config = Some(wc);
                } else {
                    self.settings.workspace_config = Some(WorkspaceConfig::new_empty());
                }
                self.settings.config_version = 2;
            }
            if let Some(ref mut wc) = self.settings.workspace_config {
                wc.workspaces.insert(name.clone(), entry);
                // If this is the only workspace (no prior ones), make it current.
                if wc.global.current_workspace.is_empty() {
                    wc.global.current_workspace = name;
                }
            }
            self.workspaces_section.refresh(&self.settings);
            self.indexing_section.set_vault_available(true);
        } else {
            // Browsing path for the selected workspace.
            self.settings.set_workspace(&chosen);
            // Also update the workspace entry in workspace_config.
            if let Some(name) = self.workspaces_section.selected_name().map(|s| s.to_string())
                && let Some(ref mut wc) = self.settings.workspace_config
                && let Some(entry) = wc.workspaces.get_mut(&name)
            {
                entry.path = chosen;
            }
            self.workspaces_section.refresh(&self.settings);
            self.indexing_section.set_vault_available(true);
        }
        self.overlay = Overlay::None;
    }
}

// ── AppScreen impl ────────────────────────────────────────────────────────────

#[async_trait]
impl AppScreen for SettingsScreen {
    fn get_kind(&self) -> ScreenKind {
        ScreenKind::Settings
    }

    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
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
                    KeyCode::Up => {
                        if total > 0 {
                            let cur = fb.list_state.selected().unwrap_or(0);
                            fb.list_state.select(Some((cur + total - 1) % total));
                        }
                    }
                    KeyCode::Down => {
                        if total > 0 {
                            let cur = fb.list_state.selected().unwrap_or(0);
                            fb.list_state.select(Some((cur + 1) % total));
                        }
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
                    KeyCode::Char('c') => {
                        let chosen = fb.current_path.clone();
                        self.confirm_file_browser(chosen, tx);
                    }
                    KeyCode::Char(c) => {
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
                            let Some(workspace) = self.settings.workspace_dir.clone() else {
                                self.overlay = Overlay::None;
                                return EventState::Consumed;
                            };
                            let tx2 = tx.clone();
                            let handle = tokio::spawn(async move {
                                let event = match NoteVault::new(&workspace).await {
                                    Err(e) => AppEvent::IndexingDone(Err(e.to_string())),
                                    Ok(vault) => match vault.recreate_index().await {
                                        Ok(r) => AppEvent::IndexingDone(Ok(r.duration)),
                                        Err(e @ VaultError::CaseConflict { .. }) => {
                                            AppEvent::VaultConflict(e.to_string())
                                        }
                                        Err(e) => AppEvent::IndexingDone(Err(e.to_string())),
                                    },
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
                            tx.send(AppEvent::CloseSettings).ok();
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
                if self.settings == self.initial_settings {
                    tx.send(AppEvent::CloseSettings).ok();
                } else {
                    self.overlay = Overlay::ConfirmSave {
                        focused_button: SaveButton::Save,
                    };
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
            _ => match self.focus {
                SettingsFocus::Sidebar => match key.code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.section = match self.section {
                            SettingsSection::Workspaces => SettingsSection::Appearance,
                            SettingsSection::Appearance => SettingsSection::Display,
                            SettingsSection::Display => SettingsSection::Sorting,
                            SettingsSection::Sorting => SettingsSection::Indexing,
                            SettingsSection::Indexing => SettingsSection::Editor,
                            SettingsSection::Editor => SettingsSection::Workspaces,
                        };
                        EventState::Consumed
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.section = match self.section {
                            SettingsSection::Workspaces => SettingsSection::Editor,
                            SettingsSection::Appearance => SettingsSection::Workspaces,
                            SettingsSection::Display => SettingsSection::Appearance,
                            SettingsSection::Sorting => SettingsSection::Display,
                            SettingsSection::Indexing => SettingsSection::Sorting,
                            SettingsSection::Editor => SettingsSection::Indexing,
                        };
                        EventState::Consumed
                    }
                    KeyCode::Enter => {
                        self.focus = SettingsFocus::Content;
                        EventState::Consumed
                    }
                    _ => EventState::NotConsumed,
                },
                SettingsFocus::Content => {
                    let app_event = InputEvent::Key(*key);
                    match self.section {
                        SettingsSection::Appearance => {
                            let r = self.appearance_section.handle_input(&app_event, tx);
                            // Live theme preview on every navigation step.
                            let name = self.appearance_section.selected_theme_name().to_string();
                            self.settings.set_theme(name);
                            self.theme = self.settings.get_theme();
                            r
                        }
                        SettingsSection::Display => {
                            let r = self.display_section.handle_input(&app_event, tx);
                            self.settings.use_nerd_fonts = self.display_section.use_nerd_fonts;
                            r
                        }
                        SettingsSection::Sorting => {
                            let r = self.sorting_section.handle_input(&app_event, tx);
                            self.settings.default_sort_field = self.sorting_section.default_sort_field;
                            self.settings.default_sort_order = self.sorting_section.default_sort_order;
                            self.settings.journal_sort_field = self.sorting_section.journal_sort_field;
                            self.settings.journal_sort_order = self.sorting_section.journal_sort_order;
                            r
                        }
                        SettingsSection::Workspaces => {
                            // Capture pre-action state for rename/delete
                            let pre_mode = self.workspaces_section.mode().clone();
                            let pre_selected = self.workspaces_section.selected_name().map(|s| s.to_string());

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
                                    let exists = self.settings.workspace_config
                                        .as_ref()
                                        .is_some_and(|wc| wc.workspaces.contains_key(&name));
                                    if exists {
                                        self.workspaces_section.set_error(
                                            format!("Workspace '{}' already exists.", name),
                                        );
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
                                    && self.settings.workspace_config
                                        .as_ref()
                                        .is_some_and(|wc| wc.workspaces.contains_key(&new_name));
                                if duplicate {
                                    self.workspaces_section.set_error(
                                        format!("Workspace '{}' already exists.", new_name),
                                    );
                                } else if let Some(old_name) = pre_selected.as_deref()
                                    && !new_name.is_empty()
                                    && new_name != old_name
                                    && let Some(ref mut wc) = self.settings.workspace_config
                                    && let Some(entry) = wc.workspaces.remove(old_name)
                                {
                                    wc.workspaces.insert(new_name.clone(), entry);
                                    if wc.global.current_workspace == old_name {
                                        wc.global.current_workspace = new_name.clone();
                                    }
                                }
                                self.workspaces_section.reset_mode();
                                self.workspaces_section.refresh(&self.settings);
                            }

                            // Delete confirmation: section stays in ConfirmDelete after 'y'.
                            if pre_mode == WorkspaceMode::ConfirmDelete
                                && post_mode == WorkspaceMode::ConfirmDelete
                                && key.code == KeyCode::Char('y')
                            {
                                if let Some(name) = pre_selected.as_deref()
                                    && let Some(ref mut wc) = self.settings.workspace_config
                                    && name != wc.global.current_workspace
                                {
                                    wc.workspaces.remove(name);
                                }
                                self.workspaces_section.reset_mode();
                                self.workspaces_section.refresh(&self.settings);
                            }

                            r
                        }
                        SettingsSection::Indexing => {
                            self.indexing_section.handle_input(&app_event, tx)
                        }
                        SettingsSection::Editor => {
                            let r = self.editor_section.handle_input(&app_event, tx);
                            self.settings.autosave_interval_secs =
                                self.editor_section.autosave_interval_secs;
                            r
                        }
                    }
                }
            },
        }
    }

    async fn handle_app_message(&mut self, msg: AppEvent, tx: &AppTx) -> Option<AppEvent> {
        match msg {
            AppEvent::OpenFileBrowser => {
                let starting_dir = self
                    .settings
                    .workspace_dir
                    .clone()
                    .or_else(|| std::env::var("HOME").ok().map(PathBuf::from))
                    .unwrap_or_else(|| PathBuf::from("/"));
                self.overlay = Overlay::FileBrowser(FileBrowserState::load(starting_dir));
                None
            }
            AppEvent::TriggerFastReindex => {
                // Fast reindex starts immediately (no confirmation overlay) — it is a
                // low-cost incremental operation unlike full reindex.
                let Some(workspace) = self.settings.workspace_dir.clone() else {
                    tx.send(AppEvent::IndexingDone(Err("No workspace set".to_string())))
                        .ok();
                    return None;
                };
                let tx2 = tx.clone();
                let handle = tokio::spawn(async move {
                    let result = async {
                        let vault = NoteVault::new(&workspace)
                            .await
                            .map_err(|e| e.to_string())?;
                        vault
                            .index_notes(NotesValidation::Fast)
                            .await
                            .map_err(|e| e.to_string())
                            .map(|r| r.duration)
                    }
                    .await;
                    tx2.send(AppEvent::IndexingDone(result)).ok();
                });
                self.overlay = Overlay::IndexingProgress(spawn_running(handle, tx));
                None
            }
            AppEvent::TriggerFullReindex => {
                self.overlay = Overlay::ConfirmFullReindex {
                    focused_button: ConfirmButton::Cancel,
                };
                None
            }
            AppEvent::IndexingDone(result) => {
                match result {
                    Ok(duration) => {
                        self.settings.report_indexed();
                        if self.pending_save_after_index {
                            self.pending_save_after_index = false;
                            self.settings.save_to_disk().ok();
                            tx.send(AppEvent::SettingsSaved).ok();
                        } else {
                            self.overlay =
                                Overlay::IndexingProgress(IndexingProgressState::Done(duration));
                        }
                    }
                    Err(msg) => {
                        self.pending_save_after_index = false;
                        self.overlay =
                            Overlay::IndexingProgress(IndexingProgressState::Failed(msg));
                    }
                }
                None
            }
            other => Some(other),
        }
    }

    fn render(&mut self, f: &mut Frame) {
        let theme = self.theme.clone();
        f.render_widget(Block::default().style(theme.base_style()), f.area());

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(f.area());

        let header = Block::default()
            .title("Settings")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border.to_ratatui()))
            .style(theme.base_style())
            .title_style(Style::default().fg(theme.accent.to_ratatui()));
        f.render_widget(header, rows[0]);

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(20), Constraint::Min(0)])
            .split(rows[1]);

        // Sidebar navigation
        let sidebar_focused = self.focus == SettingsFocus::Sidebar;
        let active_idx = match self.section {
            SettingsSection::Workspaces => 0,
            SettingsSection::Appearance => 1,
            SettingsSection::Display => 2,
            SettingsSection::Sorting => 3,
            SettingsSection::Indexing => 4,
            SettingsSection::Editor => 5,
        };
        let items: Vec<ListItem> = ["Workspaces", "Appearance", "Display", "Sorting", "Indexing", "Editor"]
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
        let sidebar_list = List::new(items).block(sidebar_block);
        f.render_widget(sidebar_list, cols[0]);

        // Content panel
        let content_focused = self.focus == SettingsFocus::Content;
        match self.section {
            SettingsSection::Appearance => self
                .appearance_section
                .render(f, cols[1], &theme, content_focused),
            SettingsSection::Display => self
                .display_section
                .render(f, cols[1], &theme, content_focused),
            SettingsSection::Sorting => self
                .sorting_section
                .render(f, cols[1], &theme, content_focused),
            SettingsSection::Workspaces => {
                self.workspaces_section
                    .render(f, cols[1], &theme, content_focused)
            }
            SettingsSection::Indexing => {
                self.indexing_section
                    .render(f, cols[1], &theme, content_focused)
            }
            SettingsSection::Editor => {
                self.editor_section
                    .render(f, cols[1], &theme, content_focused)
            }
        }

        self.render_overlay(f, &theme);
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
                f.render_widget(
                    Paragraph::new("Enter: open  c: confirm  Esc: cancel  a-z: jump")
                        .style(theme.base_style()),
                    rows[2],
                );
            }

            Overlay::ConfirmFullReindex { focused_button } => {
                let area = fixed_centered_rect(44, 6, f.area());
                f.render_widget(Clear, area);
                let block = Block::default()
                    .title("Full Reindex")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.accent.to_ratatui()))
                    .style(theme.base_style());
                let inner = block.inner(area);
                f.render_widget(block, area);
                let cancel = if *focused_button == ConfirmButton::Cancel {
                    "[ Cancel ]"
                } else {
                    "  Cancel  "
                };
                let confirm = if *focused_button == ConfirmButton::Confirm {
                    "[ Confirm ]"
                } else {
                    "  Confirm  "
                };
                f.render_widget(
                    Paragraph::new(format!(
                        "\n  This may take a while.\n\n  {}    {}",
                        cancel, confirm
                    ))
                    .style(theme.base_style()),
                    inner,
                );
            }

            Overlay::ConfirmSave { focused_button } => {
                let area = fixed_centered_rect(44, 6, f.area());
                f.render_widget(Clear, area);
                let block = Block::default()
                    .title("Save Settings?")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.accent.to_ratatui()))
                    .style(theme.base_style());
                let inner = block.inner(area);
                f.render_widget(block, area);
                let save = if *focused_button == SaveButton::Save {
                    "[ Save ]"
                } else {
                    "  Save  "
                };
                let discard = if *focused_button == SaveButton::Discard {
                    "[ Discard ]"
                } else {
                    "  Discard  "
                };
                f.render_widget(
                    Paragraph::new(format!(
                        "\n  You have unsaved changes.\n\n  {}    {}",
                        save, discard
                    ))
                    .style(theme.base_style()),
                    inner,
                );
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

    fn make_screen() -> SettingsScreen {
        SettingsScreen::new(AppSettings::default())
    }

    #[test]
    fn esc_sends_close_settings_when_no_changes() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.handle_input(&key(KeyCode::Esc), &tx);
        let msg = rx.try_recv().expect("expected message");
        assert!(matches!(msg, AppEvent::CloseSettings));
    }

    #[test]
    fn esc_shows_confirm_save_when_settings_changed() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.settings.set_theme("Gruvbox Light".to_string());
        screen.handle_input(&key(KeyCode::Esc), &tx);
        assert!(rx.try_recv().is_err(), "no message should be sent yet");
        assert!(matches!(screen.overlay, Overlay::ConfirmSave { .. }));
    }

    #[test]
    fn confirm_save_discard_sends_close_settings() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.settings.set_theme("Gruvbox Light".to_string());
        screen.overlay = Overlay::ConfirmSave {
            focused_button: SaveButton::Discard,
        };
        screen.handle_input(&key(KeyCode::Enter), &tx);
        let msg = rx.try_recv().expect("expected message");
        assert!(matches!(msg, AppEvent::CloseSettings));
    }

    #[test]
    fn confirm_save_save_vault_unchanged_sends_settings_saved() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.settings.set_theme("Gruvbox Light".to_string());
        screen.overlay = Overlay::ConfirmSave {
            focused_button: SaveButton::Save,
        };
        screen.handle_input(&key(KeyCode::Enter), &tx);
        let msg = rx.try_recv().expect("expected message");
        assert!(matches!(msg, AppEvent::SettingsSaved));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn confirm_save_vault_changed_sets_pending_and_shows_progress() {
        let (tx, _rx) = unbounded_channel();
        let mut settings = AppSettings::default();
        settings.set_workspace(&PathBuf::from("/original/path"));
        let mut screen = SettingsScreen::new(settings);
        screen.settings.set_workspace(&PathBuf::from("/new/path"));
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
        let msg = rx.try_recv().expect("expected SettingsSaved");
        assert!(matches!(msg, AppEvent::SettingsSaved));
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
        assert!(rx.try_recv().is_err(), "no SettingsSaved when index failed");
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
        let settings = AppSettings::default();
        let screen = SettingsScreen::new_with_error(settings, "test error msg".to_string());
        match screen.overlay {
            Overlay::VaultConflict(ref msg) => {
                assert_eq!(msg, "test error msg");
            }
            _ => panic!("expected Overlay::VaultConflict(...)"),
        }
    }
}
