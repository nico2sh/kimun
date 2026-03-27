# CLI Note Show Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `kimun note show <path> [<path>...] [--format text|json]` to print note content and metadata from the command line.

**Architecture:** Add a `Show` variant to the existing `NoteSubcommand` enum, wire it through `run()` and `run_cli`, and implement `run_show` which loads each note via `vault.load_note`, fetches backlinks, and formats output as either a metadata header + content (text) or a `JsonOutput` envelope (JSON). The `backlinks` field is added to `JsonNoteEntry` as `Option<Vec<String>>`.

**Tech Stack:** Rust, Tokio, Clap, Serde/serde_json, color_eyre, kimun_core (`NoteVault`, `NoteDetails`, `NoteEntryData`), chrono

---

## File Map

| File | Change |
|------|--------|
| `tui/src/cli/json_output.rs` | Add `backlinks: Option<Vec<String>>` to `JsonNoteEntry`; set `backlinks: None` in existing struct literal |
| `tui/src/cli/commands/note_ops.rs` | Add `Show` variant; update `run()` signature; add `run_show`, `format_note_show_text`, `NOTE_SEPARATOR` |
| `tui/src/cli/mod.rs` | Rename `_workspace_name` → `workspace_name`; pass to `note_ops::run()` |
| `tui/tests/note_commands_test.rs` | Add 5 integration tests for `note show` |

---

## Chunk 1: Foundation + Plumbing

### Task 1: Add `backlinks` field to `JsonNoteEntry`

**Files:**
- Modify: `tui/src/cli/json_output.rs:34-46` (struct definition) and `:108-123` (struct literal in `format_notes_with_content_as_json`)

- [ ] **Step 1: Add the field to the struct**

In `tui/src/cli/json_output.rs`, add to `JsonNoteEntry` after `journal_date`:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub backlinks: Option<Vec<String>>,
```

The struct should now look like:

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct JsonNoteEntry {
    pub path: String,
    pub title: String,
    pub content: String,
    pub size: u64,
    pub modified: u64,
    pub created: u64,
    pub hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journal_date: Option<String>,
    pub metadata: JsonNoteMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backlinks: Option<Vec<String>>,
}
```

- [ ] **Step 2: Update the struct literal in `format_notes_with_content_as_json`**

Find the `JsonNoteEntry { ... }` struct literal (around line 108). Add `backlinks: None` as the last field:

```rust
JsonNoteEntry {
    path: path_with_ext,
    title: content_data.title.clone(),
    content,
    size: entry_data.size,
    modified: entry_data.modified_secs,
    created,
    hash: format!("{:x}", content_data.hash),
    journal_date,
    metadata: JsonNoteMetadata {
        tags,
        links,
        headers,
    },
    backlinks: None,
}
```

- [ ] **Step 3: Verify it compiles and existing tests still pass**

```bash
cargo test -p kimun-notes 2>&1 | tail -20
```

Expected: all existing tests pass, no compile errors.

- [ ] **Step 4: Commit**

```bash
git add tui/src/cli/json_output.rs
git commit -m "feat: add backlinks field to JsonNoteEntry"
```

---

### Task 2: Add `Show` variant and update plumbing

**Files:**
- Modify: `tui/src/cli/commands/note_ops.rs` (enum + `run()` signature)
- Modify: `tui/src/cli/mod.rs` (call site)

- [ ] **Step 1: Add `Show` variant to `NoteSubcommand`**

In `tui/src/cli/commands/note_ops.rs`, add to the `NoteSubcommand` enum after `Journal`:

```rust
/// Show note content and metadata (read one or more notes)
Show {
    /// One or more note paths (relative to quick_note_path or absolute from vault root)
    paths: Vec<String>,
    #[arg(long, value_enum, default_value = "text")]
    format: crate::cli::output::OutputFormat,
},
```

- [ ] **Step 2: Update `run()` to accept `workspace_name` and handle `Show`**

Change the signature of `pub async fn run(...)` to:

```rust
pub async fn run(
    subcommand: NoteSubcommand,
    vault: &kimun_core::NoteVault,
    quick_note_path: &str,
    workspace_name: &str,
) -> color_eyre::eyre::Result<()> {
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
        NoteSubcommand::Show { paths, format } => {
            run_show(vault, &paths, quick_note_path, format, workspace_name).await
        }
    }
}
```

- [ ] **Step 3: Add a stub `run_show` so it compiles**

Add after `run_journal`:

```rust
async fn run_show(
    _vault: &kimun_core::NoteVault,
    _path_inputs: &[String],
    _quick_note_path: &str,
    _format: crate::cli::output::OutputFormat,
    _workspace_name: &str,
) -> color_eyre::eyre::Result<()> {
    todo!("note show not yet implemented")
}
```

- [ ] **Step 4: Update the call site in `tui/src/cli/mod.rs`**

Find the `CliCommand::Note` branch (around line 50). Change:

```rust
let (settings, workspace_path, _workspace_name) = load_and_resolve_workspace(config_path)?;
let quick_note_path = resolve_quick_note_path(&settings);
let vault = kimun_core::NoteVault::new(&workspace_path).await?;
vault.init_and_validate().await?;
return commands::note_ops::run(subcommand, &vault, &quick_note_path).await;
```

To:

```rust
let (settings, workspace_path, workspace_name) = load_and_resolve_workspace(config_path)?;
let quick_note_path = resolve_quick_note_path(&settings);
let vault = kimun_core::NoteVault::new(&workspace_path).await?;
vault.init_and_validate().await?;
return commands::note_ops::run(subcommand, &vault, &quick_note_path, &workspace_name).await;
```

- [ ] **Step 5: Verify it compiles**

```bash
cargo build -p kimun-notes 2>&1 | tail -10
```

Expected: compiles cleanly (the stub `todo!` is fine at this stage).

- [ ] **Step 6: Commit**

```bash
git add tui/src/cli/commands/note_ops.rs tui/src/cli/mod.rs
git commit -m "feat: add Show variant to NoteSubcommand and update run() plumbing"
```

---

## Chunk 2: Tests + Implementation

### Task 3: Write failing integration tests for `note show`

**Files:**
- Modify: `tui/tests/note_commands_test.rs`

The tests use the same pattern as existing ones: write a minimal TOML config pointing at a temp workspace dir, call `run_cli(CliCommand::Note { subcommand: NoteSubcommand::Show { ... } }, Some(config_path)).await`.

`NoteSubcommand::Show` is already re-exported via `kimun_notes::cli::commands::NoteSubcommand`.

For JSON tests, import `serde_json` which is already a dependency.

- [ ] **Step 1: Write 5 failing tests**

Add to `tui/tests/note_commands_test.rs`:

```rust
// --- note show ---

#[tokio::test]
async fn test_note_show_text_returns_ok() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    write_config(&config_path, workspace_dir.path());

    // Pre-create the note
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
```

- [ ] **Step 2: Run tests to verify they fail (not panic on todo!)**

```bash
cargo test -p kimun-notes note_show 2>&1
```

Expected: tests compile but fail/panic with `not yet implemented` from the `todo!()` stub.

- [ ] **Step 3: Commit the failing tests**

```bash
git add tui/tests/note_commands_test.rs
git commit -m "test: add failing integration tests for note show"
```

---

### Task 4: Implement `run_show`, `format_note_show_text`, and `NOTE_SEPARATOR`

**Files:**
- Modify: `tui/src/cli/commands/note_ops.rs`

Replace the `todo!()` stub with the full implementation.

- [ ] **Step 1: Add `NOTE_SEPARATOR` constant**

Near the top of `note_ops.rs` (after the `use` statements), add:

```rust
const NOTE_SEPARATOR: &str = "================================================================================";
```

- [ ] **Step 2: Add `format_note_show_text` function**

Add after `run_journal` and before `resolve_content`:

```rust
fn format_note_show_text(
    path: &kimun_core::nfs::VaultPath,
    content: &str,
    title: &str,
    tags: &[String],
    links: &[String],
    backlinks: &[String],
) -> String {
    let mut out = String::new();
    out.push_str(&format!("Path:      {}\n", path));
    if !title.is_empty() {
        out.push_str(&format!("Title:     {}\n", title));
    }
    if !tags.is_empty() {
        out.push_str(&format!("Tags:      {}\n", tags.join(" ")));
    }
    if !links.is_empty() {
        out.push_str(&format!("Links:     {}\n", links.join(", ")));
    }
    if !backlinks.is_empty() {
        out.push_str(&format!("Backlinks: {}\n", backlinks.join(", ")));
    }
    out.push_str("---\n");
    out.push_str(content);
    out
}
```

- [ ] **Step 3: Replace the `run_show` stub with the full implementation**

Replace the stub `run_show` with:

```rust
async fn run_show(
    vault: &kimun_core::NoteVault,
    path_inputs: &[String],
    quick_note_path: &str,
    format: crate::cli::output::OutputFormat,
    workspace_name: &str,
) -> color_eyre::eyre::Result<()> {
    use crate::cli::helpers::resolve_note_path;
    use crate::cli::metadata_extractor::{extract_tags, extract_links, extract_headers};
    use crate::cli::json_output::{JsonNoteEntry, JsonNoteMetadata, JsonOutput, JsonOutputMetadata};
    use crate::cli::output::OutputFormat;
    use kimun_core::nfs::NoteEntryData;
    use kimun_core::error::{VaultError, FSError};
    use chrono::Utc;
    use std::time::UNIX_EPOCH;

    let mut text_entries: Vec<String> = Vec::new();
    let mut json_entries: Vec<JsonNoteEntry> = Vec::new();
    let mut had_errors = false;

    for input in path_inputs {
        let vault_path = match resolve_note_path(input, quick_note_path) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Error: {}", e);
                had_errors = true;
                continue;
            }
        };

        let note_details = match vault.load_note(&vault_path).await {
            Ok(nd) => nd,
            Err(VaultError::FSError(FSError::VaultPathNotFound { .. })) => {
                eprintln!("Error: Note not found: {}", vault_path);
                had_errors = true;
                continue;
            }
            Err(e) => return Err(color_eyre::eyre::eyre!("{}", e)),
        };

        let content = note_details.raw_text.clone();
        let content_data = note_details.get_content_data();

        let meta = tokio::fs::metadata(vault.path_to_pathbuf(&vault_path))
            .await
            .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
        let modified_secs = meta
            .modified()
            .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs())
            .unwrap_or(0);
        let entry_data = NoteEntryData {
            path: vault_path.clone(),
            size: meta.len(),
            modified_secs,
        };

        let backlink_results = vault
            .get_backlinks(&vault_path)
            .await
            .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
        let backlink_paths: Vec<String> = backlink_results
            .iter()
            .map(|(e, _)| e.path.to_string())
            .collect();

        match format {
            OutputFormat::Text => {
                let tags = extract_tags(&content);
                let links = extract_links(&content);
                let text = format_note_show_text(
                    &vault_path,
                    &content,
                    &content_data.title,
                    &tags,
                    &links,
                    &backlink_paths,
                );
                text_entries.push(text);
            }
            OutputFormat::Json => {
                let tags = extract_tags(&content);
                let links = extract_links(&content);
                let headers = extract_headers(&content);
                let journal_date = vault
                    .journal_date(&vault_path)
                    .map(|d| d.format("%Y-%m-%d").to_string());
                let path_str = vault_path.to_string();
                let path_with_ext = if path_str.ends_with(".md") {
                    path_str.clone()
                } else {
                    format!("{}.md", path_str)
                };
                json_entries.push(JsonNoteEntry {
                    path: path_with_ext,
                    title: content_data.title.clone(),
                    content: content.clone(),
                    size: entry_data.size,
                    modified: entry_data.modified_secs,
                    created: entry_data.modified_secs,
                    hash: format!("{:x}", content_data.hash),
                    journal_date,
                    metadata: JsonNoteMetadata { tags, links, headers },
                    backlinks: if backlink_paths.is_empty() {
                        None
                    } else {
                        Some(backlink_paths)
                    },
                });
            }
        }
    }

    if text_entries.is_empty() && json_entries.is_empty() {
        return Err(color_eyre::eyre::eyre!(
            "No notes found — all specified paths were missing"
        ));
    }

    match format {
        OutputFormat::Text => {
            let sep = format!("\n{}\n\n", NOTE_SEPARATOR);
            print!("{}", text_entries.join(&sep));
        }
        OutputFormat::Json => {
            let output = JsonOutput {
                metadata: JsonOutputMetadata {
                    workspace: workspace_name.to_string(),
                    workspace_path: vault.workspace_path.to_string_lossy().to_string(),
                    total_results: json_entries.len(),
                    query: None,
                    is_listing: false,
                    generated_at: Utc::now().to_rfc3339(),
                },
                notes: json_entries,
            };
            print!(
                "{}",
                serde_json::to_string(&output)
                    .map_err(|e| color_eyre::eyre::eyre!("{}", e))?
            );
        }
    }

    if had_errors {
        return Err(color_eyre::eyre::eyre!("One or more notes could not be found"));
    }

    Ok(())
}
```

- [ ] **Step 4: Run the new tests**

```bash
cargo test -p kimun-notes note_show 2>&1
```

Expected: all 5 `test_note_show_*` tests pass.

- [ ] **Step 5: Run the full test suite**

```bash
cargo test -p kimun-notes 2>&1 | tail -30
```

Expected: all tests pass (doc-test for `config_dir` will be ignored, that is pre-existing).

- [ ] **Step 6: Commit**

```bash
git add tui/src/cli/commands/note_ops.rs
git commit -m "feat: implement note show command (text and json output)"
```

---

## Done

At this point `kimun note show <path> [<path>...] [--format text|json]` is fully implemented. Verify manually:

```bash
cargo run -p kimun-notes -- note show /index
cargo run -p kimun-notes -- note show /index --format json
cargo run -p kimun-notes -- note show /index /does-not-exist
```
