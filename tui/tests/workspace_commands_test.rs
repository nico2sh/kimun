// tui/tests/workspace_commands_test.rs
//
// Integration tests for workspace management CLI commands.
// These tests follow the TDD approach: written before implementation.

use kimun_notes::cli::commands::WorkspaceSubcommand;
use kimun_notes::cli::{CliCommand, run_cli};
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

    assert!(
        result.is_ok(),
        "workspace init should succeed: {:?}",
        result
    );

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

/// An invalid name must be rejected before it is used to build a path.
///
/// The name becomes `<cache_dir>/<name>.kimuncache`, so a `..` or a separator
/// in it aims that file — and its `-wal`/`-shm` sidecars, and any parent
/// directories — outside the cache directory. Validating only when the entry
/// reaches the config (as `add_workspace` does) is too late: the database has
/// been created and indexed by then, and the command aborts leaving it behind.
///
/// The config lives one level down so an escaping name lands inside the
/// tempdir, where the assertions can see it, rather than in the system temp
/// directory.
#[tokio::test]
async fn test_workspace_init_rejects_names_that_escape_the_cache_dir() {
    for name in ["../escape", "sub/escape", "con"] {
        let config_dir = TempDir::new().unwrap();
        let cache_dir = config_dir.path().join("nested");
        std::fs::create_dir(&cache_dir).unwrap();
        let config_path = cache_dir.join("config.toml");
        std::fs::write(&config_path, "# empty config\n").unwrap();
        let workspace_dir = TempDir::new().unwrap();

        let result = run_cli(
            CliCommand::Workspace {
                subcommand: WorkspaceSubcommand::Init {
                    name: Some(name.to_string()),
                    path: workspace_dir.path().to_path_buf(),
                },
            },
            Some(config_path.clone()),
        )
        .await;

        assert!(result.is_err(), "init with name '{name}' should fail");

        let stray: Vec<_> = std::fs::read_dir(config_dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != "nested")
            .collect();
        assert!(
            stray.is_empty(),
            "init with name '{name}' wrote outside the cache dir: {stray:?}"
        );
        let inside: Vec<_> = std::fs::read_dir(&cache_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != "config.toml")
            .collect();
        assert!(
            inside.is_empty(),
            "init with name '{name}' left files in the cache dir: {inside:?}"
        );
    }
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

    assert!(
        result.is_ok(),
        "workspace list on empty config should succeed: {:?}",
        result
    );
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

    assert!(
        result.is_ok(),
        "workspace list should succeed: {:?}",
        result
    );
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

    assert!(
        result.is_err(),
        "workspace use with nonexistent name should fail"
    );
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

    assert!(
        result.is_ok(),
        "workspace rename should succeed: {:?}",
        result
    );

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

/// An index is three files when it is open or was not closed cleanly: the
/// `.kimuncache` plus SQLite's `-wal` and `-shm`. Renaming only the first
/// leaves the WAL behind — the renamed index then silently lacks whatever
/// transactions the WAL still held, and the orphans sit in the cache
/// directory forever.
#[tokio::test]
async fn test_workspace_rename_moves_the_index_sidecars() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    std::fs::write(&config_path, "# empty config\n").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("oldname".to_string()),
                path: workspace_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init should succeed");

    // Stand in for an index left open by a crashed process.
    let cache_dir = config_dir.path().canonicalize().unwrap();
    let old_wal = cache_dir.join("oldname.kimuncache-wal");
    let old_shm = cache_dir.join("oldname.kimuncache-shm");
    std::fs::write(&old_wal, b"wal").unwrap();
    std::fs::write(&old_shm, b"shm").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Rename {
                old_name: "oldname".to_string(),
                new_name: "newname".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("rename should succeed");

    assert!(cache_dir.join("newname.kimuncache").exists());
    assert_eq!(
        std::fs::read(cache_dir.join("newname.kimuncache-wal")).unwrap(),
        b"wal",
        "the WAL must travel with the index"
    );
    assert_eq!(
        std::fs::read(cache_dir.join("newname.kimuncache-shm")).unwrap(),
        b"shm",
        "the shared-memory file must travel with the index"
    );
    assert!(!old_wal.exists(), "orphaned WAL left behind");
    assert!(!old_shm.exists(), "orphaned shm left behind");
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

    assert!(
        result.is_ok(),
        "workspace remove should succeed: {:?}",
        result
    );

    let settings = AppSettings::load_from_file(config_path).unwrap();
    let ws_config = settings.workspace_config.as_ref().unwrap();
    assert!(
        !ws_config.workspaces.contains_key("ws2"),
        "removed workspace should no longer exist"
    );
}

/// Removing a workspace has to take the whole index with it, sidecars
/// included — otherwise the cache directory accumulates `-wal`/`-shm` files
/// belonging to workspaces that no longer exist.
#[tokio::test]
async fn test_workspace_remove_deletes_the_index_sidecars() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let keep_dir = TempDir::new().unwrap();
    let doomed_dir = TempDir::new().unwrap();
    std::fs::write(&config_path, "# empty config\n").unwrap();

    for (name, dir) in [("keep", &keep_dir), ("doomed", &doomed_dir)] {
        run_cli(
            CliCommand::Workspace {
                subcommand: WorkspaceSubcommand::Init {
                    name: Some(name.to_string()),
                    path: dir.path().to_path_buf(),
                },
            },
            Some(config_path.clone()),
        )
        .await
        .unwrap_or_else(|e| panic!("init of '{name}' should succeed: {e:?}"));
    }

    let cache_dir = config_dir.path().canonicalize().unwrap();
    let wal = cache_dir.join("doomed.kimuncache-wal");
    let shm = cache_dir.join("doomed.kimuncache-shm");
    std::fs::write(&wal, b"wal").unwrap();
    std::fs::write(&shm, b"shm").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Remove {
                name: "doomed".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("remove should succeed");

    assert!(!cache_dir.join("doomed.kimuncache").exists());
    assert!(!wal.exists(), "orphaned WAL left behind");
    assert!(!shm.exists(), "orphaned shm left behind");
    assert!(
        cache_dir.join("keep.kimuncache").exists(),
        "the other workspace's index must be untouched"
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

    assert!(
        result.is_ok(),
        "workspace reindex should succeed: {:?}",
        result
    );
}
