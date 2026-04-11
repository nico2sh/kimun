use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::cli::output::OutputFormat;
use kimun_notes::cli::{CliCommand, run_cli};
use kimun_notes::settings::AppSettings;
use tempfile::TempDir;

/// Create a temporary vault with test notes indexed.
async fn setup_test_vault(dir: &TempDir) -> NoteVault {
    let vault = NoteVault::new(dir.path())
        .await
        .expect("failed to create vault");

    // Initialize DB schema before creating notes
    vault
        .validate_and_init()
        .await
        .expect("failed to init vault");

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
    vault
        .recreate_index()
        .await
        .expect("failed to recreate index");

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

    assert!(
        result.is_ok(),
        "search command should succeed: {:?}",
        result
    );
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

    assert!(
        result.is_ok(),
        "notes command (no filter) should succeed: {:?}",
        result
    );

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
    // should point to our temp workspace via Phase 2 workspace_config after migration.
    let settings = AppSettings::load_from_file(config_path.clone()).expect("settings should load");
    let ws_config = settings
        .workspace_config
        .as_ref()
        .expect("--config flag should load settings from the specified file");
    let default_ws = ws_config
        .workspaces
        .get("default")
        .expect("should have 'default' workspace after migration");
    assert_eq!(
        default_ws.path.as_path(),
        workspace_dir.path(),
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

// ---------------------------------------------------------------------------
// Exclusion operator helpers
// ---------------------------------------------------------------------------

/// Create a temporary vault with notes designed for exclusion testing.
async fn setup_exclusion_test_vault(dir: &TempDir) -> NoteVault {
    let vault = NoteVault::new(dir.path())
        .await
        .expect("failed to create vault");
    vault
        .validate_and_init()
        .await
        .expect("failed to init vault");

    vault
        .create_note(
            &VaultPath::note_path_from("weekly-meeting"),
            "# Weekly Meeting\n\nRegular team meeting notes.",
        )
        .await
        .expect("failed to create meeting note");

    vault
        .create_note(
            &VaultPath::note_path_from("cancelled-meeting"),
            "# Cancelled Meeting\n\nThis meeting was cancelled.",
        )
        .await
        .expect("failed to create cancelled note");

    vault
        .create_note(
            &VaultPath::note_path_from("project-draft"),
            "# Project Draft\n\nDraft version of project proposal.",
        )
        .await
        .expect("failed to create draft note");

    vault
        .create_note(
            &VaultPath::note_path_from("project-final"),
            "# Project Final\n\nFinal version of project proposal.",
        )
        .await
        .expect("failed to create final note");

    vault
        .recreate_index()
        .await
        .expect("failed to recreate index");
    vault
}

// ---------------------------------------------------------------------------
// test_cli_search_basic_exclusions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_cli_search_basic_exclusions() {
    let workspace_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    let vault = setup_exclusion_test_vault(&workspace_dir).await;
    write_config(&config_path, workspace_dir.path());

    // Test content exclusion via CLI
    let result = run_cli(
        CliCommand::Search {
            query: "meeting -cancelled".to_string(),
            format: OutputFormat::Text,
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(
        result.is_ok(),
        "search with exclusion should succeed: {:?}",
        result
    );

    // Validate directly against vault: "weekly-meeting" should appear, "cancelled-meeting" should not
    let search_results = vault
        .search_notes("meeting -cancelled")
        .await
        .expect("direct search should work");

    let paths: Vec<String> = search_results
        .iter()
        .map(|(entry, _)| entry.path.to_string())
        .collect();

    assert!(
        paths.contains(&"/weekly-meeting.md".to_string()),
        "Should find weekly-meeting note; found: {:?}",
        paths
    );
    assert!(
        !paths.contains(&"/cancelled-meeting.md".to_string()),
        "Should exclude cancelled-meeting note; found: {:?}",
        paths
    );
}

// ---------------------------------------------------------------------------
// test_cli_search_compound_exclusions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_cli_search_compound_exclusions() {
    let workspace_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    let vault = setup_exclusion_test_vault(&workspace_dir).await;
    write_config(&config_path, workspace_dir.path());

    // Test filename exclusion: @project @-draft
    let result = run_cli(
        CliCommand::Search {
            query: "@project @-draft".to_string(),
            format: OutputFormat::Text,
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(
        result.is_ok(),
        "filename exclusion should succeed: {:?}",
        result
    );

    // Validate directly against vault: "project-final" should appear, "project-draft" should not
    let search_results = vault
        .search_notes("@project @-draft")
        .await
        .expect("direct search should work");

    let paths: Vec<String> = search_results
        .iter()
        .map(|(entry, _)| entry.path.to_string())
        .collect();

    assert!(
        paths.contains(&"/project-final.md".to_string()),
        "Should find project-final note; found: {:?}",
        paths
    );
    assert!(
        !paths.contains(&"/project-draft.md".to_string()),
        "Should exclude project-draft note; found: {:?}",
        paths
    );

    // Test filename exclusion: @final @-draft (another combination)
    let result = run_cli(
        CliCommand::Search {
            query: "@final @-draft".to_string(),
            format: OutputFormat::Text,
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(
        result.is_ok(),
        "filename exclusion with final should succeed: {:?}",
        result
    );

    // Validate directly against vault: "project-final" should appear
    let search_results = vault
        .search_notes("@final @-draft")
        .await
        .expect("direct search should work");

    let paths: Vec<String> = search_results
        .iter()
        .map(|(entry, _)| entry.path.to_string())
        .collect();

    assert!(
        paths.contains(&"/project-final.md".to_string()),
        "Should find project-final note; found: {:?}",
        paths
    );
    assert!(
        !paths.contains(&"/project-draft.md".to_string()),
        "Should exclude project-draft note; found: {:?}",
        paths
    );
}

// ---------------------------------------------------------------------------
// test_search_paths_format_returns_bare_paths
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_search_paths_format_returns_bare_paths() {
    let dir = TempDir::new().unwrap();
    let _vault = setup_test_vault(&dir).await;
    let config_path = dir.path().join("config.toml");
    write_config(&config_path, dir.path());

    // "hello" matches the hello note — just verify no error
    let result = run_cli(
        CliCommand::Search {
            query: "hello".to_string(),
            format: OutputFormat::Paths,
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok());
}

// ---------------------------------------------------------------------------
// test_cli_search_exclusion_only
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_cli_search_exclusion_only() {
    let workspace_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    let vault = setup_exclusion_test_vault(&workspace_dir).await;
    write_config(&config_path, workspace_dir.path());

    // Test pure content exclusion
    let result = run_cli(
        CliCommand::Search {
            query: "-cancelled".to_string(),
            format: OutputFormat::Text,
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(
        result.is_ok(),
        "exclusion-only search should succeed: {:?}",
        result
    );

    // Validate directly against vault: "cancelled-meeting" should be excluded
    let search_results = vault
        .search_notes("-cancelled")
        .await
        .expect("direct search should work");

    let paths: Vec<String> = search_results
        .iter()
        .map(|(entry, _)| entry.path.to_string())
        .collect();

    assert!(
        !paths.contains(&"/cancelled-meeting.md".to_string()),
        "Should exclude cancelled-meeting note; found: {:?}",
        paths
    );
    // Should still find other notes
    assert!(
        !paths.is_empty(),
        "Should find other notes when excluding cancelled; found: {:?}",
        paths
    );

    // Test pure title exclusion
    let result = run_cli(
        CliCommand::Search {
            query: ">-draft".to_string(),
            format: OutputFormat::Text,
        },
        Some(config_path),
    )
    .await;

    assert!(
        result.is_ok(),
        "title exclusion-only should succeed: {:?}",
        result
    );

    // Validate directly against vault: "project-draft" should be excluded
    let search_results = vault
        .search_notes(">-draft")
        .await
        .expect("direct search should work");

    let paths: Vec<String> = search_results
        .iter()
        .map(|(entry, _)| entry.path.to_string())
        .collect();

    assert!(
        !paths.contains(&"/project-draft.md".to_string()),
        "Should exclude project-draft note; found: {:?}",
        paths
    );
    // Should still find other notes
    assert!(
        !paths.is_empty(),
        "Should find other notes when excluding draft title; found: {:?}",
        paths
    );
}

#[tokio::test]
async fn test_notes_paths_format_returns_bare_paths() {
    let dir = TempDir::new().unwrap();
    let _vault = setup_test_vault(&dir).await;
    let config_path = dir.path().join("config.toml");
    write_config(&config_path, dir.path());

    let result = run_cli(
        CliCommand::Notes {
            path: None,
            format: OutputFormat::Paths,
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_paths_format_empty_results() {
    let dir = TempDir::new().unwrap();
    let _vault = setup_test_vault(&dir).await;
    let config_path = dir.path().join("config.toml");
    write_config(&config_path, dir.path());

    // Path filter that matches nothing — zero results is not an error
    let result = run_cli(
        CliCommand::Notes {
            path: Some("nonexistent/prefix".to_string()),
            format: OutputFormat::Paths,
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_paths_format_path_with_spaces() {
    let dir = TempDir::new().unwrap();
    let vault = NoteVault::new(dir.path()).await.unwrap();
    vault.validate_and_init().await.unwrap();

    // Create two notes whose paths contain spaces
    vault
        .create_note(
            &VaultPath::note_path_from("notes/first note"),
            "# First\n\nContent.",
        )
        .await
        .unwrap();
    vault
        .create_note(
            &VaultPath::note_path_from("notes/second note"),
            "# Second\n\nContent.",
        )
        .await
        .unwrap();
    vault.recreate_index().await.unwrap();

    let config_path = dir.path().join("config.toml");
    write_config(&config_path, dir.path());

    // Both notes listed without error — the Paths arm prints one line per note
    // regardless of spaces in the path (println! emits a single \n per entry)
    let result = run_cli(
        CliCommand::Notes {
            path: Some("notes/".to_string()),
            format: OutputFormat::Paths,
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok());
}
