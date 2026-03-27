# CLI Note Operations Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `note create`, `note append`, and `note journal` CLI subcommands with a configurable `quick_note_path` per workspace.

**Architecture:** Add `quick_note_path` to `WorkspaceEntry`, add two helpers (`resolve_quick_note_path`, `resolve_note_path`) to `cli/helpers.rs`, create a new `cli/commands/note_ops.rs` with the `NoteSubcommand` enum, and wire everything into `cli/mod.rs`. Note commands call `load_and_resolve_workspace` directly (not `create_and_init_vault`) so that `quick_note_path` stays accessible.

**Tech Stack:** Rust, clap (subcommands), kimun_core (`NoteVault`), tokio (async), tempfile (tests), `std::io::IsTerminal` (stdin TTY detection, stable since Rust 1.70)

---

## Chunk 1: Config — add `quick_note_path` to `WorkspaceEntry`

**Files:**
- Modify: `tui/src/settings/workspace_config.rs`

### Task 1: Add `quick_note_path` field and helper to `WorkspaceEntry`

- [ ] **Step 1: Add the field with serde default**

In `tui/src/settings/workspace_config.rs`, update `WorkspaceEntry`:

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceEntry {
    pub path: PathBuf,
    pub last_paths: Vec<String>,
    pub created: DateTime<Utc>,
    #[serde(default)]
    pub quick_note_path: Option<String>,
}
```

- [ ] **Step 2: Update existing `WorkspaceEntry` struct construction sites**

`add_workspace` (around line 64) and `from_phase1_migration` (around line 96) construct `WorkspaceEntry` literals. Both must include the new field or the code won't compile.

In `add_workspace`, change the `WorkspaceEntry` literal to:
```rust
let entry = WorkspaceEntry {
    path,
    last_paths: Vec::new(),
    created: Utc::now(),
    quick_note_path: None,
};
```

In `from_phase1_migration`, change the `WorkspaceEntry` literal to:
```rust
let entry = WorkspaceEntry {
    path: workspace_dir,
    last_paths,
    created: Utc::now(),
    quick_note_path: None,
};
```

- [ ] **Step 4: Add a helper method**

Below the closing brace of the struct, add an `impl` block (or add to an existing one if present):

```rust
impl WorkspaceEntry {
    pub fn quick_note_path(&self) -> &str {
        self.quick_note_path.as_deref().unwrap_or("/")
    }
}
```

- [ ] **Step 5: Verify existing tests still compile and pass**

Run: `cargo test -p kimun-notes 2>&1 | tail -20`
Expected: all existing tests pass, no compile errors.

- [ ] **Step 6: Commit**

```bash
git add tui/src/settings/workspace_config.rs
git commit -m "feat: add quick_note_path to WorkspaceEntry config"
```

---

## Chunk 2: Helpers — `resolve_quick_note_path` and `resolve_note_path`

**Files:**
- Modify: `tui/src/cli/helpers.rs`
- Test: `tui/tests/note_path_resolution_test.rs` (new file)

### Task 2: Write failing tests for path resolution helpers

- [ ] **Step 1: Create the test file**

Create `tui/tests/note_path_resolution_test.rs`:

```rust
// tui/tests/note_path_resolution_test.rs
//
// Unit tests for CLI note path resolution helpers.

use kimun_notes::cli::helpers::resolve_note_path;

// resolve_note_path: relative path joined with quick_note_path
#[test]
fn test_relative_path_joined_with_quick_note_path() {
    let path = resolve_note_path("my-note", "/inbox").unwrap();
    assert_eq!(path.to_string(), "/inbox/my-note.md");
}

// resolve_note_path: relative path without extension gets .md
#[test]
fn test_relative_path_no_extension_gets_md() {
    let path = resolve_note_path("ideas/thing", "/notes").unwrap();
    assert_eq!(path.to_string(), "/notes/ideas/thing.md");
}

// resolve_note_path: explicit .md extension is not doubled
#[test]
fn test_explicit_md_extension_not_doubled() {
    let path = resolve_note_path("my-note.md", "/inbox").unwrap();
    assert_eq!(path.to_string(), "/inbox/my-note.md");
}

// resolve_note_path: absolute path (leading /) ignores quick_note_path
#[test]
fn test_absolute_path_ignores_quick_note_path() {
    let path = resolve_note_path("/projects/idea", "/inbox").unwrap();
    assert_eq!(path.to_string(), "/projects/idea.md");
}

// resolve_note_path: absolute path with .md is not doubled
#[test]
fn test_absolute_path_with_md_not_doubled() {
    let path = resolve_note_path("/projects/idea.md", "/inbox").unwrap();
    assert_eq!(path.to_string(), "/projects/idea.md");
}

// resolve_note_path: empty string returns error
#[test]
fn test_empty_path_returns_error() {
    let result = resolve_note_path("", "/inbox");
    assert!(result.is_err(), "empty path should return an error");
}

// resolve_note_path: whitespace-only returns error
#[test]
fn test_whitespace_only_path_returns_error() {
    let result = resolve_note_path("   ", "/inbox");
    assert!(result.is_err(), "whitespace-only path should return an error");
}

// resolve_note_path: quick_note_path defaults to root when "/"
#[test]
fn test_quick_note_path_root_default() {
    let path = resolve_note_path("my-note", "/").unwrap();
    assert_eq!(path.to_string(), "/my-note.md");
}
```

- [ ] **Step 2: Run tests to confirm they fail (function not yet defined)**

Run: `cargo test -p kimun-notes --test note_path_resolution_test 2>&1 | tail -20`
Expected: compile error — `resolve_note_path` not found.

### Task 3: Implement the helpers

- [ ] **Step 3: Add `resolve_quick_note_path` to helpers.rs**

In `tui/src/cli/helpers.rs`, add after the existing `load_and_resolve_workspace` function:

```rust
/// Returns the configured quick_note_path for the active workspace.
/// Falls back to "/" for Phase 1 workspaces (no WorkspaceEntry) or if not configured.
pub fn resolve_quick_note_path(settings: &AppSettings) -> String {
    // Phase 1 legacy: workspace_dir only, no WorkspaceEntry
    if settings.workspace_dir.is_some() {
        return "/".to_string();
    }
    // Phase 2: workspace_config
    if let Some(ref ws_config) = settings.workspace_config {
        if let Some(entry) = ws_config.get_current_workspace() {
            return entry.quick_note_path().to_string();
        }
    }
    "/".to_string()
}
```

- [ ] **Step 4: Add `resolve_note_path` to helpers.rs**

Add the following import at the top of `tui/src/cli/helpers.rs` if not already present:
```rust
use kimun_core::nfs::VaultPath;
```

Then add the function:

```rust
/// Resolve a user-provided note path string into a VaultPath.
///
/// Rules:
/// - Empty or whitespace-only input → error
/// - Starts with "/" → absolute from vault root (quick_note_path ignored)
/// - Otherwise → relative, joined with quick_note_path
/// - VaultPath::note_path_from normalizes path and ensures .md extension
pub fn resolve_note_path(input: &str, quick_note_path: &str) -> color_eyre::eyre::Result<VaultPath> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(color_eyre::eyre::eyre!("Note path cannot be empty"));
    }
    let raw = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        let base = quick_note_path.trim_end_matches('/');
        format!("{}/{}", base, trimmed)
    };
    Ok(VaultPath::note_path_from(&raw))
}
```

- [ ] **Step 5: Make `resolve_note_path` and `resolve_quick_note_path` public in the module**

Check that `helpers.rs` already has `pub fn` for both (the code above uses `pub fn` — confirm it's consistent).

- [ ] **Step 6: Run tests to confirm they pass**

Run: `cargo test -p kimun-notes --test note_path_resolution_test 2>&1 | tail -20`
Expected: all 8 tests pass.

- [ ] **Step 7: Run full test suite**

Run: `cargo test -p kimun-notes 2>&1 | tail -20`
Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add tui/src/cli/helpers.rs tui/tests/note_path_resolution_test.rs
git commit -m "feat: add resolve_note_path and resolve_quick_note_path helpers"
```

---

## Chunk 3: `note_ops.rs` — NoteSubcommand and run function

**Files:**
- Create: `tui/src/cli/commands/note_ops.rs`
- Modify: `tui/src/cli/commands/mod.rs`
- Test: `tui/tests/note_commands_test.rs` (new file)

### Task 4: Write failing integration tests for note commands

- [ ] **Step 1: Create the test file**

Create `tui/tests/note_commands_test.rs`:

```rust
// tui/tests/note_commands_test.rs
//
// Integration tests for note create/append/journal CLI commands.

use kimun_notes::cli::{run_cli, CliCommand};
use kimun_notes::cli::commands::NoteSubcommand;
use kimun_notes::settings::AppSettings;
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

    // Config with quick_note_path = "/inbox"
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

    // Today's journal lives at /journal/YYYY-MM-DD.md
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

    // Pre-create today's entry
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
```

- [ ] **Step 2: Run tests to confirm compile error (NoteSubcommand not defined)**

Run: `cargo test -p kimun-notes --test note_commands_test 2>&1 | head -30`
Expected: compile error — `NoteSubcommand` not found.

### Task 5: Implement `note_ops.rs`

- [ ] **Step 3: Create `tui/src/cli/commands/note_ops.rs`**

```rust
// tui/src/cli/commands/note_ops.rs
//
// CLI commands for note create, append, and journal operations.

use clap::Subcommand;
use color_eyre::eyre::Result;
use kimun_core::{NoteVault, error::VaultError};
use kimun_core::nfs::VaultPath;

#[derive(Subcommand, Debug)]
pub enum NoteSubcommand {
    /// Create a new note (fails if the note already exists)
    Create {
        /// Note path, relative to quick_note_path or absolute from vault root
        path: String,
        /// Note content (reads from stdin if omitted and stdin is not a TTY)
        content: Option<String>,
    },
    /// Append text to a note (creates the note if it does not exist)
    Append {
        /// Note path, relative to quick_note_path or absolute from vault root
        path: String,
        /// Text to append (reads from stdin if omitted and stdin is not a TTY)
        content: Option<String>,
    },
    /// Append text to today's journal entry (creates it if it does not exist)
    Journal {
        /// Text to append (reads from stdin if omitted and stdin is not a TTY)
        content: Option<String>,
    },
}

pub async fn run(
    subcommand: NoteSubcommand,
    vault: &NoteVault,
    quick_note_path: &str,
) -> Result<()> {
    match subcommand {
        NoteSubcommand::Create { path, content } => {
            run_create(vault, &path, content, quick_note_path).await
        }
        NoteSubcommand::Append { path, content } => {
            run_append(vault, &path, content, quick_note_path).await
        }
        NoteSubcommand::Journal { content } => {
            run_journal(vault, content).await
        }
    }
}

async fn run_create(
    vault: &NoteVault,
    path_input: &str,
    content: Option<String>,
    quick_note_path: &str,
) -> Result<()> {
    use crate::cli::helpers::resolve_note_path;

    let vault_path = resolve_note_path(path_input, quick_note_path)?;
    let text = resolve_content(content);

    vault.create_note(&vault_path, &text).await.map_err(|e| {
        match &e {
            VaultError::NoteExists { path } => {
                color_eyre::eyre::eyre!("Note already exists: {}", path)
            }
            _ => color_eyre::eyre::eyre!("{}", e),
        }
    })?;

    println!("Note saved: {}", vault_path);
    Ok(())
}

async fn run_append(
    vault: &NoteVault,
    path_input: &str,
    content: Option<String>,
    quick_note_path: &str,
) -> Result<()> {
    use crate::cli::helpers::resolve_note_path;
    use kimun_core::error::FSError;

    let vault_path = resolve_note_path(path_input, quick_note_path)?;
    let text = resolve_content(content);

    if text.is_empty() {
        return Ok(());
    }

    match vault.get_note_text(&vault_path).await {
        Ok(existing) => {
            let combined = format!("{}\n{}", existing, text);
            vault.save_note(&vault_path, &combined).await
                .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
        }
        Err(VaultError::FSError(FSError::VaultPathNotFound { .. })) => {
            // Note does not exist — create it. Handle race condition: if another
            // process created the note between our read and create, re-read and save.
            match vault.create_note(&vault_path, &text).await {
                Ok(_) => {}
                Err(VaultError::NoteExists { .. }) => {
                    // Race: note was created between our get and create
                    let existing = vault.get_note_text(&vault_path).await
                        .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
                    let combined = format!("{}\n{}", existing, text);
                    vault.save_note(&vault_path, &combined).await
                        .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
                }
                Err(e) => return Err(color_eyre::eyre::eyre!("{}", e)),
            }
        }
        Err(e) => return Err(color_eyre::eyre::eyre!("{}", e)),
    }

    println!("Note saved: {}", vault_path);
    Ok(())
}

async fn run_journal(vault: &NoteVault, content: Option<String>) -> Result<()> {
    let text = resolve_content(content);

    if text.is_empty() {
        return Ok(());
    }

    let (details, existing) = vault.journal_entry().await
        .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;

    let combined = format!("{}\n{}", existing, text);
    vault.save_note(&details.path, &combined).await
        .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;

    println!("Note saved: {}", details.path);
    Ok(())
}

/// Returns content from the Option, or reads from stdin if not a TTY.
/// Returns an empty string if content is None and stdin is a TTY.
fn resolve_content(content: Option<String>) -> String {
    use std::io::IsTerminal;
    match content {
        Some(c) => c,
        None => {
            if std::io::stdin().is_terminal() {
                String::new()
            } else {
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf).unwrap_or(0);
                buf.trim_end_matches('\n').to_string()
            }
        }
    }
}
```

- [ ] **Step 4: Register the new module in `commands/mod.rs`**

In `tui/src/cli/commands/mod.rs`, add:

```rust
pub mod note_ops;
pub use note_ops::NoteSubcommand;
```

So the file becomes:

```rust
// tui/src/cli/commands/mod.rs
pub mod search;
pub mod notes;
pub mod workspace;
pub mod note_ops;

// Re-export for convenience
pub use workspace::WorkspaceSubcommand;
pub use note_ops::NoteSubcommand;
```

- [ ] **Step 5: Run tests to confirm they still compile (even if failing due to missing wiring)**

Run: `cargo test -p kimun-notes --test note_commands_test 2>&1 | head -30`
Expected: compile progresses further; may fail at `CliCommand::Note` not yet defined.

- [ ] **Step 6: Commit the commands module**

```bash
git add tui/src/cli/commands/note_ops.rs tui/src/cli/commands/mod.rs
git commit -m "feat: implement NoteSubcommand (create/append/journal)"
```

---

## Chunk 4: Wire `CliCommand::Note` into `cli/mod.rs`

**Files:**
- Modify: `tui/src/cli/mod.rs`

### Task 6: Add `CliCommand::Note` and dispatch

- [ ] **Step 1: Update `cli/mod.rs`**

Replace the current content of `tui/src/cli/mod.rs` with:

```rust
// tui/src/cli/mod.rs
pub mod commands;
pub mod output;
pub mod json_output;
pub mod metadata_extractor;
pub mod helpers;

use clap::Subcommand;
use color_eyre::eyre::Result;
use output::OutputFormat;
use commands::workspace::WorkspaceSubcommand;
use commands::note_ops::NoteSubcommand;
use helpers::{create_and_init_vault, load_settings, load_and_resolve_workspace, resolve_quick_note_path};

#[derive(Subcommand)]
pub enum CliCommand {
    /// Search notes by query
    Search {
        query: String,
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// List all notes
    Notes {
        #[arg(long, help = "Filter notes by path prefix")]
        path: Option<String>,
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// Manage workspaces
    Workspace {
        #[command(subcommand)]
        subcommand: WorkspaceSubcommand,
    },
    /// Note operations (create, append, journal)
    Note {
        #[command(subcommand)]
        subcommand: NoteSubcommand,
    },
}

pub async fn run_cli(command: CliCommand, config_path: Option<std::path::PathBuf>) -> Result<()> {
    // Workspace commands need mutable settings
    if let CliCommand::Workspace { subcommand } = command {
        let mut settings = load_settings(config_path)?;
        return commands::workspace::run(subcommand, &mut settings).await;
    }

    // Note commands need settings for quick_note_path
    if let CliCommand::Note { subcommand } = command {
        let (settings, workspace_path, _workspace_name) = load_and_resolve_workspace(config_path)?;
        let quick_note_path = resolve_quick_note_path(&settings);
        let vault = kimun_core::NoteVault::new(&workspace_path).await?;
        vault.init_and_validate().await?;
        return commands::note_ops::run(subcommand, &vault, &quick_note_path).await;
    }

    // Search and Notes commands
    let (vault, workspace_name) = create_and_init_vault(config_path).await?;

    match command {
        CliCommand::Search { query, format } => {
            commands::search::run(&vault, &query, format, &workspace_name, false).await
        }
        CliCommand::Notes { path, format } => {
            commands::notes::run(&vault, path.as_deref(), format, &workspace_name, false).await
        }
        CliCommand::Workspace { .. } => unreachable!("handled above"),
        CliCommand::Note { .. } => unreachable!("handled above"),
    }
}
```

- [ ] **Step 2: Run the integration tests**

Run: `cargo test -p kimun-notes --test note_commands_test 2>&1 | tail -30`
Expected: all tests pass.

- [ ] **Step 3: Run the full test suite**

Run: `cargo test -p kimun-notes 2>&1 | tail -20`
Expected: all tests pass.

- [ ] **Step 4: Run path resolution tests to confirm nothing regressed**

Run: `cargo test -p kimun-notes --test note_path_resolution_test 2>&1 | tail -10`
Expected: all 8 tests pass.

- [ ] **Step 5: Commit**

```bash
git add tui/src/cli/mod.rs tui/tests/note_commands_test.rs
git commit -m "feat: wire CliCommand::Note into CLI dispatcher"
```

---

## Chunk 5: Manual smoke test and final commit

### Task 7: Smoke test the CLI

- [ ] **Step 1: Build the binary**

Run: `cargo build -p kimun-notes 2>&1 | tail -10`
Expected: builds cleanly with no errors.

- [ ] **Step 2: Verify help output**

Run: `./target/debug/kimun note --help`
Expected: shows `create`, `append`, `journal` subcommands.

- [ ] **Step 3: Test `note create`**

```bash
./target/debug/kimun note create test-smoke "# Smoke Test\n\nIt works!"
```
Expected output: `Note saved: /test-smoke.md`

- [ ] **Step 4: Test `note create` fails on duplicate**

```bash
./target/debug/kimun note create test-smoke "duplicate"
```
Expected: exits with non-zero, prints error containing "already exists".

- [ ] **Step 5: Test `note append`**

```bash
./target/debug/kimun note append test-smoke "appended line"
```
Expected output: `Note saved: /test-smoke.md`

- [ ] **Step 6: Test `note journal`**

```bash
./target/debug/kimun note journal "journal entry from CLI"
```
Expected output: `Note saved: /journal/YYYY-MM-DD.md`

- [ ] **Step 7: Test stdin input**

```bash
echo "from stdin" | ./target/debug/kimun note append test-smoke
```
Expected output: `Note saved: /test-smoke.md`

- [ ] **Step 8: Clean up smoke test note (optional)**

```bash
rm ~/Documents/Notes/test-smoke.md
```

- [ ] **Step 9: Final commit**

```bash
git add -p  # stage anything not yet committed
git commit -m "chore: complete CLI note operations feature"
```
