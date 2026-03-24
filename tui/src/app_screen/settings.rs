use std::path::PathBuf;

use async_trait::async_trait;
use kimun_core::{NoteVault, NotesValidation};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
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
use crate::components::settings::vault_section::VaultSection;
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
}

// ── Section / Focus enums ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum SettingsSection {
    Vault,
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
    vault_section: VaultSection,
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
        let vault_path = settings.workspace_dir.clone();
        let vault_available = vault_path.is_some();
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
            vault_section: VaultSection::new(vault_path),
            indexing_section: IndexingSection::new(vault_available),
            editor_section: EditorSection::new(autosave_interval_secs),
            settings,
            initial_settings,
            theme,
            section: SettingsSection::Vault,
            focus: SettingsFocus::Sidebar,
            overlay: Overlay::None,
            pending_save_after_index: false,
            throbber_state: ThrobberState::default(),
        }
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
                let result = async {
                    let vault = NoteVault::new(&workspace)
                        .await
                        .map_err(|e| e.to_string())?;
                    vault
                        .recreate_index()
                        .await
                        .map_err(|e| e.to_string())
                        .map(|r| r.duration)
                }
                .await;
                tx2.send(AppEvent::IndexingDone(result)).ok();
            });
            self.overlay = Overlay::IndexingProgress(spawn_running(handle, tx));
        } else {
            self.settings.save_to_disk().ok();
            let settings = self.settings.clone();
            tx.send(AppEvent::SettingsSaved(settings)).ok();
        }
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
                        self.settings.set_workspace(&chosen);
                        self.vault_section.set_path(Some(chosen));
                        self.indexing_section.set_vault_available(true);
                        self.overlay = Overlay::None;
                    }
                    _ => {
                        // Ctrl+Enter confirms too
                        if key.code == KeyCode::Enter
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            let chosen = fb.current_path.clone();
                            self.settings.set_workspace(&chosen);
                            self.vault_section.set_path(Some(chosen));
                            self.indexing_section.set_vault_available(true);
                            self.overlay = Overlay::None;
                        }
                    }
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
                                let result = async {
                                    let vault = NoteVault::new(&workspace)
                                        .await
                                        .map_err(|e| e.to_string())?;
                                    vault
                                        .recreate_index()
                                        .await
                                        .map_err(|e| e.to_string())
                                        .map(|r| r.duration)
                                }
                                .await;
                                tx2.send(AppEvent::IndexingDone(result)).ok();
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
                            SettingsSection::Vault => SettingsSection::Appearance,
                            SettingsSection::Appearance => SettingsSection::Display,
                            SettingsSection::Display => SettingsSection::Sorting,
                            SettingsSection::Sorting => SettingsSection::Indexing,
                            SettingsSection::Indexing => SettingsSection::Editor,
                            SettingsSection::Editor => SettingsSection::Vault,
                        };
                        EventState::Consumed
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.section = match self.section {
                            SettingsSection::Vault => SettingsSection::Editor,
                            SettingsSection::Appearance => SettingsSection::Vault,
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
                        SettingsSection::Vault => self.vault_section.handle_input(&app_event, tx),
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
                            let settings = self.settings.clone();
                            tx.send(AppEvent::SettingsSaved(settings)).ok();
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
            SettingsSection::Vault => 0,
            SettingsSection::Appearance => 1,
            SettingsSection::Display => 2,
            SettingsSection::Sorting => 3,
            SettingsSection::Indexing => 4,
            SettingsSection::Editor => 5,
        };
        let items: Vec<ListItem> = ["Vault", "Appearance", "Display", "Sorting", "Indexing", "Editor"]
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
            SettingsSection::Vault => {
                self.vault_section
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
                    Paragraph::new("Enter: open  c: confirm  Esc: cancel")
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
        assert!(matches!(msg, AppEvent::SettingsSaved(_)));
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
        assert!(matches!(msg, AppEvent::SettingsSaved(_)));
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
}
