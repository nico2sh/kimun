// tui/tests/workspace_commands_test.rs
//
// Integration tests for workspace management CLI commands.
// These tests follow the TDD approach: written before implementation.

use kimun_notes::cli::{run_cli, CliCommand};
use kimun_notes::cli::commands::WorkspaceSubcommand;
use kimun_notes::settings::AppSettings;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// workspace init tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_init_creates_new_workspace() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();

    // Start with empty config
    std::fs::write(&config_path, "# empty config\n").unwrap();

    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("myworkspace".to_string()),
                path: workspace_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(result.is_ok(), "workspace init should succeed: {:?}", result);

    // Verify the workspace was added to the config
    let settings = AppSettings::load_from_file(config_path).expect("settings should load");
    let ws_config = settings
        .workspace_config
        .as_ref()
        .expect("workspace_config should be set after init");

    assert!(
        ws_config.workspaces.contains_key("myworkspace"),
        "workspace 'myworkspace' should be in config; workspaces: {:?}",
        ws_config.workspaces.keys().collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn test_workspace_init_first_workspace_defaults_to_default_name() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();

    // Empty config, no workspaces yet
    std::fs::write(&config_path, "# empty config\n").unwrap();

    // Init without a name — should use "default" since no workspaces exist
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: None,
                path: workspace_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(
        result.is_ok(),
        "workspace init without name (first workspace) should succeed: {:?}",
        result
    );

    let settings = AppSettings::load_from_file(config_path).unwrap();
    let ws_config = settings.workspace_config.as_ref().unwrap();
    assert!(
        ws_config.workspaces.contains_key("default"),
        "should default to 'default' name for first workspace"
    );
}

#[tokio::test]
async fn test_workspace_init_duplicate_name_fails() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir1 = TempDir::new().unwrap();
    let workspace_dir2 = TempDir::new().unwrap();

    // Add first workspace
    std::fs::write(&config_path, "# empty config\n").unwrap();
    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("myworkspace".to_string()),
                path: workspace_dir1.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("first init should succeed");

    // Try to add another workspace with the same name
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("myworkspace".to_string()),
                path: workspace_dir2.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(
        result.is_err(),
        "workspace init with duplicate name should fail"
    );
}

// ---------------------------------------------------------------------------
// workspace list tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_list_empty() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    std::fs::write(&config_path, "# empty config\n").unwrap();

    // list should succeed even when no workspaces configured
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::List,
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok(), "workspace list on empty config should succeed: {:?}", result);
}

#[tokio::test]
async fn test_workspace_list_shows_workspaces() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();

    std::fs::write(&config_path, "# empty config\n").unwrap();

    // Add a workspace first
    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("work".to_string()),
                path: workspace_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init should succeed");

    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::List,
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok(), "workspace list should succeed: {:?}", result);
}

// ---------------------------------------------------------------------------
// workspace use tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_use_switches_current() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let dir1 = TempDir::new().unwrap();
    let dir2 = TempDir::new().unwrap();

    std::fs::write(&config_path, "# empty config\n").unwrap();

    // Create two workspaces
    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("ws1".to_string()),
                path: dir1.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init ws1 should succeed");

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("ws2".to_string()),
                path: dir2.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init ws2 should succeed");

    // Switch to ws2
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Use {
                name: "ws2".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(result.is_ok(), "workspace use should succeed: {:?}", result);

    let settings = AppSettings::load_from_file(config_path).unwrap();
    let ws_config = settings.workspace_config.as_ref().unwrap();
    assert_eq!(
        ws_config.global.current_workspace, "ws2",
        "current workspace should be 'ws2'"
    );
}

#[tokio::test]
async fn test_workspace_use_nonexistent_fails() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let dir1 = TempDir::new().unwrap();

    std::fs::write(&config_path, "# empty config\n").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("ws1".to_string()),
                path: dir1.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init should succeed");

    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Use {
                name: "nonexistent".to_string(),
            },
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_err(), "workspace use with nonexistent name should fail");
}

// ---------------------------------------------------------------------------
// workspace rename tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_rename_succeeds() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let dir1 = TempDir::new().unwrap();

    std::fs::write(&config_path, "# empty config\n").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("oldname".to_string()),
                path: dir1.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init should succeed");

    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Rename {
                old_name: "oldname".to_string(),
                new_name: "newname".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(result.is_ok(), "workspace rename should succeed: {:?}", result);

    let settings = AppSettings::load_from_file(config_path).unwrap();
    let ws_config = settings.workspace_config.as_ref().unwrap();
    assert!(
        ws_config.workspaces.contains_key("newname"),
        "renamed workspace should exist under new name"
    );
    assert!(
        !ws_config.workspaces.contains_key("oldname"),
        "old workspace name should no longer exist"
    );
}

// ---------------------------------------------------------------------------
// workspace remove tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_remove_succeeds() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let dir1 = TempDir::new().unwrap();
    let dir2 = TempDir::new().unwrap();

    std::fs::write(&config_path, "# empty config\n").unwrap();

    // Create two workspaces (ws1 is current, ws2 is the extra)
    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("ws1".to_string()),
                path: dir1.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init ws1 should succeed");

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("ws2".to_string()),
                path: dir2.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init ws2 should succeed");

    // Remove ws2 (not current)
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Remove {
                name: "ws2".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(result.is_ok(), "workspace remove should succeed: {:?}", result);

    let settings = AppSettings::load_from_file(config_path).unwrap();
    let ws_config = settings.workspace_config.as_ref().unwrap();
    assert!(
        !ws_config.workspaces.contains_key("ws2"),
        "removed workspace should no longer exist"
    );
}

#[tokio::test]
async fn test_workspace_remove_current_fails() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let dir1 = TempDir::new().unwrap();

    std::fs::write(&config_path, "# empty config\n").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("ws1".to_string()),
                path: dir1.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init should succeed");

    // Try to remove the current workspace
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Remove {
                name: "ws1".to_string(),
            },
        },
        Some(config_path),
    )
    .await;

    assert!(
        result.is_err(),
        "removing the current workspace should fail with helpful error"
    );
}

// ---------------------------------------------------------------------------
// workspace reindex tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_reindex_succeeds() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();

    std::fs::write(&config_path, "# empty config\n").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("myws".to_string()),
                path: workspace_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init should succeed");

    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Reindex {
                name: None, // use current workspace
            },
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok(), "workspace reindex should succeed: {:?}", result);
}
