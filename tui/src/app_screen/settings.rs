use std::path::PathBuf;
use async_trait::async_trait;
use ratatui::crossterm::event::KeyCode;
use ratatui::widgets::{Block, Borders, ListState};

use crate::app_screen::AppScreen;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

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

#[derive(Debug, Clone, Copy, PartialEq)]
enum SettingsSection { Theme, Vault, Indexing }

#[derive(Debug, Clone, Copy, PartialEq)]
enum SettingsFocus { Sidebar, Content }

pub struct SettingsScreen {
    pub settings: AppSettings,
    pub initial_settings: AppSettings,
    pub theme: Theme,
    section: SettingsSection,
    focus: SettingsFocus,
    pub pending_save_after_index: bool,
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
