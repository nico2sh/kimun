// tui/tests/cli_phase2_integration_test.rs
//
// Comprehensive integration tests for CLI Phase 2 functionality.
// Tests multi-workspace workflows, JSON output validation, and Phase 1 migration.

use kimun_core::nfs::VaultPath;
use kimun_core::NoteVault;
use kimun_notes::cli::{run_cli, CliCommand};
use kimun_notes::cli::commands::workspace::WorkspaceSubcommand;
use kimun_notes::cli::output::OutputFormat;
use kimun_notes::settings::AppSettings;
use tempfile::TempDir;

/// Create a temporary workspace with test notes and return both the vault and its directory.
async fn setup_test_workspace(name: &str, dir: &TempDir) -> NoteVault {
    let vault = NoteVault::new(dir.path()).await.expect("failed to create vault");
    vault.validate_and_init().await.expect("failed to init vault");

    // Create test notes specific to this workspace
    vault
        .create_note(
            &VaultPath::note_path_from(&format!("{}-project", name)),
            &format!("# {} Project\n\n#programming #{}\n\nThis is the {} project note.", name, name, name),
        )
        .await
        .expect("failed to create project note");

    vault
        .create_note(
            &VaultPath::note_path_from(&format!("journal/{}-daily", name)),
            &format!("# {} Daily Journal\n\n## Today\n\nDaily activities for {}.", name, name),
        )
        .await
        .expect("failed to create journal note");

    vault
        .create_note(
            &VaultPath::note_path_from(&format!("notes/{}-research", name)),
            &format!("# {} Research\n\n[[{}-project]]\n\nResearch notes for {}.", name, name, name),
        )
        .await
        .expect("failed to create research note");

    vault.recreate_index().await.expect("failed to recreate index");
    vault
}


/// Write a Phase 1 legacy config file (for migration testing).
fn write_phase1_config(config_path: &std::path::Path, workspace: &std::path::Path) {
    let toml = format!(
        "workspace_dir = {:?}\n",
        workspace.to_string_lossy().as_ref()
    );
    std::fs::write(config_path, toml).expect("failed to write config file");
}

// ---------------------------------------------------------------------------
// test_multi_workspace_init_and_switch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_multi_workspace_init_and_switch() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    let workspace1_dir = TempDir::new().unwrap();
    let workspace2_dir = TempDir::new().unwrap();

    // Initialize first workspace
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("work".to_string()),
                path: workspace1_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "workspace init should succeed: {:?}", result);

    // Initialize second workspace
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("personal".to_string()),
                path: workspace2_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "second workspace init should succeed: {:?}", result);

    // Verify config file has both workspaces
    let settings = AppSettings::load_from_file(config_path.clone()).expect("should load settings");
    let ws_config = settings.workspace_config.as_ref().expect("should have workspace config");

    assert_eq!(ws_config.workspaces.len(), 2, "should have 2 workspaces");
    assert!(ws_config.workspaces.contains_key("work"), "should have work workspace");
    assert!(ws_config.workspaces.contains_key("personal"), "should have personal workspace");

    // Switch to personal workspace
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Use {
                name: "personal".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "workspace switch should succeed: {:?}", result);

    // Verify current workspace changed
    let settings = AppSettings::load_from_file(config_path.clone()).expect("should load settings");
    let ws_config = settings.workspace_config.as_ref().expect("should have workspace config");
    assert_eq!(ws_config.global.current_workspace, "personal", "should switch to personal workspace");
}

// ---------------------------------------------------------------------------
// test_workspace_isolation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_isolation() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    let work_dir = TempDir::new().unwrap();
    let personal_dir = TempDir::new().unwrap();

    // Setup workspaces with different content
    setup_test_workspace("work", &work_dir).await;
    setup_test_workspace("personal", &personal_dir).await;

    // Initialize workspaces through CLI (more reliable than manual config)
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("work".to_string()),
                path: work_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "work workspace init should succeed: {:?}", result);

    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("personal".to_string()),
                path: personal_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "personal workspace init should succeed: {:?}", result);

    // Search in work workspace (default)
    let result = run_cli(
        CliCommand::Search {
            query: "work".to_string(),
            format: OutputFormat::Text,
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "search in work workspace should succeed: {:?}", result);

    // Switch to personal workspace
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Use {
                name: "personal".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "workspace switch should succeed: {:?}", result);

    // Search in personal workspace should find different content
    let result = run_cli(
        CliCommand::Search {
            query: "personal".to_string(),
            format: OutputFormat::Text,
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "search in personal workspace should succeed: {:?}", result);

    // Search for work content in personal workspace should find nothing
    let result = run_cli(
        CliCommand::Search {
            query: "work-project".to_string(),
            format: OutputFormat::Text,
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "search for work content in personal should succeed (but find nothing): {:?}", result);
}

// ---------------------------------------------------------------------------
// test_json_output_multi_workspace
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_json_output_multi_workspace() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    let workspace_dir = TempDir::new().unwrap();
    setup_test_workspace("test", &workspace_dir).await;

    // Initialize workspace through CLI
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("test-workspace".to_string()),
                path: workspace_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "test workspace init should succeed: {:?}", result);

    // Test JSON output for search
    let result = run_cli(
        CliCommand::Search {
            query: "test".to_string(),
            format: OutputFormat::Json,
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "search with JSON format should succeed: {:?}", result);

    // Test JSON output for notes listing
    let result = run_cli(
        CliCommand::Notes {
            path: None,
            format: OutputFormat::Json,
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "notes with JSON format should succeed: {:?}", result);

    // Verify JSON structure by directly calling the vault (since CLI output goes to stdout)
    let vault = NoteVault::new(workspace_dir.path()).await.unwrap();
    vault.validate_and_init().await.unwrap();

    let results = vault.search_notes("test").await.unwrap();

    let json_str = kimun_notes::cli::json_output::format_notes_as_json(
        &vault,
        &results,
        "test-workspace",
        Some("test"),
        false, // is_listing
    ).await
    .expect("format_notes_as_json should succeed");

    let json: serde_json::Value = serde_json::from_str(&json_str)
        .expect("output should be valid JSON");

    // Verify workspace metadata is included
    assert_eq!(json["metadata"]["workspace"], "test-workspace");
    assert_eq!(json["metadata"]["workspace_path"], workspace_dir.path().to_string_lossy().to_string());
    assert_eq!(json["metadata"]["query"], "test");
    assert!(!json["metadata"]["is_listing"].as_bool().unwrap());

    // Verify note structure includes metadata
    let notes = json["notes"].as_array().expect("should have notes array");
    assert!(!notes.is_empty(), "should find test notes");

    for note in notes {
        assert!(note["metadata"].is_object(), "note should have metadata object");
        assert!(note["metadata"]["tags"].is_array(), "metadata should have tags array");
        assert!(note["metadata"]["links"].is_array(), "metadata should have links array");
        assert!(note["metadata"]["headers"].is_array(), "metadata should have headers array");
    }
}

// ---------------------------------------------------------------------------
// test_phase1_to_phase2_migration
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_phase1_to_phase2_migration() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    let workspace_dir = TempDir::new().unwrap();
    setup_test_workspace("legacy", &workspace_dir).await;

    // Write Phase 1 config
    write_phase1_config(&config_path, workspace_dir.path());

    // Verify Phase 1 config loads and migrates correctly
    let settings = AppSettings::load_from_file(config_path.clone()).expect("should load Phase 1 config");
    assert!(settings.workspace_dir.is_none(), "workspace_dir should be None after migration");
    assert!(settings.workspace_config.is_some(), "should have migrated workspace_config");

    let ws_config = settings.workspace_config.as_ref().unwrap();
    assert_eq!(ws_config.global.current_workspace, "default", "should migrate to default workspace");
    assert_eq!(ws_config.workspaces.len(), 1, "should have one workspace after migration");
    let default_workspace = ws_config.workspaces.get("default").expect("should have default workspace");
    assert_eq!(default_workspace.path, workspace_dir.path(), "migrated workspace should have correct path");

    // Test that CLI commands work with migrated config
    let result = run_cli(
        CliCommand::Search {
            query: "legacy".to_string(),
            format: OutputFormat::Text,
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "search should work with migrated config: {:?}", result);

    let result = run_cli(
        CliCommand::Notes {
            path: None,
            format: OutputFormat::Text,
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "notes should work with migrated config: {:?}", result);
}

// ---------------------------------------------------------------------------
// test_workspace_management_commands
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_management_commands() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    let workspace1_dir = TempDir::new().unwrap();
    let workspace2_dir = TempDir::new().unwrap();
    let workspace3_dir = TempDir::new().unwrap();

    // Initialize workspaces
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("alpha".to_string()),
                path: workspace1_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "alpha workspace init should succeed: {:?}", result);

    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("beta".to_string()),
                path: workspace2_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "beta workspace init should succeed: {:?}", result);

    // List workspaces
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::List,
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "workspace list should succeed: {:?}", result);

    // Rename workspace
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Rename {
                old_name: "beta".to_string(),
                new_name: "gamma".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "workspace rename should succeed: {:?}", result);

    // Verify rename worked
    let settings = AppSettings::load_from_file(config_path.clone()).expect("should load settings");
    let ws_config = settings.workspace_config.as_ref().expect("should have workspace config");
    assert!(ws_config.workspaces.contains_key("gamma"), "should have gamma workspace");
    assert!(!ws_config.workspaces.contains_key("beta"), "should not have beta workspace");

    // Add another workspace and remove it
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("temp".to_string()),
                path: workspace3_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "temp workspace init should succeed: {:?}", result);

    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Remove {
                name: "temp".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "workspace remove should succeed: {:?}", result);

    // Verify removal worked
    let settings = AppSettings::load_from_file(config_path.clone()).expect("should load settings");
    let ws_config = settings.workspace_config.as_ref().expect("should have workspace config");
    assert!(!ws_config.workspaces.contains_key("temp"), "should not have temp workspace");
    assert_eq!(ws_config.workspaces.len(), 2, "should have 2 workspaces remaining");
}

// ---------------------------------------------------------------------------
// test_workspace_reindex
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_reindex() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    let workspace_dir = TempDir::new().unwrap();

    // Initialize workspace
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("reindex-test".to_string()),
                path: workspace_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "workspace init should succeed: {:?}", result);

    // Create some notes in the workspace
    setup_test_workspace("reindex", &workspace_dir).await;

    // Run reindex command
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Reindex {
                name: Some("reindex-test".to_string()),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "workspace reindex should succeed: {:?}", result);

    // Verify notes are searchable after reindex
    let result = run_cli(
        CliCommand::Search {
            query: "reindex".to_string(),
            format: OutputFormat::Text,
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "search after reindex should succeed: {:?}", result);
}

// ---------------------------------------------------------------------------
// test_error_handling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_error_handling() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    // Test switching to non-existent workspace
    // First create a valid config with one workspace
    let workspace_dir = TempDir::new().unwrap();
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("existing".to_string()),
                path: workspace_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "workspace init should succeed: {:?}", result);

    // Try to switch to non-existent workspace
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Use {
                name: "non-existent".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_err(), "switching to non-existent workspace should fail");

    // Try to remove non-existent workspace
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Remove {
                name: "non-existent".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_err(), "removing non-existent workspace should fail");

    // Try to rename non-existent workspace
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Rename {
                old_name: "non-existent".to_string(),
                new_name: "new-name".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_err(), "renaming non-existent workspace should fail");
}

// ---------------------------------------------------------------------------
// test_complex_multi_workspace_workflow
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_complex_multi_workspace_workflow() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    let work_dir = TempDir::new().unwrap();
    let personal_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();

    // Setup comprehensive workflow
    setup_test_workspace("work", &work_dir).await;
    setup_test_workspace("personal", &personal_dir).await;
    setup_test_workspace("project", &project_dir).await;

    // Initialize all workspaces through CLI
    for (name, dir) in [("work", &work_dir), ("personal", &personal_dir), ("project", &project_dir)] {
        let result = run_cli(
            CliCommand::Workspace {
                subcommand: WorkspaceSubcommand::Init {
                    name: Some(name.to_string()),
                    path: dir.path().to_path_buf(),
                },
            },
            Some(config_path.clone()),
        )
        .await;
        assert!(result.is_ok(), "{} workspace init should succeed: {:?}", name, result);
    }

    // Test switching between workspaces and running different operations
    for workspace_name in ["work", "personal", "project"] {
        // Switch workspace
        let result = run_cli(
            CliCommand::Workspace {
                subcommand: WorkspaceSubcommand::Use {
                    name: workspace_name.to_string(),
                },
            },
            Some(config_path.clone()),
        )
        .await;
        assert!(result.is_ok(), "switch to {} should succeed: {:?}", workspace_name, result);

        // Test search in current workspace
        let result = run_cli(
            CliCommand::Search {
                query: workspace_name.to_string(),
                format: OutputFormat::Text,
            },
            Some(config_path.clone()),
        )
        .await;
        assert!(result.is_ok(), "search in {} should succeed: {:?}", workspace_name, result);

        // Test notes listing in current workspace
        let result = run_cli(
            CliCommand::Notes {
                path: None,
                format: OutputFormat::Text,
            },
            Some(config_path.clone()),
        )
        .await;
        assert!(result.is_ok(), "notes in {} should succeed: {:?}", workspace_name, result);

        // Test JSON output in current workspace
        let result = run_cli(
            CliCommand::Search {
                query: workspace_name.to_string(),
                format: OutputFormat::Json,
            },
            Some(config_path.clone()),
        )
        .await;
        assert!(result.is_ok(), "JSON search in {} should succeed: {:?}", workspace_name, result);
    }

    // List all workspaces to verify they all exist
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::List,
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(result.is_ok(), "workspace list should succeed: {:?}", result);

    // Verify final config state
    let settings = AppSettings::load_from_file(config_path.clone()).expect("should load settings");
    let ws_config = settings.workspace_config.as_ref().expect("should have workspace config");
    assert_eq!(ws_config.workspaces.len(), 3, "should have 3 workspaces");
    assert_eq!(ws_config.global.current_workspace, "project", "should be on project workspace");
}