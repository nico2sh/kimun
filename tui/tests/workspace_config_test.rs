use chrono::Utc;
use kimun_notes::settings::workspace_config::{GlobalConfig, WorkspaceConfig, WorkspaceEntry, WorkspaceConfigError};
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
                quick_note_path: None,
                inbox_path: None,
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
    let err = result.unwrap_err();
    match err {
        WorkspaceConfigError::DuplicateWorkspace { name, existing_path } => {
            assert_eq!(name, "default");
            assert_eq!(existing_path, PathBuf::from("/Users/user/notes"));
        }
    }
}

#[test]
fn workspace_config_get_current_workspace() {
    let mut config = WorkspaceConfig::new_empty();
    config.add_workspace("default".to_string(), PathBuf::from("/Users/user/notes")).unwrap();

    let current = config.get_current_workspace();
    assert!(current.is_some());
    assert_eq!(current.unwrap().path, PathBuf::from("/Users/user/notes"));
}

#[test]
fn workspace_config_empty_has_no_current_workspace() {
    let config = WorkspaceConfig::new_empty();
    assert!(config.get_current_workspace().is_none());
    assert_eq!(config.global.current_workspace, "");
}

#[test]
fn workspace_config_round_trip_serialization() {
    let mut config = WorkspaceConfig::new_empty();
    let path = PathBuf::from("/test/path");
    let last_paths = vec!["path1".to_string(), "path2".to_string()];

    config.add_workspace("test".to_string(), path.clone()).unwrap();
    config.workspaces.get_mut("test").unwrap().last_paths = last_paths.clone();

    // Serialize to TOML
    let toml_str = toml::to_string(&config).unwrap();

    // Deserialize back from TOML
    let deserialized: WorkspaceConfig = toml::from_str(&toml_str).unwrap();

    // Verify all data is preserved
    assert_eq!(config.global.current_workspace, deserialized.global.current_workspace);
    assert_eq!(config.global.theme, deserialized.global.theme);
    assert_eq!(config.workspaces.len(), deserialized.workspaces.len());

    let original_entry = config.workspaces.get("test").unwrap();
    let deserialized_entry = deserialized.workspaces.get("test").unwrap();
    assert_eq!(original_entry.path, deserialized_entry.path);
    assert_eq!(original_entry.last_paths, deserialized_entry.last_paths);
    // DateTime should round-trip correctly
    assert_eq!(original_entry.created.timestamp(), deserialized_entry.created.timestamp());
}
