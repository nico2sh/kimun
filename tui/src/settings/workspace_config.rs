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
    /// Whether kimün may contact GitHub to check for a newer release. User-owned
    /// (toggled in onboarding and preferences); defaults on. All machine-managed
    /// update state lives separately in `update_state.toml`, never here.
    #[serde(default = "default_update_check")]
    pub update_check: bool,
    /// Whether kimün captures the mouse for in-app use (divider drag, list
    /// scroll, click-to-focus). When off, the mouse is left to the terminal so
    /// its native selection and middle-click paste work; mouse reporting is
    /// all-or-nothing, so there is no per-button middle ground (see adr/0015).
    /// Read only at startup. Defaults on (today's behavior).
    #[serde(default = "default_mouse")]
    pub mouse: bool,
    /// Base URL of the optional RAG server (e.g. `http://localhost:7573`). When
    /// set and reachable, kimün enables semantic search and Q&A (adr/0018).
    /// Global (one server serves many vaults, each as its own collection —
    /// adr/0020); `None` means the feature is off.
    #[serde(default)]
    pub kimun_server_url: Option<String>,
    /// Bearer token for the RAG server, when it requires one.
    #[serde(default)]
    pub kimun_server_token: Option<String>,
}

fn default_update_check() -> bool {
    true
}

fn default_mouse() -> bool {
    true
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
                update_check: true,
                mouse: true,
                kimun_server_url: None,
                kimun_server_token: None,
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

        // Set as current if there is no valid current workspace (first
        // workspace, or the previous current was removed/cleared)
        if !self.workspaces.contains_key(&self.global.current_workspace) {
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

    #[test]
    fn add_workspace_sets_current_when_first() {
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("notes".to_string(), PathBuf::from("/tmp/x"))
            .unwrap();
        assert_eq!(wc.global.current_workspace, "notes");
    }

    #[test]
    fn add_workspace_keeps_valid_current() {
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("first".to_string(), PathBuf::from("/tmp/a"))
            .unwrap();
        wc.add_workspace("second".to_string(), PathBuf::from("/tmp/b"))
            .unwrap();
        assert_eq!(wc.global.current_workspace, "first");
    }

    #[test]
    fn add_workspace_repairs_dangling_current() {
        // After clear_workspace the current entry is removed but other
        // workspaces remain; the next add must become current or the
        // app can never activate a workspace again.
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("other".to_string(), PathBuf::from("/tmp/a"))
            .unwrap();
        wc.global.current_workspace = String::new();
        wc.add_workspace("fresh".to_string(), PathBuf::from("/tmp/b"))
            .unwrap();
        assert_eq!(wc.global.current_workspace, "fresh");
    }
}
