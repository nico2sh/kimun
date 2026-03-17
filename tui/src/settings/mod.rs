use crate::keys::KeyBindBatch;
use crate::keys::action_shortcuts::{ActionShortcuts, TextAction};
use crate::keys::key_strike::KeyStrike;
use crate::settings::config_dir::get_or_create_config_dir;
use crate::settings::themes::Theme;
use std::io::{Read, Write};
use std::path::PathBuf;

use std::fs::{self, File};

use color_eyre::eyre;
use kimun_core::nfs::VaultPath;
use log::debug;

use crate::keys::KeyBindings;
mod config_dir;
pub mod themes;

// pub mod theme;

#[cfg(debug_assertions)]
const CONFIG_DIR: &str = "kimun_debug";
#[cfg(not(debug_assertions))]
const CONFIG_DIR: &str = "kimun";

const BASE_CONFIG_FILE: &str = "config.toml";
const THEMES_DIR: &str = "themes";

const LAST_PATH_HISTORY_SIZE: usize = 10;

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
}

#[cfg(target_os = "macos")]
fn get_kb_buildr_ctrl_meta(key_bindings: &mut KeyBindings) -> KeyBindBatch<'_> {
    key_bindings.batch_add().with_meta()
}

#[cfg(not(target_os = "macos"))]
fn get_kb_buildr_ctrl_meta(key_bindings: &mut KeyBindings) -> KeyBindBatch<'_> {
    key_bindings.batch_add().with_ctrl()
}

fn default_keybindings() -> KeyBindings {
    let mut kb = KeyBindings::empty();
    // We use meta on macOS, ctrl on Windows/Linux for desktop-app shortcuts.
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

    // TUI navigation shortcuts (always Ctrl — terminal apps don't use Cmd/Meta).
    kb.batch_add()
        .with_ctrl()
        .add(KeyStrike::KeyB, ActionShortcuts::ToggleSidebar)
        .add(KeyStrike::KeyN, ActionShortcuts::SortByName)
        .add(KeyStrike::KeyT, ActionShortcuts::SortByTitle)
        .add(KeyStrike::KeyR, ActionShortcuts::SortReverseOrder);

    // Tab / Shift+Tab for focus switching (no modifier / shift only).
    kb.batch_add()
        .add(KeyStrike::Tab, ActionShortcuts::FocusEditor);
    kb.batch_add()
        .with_shift()
        .add(KeyStrike::Tab, ActionShortcuts::FocusSidebar);

    kb
}

fn yes() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            last_paths: vec![],
            workspace_dir: None,
            theme: Default::default(),
            needs_indexing: true,
            key_bindings: default_keybindings(),
        }
    }
}

impl AppSettings {
    pub fn theme_list(&self) -> Vec<Theme> {
        let mut list = vec![
            Theme::gruvbox_dark(),
            Theme::gruvbox_light(),
            Theme::catppuccin_mocha(),
            Theme::catppuccin_latte(),
            Theme::tokyo_night(),
            Theme::tokyo_night_storm(),
            Theme::solarized_dark(),
            Theme::solarized_light(),
            Theme::nord(),
        ];
        list.append(&mut Self::load_custom_themes());
        // Merge the user's default.toml override if present.
        if let Ok(custom_default) = Self::load_default_theme() {
            list.push(custom_default);
        }
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    fn get_config_file_path() -> eyre::Result<PathBuf> {
        let config_home = get_or_create_config_dir(CONFIG_DIR)?;
        Ok(config_home.join(BASE_CONFIG_FILE))
    }

    fn get_themes_path() -> eyre::Result<PathBuf> {
        let config_home = get_or_create_config_dir(CONFIG_DIR)?;
        Ok(config_home.join(THEMES_DIR))
    }

    fn load_theme_from_path(path: &std::path::Path) -> eyre::Result<Theme> {
        let theme_string = fs::read_to_string(path)?;
        match toml::from_str::<Theme>(&theme_string) {
            Ok(theme) => Ok(theme),
            Err(e) => {
                debug!("Failed to deserialize theme file {:?}: {}. Removing.", path, e);
                let _ = fs::remove_file(path);
                Err(eyre::eyre!("corrupt theme file: {}", e))
            }
        }
    }

    fn load_default_theme() -> eyre::Result<Theme> {
        let theme_path = AppSettings::get_themes_path()?.join("default.toml");
        Self::load_theme_from_path(&theme_path)
    }

    fn load_custom_themes() -> Vec<Theme> {
        let mut themes = Vec::new();

        // Get themes directory, return empty vec if it fails
        let themes_path = match Self::get_themes_path() {
            Ok(path) => path,
            Err(_) => return themes,
        };

        // Read directory entries, return empty vec if it fails
        let entries = match fs::read_dir(&themes_path) {
            Ok(entries) => entries,
            Err(_) => return themes,
        };

        // Iterate through all entries in the themes directory
        for entry in entries.flatten() {
            let path = entry.path();

            // Skip if not a file
            if !path.is_file() {
                continue;
            }

            // Skip if not a .toml file
            if path.extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }

            // Skip default.toml
            if path.file_name().and_then(|s| s.to_str()) == Some("default.toml") {
                continue;
            }

            // Try to read and deserialize the theme file
            if let Ok(theme_string) = fs::read_to_string(&path) {
                if let Ok(theme) = toml::from_str::<Theme>(&theme_string) {
                    themes.push(theme);
                } else {
                    debug!("Failed to deserialize theme file: {:?}", path);
                }
            }
        }

        themes
    }

    pub fn save_to_disk(&self) -> eyre::Result<()> {
        log::debug!("Saving settings to disk");
        let settings_file_path = Self::get_config_file_path()?;
        let mut file = File::create(settings_file_path)?;
        let toml = toml::to_string(&self)?;
        file.write_all(toml.as_bytes())?;
        Ok(())
    }

    pub fn load_from_disk() -> eyre::Result<Self> {
        let settings_file_path = Self::get_config_file_path()?;

        if !settings_file_path.exists() {
            let default_settings = Self::default();
            default_settings.save_to_disk()?;
            Ok(default_settings)
        } else {
            let mut settings_file = File::open(&settings_file_path)?;

            let mut toml = String::new();
            settings_file.read_to_string(&mut toml)?;

            let mut setting: AppSettings = toml::from_str(toml.as_ref())?;
            // Ensure any new default bindings added after the config was saved are present.
            setting.merge_missing_default_bindings();
            Ok(setting)
        }
    }

    /// Fills in any actions from `default_keybindings()` that are absent in the loaded config.
    /// Existing user-customised bindings are never overwritten.
    fn merge_missing_default_bindings(&mut self) {
        let mut current = self.key_bindings.to_hashmap();
        for (action, combos) in default_keybindings().to_hashmap() {
            current.entry(action).or_insert(combos);
        }
        self.key_bindings = KeyBindings::from_hashmap(current);
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

    /// Resolve the active theme by name, falling back to the default.
    pub fn get_theme(&self) -> Theme {
        if self.theme.is_empty() {
            return Theme::default();
        }
        self.theme_list()
            .into_iter()
            .find(|t| t.name == self.theme)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_theme_from_nonexistent_path_returns_err_without_creating_file() {
        // RED: fails to compile because load_theme_from_path doesn't exist.
        // GREEN: method exists, returns Err, and does NOT create the file.
        let path = std::env::temp_dir().join("kimun_tdd_test_theme_absent.toml");
        let _ = std::fs::remove_file(&path); // ensure clean state

        let result = AppSettings::load_theme_from_path(&path);

        assert!(result.is_err(), "should return Err when file is absent");
        assert!(!path.exists(), "must not create the file as a side effect");
    }

    #[test]
    fn load_theme_from_corrupt_path_returns_err_without_recreating_file() {
        // After a corrupt file is removed, no replacement must be written.
        let path = std::env::temp_dir().join("kimun_tdd_test_theme_corrupt.toml");
        std::fs::write(&path, b"not valid toml {{{{").unwrap();

        let result = AppSettings::load_theme_from_path(&path);

        assert!(result.is_err(), "should return Err for corrupt TOML");
        assert!(!path.exists(), "corrupt file must be removed, not recreated");
    }
}
