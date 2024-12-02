use std::io::{Read, Write};
use std::path::PathBuf;

use std::fs::File;

use core_notes::error::NoteInitError;
use core_notes::nfs::NotePath;

const BASE_CONFIG_FILE: &str = ".note.toml";

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub last_path: NotePath,
    pub workspace_dir: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            last_path: NotePath::root(),
            workspace_dir: None,
        }
    }
}

impl Settings {
    fn get_config_file_path() -> Result<PathBuf, NoteInitError> {
        let home = dirs::home_dir();
        match home {
            Some(directory) => Ok(directory.join(BASE_CONFIG_FILE)),
            None => Err(NoteInitError::PathNotFound {
                path: "Home Path".to_string(),
            }),
        }
    }

    pub fn save(&self) -> Result<(), NoteInitError> {
        let settings_file_path = Self::get_config_file_path()?;
        let mut file = File::create(settings_file_path).map_err(|e| NoteInitError::IOError {
            source: e,
            operation: "Error creating config file".to_string(),
        })?;
        let toml = toml::to_string(&self)?;
        file.write_all(toml.as_bytes())
            .map_err(|e| NoteInitError::IOError {
                source: e,
                operation: "Error writing config file".to_string(),
            })?;
        Ok(())
    }

    pub fn load() -> Result<Self, NoteInitError> {
        let settings_file_path = Self::get_config_file_path()?;

        if !settings_file_path.exists() {
            let default_settings = Self::default();
            default_settings.save()?;
            Ok(default_settings)
        } else {
            let mut settings_file =
                File::open(&settings_file_path).map_err(|e| NoteInitError::IOError {
                    source: e,
                    operation: "Error opening config file".to_string(),
                })?;

            let mut toml = String::new();
            settings_file
                .read_to_string(&mut toml)
                .map_err(|e| NoteInitError::IOError {
                    source: e,
                    operation: "Error loading config file".to_string(),
                })?;

            let setting = toml::from_str(toml.as_ref())?;
            Ok(setting)
        }
    }

    pub fn set_workspace(&mut self, workspace_path: PathBuf) -> Result<(), NoteInitError> {
        self.workspace_dir = Some(workspace_path);
        self.save()
    }
}
