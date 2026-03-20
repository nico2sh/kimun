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

const CONFIG_HEADER: &str = "\
# ─── Kimün configuration ────────────────────────────────────────────────────
#
# KEY BINDINGS
# ────────────
# Supported combinations: ctrl and/or alt (with optional shift) + a letter (a-z).
# Any combo that does not follow this rule is silently ignored when loaded.
#
# Format per action:
#   ActionName = [\"<modifiers> & <letter>\", ...]
#
# Available modifiers (combine with +):  ctrl   alt   shift
#
# Examples:
#   Quit         = [\"ctrl & Q\"]          # Ctrl+Q
#   SearchNotes  = [\"alt & E\"]           # Alt+E
#   OpenSettings = [\"ctrl+shift & P\"]    # Ctrl+Shift+P
#   NewJournal   = [\"ctrl+alt & J\"]      # Ctrl+Alt+J
#
# ─────────────────────────────────────────────────────────────────────────────
";


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
    #[serde(default = "default_autosave_interval")]
    pub autosave_interval_secs: u64,
    /// Custom config file path. `None` means use the default location.
    /// Not serialized — it's a runtime-only override.
    #[serde(skip)]
    pub config_file: Option<PathBuf>,
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
    // Only ctrl/alt (with optional shift) + a letter key (a-z) are valid.
    get_kb_buildr_ctrl_meta(&mut kb)
        .add(KeyStrike::KeyF, ActionShortcuts::ToggleNoteBrowser)
        .add(KeyStrike::KeyE, ActionShortcuts::SearchNotes)
        .add(KeyStrike::KeyO, ActionShortcuts::OpenNote)
        .add(KeyStrike::KeyY, ActionShortcuts::TogglePreview)
        .add(KeyStrike::KeyB, ActionShortcuts::Text(TextAction::Bold))
        .add(KeyStrike::KeyI, ActionShortcuts::Text(TextAction::Italic))
        .add(KeyStrike::KeyU, ActionShortcuts::Text(TextAction::Underline))
        .add(KeyStrike::KeyS, ActionShortcuts::Text(TextAction::Strikethrough))
        .add(KeyStrike::KeyL, ActionShortcuts::Text(TextAction::Link))
        .add(KeyStrike::KeyT, ActionShortcuts::Text(TextAction::ToggleHeader))
        // =============================
        // We add shift to the modifiers
        // =============================
        .with_shift()
        .add(KeyStrike::KeyL, ActionShortcuts::Text(TextAction::Image));

    // TUI navigation shortcuts (always Ctrl — terminal apps don't use Cmd/Meta).
    kb.batch_add()
        .with_ctrl()
        .add(KeyStrike::KeyP, ActionShortcuts::OpenSettings)
        .add(KeyStrike::KeyQ, ActionShortcuts::Quit)
        .add(KeyStrike::KeyJ, ActionShortcuts::NewJournal)
        .add(KeyStrike::KeyB, ActionShortcuts::ToggleSidebar)
        .add(KeyStrike::KeyN, ActionShortcuts::SortByName)
        .add(KeyStrike::KeyG, ActionShortcuts::SortByTitle)
        .add(KeyStrike::KeyR, ActionShortcuts::SortReverseOrder)
        .add(KeyStrike::KeyH, ActionShortcuts::FocusSidebar)
        .add(KeyStrike::KeyL, ActionShortcuts::FocusEditor);

    kb
}

fn yes() -> bool {
    true
}

fn default_autosave_interval() -> u64 {
    5
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            last_paths: vec![],
            workspace_dir: None,
            theme: Default::default(),
            needs_indexing: true,
            key_bindings: default_keybindings(),
            autosave_interval_secs: default_autosave_interval(),
            config_file: None,
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

    fn default_config_file_path() -> eyre::Result<PathBuf> {
        let config_home = get_or_create_config_dir(CONFIG_DIR)?;
        Ok(config_home.join(BASE_CONFIG_FILE))
    }

    fn get_config_file_path(&self) -> eyre::Result<PathBuf> {
        if let Some(ref path) = self.config_file {
            Ok(path.clone())
        } else {
            Self::default_config_file_path()
        }
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
                debug!(
                    "Failed to deserialize theme file {:?}: {}. Removing.",
                    path, e
                );
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
            match fs::read_to_string(&path)
                .and_then(|s| toml::from_str::<Theme>(&s).map_err(|e| std::io::Error::other(e)))
            {
                Ok(theme) => themes.push(theme),
                Err(e) => log::warn!("Skipping theme file {:?}: {}", path, e),
            }
        }

        themes
    }

    pub fn save_to_disk(&self) -> eyre::Result<()> {
        log::debug!("Saving settings to disk");
        let settings_file_path = self.get_config_file_path()?;
        let mut file = File::create(settings_file_path)?;
        file.write_all(CONFIG_HEADER.as_bytes())?;
        let toml = toml::to_string(&self)?;
        file.write_all(toml.as_bytes())?;
        Ok(())
    }

    pub fn load_from_disk() -> eyre::Result<Self> {
        let settings_file_path = Self::default_config_file_path()?;

        if !settings_file_path.exists() {
            let default_settings = Self::default();
            default_settings.save_to_disk()?;
            Ok(default_settings)
        } else {
            let mut settings_file = File::open(&settings_file_path)?;

            let mut toml = String::new();
            settings_file.read_to_string(&mut toml)?;

            match toml::from_str::<AppSettings>(toml.as_ref()) {
                Ok(mut setting) => {
                    setting.merge_missing_default_bindings();
                    Ok(setting)
                }
                Err(e) => {
                    log::warn!(
                        "Config file at {:?} could not be parsed ({}). \
                         Renaming to .corrupt and starting with defaults.",
                        settings_file_path,
                        e
                    );
                    let corrupt_path = settings_file_path.with_extension("toml.corrupt");
                    let _ = fs::rename(&settings_file_path, &corrupt_path);
                    let defaults = Self::default();
                    defaults.save_to_disk()?;
                    Ok(defaults)
                }
            }
        }
    }

    pub fn load_from_file(path: PathBuf) -> eyre::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            let mut default_settings = Self::default();
            default_settings.config_file = Some(path);
            default_settings.save_to_disk()?;
            return Ok(default_settings);
        }
        let mut toml_str = String::new();
        File::open(&path)?.read_to_string(&mut toml_str)?;
        match toml::from_str::<AppSettings>(&toml_str) {
            Ok(mut setting) => {
                setting.config_file = Some(path);
                setting.merge_missing_default_bindings();
                Ok(setting)
            }
            Err(e) => {
                log::warn!(
                    "Config file at {:?} could not be parsed ({}). \
                     Renaming to .corrupt and starting with defaults.",
                    path, e
                );
                let corrupt_path = path.with_extension("toml.corrupt");
                let _ = fs::rename(&path, &corrupt_path);
                let mut defaults = Self::default();
                defaults.config_file = Some(path);
                defaults.save_to_disk()?;
                Ok(defaults)
            }
        }
    }

    /// Fills in any actions from `default_keybindings()` that are absent in the loaded config.
    /// Existing user-customised bindings are never overwritten.
    fn merge_missing_default_bindings(&mut self) {
        let defaults = default_keybindings().to_hashmap();
        let mut current = self.key_bindings.to_hashmap();
        for (action, combos) in defaults {
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
        assert!(
            !path.exists(),
            "corrupt file must be removed, not recreated"
        );
    }

    #[test]
    fn autosave_interval_defaults_to_five() {
        let settings = AppSettings::default();
        assert_eq!(settings.autosave_interval_secs, 5);
    }

    #[test]
    fn autosave_interval_deserializes_from_toml() {
        let toml = "autosave_interval_secs = 30\n";
        let settings: AppSettings = toml::from_str(toml).unwrap();
        assert_eq!(settings.autosave_interval_secs, 30);
    }

    #[test]
    fn autosave_interval_defaults_when_missing_from_toml() {
        let toml = ""; // no autosave_interval_secs key
        let settings: AppSettings = toml::from_str(toml).unwrap();
        assert_eq!(settings.autosave_interval_secs, 5);
    }
}
