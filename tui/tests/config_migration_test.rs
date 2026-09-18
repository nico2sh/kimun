use kimun_notes::settings::AppSettings;
use kimun_notes::settings::config_migration::CURRENT_CONFIG_VERSION;
use tempfile::TempDir;

mod common;

#[test]
fn current_version_config_loads_without_migration() {
    let temp_dir = TempDir::new().unwrap();
    let workspace_dir = temp_dir.path().join("notes");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    let config_path = temp_dir.path().join("config.toml");

    // Write a config already at the current version, so no migration
    // should run. Sourced from the constant rather than a literal: a config
    // "at the current version" is whatever that constant says today, and
    // hardcoding it made this test fail on the v7 bump for no reason of its
    // own.
    let v3_toml = format!(
        r#"
config_version = {CURRENT_CONFIG_VERSION}

[global]
current_workspace = "default"
theme = "dark"

[workspaces.default]
path = '{}'
last_paths = []
created = "2024-01-15T10:30:00Z"
"#,
        workspace_dir.display()
    );
    std::fs::write(&config_path, &v3_toml).unwrap();

    let settings = AppSettings::load_from_file(config_path.clone()).unwrap();

    assert!(settings.workspace_config.is_some());
    assert_eq!(settings.config_version, CURRENT_CONFIG_VERSION);
}

#[test]
fn v3_save_does_not_write_last_paths() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let workspace_dir = tempfile::TempDir::new().unwrap();

    std::fs::write(
        &cfg_path,
        format!(
            r#"
config_version = 3
cache_dir = "."
history_dir = "history"
theme = "gruvbox_dark"

[global]
current_workspace = "notes"

[workspaces.notes]
path = '{}'
created = "2026-01-01T00:00:00Z"
"#,
            workspace_dir.path().display()
        ),
    )
    .unwrap();

    let mut settings =
        kimun_notes::settings::AppSettings::load_from_file(cfg_path.clone()).unwrap();
    if let Some(wc) = settings.workspace_config.as_mut() {
        wc.workspaces
            .get_mut("notes")
            .unwrap()
            .last_paths
            .push("ghost.md".into());
    }
    settings.save_to_disk().unwrap();
    let raw = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(
        !raw.contains("last_paths"),
        "v3 config should never write last_paths, got:\n{raw}"
    );
}
