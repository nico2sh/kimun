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

    // Verify migration marker — Phase 1 migrates straight through to the
    // current version (v3 today).
    let config_content = std::fs::read_to_string(&config_path).unwrap();
    assert!(config_content.contains("config_version = 3"));
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

    // Theme stays at the top level, not in GlobalConfig.
    assert_eq!(settings.theme, "gruvbox_dark");

    // v1 → v2 migration moves last_paths into the workspace entry; the
    // subsequent v2 → v3 migration extracts them into a history file and
    // clears the in-memory copy. Verify both: the file exists with the
    // expected content, and the in-memory entry is empty.
    let ws_config = settings.workspace_config.as_ref().unwrap();
    let default_ws = ws_config.workspaces.get("default").unwrap();
    assert!(default_ws.last_paths.is_empty());

    let hist_path = temp_dir
        .path()
        .canonicalize()
        .unwrap()
        .join("history")
        .join("default.txt");
    let body = std::fs::read_to_string(&hist_path).unwrap();
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines, vec!["/journal", "/tasks"]);
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
fn current_version_config_loads_without_migration() {
    let temp_dir = TempDir::new().unwrap();
    let workspace_dir = temp_dir.path().join("notes");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    let config_path = temp_dir.path().join("config.toml");

    // Write a v3 config directly — already at the current version, so no
    // migration should run.
    let v3_toml = format!(
        r#"
config_version = 3

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
    std::fs::write(&config_path, &v3_toml).unwrap();

    let settings = AppSettings::load_from_file(config_path.clone()).unwrap();

    assert!(settings.workspace_config.is_some());
    assert_eq!(settings.config_version, 3);
    assert!(settings.workspace_dir.is_none());
}

#[test]
fn v2_to_v3_moves_db_and_extracts_history() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let workspace_dir = tempfile::TempDir::new().unwrap();
    let old_db = workspace_dir.path().join("kimun.sqlite");
    std::fs::write(&old_db, b"fake sqlite contents").unwrap();

    std::fs::write(
        &cfg_path,
        format!(
            r#"
config_version = 2
cache_dir = "."
history_dir = "history"
theme = "gruvbox_dark"

[global]
current_workspace = "notes"

[workspaces.notes]
path = "{}"
last_paths = ["a.md", "b.md", "c.md"]
created = "2026-01-01T00:00:00Z"
"#,
            workspace_dir.path().display()
        ),
    )
    .unwrap();

    let settings = kimun_notes::settings::AppSettings::load_from_file(cfg_path.clone()).unwrap();

    assert_eq!(settings.config_version, 3);
    let new_db = tmp.path().canonicalize().unwrap().join("notes.kimuncache");
    assert!(
        new_db.exists(),
        "DB should be moved to cache dir, looked at {new_db:?}"
    );
    assert!(
        !old_db.exists(),
        "Old DB should no longer exist at workspace"
    );

    let hist_path = tmp
        .path()
        .canonicalize()
        .unwrap()
        .join("history")
        .join("notes.txt");
    assert!(
        hist_path.exists(),
        "history file should be written at {hist_path:?}"
    );
    let body = std::fs::read_to_string(&hist_path).unwrap();
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines, vec!["a.md", "b.md", "c.md"]);
}

#[test]
fn v2_to_v3_aborts_on_invalid_workspace_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let workspace_dir = tempfile::TempDir::new().unwrap();

    std::fs::write(
        &cfg_path,
        format!(
            r#"
config_version = 2
cache_dir = "."
history_dir = "history"
theme = "gruvbox_dark"

[global]
current_workspace = "bad/name"

[workspaces."bad/name"]
path = "{}"
last_paths = []
created = "2026-01-01T00:00:00Z"
"#,
            workspace_dir.path().display()
        ),
    )
    .unwrap();

    let result = kimun_notes::settings::AppSettings::load_from_file(cfg_path.clone());
    let err = result.unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("bad/name"),
        "error should name the bad workspace: {msg}"
    );
}

#[test]
fn v2_to_v3_is_idempotent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let workspace_dir = tempfile::TempDir::new().unwrap();
    std::fs::write(workspace_dir.path().join("kimun.sqlite"), b"fake").unwrap();

    std::fs::write(
        &cfg_path,
        format!(
            r#"
config_version = 2
cache_dir = "."
history_dir = "history"
theme = "gruvbox_dark"

[global]
current_workspace = "notes"

[workspaces.notes]
path = "{}"
last_paths = ["x.md"]
created = "2026-01-01T00:00:00Z"
"#,
            workspace_dir.path().display()
        ),
    )
    .unwrap();

    let _ = kimun_notes::settings::AppSettings::load_from_file(cfg_path.clone()).unwrap();
    let s = kimun_notes::settings::AppSettings::load_from_file(cfg_path.clone()).unwrap();
    assert_eq!(s.config_version, 3);
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
path = "{}"
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

#[test]
fn v2_to_v3_creates_config_backup() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let workspace_dir = tempfile::TempDir::new().unwrap();
    let original = format!(
        r#"
config_version = 2
cache_dir = "."
history_dir = "history"
theme = "gruvbox_dark"

[global]
current_workspace = "notes"

[workspaces.notes]
path = "{}"
last_paths = []
created = "2026-01-01T00:00:00Z"
"#,
        workspace_dir.path().display()
    );
    std::fs::write(&cfg_path, &original).unwrap();

    let _ = kimun_notes::settings::AppSettings::load_from_file(cfg_path.clone()).unwrap();

    let bak = cfg_path.with_extension("toml.bak.v2");
    assert!(bak.exists(), "backup file should exist at {bak:?}");
    let backed_up = std::fs::read_to_string(&bak).unwrap();
    assert_eq!(backed_up, original);
}
