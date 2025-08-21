use crate::utils::keys::action_shortcuts::{ActionShortcuts, TextAction};
use crate::utils::keys::key_strike::KeyStrike;
use crate::utils::keys::KeyBindBatch;
use std::io::{Read, Write};
use std::path::PathBuf;

use std::fs::File;

use anyhow::bail;
use dioxus::logger::tracing::debug;
use dioxus::prelude::*;
use kimun_core::nfs::VaultPath;

use crate::settings::theme::Theme;
use crate::utils::keys::KeyBindings;

pub mod theme;

#[cfg(debug_assertions)]
const BASE_CONFIG_FILE: &str = ".kimun_debug.toml";
#[cfg(not(debug_assertions))]
const BASE_CONFIG_FILE: &str = ".kimun.toml";

const LAST_PATH_HISTORY_SIZE: usize = 10;

const THEME_GRUVBOX_DARK: Asset = asset!("/assets/styling/gruvbox_dark.css");
const THEME_GRUVBOX_LIGHT: Asset = asset!("/assets/styling/gruvbox_light.css");

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct AppSettings {
    #[serde(default)]
    pub last_paths: Vec<VaultPath>,
    pub workspace_dir: Option<PathBuf>,
    #[serde(default)]
    pub theme: String,
    #[serde(skip, default = "yes")]
    needs_indexing: bool,
    #[serde(default = "default_keybindings")]
    pub key_bindings: KeyBindings,
    #[serde(skip, default = "load_theme_list")]
    pub theme_list: Vec<Theme>,
}

#[cfg(target_os = "macos")]
fn get_kb_buildr_ctrl_meta(key_bindings: &mut KeyBindings) -> KeyBindBatch {
    key_bindings.batch_add().with_meta()
}

#[cfg(not(target_os = "macos"))]
fn get_kb_buildr_ctrl_meta(key_bindings: &mut KeyBindings) -> KeyBindBatch {
    key_bindings.batch_add().with_ctrl()
}

fn default_keybindings() -> KeyBindings {
    let mut kb = KeyBindings::empty();
    // We use meta on macOS, ctrl on Windows
    get_kb_buildr_ctrl_meta(&mut kb)
        .add(KeyStrike::Comma, ActionShortcuts::OpenSettings)
        .add(KeyStrike::Slash, ActionShortcuts::ToggleNoteBrowser)
        .add(KeyStrike::KeyE, ActionShortcuts::SearchNotes)
        .add(KeyStrike::KeyO, ActionShortcuts::OpenNote)
        .add(KeyStrike::KeyJ, ActionShortcuts::NewJournal)
        .add(KeyStrike::KeyY, ActionShortcuts::TogglePreview)
        .add(KeyStrike::KeyB, ActionShortcuts::Text(TextAction::Bold))
        .add(KeyStrike::KeyI, ActionShortcuts::Text(TextAction::Italic))
        .add(
            KeyStrike::KeyU,
            ActionShortcuts::Text(TextAction::Underline),
        )
        .add(
            KeyStrike::KeyS,
            ActionShortcuts::Text(TextAction::Strikethrough),
        )
        .add(KeyStrike::KeyL, ActionShortcuts::Text(TextAction::Link))
        .add(
            KeyStrike::KeyT,
            ActionShortcuts::Text(TextAction::ToggleHeader),
        )
        .add(
            KeyStrike::Digit1,
            ActionShortcuts::Text(TextAction::Header(1)),
        )
        .add(
            KeyStrike::Digit2,
            ActionShortcuts::Text(TextAction::Header(2)),
        )
        .add(
            KeyStrike::Digit3,
            ActionShortcuts::Text(TextAction::Header(3)),
        )
        // =============================
        // We add shift to the modifiers
        // =============================
        .with_shift()
        .add(KeyStrike::KeyL, ActionShortcuts::Text(TextAction::Image));
    kb
}

fn yes() -> bool {
    true
}

fn load_theme_list() -> Vec<Theme> {
    let list = vec![
        Theme::default(),
        Theme::new(THEME_GRUVBOX_LIGHT.to_string(), "Gruvbox Light"),
        Theme::new(THEME_GRUVBOX_DARK.to_string(), "Gruvbox Dark"),
    ];
    list
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            last_paths: vec![],
            workspace_dir: None,
            theme: Default::default(),
            needs_indexing: true,
            key_bindings: default_keybindings(),
            theme_list: load_theme_list(),
        }
    }
}

impl AppSettings {
    fn get_config_file_path() -> anyhow::Result<PathBuf> {
        let home = dirs::home_dir();
        match home {
            Some(directory) => Ok(directory.join(BASE_CONFIG_FILE)),
            None => bail!("Home path not found"),
        }
    }

    pub fn save_to_disk(&self) -> anyhow::Result<()> {
        debug!("Saving settings to disk");
        let settings_file_path = Self::get_config_file_path()?;
        let mut file = File::create(settings_file_path)?;
        let toml = toml::to_string(&self)?;
        file.write_all(toml.as_bytes())?;
        Ok(())
    }

    pub fn load_from_disk() -> anyhow::Result<Self> {
        let settings_file_path = Self::get_config_file_path()?;

        if !settings_file_path.exists() {
            let default_settings = Self::default();
            default_settings.save_to_disk()?;
            Ok(default_settings)
        } else {
            let mut settings_file = File::open(&settings_file_path)?;

            let mut toml = String::new();
            settings_file.read_to_string(&mut toml)?;

            let setting = toml::from_str(toml.as_ref())?;
            Ok(setting)
        }
    }

    pub fn get_workspace_string(&self) -> String {
        self.workspace_dir.as_ref().map_or_else(
            || "<NONE>".to_string(),
            |dir| dir.to_string_lossy().to_string(),
        )
    }

    // We set a new workspace to work with, remember to save the data
    // to persist it in disk
    pub fn set_workspace(&mut self, workspace_path: &PathBuf) {
        if let Some(current_workspace_dir) = &self.workspace_dir {
            if workspace_path != current_workspace_dir {
                // We clean up the data related with the workspace
                self.last_paths = vec![];
                self.needs_indexing = true;
            }
        }

        self.workspace_dir = Some(workspace_path.to_owned());
    }

    pub fn set_theme(&mut self, theme: String) {
        self.theme = theme;
    }

    pub fn report_indexed(&mut self) {
        self.needs_indexing = false;
    }

    pub fn needs_indexing(&self) -> bool {
        self.needs_indexing
    }

    pub fn add_path_history(&mut self, note_path: &VaultPath) {
        if note_path.is_note() {
            // If the path already is in the history, we remove it
            self.last_paths.retain(|path| !path.eq(note_path));
            // Maximum size of the path list
            // removing an element at a position is not very efficient
            // but since is a short list, shouldn't be a major problem
            while self.last_paths.len() >= LAST_PATH_HISTORY_SIZE {
                self.last_paths.remove(0);
            }
            self.last_paths.push(note_path.to_owned());
        }
    }

    pub fn get_theme(&self) -> Theme {
        self.theme_list
            .iter()
            .find_map(|t| {
                if t.name == self.theme {
                    Some(t.to_owned())
                } else {
                    None
                }
            })
            .unwrap_or_default()
    }
}
