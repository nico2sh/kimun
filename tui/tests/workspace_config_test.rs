use chrono::Utc;
use kimun_notes::settings::workspace_config::{GlobalConfig, WorkspaceConfig, WorkspaceEntry};
use std::collections::HashMap;
use std::path::PathBuf;

#[test]
fn workspace_config_serializes_to_toml() {
    let config = WorkspaceConfig {
        global: GlobalConfig {
            current_workspace: "default".to_string(),
            theme: "dark".to_string(),
        },
        workspaces: HashMap::from([(
            "default".to_string(),
            WorkspaceEntry {
                path: PathBuf::from("/Users/user/notes"),
                last_paths: vec!["/journal".to_string(), "/projects".to_string()],
                created: chrono::DateTime::parse_from_rfc3339("2024-01-15T10:30:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            },
        )]),
    };

    let toml = toml::to_string(&config).unwrap();
    assert!(toml.contains("[global]"));
    assert!(toml.contains("current_workspace = \"default\""));
    assert!(toml.contains("[workspaces.default]"));
}

#[test]
fn workspace_config_add_workspace() {
    let mut config = WorkspaceConfig::new_empty();

    // Add first workspace
    let result = config.add_workspace("default".to_string(), PathBuf::from("/Users/user/notes"));
    assert!(result.is_ok());
    assert_eq!(config.global.current_workspace, "default");

    // Verify it was added
    assert!(config.workspaces.contains_key("default"));
    let entry = config.get_workspace("default");
    assert!(entry.is_some());
    assert_eq!(entry.unwrap().path, PathBuf::from("/Users/user/notes"));

    // Add second workspace
    let result = config.add_workspace("work".to_string(), PathBuf::from("/Users/user/work"));
    assert!(result.is_ok());
    // Current should still be default
    assert_eq!(config.global.current_workspace, "default");

    // Try to add duplicate
    let result = config.add_workspace("default".to_string(), PathBuf::from("/Users/user/other"));
    assert!(result.is_err());
}

#[test]
fn workspace_config_get_current_workspace() {
    let mut config = WorkspaceConfig::new_empty();
    config.add_workspace("default".to_string(), PathBuf::from("/Users/user/notes")).unwrap();

    let current = config.get_current_workspace();
    assert!(current.is_some());
    assert_eq!(current.unwrap().path, PathBuf::from("/Users/user/notes"));
}
