use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum WorkspaceConfigError {
    DuplicateWorkspace {
        name: String,
        existing_path: PathBuf,
    },
}

impl std::fmt::Display for WorkspaceConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkspaceConfigError::DuplicateWorkspace { name, existing_path } => {
                write!(f, "Workspace '{}' already exists at {:?}", name, existing_path)
            }
        }
    }
}

impl std::error::Error for WorkspaceConfigError {}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GlobalConfig {
    pub current_workspace: String,
    pub theme: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceEntry {
    pub path: PathBuf,
    pub last_paths: Vec<String>,
    pub created: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceConfig {
    pub global: GlobalConfig,
    pub workspaces: HashMap<String, WorkspaceEntry>,
}

impl WorkspaceConfig {
    pub fn new_empty() -> Self {
        Self {
            global: GlobalConfig {
                current_workspace: String::new(),
                theme: "dark".to_string(),
            },
            workspaces: HashMap::new(),
        }
    }

    pub fn add_workspace(&mut self, name: String, path: PathBuf) -> Result<(), WorkspaceConfigError> {
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
        };

        self.workspaces.insert(name.clone(), entry);

        // Set as current if it's the first workspace
        if self.workspaces.len() == 1 {
            self.global.current_workspace = name;
        }

        Ok(())
    }

    pub fn get_current_workspace(&self) -> Option<&WorkspaceEntry> {
        self.workspaces.get(&self.global.current_workspace)
    }

    pub fn get_workspace(&self, name: &str) -> Option<&WorkspaceEntry> {
        self.workspaces.get(name)
    }
}
