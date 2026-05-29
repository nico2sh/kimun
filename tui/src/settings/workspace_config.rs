use chrono::{DateTime, Utc};
use kimun_core::nfs::filename::{InvalidFilenameError, validate_filename};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum WorkspaceConfigError {
    DuplicateWorkspace {
        name: String,
        existing_path: PathBuf,
    },
    InvalidName {
        name: String,
        error: InvalidFilenameError,
    },
}

impl std::fmt::Display for WorkspaceConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkspaceConfigError::DuplicateWorkspace {
                name,
                existing_path,
            } => {
                write!(
                    f,
                    "Workspace '{}' already exists at {:?}",
                    name, existing_path
                )
            }
            WorkspaceConfigError::InvalidName { error, .. } => {
                write!(f, "Workspace {error}")
            }
        }
    }
}

impl std::error::Error for WorkspaceConfigError {}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GlobalConfig {
    pub current_workspace: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceEntry {
    pub path: PathBuf,
    #[serde(default, skip_serializing)]
    pub last_paths: Vec<String>,
    pub created: DateTime<Utc>,
    #[serde(default)]
    pub quick_note_path: Option<String>,
    #[serde(default)]
    pub inbox_path: Option<String>,
    /// Absolute resolved path for runtime use. Not serialized — `path` is
    /// written to disk as the user configured it (relative, ~/..., or absolute).
    #[serde(skip)]
    pub resolved_path: Option<PathBuf>,
}

impl WorkspaceEntry {
    /// Returns the resolved absolute path if available, otherwise the original path.
    pub fn effective_path(&self) -> &PathBuf {
        self.resolved_path.as_ref().unwrap_or(&self.path)
    }

    pub fn effective_quick_note_path(&self) -> String {
        self.quick_note_path
            .clone()
            .unwrap_or_else(|| kimun_core::nfs::VaultPath::root().to_string())
    }

    pub fn effective_inbox_path(&self) -> String {
        self.inbox_path
            .clone()
            .unwrap_or_else(|| kimun_core::DEFAULT_INBOX_PATH.to_string())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceConfig {
    pub global: GlobalConfig,
    /// Keyed by workspace name. `BTreeMap` (not `HashMap`) so serialization
    /// order is deterministic — otherwise every config save reshuffles the
    /// `[workspaces.*]` sections in the TOML file.
    pub workspaces: BTreeMap<String, WorkspaceEntry>,
}

impl WorkspaceConfig {
    pub fn new_empty() -> Self {
        Self {
            global: GlobalConfig {
                current_workspace: String::new(),
            },
            workspaces: BTreeMap::new(),
        }
    }

    pub fn add_workspace(
        &mut self,
        name: String,
        path: PathBuf,
    ) -> Result<(), WorkspaceConfigError> {
        if let Err(error) = validate_filename(&name) {
            return Err(WorkspaceConfigError::InvalidName {
                name: name.clone(),
                error,
            });
        }
        if self.workspaces.contains_key(&name) {
            return Err(WorkspaceConfigError::DuplicateWorkspace {
                name: name.clone(),
                existing_path: self.workspaces[&name].path.clone(),
            });
        }

        let entry = WorkspaceEntry {
            path,
            last_paths: Vec::new(),
            created: Utc::now(),
            quick_note_path: None,
            inbox_path: None,
            resolved_path: None,
        };

        self.workspaces.insert(name.clone(), entry);

        // Set as current if it's the first workspace
        if self.workspaces.len() == 1 {
            self.global.current_workspace = name.clone();
        }

        Ok(())
    }

    pub fn get_current_workspace(&self) -> Option<&WorkspaceEntry> {
        self.workspaces.get(&self.global.current_workspace)
    }

    pub fn get_workspace(&self, name: &str) -> Option<&WorkspaceEntry> {
        self.workspaces.get(name)
    }

    pub fn from_phase1_migration(workspace_dir: PathBuf, last_paths: Vec<String>) -> Self {
        let mut config = Self::new_empty();

        let entry = WorkspaceEntry {
            path: workspace_dir,
            last_paths,
            created: Utc::now(),
            quick_note_path: None,
            inbox_path: None,
            resolved_path: None,
        };

        config.workspaces.insert("default".to_string(), entry);
        config.global.current_workspace = "default".to_string();

        config
    }
}

#[cfg(test)]
mod validate_tests {
    use super::*;

    #[test]
    fn add_workspace_rejects_disallowed_chars() {
        let mut wc = WorkspaceConfig::new_empty();
        let err = wc
            .add_workspace("bad/name".to_string(), PathBuf::from("/tmp/x"))
            .unwrap_err();
        match err {
            WorkspaceConfigError::InvalidName { name, .. } => assert_eq!(name, "bad/name"),
            _ => panic!("expected InvalidName"),
        }
    }

    #[test]
    fn add_workspace_rejects_windows_reserved() {
        let mut wc = WorkspaceConfig::new_empty();
        assert!(
            wc.add_workspace("con".to_string(), PathBuf::from("/tmp/x"))
                .is_err()
        );
    }

    #[test]
    fn add_workspace_accepts_simple_names() {
        let mut wc = WorkspaceConfig::new_empty();
        assert!(
            wc.add_workspace("notes".to_string(), PathBuf::from("/tmp/x"))
                .is_ok()
        );
    }
}
