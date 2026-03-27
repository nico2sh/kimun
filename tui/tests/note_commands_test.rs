// tui/tests/note_commands_test.rs
//
// Integration tests for note create/append/journal CLI commands.

use kimun_notes::cli::{run_cli, CliCommand};
use kimun_notes::cli::commands::NoteSubcommand;
use tempfile::TempDir;

/// Helper: write a minimal Phase 2 config with a single workspace pointing at `workspace_dir`.
fn write_config(config_path: &std::path::Path, workspace_dir: &std::path::Path) {
    let content = format!(
        r#"config_version = 2
[global]
current_workspace = "default"
theme = "Nord"

[workspaces.default]
path = "{}"
last_paths = []
created = "2026-01-01T00:00:00Z"
"#,
        workspace_dir.display()
    );
    std::fs::write(config_path, content).unwrap();
}

// --- note create ---

#[tokio::test]
async fn test_note_create_creates_new_note() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    write_config(&config_path, workspace_dir.path());

    let result = run_cli(
        CliCommand::Note {
            subcommand: NoteSubcommand::Create {
                path: "my-note".to_string(),
                content: Some("# My Note\n\nHello".to_string()),
            },
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok(), "note create should succeed: {:?}", result);

    let note_file = workspace_dir.path().join("my-note.md");
    assert!(note_file.exists(), "note file should exist at {:?}", note_file);
    let content = std::fs::read_to_string(&note_file).unwrap();
    assert!(content.contains("Hello"), "note should contain the provided content");
}

#[tokio::test]
async fn test_note_create_fails_if_note_exists() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    write_config(&config_path, workspace_dir.path());

    // Pre-create the note
    std::fs::write(workspace_dir.path().join("existing.md"), "# Existing").unwrap();

    let result = run_cli(
        CliCommand::Note {
            subcommand: NoteSubcommand::Create {
                path: "existing".to_string(),
                content: Some("new content".to_string()),
            },
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_err(), "note create should fail when note already exists");
    let err = format!("{:?}", result.unwrap_err());
    assert!(err.contains("already exists"), "error should mention 'already exists': {}", err);
}

#[tokio::test]
async fn test_note_create_uses_quick_note_path() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();

    let content = format!(
        r#"config_version = 2
[global]
current_workspace = "default"
theme = "Nord"

[workspaces.default]
path = "{}"
last_paths = []
created = "2026-01-01T00:00:00Z"
quick_note_path = "/inbox"
"#,
        workspace_dir.path().display()
    );
    std::fs::write(&config_path, content).unwrap();

    let result = run_cli(
        CliCommand::Note {
            subcommand: NoteSubcommand::Create {
                path: "idea".to_string(),
                content: Some("an idea".to_string()),
            },
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok(), "note create should succeed: {:?}", result);
    let note_file = workspace_dir.path().join("inbox").join("idea.md");
    assert!(note_file.exists(), "note should be at {:?}", note_file);
}

#[tokio::test]
async fn test_note_create_absolute_path_ignores_quick_note_path() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();

    let content = format!(
        r#"config_version = 2
[global]
current_workspace = "default"
theme = "Nord"

[workspaces.default]
path = "{}"
last_paths = []
created = "2026-01-01T00:00:00Z"
quick_note_path = "/inbox"
"#,
        workspace_dir.path().display()
    );
    std::fs::write(&config_path, content).unwrap();

    let result = run_cli(
        CliCommand::Note {
            subcommand: NoteSubcommand::Create {
                path: "/projects/plan".to_string(),
                content: Some("a plan".to_string()),
            },
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok(), "note create should succeed: {:?}", result);
    let note_file = workspace_dir.path().join("projects").join("plan.md");
    assert!(note_file.exists(), "note should be at {:?}", note_file);
}

// --- note append ---

#[tokio::test]
async fn test_note_append_creates_if_not_exists() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    write_config(&config_path, workspace_dir.path());

    let result = run_cli(
        CliCommand::Note {
            subcommand: NoteSubcommand::Append {
                path: "new-note".to_string(),
                content: Some("first line".to_string()),
            },
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok(), "note append should succeed: {:?}", result);
    let note_file = workspace_dir.path().join("new-note.md");
    assert!(note_file.exists(), "note should be created");
    let content = std::fs::read_to_string(&note_file).unwrap();
    assert!(content.contains("first line"));
}

#[tokio::test]
async fn test_note_append_appends_to_existing() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    write_config(&config_path, workspace_dir.path());

    std::fs::write(workspace_dir.path().join("log.md"), "# Log\n\nFirst entry").unwrap();

    let result = run_cli(
        CliCommand::Note {
            subcommand: NoteSubcommand::Append {
                path: "log".to_string(),
                content: Some("Second entry".to_string()),
            },
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok(), "note append should succeed: {:?}", result);
    let content = std::fs::read_to_string(workspace_dir.path().join("log.md")).unwrap();
    assert!(content.contains("First entry"), "original content preserved");
    assert!(content.contains("Second entry"), "new content appended");
}

#[tokio::test]
async fn test_note_append_empty_content_is_noop() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    write_config(&config_path, workspace_dir.path());

    std::fs::write(workspace_dir.path().join("original.md"), "# Original").unwrap();

    let result = run_cli(
        CliCommand::Note {
            subcommand: NoteSubcommand::Append {
                path: "original".to_string(),
                content: Some("".to_string()),
            },
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok());
    let content = std::fs::read_to_string(workspace_dir.path().join("original.md")).unwrap();
    assert_eq!(content, "# Original", "content should be unchanged on empty append");
}

// --- note journal ---

#[tokio::test]
async fn test_note_journal_creates_todays_entry() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    write_config(&config_path, workspace_dir.path());

    let result = run_cli(
        CliCommand::Note {
            subcommand: NoteSubcommand::Journal {
                content: Some("Today's thought".to_string()),
            },
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok(), "note journal should succeed: {:?}", result);

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let journal_file = workspace_dir.path()
        .join("journal")
        .join(format!("{}.md", today));
    assert!(journal_file.exists(), "journal entry should exist at {:?}", journal_file);
    let content = std::fs::read_to_string(&journal_file).unwrap();
    assert!(content.contains("Today's thought"));
}

#[tokio::test]
async fn test_note_journal_appends_to_existing_entry() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    write_config(&config_path, workspace_dir.path());

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let journal_dir = workspace_dir.path().join("journal");
    std::fs::create_dir_all(&journal_dir).unwrap();
    std::fs::write(
        journal_dir.join(format!("{}.md", today)),
        format!("# {}\n\nFirst entry", today),
    ).unwrap();

    run_cli(
        CliCommand::Note {
            subcommand: NoteSubcommand::Journal {
                content: Some("Second entry".to_string()),
            },
        },
        Some(config_path),
    )
    .await
    .unwrap();

    let content = std::fs::read_to_string(journal_dir.join(format!("{}.md", today))).unwrap();
    assert!(content.contains("First entry"), "original content preserved");
    assert!(content.contains("Second entry"), "new content appended");
}

#[tokio::test]
async fn test_note_journal_empty_content_is_noop() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    write_config(&config_path, workspace_dir.path());

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let journal_dir = workspace_dir.path().join("journal");
    std::fs::create_dir_all(&journal_dir).unwrap();
    let journal_file = journal_dir.join(format!("{}.md", today));
    std::fs::write(&journal_file, format!("# {}", today)).unwrap();

    run_cli(
        CliCommand::Note {
            subcommand: NoteSubcommand::Journal {
                content: Some("".to_string()),
            },
        },
        Some(config_path),
    )
    .await
    .unwrap();

    let content = std::fs::read_to_string(&journal_file).unwrap();
    assert_eq!(content, format!("# {}", today), "content should be unchanged on empty journal");
}

// --- note show ---

#[tokio::test]
async fn test_note_show_text_returns_ok() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    write_config(&config_path, workspace_dir.path());

    std::fs::write(
        workspace_dir.path().join("my-note.md"),
        "# My Note\n\nHello world",
    ).unwrap();

    let result = run_cli(
        CliCommand::Note {
            subcommand: NoteSubcommand::Show {
                paths: vec!["my-note".to_string()],
                format: kimun_notes::cli::output::OutputFormat::Text,
            },
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok(), "note show should succeed: {:?}", result);
}

#[tokio::test]
async fn test_note_show_missing_note_fails() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    write_config(&config_path, workspace_dir.path());

    // Pre-create an unrelated note so the vault initializes cleanly;
    // the test target ("does-not-exist") is intentionally absent.
    std::fs::write(workspace_dir.path().join("unrelated.md"), "# Unrelated").unwrap();

    let result = run_cli(
        CliCommand::Note {
            subcommand: NoteSubcommand::Show {
                paths: vec!["does-not-exist".to_string()],
                format: kimun_notes::cli::output::OutputFormat::Text,
            },
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_err(), "note show on missing note should fail");
}

#[tokio::test]
async fn test_note_show_json_returns_ok() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    write_config(&config_path, workspace_dir.path());

    std::fs::write(
        workspace_dir.path().join("json-note.md"),
        "# JSON Note\n\nsome content",
    ).unwrap();

    let result = run_cli(
        CliCommand::Note {
            subcommand: NoteSubcommand::Show {
                paths: vec!["json-note".to_string()],
                format: kimun_notes::cli::output::OutputFormat::Json,
            },
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok(), "note show --format json should succeed: {:?}", result);
}

#[tokio::test]
async fn test_note_show_multiple_notes_ok() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    write_config(&config_path, workspace_dir.path());

    std::fs::write(workspace_dir.path().join("note-a.md"), "# Note A").unwrap();
    std::fs::write(workspace_dir.path().join("note-b.md"), "# Note B").unwrap();

    let result = run_cli(
        CliCommand::Note {
            subcommand: NoteSubcommand::Show {
                paths: vec!["note-a".to_string(), "note-b".to_string()],
                format: kimun_notes::cli::output::OutputFormat::Text,
            },
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok(), "note show with multiple notes should succeed: {:?}", result);
}

#[tokio::test]
async fn test_note_show_format_paths_returns_error() {
    use kimun_notes::cli::output::OutputFormat;
    use kimun_core::nfs::VaultPath;
    let dir = TempDir::new().unwrap();
    let vault = kimun_core::NoteVault::new(dir.path()).await.unwrap();
    vault.validate_and_init().await.unwrap();
    vault
        .create_note(
            &VaultPath::note_path_from("test/note"),
            "# Test\n\nContent.",
        )
        .await
        .unwrap();

    let config_path = dir.path().join("config.toml");
    write_config(&config_path, dir.path());

    let result = run_cli(
        CliCommand::Note {
            subcommand: NoteSubcommand::Show {
                paths: vec!["test/note".to_string()],
                format: OutputFormat::Paths,
            },
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("--format paths is not valid for note show"),
        "got: {}",
        msg
    );
}

#[tokio::test]
async fn test_note_show_partial_failure_returns_err() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    write_config(&config_path, workspace_dir.path());

    std::fs::write(workspace_dir.path().join("exists.md"), "# Exists").unwrap();

    // One valid, one missing — should return Err (partial failure)
    let result = run_cli(
        CliCommand::Note {
            subcommand: NoteSubcommand::Show {
                paths: vec!["exists".to_string(), "missing".to_string()],
                format: kimun_notes::cli::output::OutputFormat::Text,
            },
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_err(), "partial failure should return Err");
}
