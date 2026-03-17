use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use kimun_core::{NoteVault, NotesValidation};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use throbber_widgets_tui::{Throbber, ThrobberState};

use crate::app_screen::AppScreen;
use crate::components::Component;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::components::settings::indexing_section::IndexingSection;
use crate::components::settings::theme_picker::ThemePicker;
use crate::components::settings::vault_section::VaultSection;
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

// ── Overlay types ─────────────────────────────────────────────────────────────

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

// ── Section / Focus enums ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum SettingsSection { Theme, Vault, Indexing }

#[derive(Debug, Clone, Copy, PartialEq)]
enum SettingsFocus { Sidebar, Content }

// ── SettingsScreen ────────────────────────────────────────────────────────────

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
            self.settings.save_to_disk().ok();
            let settings = self.settings.clone();
            tx.send(AppMessage::SettingsSaved(settings)).ok();
        }
    }
}

// ── AppScreen impl ────────────────────────────────────────────────────────────

#[async_trait]
impl AppScreen for SettingsScreen {
    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState {
        // Route to active overlay first.
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
                match state {
                    IndexingProgressState::Running(_) => {
                        return EventState::Consumed; // block all input while running
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

        // No active overlay — handle global keys.
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
            _ => match self.focus {
                SettingsFocus::Sidebar => match key.code {
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
                },
                SettingsFocus::Content => {
                    let app_event = AppEvent::Key(*key);
                    let result = match self.section {
                        SettingsSection::Theme => {
                            let r = self.theme_picker.handle_event(&app_event, tx);
                            // Live theme preview on every navigation step.
                            let name = self.theme_picker.selected_theme_name().to_string();
                            self.settings.set_theme(name);
                            self.theme = self.settings.get_theme();
                            r
                        }
                        SettingsSection::Vault => self.vault_section.handle_event(&app_event, tx),
                        SettingsSection::Indexing => self.indexing_section.handle_event(&app_event, tx),
                    };
                    result
                }
            },
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
        let theme = self.theme.clone();

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(f.area());

        let header = Block::default()
            .title("Settings")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border.to_ratatui()))
            .title_style(Style::default().fg(theme.accent.to_ratatui()));
        f.render_widget(header, rows[0]);

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(20), Constraint::Min(0)])
            .split(rows[1]);

        // Sidebar navigation
        let sidebar_focused = self.focus == SettingsFocus::Sidebar;
        let active_idx = match self.section {
            SettingsSection::Theme => 0,
            SettingsSection::Vault => 1,
            SettingsSection::Indexing => 2,
        };
        let items: Vec<ListItem> = ["Theme", "Vault", "Indexing"].iter().enumerate().map(|(i, name)| {
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
            SettingsSection::Theme => self.theme_picker.render(f, cols[1], &theme, content_focused),
            SettingsSection::Vault => self.vault_section.render(f, cols[1], &theme, content_focused),
            SettingsSection::Indexing => self.indexing_section.render(f, cols[1], &theme, content_focused),
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
                    .border_style(Style::default().fg(theme.accent.to_ratatui()));
                let inner = block.inner(area);
                f.render_widget(block, area);

                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)])
                    .split(inner);

                let path_str = fb.current_path.to_string_lossy().into_owned();
                f.render_widget(Paragraph::new(path_str), rows[0]);

                let items: Vec<ListItem> = fb.entries.iter().map(|e| {
                    let name = e.file_name().unwrap_or_default().to_string_lossy();
                    ListItem::new(format!("  {}/", name))
                }).collect();
                let list = List::new(items)
                    .highlight_symbol("▶ ")
                    .highlight_style(Style::default().add_modifier(Modifier::BOLD));
                f.render_stateful_widget(list, rows[1], &mut fb.list_state);
                f.render_widget(Paragraph::new("Enter: open  c: confirm  Esc: cancel"), rows[2]);
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
                let cancel = if *focused_button == ConfirmButton::Cancel { "[ Cancel ]" } else { "  Cancel  " };
                let confirm = if *focused_button == ConfirmButton::Confirm { "[ Confirm ]" } else { "  Confirm  " };
                f.render_widget(Paragraph::new(format!("\n  This may take a while.\n\n  {}    {}", cancel, confirm)), inner);
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
                let save = if *focused_button == SaveButton::Save { "[ Save ]" } else { "  Save  " };
                let discard = if *focused_button == SaveButton::Discard { "[ Discard ]" } else { "  Discard  " };
                f.render_widget(Paragraph::new(format!("\n  You have unsaved changes.\n\n  {}    {}", save, discard)), inner);
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
                        let throbber = Throbber::default().label("  Reindex in progress…");
                        f.render_stateful_widget(throbber, inner, &mut self.throbber_state);
                    }
                    IndexingProgressState::Done(dur) => {
                        f.render_widget(Paragraph::new(format!("  ✓  Done in {}s\n\n       [ OK ]", dur.as_secs())), inner);
                    }
                    IndexingProgressState::Failed(msg) => {
                        f.render_widget(Paragraph::new(format!("  ✗  Error: {}\n\n       [ OK ]", msg)), inner);
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
        assert_eq!(state.list_state.selected(), Option::None);
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
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, KeyEventKind, KeyEventState};
    use tokio::sync::mpsc::unbounded_channel;

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
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn confirm_save_vault_changed_sets_pending_and_shows_progress() {
        let (tx, _rx) = unbounded_channel();
        let mut settings = AppSettings::default();
        settings.set_workspace(&PathBuf::from("/original/path"));
        let mut screen = SettingsScreen::new(settings);
        screen.settings.set_workspace(&PathBuf::from("/new/path"));
        screen.overlay = Overlay::ConfirmSave { focused_button: SaveButton::Save };
        screen.handle_event(&key(KeyCode::Enter), &tx);
        assert!(screen.pending_save_after_index);
        assert!(matches!(screen.overlay, Overlay::IndexingProgress(IndexingProgressState::Running(_))));
    }

    #[tokio::test]
    async fn indexing_done_ok_with_pending_auto_closes() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.pending_save_after_index = true;
        screen.overlay = Overlay::IndexingProgress(IndexingProgressState::Running(tokio::spawn(async {})));
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
        screen.overlay = Overlay::IndexingProgress(IndexingProgressState::Running(tokio::spawn(async {})));
        screen.handle_app_message(AppMessage::IndexingDone(Err("disk error".to_string())), &tx).await;
        assert!(rx.try_recv().is_err(), "no SettingsSaved when index failed");
        assert!(!screen.pending_save_after_index);
        assert!(matches!(screen.overlay, Overlay::IndexingProgress(IndexingProgressState::Failed(_))));
    }

    #[tokio::test]
    async fn indexing_done_ok_without_pending_shows_done() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.pending_save_after_index = false;
        screen.overlay = Overlay::IndexingProgress(IndexingProgressState::Running(tokio::spawn(async {})));
        screen.handle_app_message(AppMessage::IndexingDone(Ok(Duration::from_secs(2))), &tx).await;
        assert!(rx.try_recv().is_err(), "no auto-close when pending is false");
        assert!(matches!(screen.overlay, Overlay::IndexingProgress(IndexingProgressState::Done(_))));
    }

    #[test]
    fn esc_blocked_while_indexing_running() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (tx, mut rx) = unbounded_channel();
        let mut screen = make_screen();
        screen.overlay = Overlay::IndexingProgress(IndexingProgressState::Running(
            rt.spawn(async {}),
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
