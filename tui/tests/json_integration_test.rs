// tui/tests/json_integration_test.rs
//
// Integration tests for JSON output in search and notes commands.
// These tests verify that --format json produces valid, well-structured JSON.

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::cli::output::OutputFormat;
use kimun_notes::cli::{CliCommand, run_cli};
use tempfile::TempDir;

/// Create a temporary vault with test notes indexed.
async fn setup_json_test_vault(dir: &TempDir) -> NoteVault {
    let vault = NoteVault::new(dir.path())
        .await
        .expect("failed to create vault");
    vault
        .validate_and_init()
        .await
        .expect("failed to init vault");

    vault
        .create_note(
            &VaultPath::note_path_from("rust-intro"),
            "# Introduction to Rust\n\n#programming #rust\n\nRust is a systems programming language.\n\n[[memory-safety]] [[ownership]]",
        )
        .await
        .expect("failed to create rust note");

    vault
        .create_note(
            &VaultPath::note_path_from("python-basics"),
            "# Python Basics\n\n#programming #python\n\nPython is great for scripting.",
        )
        .await
        .expect("failed to create python note");

    vault
        .create_note(
            &VaultPath::note_path_from("notes/deep-dive"),
            "# Deep Dive\n\n## Overview\n\nA nested note for testing path filtering.",
        )
        .await
        .expect("failed to create deep dive note");

    vault
        .recreate_index()
        .await
        .expect("failed to recreate index");
    vault
}

/// Write a minimal config file pointing workspace at the given path.
fn write_config(config_path: &std::path::Path, workspace: &std::path::Path) {
    let toml = format!(
        "workspace_dir = {:?}\n",
        workspace.to_string_lossy().as_ref()
    );
    std::fs::write(config_path, toml).expect("failed to write config file");
}

// ---------------------------------------------------------------------------
// test_search_json_output_is_valid
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_search_json_output_is_valid() {
    let workspace_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    setup_json_test_vault(&workspace_dir).await;
    write_config(&config_path, workspace_dir.path());

    // Capture stdout by using the vault directly
    let vault = NoteVault::new(workspace_dir.path()).await.unwrap();
    vault.validate_and_init().await.unwrap();

    let results = vault.search_notes("rust").await.unwrap();

    let json_str = kimun_notes::cli::json_output::format_notes_as_json(
        &vault,
        &results,
        "default",
        Some("rust"),
        false, // is_listing
    )
    .await
    .expect("format_notes_as_json should succeed");

    let json: serde_json::Value =
        serde_json::from_str(&json_str).expect("output should be valid JSON");

    // Verify top-level structure
    assert!(json["metadata"].is_object(), "should have metadata object");
    assert!(json["notes"].is_array(), "should have notes array");

    // Verify metadata fields
    assert_eq!(json["metadata"]["workspace"], "default");
    assert_eq!(
        json["metadata"]["workspace_path"],
        workspace_dir.path().to_string_lossy().to_string()
    );
    assert_eq!(json["metadata"]["query"], "rust");
    assert_eq!(json["metadata"]["is_listing"], false);
    assert!(json["metadata"]["total_results"].as_u64().unwrap() >= 1);
    assert!(json["metadata"]["generated_at"].is_string());

    // Verify note structure
    let note = &json["notes"][0];
    assert!(note["path"].is_string(), "note should have path");
    assert!(note["title"].is_string(), "note should have title");
    assert!(note["size"].is_number(), "note should have size");
    assert!(note["modified"].is_number(), "note should have modified");
    assert!(note["hash"].is_string(), "note should have hash");

    // Verify nested metadata structure
    assert!(
        note["metadata"].is_object(),
        "note should have metadata object"
    );
    assert!(
        note["metadata"]["tags"].is_array(),
        "metadata should have tags"
    );
    assert!(
        note["metadata"]["links"].is_array(),
        "metadata should have links"
    );
    assert!(
        note["metadata"]["headers"].is_array(),
        "metadata should have headers"
    );
}

// ---------------------------------------------------------------------------
// test_notes_json_output_is_valid
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_notes_json_output_is_valid() {
    let workspace_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    setup_json_test_vault(&workspace_dir).await;
    write_config(&config_path, workspace_dir.path());

    let vault = NoteVault::new(workspace_dir.path()).await.unwrap();
    vault.validate_and_init().await.unwrap();

    let results = vault.get_all_notes().await.unwrap();
    let json_str = kimun_notes::cli::json_output::format_notes_as_json(
        &vault,
        &results,
        "my-workspace",
        None,
        true, // is_listing
    )
    .await
    .expect("format_notes_as_json should succeed");

    let json: serde_json::Value =
        serde_json::from_str(&json_str).expect("output should be valid JSON");

    assert_eq!(json["metadata"]["workspace"], "my-workspace");
    assert_eq!(json["metadata"]["is_listing"], true);
    assert!(
        json["metadata"]["query"].is_null(),
        "query should be null for listing"
    );
    assert!(json["notes"].as_array().unwrap().len() >= 3);

    // Each note should have a nested metadata object
    for note in json["notes"].as_array().unwrap() {
        assert!(
            note["metadata"].is_object(),
            "each note should have metadata: {:?}",
            note["path"]
        );
    }
}

// ---------------------------------------------------------------------------
// test_search_json_metadata_contains_tags_and_links
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_search_json_metadata_contains_tags_and_links() {
    let workspace_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    setup_json_test_vault(&workspace_dir).await;
    write_config(&config_path, workspace_dir.path());

    let vault = NoteVault::new(workspace_dir.path()).await.unwrap();
    vault.validate_and_init().await.unwrap();

    let results = vault.search_notes("rust").await.unwrap();

    let json_str = kimun_notes::cli::json_output::format_notes_as_json(
        &vault,
        &results,
        "default",
        Some("rust"),
        false, // is_listing
    )
    .await
    .expect("format_notes_as_json should succeed");

    let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let notes = json["notes"].as_array().unwrap();

    assert!(!notes.is_empty(), "should find the rust note");

    let rust_note = notes.iter().find(|n| {
        n["path"]
            .as_str()
            .map(|p| p.contains("rust"))
            .unwrap_or(false)
    });

    assert!(rust_note.is_some(), "should find rust-intro note");
    let note = rust_note.unwrap();

    let tags = note["metadata"]["tags"].as_array().unwrap();
    assert!(
        tags.iter()
            .any(|t| t.as_str() == Some("programming") || t.as_str() == Some("rust")),
        "should extract tags from content; found: {:?}",
        tags
    );

    let links = note["metadata"]["links"].as_array().unwrap();
    assert!(
        links.iter().any(|l| l
            .as_str()
            .map(|s| s.contains("memory-safety") || s.contains("ownership"))
            .unwrap_or(false)),
        "should extract wiki links from content; found: {:?}",
        links
    );
}

// ---------------------------------------------------------------------------
// test_notes_json_journal_date_field_present
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_notes_json_journal_date_field_present() {
    let workspace_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    setup_json_test_vault(&workspace_dir).await;
    write_config(&config_path, workspace_dir.path());

    let vault = NoteVault::new(workspace_dir.path()).await.unwrap();
    vault.validate_and_init().await.unwrap();

    // Create a journal note
    let journal_path = VaultPath::note_path_from("journal/2024-01-15");
    vault
        .create_note(
            &journal_path,
            "# January 15, 2024\n\nToday's journal entry.",
        )
        .await
        .expect("failed to create journal note");

    vault.recreate_index().await.unwrap();

    let results = vault.get_all_notes().await.unwrap();

    let json_str = kimun_notes::cli::json_output::format_notes_as_json(
        &vault, &results, "default", None, true, // is_listing
    )
    .await
    .expect("format_notes_as_json should succeed");

    let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let notes = json["notes"].as_array().unwrap();

    // Find the journal note
    let journal_note = notes.iter().find(|n| {
        n["path"]
            .as_str()
            .map(|p| p.contains("journal"))
            .unwrap_or(false)
    });

    assert!(journal_note.is_some(), "should find journal note");
    let note = journal_note.unwrap();

    // journal_date should be set for journal notes
    assert!(
        note["journal_date"].is_string(),
        "journal notes should have journal_date field set; got: {:?}",
        note["journal_date"]
    );
    assert_eq!(note["journal_date"], "2024-01-15");
}

// ---------------------------------------------------------------------------
// test_notes_json_created_field_present
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_notes_json_created_field_present() {
    let workspace_dir = TempDir::new().unwrap();

    setup_json_test_vault(&workspace_dir).await;

    let vault = NoteVault::new(workspace_dir.path()).await.unwrap();
    vault.validate_and_init().await.unwrap();

    let results = vault.get_all_notes().await.unwrap();

    let json_str = kimun_notes::cli::json_output::format_notes_as_json(
        &vault, &results, "default", None, true, // is_listing
    )
    .await
    .expect("format_notes_as_json should succeed");

    let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let notes = json["notes"].as_array().unwrap();

    assert!(!notes.is_empty());
    for note in notes {
        assert!(
            note["created"].is_number(),
            "note should have created field; note: {:?}",
            note["path"]
        );
    }
}

// ---------------------------------------------------------------------------
// test_search_json_via_run_cli
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_search_json_via_run_cli() {
    let workspace_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    setup_json_test_vault(&workspace_dir).await;
    write_config(&config_path, workspace_dir.path());

    let result = run_cli(
        CliCommand::Search {
            query: "rust".to_string(),
            format: OutputFormat::Json,
        },
        Some(config_path),
    )
    .await;

    assert!(
        result.is_ok(),
        "search with JSON format should succeed: {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// test_notes_json_via_run_cli
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_notes_json_via_run_cli() {
    let workspace_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    setup_json_test_vault(&workspace_dir).await;
    write_config(&config_path, workspace_dir.path());

    let result = run_cli(
        CliCommand::Notes {
            path: None,
            format: OutputFormat::Json,
        },
        Some(config_path),
    )
    .await;

    assert!(
        result.is_ok(),
        "notes with JSON format should succeed: {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// test_text_format_unaffected
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_text_format_unaffected() {
    let workspace_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    setup_json_test_vault(&workspace_dir).await;
    write_config(&config_path, workspace_dir.path());

    // Text format should still work for both commands
    let result = run_cli(
        CliCommand::Search {
            query: "python".to_string(),
            format: OutputFormat::Text,
        },
        Some(config_path.clone()),
    )
    .await;
    assert!(
        result.is_ok(),
        "search text format should still work: {:?}",
        result
    );

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
        "notes text format should still work: {:?}",
        result
    );
}
