use kimun_notes::settings::AppSettings;
use tempfile::TempDir;

#[test]
fn migrate_phase1_to_phase2_config() {
    let temp_dir = TempDir::new().unwrap();
    let workspace_dir = temp_dir.path().join("notes");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    let config_path = temp_dir.path().join("config.toml");

    // Write Phase 1 config
    let phase1_toml = format!(
        r#"
workspace_dir = "{}"
theme = "gruvbox_dark"
"#,
        workspace_dir.display()
    );
    std::fs::write(&config_path, &phase1_toml).unwrap();

    // Load and migrate
    let settings = AppSettings::load_from_file(config_path.clone()).unwrap();

    // Should have Phase 2 format
    assert!(settings.workspace_config.is_some());
    let ws_config = settings.workspace_config.unwrap();
    assert_eq!(ws_config.global.current_workspace, "default");
    assert_eq!(ws_config.workspaces.len(), 1);
    assert!(ws_config.workspaces.contains_key("default"));

    // Verify migration marker
    let config_content = std::fs::read_to_string(&config_path).unwrap();
    assert!(config_content.contains("config_version = 2"));
}

#[test]
fn migrate_phase1_preserves_last_paths() {
    let temp_dir = TempDir::new().unwrap();
    let workspace_dir = temp_dir.path().join("notes");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    let config_path = temp_dir.path().join("config.toml");

    let phase1_toml = format!(
        r#"
workspace_dir = "{}"
theme = "gruvbox_dark"
last_paths = ["/journal", "/tasks"]
"#,
        workspace_dir.display()
    );
    std::fs::write(&config_path, &phase1_toml).unwrap();

    let settings = AppSettings::load_from_file(config_path.clone()).unwrap();

    let ws_config = settings.workspace_config.unwrap();
    let default_ws = ws_config.workspaces.get("default").unwrap();
    assert_eq!(default_ws.last_paths, vec!["/journal", "/tasks"]);
    assert_eq!(ws_config.global.theme, "gruvbox_dark");
}

#[test]
fn migrate_phase1_fails_when_workspace_dir_missing() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.toml");

    // Point to a non-existent workspace directory
    let phase1_toml = r#"
workspace_dir = "/nonexistent/path/that/does/not/exist"
theme = "gruvbox_dark"
"#;
    std::fs::write(&config_path, phase1_toml).unwrap();

    let result = AppSettings::load_from_file(config_path.clone());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("Cannot migrate"));
}

#[test]
fn phase2_config_loads_without_migration() {
    let temp_dir = TempDir::new().unwrap();
    let workspace_dir = temp_dir.path().join("notes");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    let config_path = temp_dir.path().join("config.toml");

    // Write a Phase 2 config directly
    let phase2_toml = format!(
        r#"
config_version = 2

[global]
current_workspace = "default"
theme = "dark"

[workspaces.default]
path = "{}"
last_paths = []
created = "2024-01-15T10:30:00Z"
"#,
        workspace_dir.display()
    );
    std::fs::write(&config_path, &phase2_toml).unwrap();

    let settings = AppSettings::load_from_file(config_path.clone()).unwrap();

    // Should load Phase 2 format without modification
    assert!(settings.workspace_config.is_some());
    assert_eq!(settings.config_version, 2);
    assert!(settings.workspace_dir.is_none());
}
