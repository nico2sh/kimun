pub mod page;

use std::io::{Read, Write};
use std::path::PathBuf;

use std::fs::File;

use anyhow::bail;
use kimun_core::nfs::VaultPath;
use log::debug;
use serde::Deserialize;

const BASE_CONFIG_FILE: &str = ".kimun.toml";
const LAST_PATH_HISTORY_SIZE: usize = 10;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Settings {
    #[serde(default)]
    pub last_paths: Vec<VaultPath>,
    pub workspace_dir: Option<PathBuf>,
    #[serde(
        default = "def_theme",
        serialize_with = "ser_theme",
        deserialize_with = "deser_theme"
    )]
    pub theme: iced::Theme,
}

pub fn def_theme() -> iced::Theme {
    iced::Theme::ALL[0].to_owned()
}

pub fn ser_theme<S>(value: &iced::Theme, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_str(&value.to_string())
}

fn deser_theme<'de, D>(data: D) -> Result<iced::Theme, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let theme_str = String::deserialize(data)?;
    let t = iced::Theme::ALL
        .iter()
        .find(|theme| theme.to_string() == theme_str);
    if let Some(theme) = t {
        Ok(theme.to_owned())
    } else {
        Ok(iced::Theme::ALL[0].clone())
    }
}

impl Default for Settings {
    fn default() -> Self {
        let default_theme = iced::Theme::ALL[0].clone();
        Self {
            last_paths: vec![],
            workspace_dir: None,
            theme: default_theme,
        }
    }
}

impl Settings {
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

    // We set a new workspace to work with, remember to save the data
    // to persist it in disk
    pub fn set_workspace(&mut self, workspace_path: &PathBuf) {
        if let Some(current_workspace_dir) = &self.workspace_dir {
            if workspace_path != current_workspace_dir {
                // We clean up the data related with the workspace
                self.last_paths = vec![];
            }
        }

        self.workspace_dir = Some(workspace_path.to_owned());
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
}
