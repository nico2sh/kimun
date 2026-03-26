use kimun_core::nfs::VaultPath;
use kimun_core::NoteVault;
use kimun_notes::cli::{run_cli, CliCommand};
use kimun_notes::cli::output::OutputFormat;
use kimun_notes::settings::AppSettings;
use tempfile::TempDir;

/// Create a temporary vault with test notes indexed.
async fn setup_test_vault(dir: &TempDir) -> NoteVault {
    let vault = NoteVault::new(dir.path()).await.expect("failed to create vault");

    // Initialize DB schema before creating notes
    vault.init_and_validate().await.expect("failed to init vault");

    // Create a couple of test notes
    vault
        .create_note(
            &VaultPath::note_path_from("hello"),
            "# Hello World\n\nThis is a hello note.",
        )
        .await
        .expect("failed to create hello note");

    vault
        .create_note(
            &VaultPath::note_path_from("sub/nested"),
            "# Nested Note\n\nThis note lives in a subdirectory.",
        )
        .await
        .expect("failed to create nested note");

    // Index so searches and listings work
    vault.recreate_index().await.expect("failed to recreate index");

    vault
}

/// Write a minimal config file that points workspace at the given path.
fn write_config(config_path: &std::path::Path, workspace: &std::path::Path) {
    let toml = format!(
        "workspace_dir = {:?}\n",
        workspace.to_string_lossy().as_ref()
    );
    std::fs::write(config_path, toml).expect("failed to write config file");
}

// ---------------------------------------------------------------------------
// test_cli_search_command
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_cli_search_command() {
    let workspace_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    // Pre-create and index notes
    setup_test_vault(&workspace_dir).await;

    // Write config pointing to the workspace
    write_config(&config_path, workspace_dir.path());

    // Run the search command via run_cli
    let result = run_cli(
        CliCommand::Search {
            query: "hello".to_string(),
            format: OutputFormat::Text,
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok(), "search command should succeed: {:?}", result);
}

// ---------------------------------------------------------------------------
// test_cli_notes_command
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_cli_notes_command() {
    let workspace_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    setup_test_vault(&workspace_dir).await;
    write_config(&config_path, workspace_dir.path());

    // List all notes (no path filter)
    let result = run_cli(
        CliCommand::Notes {
            path: None,
            format: OutputFormat::Text,
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(result.is_ok(), "notes command (no filter) should succeed: {:?}", result);

    // List notes with path filter
    let result_filtered = run_cli(
        CliCommand::Notes {
            path: Some("sub/".to_string()),
            format: OutputFormat::Text,
        },
        Some(config_path),
    )
    .await;

    assert!(
        result_filtered.is_ok(),
        "notes command (with path filter) should succeed: {:?}",
        result_filtered
    );
}

// ---------------------------------------------------------------------------
// test_cli_no_workspace_error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_cli_no_workspace_error() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    // Write a config with no workspace_dir set
    std::fs::write(&config_path, "# empty config\n").unwrap();

    // The CLI exits the process when no workspace is configured; we verify
    // the settings layer itself returns None for workspace_dir so the CLI
    // would hit the error branch.
    let settings = AppSettings::load_from_file(config_path).expect("settings should load");
    assert!(
        settings.workspace_dir.is_none(),
        "workspace_dir should be None when not set in config"
    );
}

// ---------------------------------------------------------------------------
// test_cli_custom_config
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_cli_custom_config() {
    let workspace_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("custom_config.toml");

    setup_test_vault(&workspace_dir).await;
    write_config(&config_path, workspace_dir.path());

    // Verify the config is honoured: settings loaded from the custom path
    // should point to our temp workspace.
    let settings =
        AppSettings::load_from_file(config_path.clone()).expect("settings should load");
    assert_eq!(
        settings.workspace_dir.as_deref(),
        Some(workspace_dir.path()),
        "--config flag should load settings from the specified file"
    );

    // Also confirm run_cli works end-to-end with the custom config path.
    let result = run_cli(
        CliCommand::Notes {
            path: None,
            format: OutputFormat::Text,
        },
        Some(config_path),
    )
    .await;

    assert!(
        result.is_ok(),
        "notes command with custom config should succeed: {:?}",
        result
    );
}
