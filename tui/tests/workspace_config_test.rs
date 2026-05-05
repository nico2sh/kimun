use chrono::Utc;
use kimun_notes::settings::workspace_config::{
    GlobalConfig, WorkspaceConfig, WorkspaceConfigError, WorkspaceEntry,
};
use std::collections::HashMap;
use std::path::PathBuf;

#[test]
fn workspace_config_serializes_to_toml() {
    let config = WorkspaceConfig {
        global: GlobalConfig {
            current_workspace: "default".to_string(),
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
                resolved_path: None,
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
        WorkspaceConfigError::DuplicateWorkspace {
            name,
            existing_path,
        } => {
            assert_eq!(name, "default");
            assert_eq!(existing_path, PathBuf::from("/Users/user/notes"));
        }
        _ => panic!("expected DuplicateWorkspace"),
    }
}

#[test]
fn workspace_config_get_current_workspace() {
    let mut config = WorkspaceConfig::new_empty();
    config
        .add_workspace("default".to_string(), PathBuf::from("/Users/user/notes"))
        .unwrap();

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

    config
        .add_workspace("test".to_string(), path.clone())
        .unwrap();
    config.workspaces.get_mut("test").unwrap().last_paths = last_paths.clone();

    // Serialize to TOML
    let toml_str = toml::to_string(&config).unwrap();

    // Deserialize back from TOML
    let deserialized: WorkspaceConfig = toml::from_str(&toml_str).unwrap();

    // Verify all data is preserved
    assert_eq!(
        config.global.current_workspace,
        deserialized.global.current_workspace
    );
    assert_eq!(config.workspaces.len(), deserialized.workspaces.len());

    let original_entry = config.workspaces.get("test").unwrap();
    let deserialized_entry = deserialized.workspaces.get("test").unwrap();
    assert_eq!(original_entry.path, deserialized_entry.path);
    assert_eq!(original_entry.last_paths, deserialized_entry.last_paths);
    // DateTime should round-trip correctly
    assert_eq!(
        original_entry.created.timestamp(),
        deserialized_entry.created.timestamp()
    );
}

#[test]
fn cache_dir_defaults_to_config_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        r#"
config_version = 3
cache_dir = "."
history_dir = "history"
theme = "gruvbox_dark"
"#,
    )
    .unwrap();

    let settings =
        kimun_notes::settings::AppSettings::load_from_file(cfg_path.clone()).unwrap();
    let resolved_cache = settings.cache_dir_resolved().unwrap();
    let resolved_hist = settings.history_dir_resolved().unwrap();
    assert_eq!(resolved_cache, tmp.path().canonicalize().unwrap());
    assert_eq!(
        resolved_hist,
        tmp.path().canonicalize().unwrap().join("history")
    );
}

#[test]
fn cache_dir_supports_absolute_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let abs_cache = tempfile::TempDir::new().unwrap();
    std::fs::write(
        &cfg_path,
        format!(
            r#"
config_version = 3
cache_dir = "{}"
history_dir = "history"
theme = "gruvbox_dark"
"#,
            abs_cache.path().display()
        ),
    )
    .unwrap();

    let settings =
        kimun_notes::settings::AppSettings::load_from_file(cfg_path.clone()).unwrap();
    assert_eq!(
        settings.cache_dir_resolved().unwrap(),
        abs_cache.path().canonicalize().unwrap()
    );
}

#[test]
fn add_path_history_writes_to_history_file_not_config() {
    use kimun_core::nfs::VaultPath;
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        format!(
            r#"
config_version = 3
cache_dir = "."
history_dir = "history"
theme = "gruvbox_dark"

[workspaces.notes]
path = "{}"
last_paths = []
created = "2026-01-01T00:00:00Z"

[global]
current_workspace = "notes"
"#,
            tmp.path().display()
        ),
    )
    .unwrap();
    let mut settings =
        kimun_notes::settings::AppSettings::load_from_file(cfg_path.clone()).unwrap();

    settings.add_path_history(&VaultPath::new("a.md"));
    settings.add_path_history(&VaultPath::new("b.md"));

    let history_file = tmp
        .path()
        .canonicalize()
        .unwrap()
        .join("history")
        .join("notes.txt");
    assert!(history_file.exists(), "history file should be written at {history_file:?}");
    let loaded = settings.current_last_paths();
    assert_eq!(
        loaded.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
        vec!["b.md".to_string(), "a.md".to_string()]
    );
}

#[test]
fn cache_path_for_uses_workspace_name_and_kimuncache_extension() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        r#"
config_version = 3
cache_dir = "."
history_dir = "history"
theme = "gruvbox_dark"
"#,
    )
    .unwrap();

    let settings =
        kimun_notes::settings::AppSettings::load_from_file(cfg_path.clone()).unwrap();
    let cache = settings.cache_path_for("myvault");
    assert_eq!(
        cache,
        tmp.path()
            .canonicalize()
            .unwrap()
            .join("myvault.kimuncache")
    );
    let hist = settings.history_path_for("myvault");
    assert_eq!(
        hist,
        tmp.path()
            .canonicalize()
            .unwrap()
            .join("history")
            .join("myvault.txt")
    );
}
