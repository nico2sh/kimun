use std::io::{Read, Write};
use std::path::PathBuf;

use std::fs::File;

use anyhow::bail;
use notes_core::nfs::VaultPath;

const BASE_CONFIG_FILE: &str = ".note.toml";

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub last_path: VaultPath,
    pub workspace_dir: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            last_path: VaultPath::root(),
            workspace_dir: None,
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

    pub fn save(&self) -> anyhow::Result<()> {
        let settings_file_path = Self::get_config_file_path()?;
        let mut file = File::create(settings_file_path)?;
        let toml = toml::to_string(&self)?;
        file.write_all(toml.as_bytes())?;
        Ok(())
    }

    pub fn load() -> anyhow::Result<Self> {
        let settings_file_path = Self::get_config_file_path()?;

        if !settings_file_path.exists() {
            let default_settings = Self::default();
            default_settings.save()?;
            Ok(default_settings)
        } else {
            let mut settings_file = File::open(&settings_file_path)?;

            let mut toml = String::new();
            settings_file.read_to_string(&mut toml)?;

            let setting = toml::from_str(toml.as_ref())?;
            Ok(setting)
        }
    }

    pub fn set_workspace(&mut self, workspace_path: PathBuf) -> anyhow::Result<()> {
        self.workspace_dir = Some(workspace_path);
        self.save()
    }
}
