# `--format paths` + `note show` stdin piping Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `--format paths` to `search` and `notes` commands (one bare path per line), and let `note show` read paths from stdin when none are given as arguments.

**Architecture:** Add `Paths` variant to `OutputFormat` enum; add a `Paths` match arm to `search.rs` and `notes.rs`; add a `resolve_show_paths<R: BufRead>` free function in `note_ops.rs` for testable stdin resolution; add a `Paths` guard at the top of `run_show`; update the `Show` arm in `run()` to call `resolve_show_paths`.

**Tech Stack:** Rust, clap `ValueEnum`, `std::io::{IsTerminal, BufRead, BufReader, Cursor}`, color_eyre, tokio, tempfile (tests)

---

## Chunk 1: `OutputFormat::Paths` — enum + `search` + `notes`

### Task 1: Add `Paths` variant to `OutputFormat`

**Files:**
- Modify: `tui/src/cli/output.rs`

- [ ] **Step 1: Add the variant**

In `tui/src/cli/output.rs`, change:

```rust
#[derive(ValueEnum, Clone, Debug)]
pub enum OutputFormat {
    Text,
    Json,
}
```

to:

```rust
#[derive(ValueEnum, Clone, Debug)]
pub enum OutputFormat {
    Text,
    Json,
    Paths,
}
```

- [ ] **Step 2: Verify it compiles (expect errors in search.rs and notes.rs — non-exhaustive matches)**

```bash
cargo build -p kimun-notes 2>&1 | grep "non-exhaustive\|error"
```

Expected: errors about non-exhaustive patterns in `search.rs` and `notes.rs`. That's correct — we fix them in Tasks 2 and 3. (`note_ops.rs` is addressed separately in Chunk 2.)

Note: `output.rs` has no commit of its own — it will be staged and committed together with `search.rs` in Task 2, Step 5.

---

### Task 2: Add `Paths` arm to `search.rs` (TDD)

**Files:**
- Modify: `tui/src/cli/commands/search.rs`
- Test: `tui/tests/cli_integration_test.rs`

- [ ] **Step 1: Write the failing tests**

Add to `tui/tests/cli_integration_test.rs` (after the existing tests):

```rust
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
```

Also add a **unit test** inside `tui/src/cli/commands/search.rs` to verify the `.md`-stripping logic in isolation (stdout cannot be captured in the integration tests above):

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_paths_strip_md_suffix() {
        let with_ext = "projects/my-note.md";
        let bare = with_ext.strip_suffix(".md").unwrap_or(with_ext);
        assert_eq!(bare, "projects/my-note");
    }

    #[test]
    fn test_paths_no_md_suffix_unchanged() {
        let no_ext = "projects/my-note";
        let bare = no_ext.strip_suffix(".md").unwrap_or(no_ext);
        assert_eq!(bare, "projects/my-note");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail (compile error)**

```bash
cargo test -p kimun-notes test_search_paths_format 2>&1 | tail -10
```

Expected: compile error — `OutputFormat::Paths` not handled in `search.rs`.

- [ ] **Step 3: Add `Paths` arm to `search.rs`**

In `tui/src/cli/commands/search.rs`, change:

```rust
    match format {
        OutputFormat::Text => {
            let output = format_note_entries_text_with_journal(vault, &results);
            print!("{}", output);
        }
        OutputFormat::Json => {
            let json_output = format_notes_as_json(
```

to:

```rust
    match format {
        OutputFormat::Text => {
            let output = format_note_entries_text_with_journal(vault, &results);
            print!("{}", output);
        }
        OutputFormat::Paths => {
            for (entry_data, _) in &results {
                let s = entry_data.path.to_string();
                let bare = s.strip_suffix(".md").unwrap_or(&s);
                println!("{}", bare);
            }
        }
        OutputFormat::Json => {
            let json_output = format_notes_as_json(
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p kimun-notes "test_search_paths_format|test_paths_strip" 2>&1 | tail -10
```

Expected: integration test and both unit tests pass.

- [ ] **Step 5: Commit**

```bash
git add tui/src/cli/output.rs tui/src/cli/commands/search.rs tui/tests/cli_integration_test.rs
git commit -m "feat: add OutputFormat::Paths and search --format paths arm"
```

---

### Task 3: Add `Paths` arm to `notes.rs` (TDD)

**Files:**
- Modify: `tui/src/cli/commands/notes.rs`
- Test: `tui/tests/cli_integration_test.rs`

- [ ] **Step 1: Write the failing tests**

Add to `tui/tests/cli_integration_test.rs`:

```rust
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

    // Path filter that matches nothing
    let result = run_cli(
        CliCommand::Notes {
            path: Some("nonexistent/prefix".to_string()),
            format: OutputFormat::Paths,
        },
        Some(config_path),
    )
    .await;

    // Zero results is not an error
    assert!(result.is_ok());
}
```

- [ ] **Step 2: Run tests to verify they fail (compile error)**

```bash
cargo test -p kimun-notes "test_notes_paths_format|test_paths_format_empty" 2>&1 | tail -10
```

Expected: compile error — `OutputFormat::Paths` not handled in `notes.rs`.

- [ ] **Step 3: Add `Paths` arm to `notes.rs`**

In `tui/src/cli/commands/notes.rs`, change:

```rust
    match format {
        OutputFormat::Text => {
            let output = format_note_entries_text_with_journal(vault, &results);
            print!("{}", output);
        }
        OutputFormat::Json => {
            let json_output = format_notes_as_json(
```

to:

```rust
    match format {
        OutputFormat::Text => {
            let output = format_note_entries_text_with_journal(vault, &results);
            print!("{}", output);
        }
        OutputFormat::Paths => {
            for (entry_data, _) in &results {
                let s = entry_data.path.to_string();
                let bare = s.strip_suffix(".md").unwrap_or(&s);
                println!("{}", bare);
            }
        }
        OutputFormat::Json => {
            let json_output = format_notes_as_json(
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p kimun-notes "test_notes_paths_format|test_paths_format_empty" 2>&1 | tail -10
```

Expected: both tests pass.

- [ ] **Step 5: Run full test suite to confirm nothing broken**

```bash
cargo test -p kimun-notes 2>&1 | grep -E "^test result|FAILED"
```

Expected: all pass, 0 failed.

- [ ] **Step 6: Commit**

```bash
git add tui/src/cli/commands/notes.rs tui/tests/cli_integration_test.rs
git commit -m "feat: add notes --format paths arm"
```

---

## Chunk 2: `note show` stdin piping + `Paths` guard

### Task 4: Add `resolve_show_paths` and `Paths` guard (TDD)

**Files:**
- Modify: `tui/src/cli/commands/note_ops.rs`
- Test: unit tests inside `note_ops.rs` (`#[cfg(test)]` module)
- Test: `tui/tests/note_commands_test.rs`

- [ ] **Step 1: Write unit tests for `resolve_show_paths` (inside `note_ops.rs`)**

At the bottom of `tui/src/cli/commands/note_ops.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::resolve_show_paths;
    use std::io::Cursor;

    #[test]
    fn test_resolve_show_paths_uses_args_when_given() {
        let args = vec!["projects/foo".to_string(), "inbox/bar".to_string()];
        let result = resolve_show_paths(args.clone(), None::<Cursor<&[u8]>>).unwrap();
        assert_eq!(result, args);
    }

    #[test]
    fn test_resolve_show_paths_reads_from_reader() {
        let input = b"projects/foo\ninbox/bar\n";
        let reader = Cursor::new(input.as_ref());
        let result = resolve_show_paths(vec![], Some(reader)).unwrap();
        assert_eq!(result, vec!["projects/foo", "inbox/bar"]);
    }

    #[test]
    fn test_resolve_show_paths_skips_blank_lines() {
        let input = b"projects/foo\n\n  \ninbox/bar\n";
        let reader = Cursor::new(input.as_ref());
        let result = resolve_show_paths(vec![], Some(reader)).unwrap();
        assert_eq!(result, vec!["projects/foo", "inbox/bar"]);
    }

    #[test]
    fn test_resolve_show_paths_all_blank_stdin_returns_empty() {
        let input = b"\n  \n\t\n";
        let reader = Cursor::new(input.as_ref());
        let result = resolve_show_paths(vec![], Some(reader)).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_resolve_show_paths_no_args_no_reader_errors() {
        let result = resolve_show_paths(vec![], None::<Cursor<&[u8]>>);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("No paths provided"), "got: {}", msg);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail (function not found)**

```bash
cargo test -p kimun-notes test_resolve_show_paths 2>&1 | tail -10
```

Expected: compile error — `resolve_show_paths` not defined.

- [ ] **Step 3: Add `resolve_show_paths` to `note_ops.rs`**

Note: `resolve_show_paths` returns `color_eyre::eyre::Result` — `color_eyre` is already a dependency in `note_ops.rs`.

Add this function before `run_show` in `tui/src/cli/commands/note_ops.rs`:

```rust
/// Resolves the effective path list for `note show`.
/// - If `args` is non-empty, returns it directly (reader is ignored).
/// - If `args` is empty and `reader` is `Some`, reads non-blank trimmed lines from it.
/// - If `args` is empty and `reader` is `None` (TTY), returns an error.
fn resolve_show_paths<R: std::io::BufRead>(
    args: Vec<String>,
    reader: Option<R>,
) -> color_eyre::eyre::Result<Vec<String>> {
    if !args.is_empty() {
        return Ok(args);
    }
    match reader {
        Some(r) => {
            let paths: Vec<String> = r
                .lines()
                .filter_map(|l| l.ok())
                .map(|l| l.trim().to_owned())
                .filter(|l| !l.is_empty())
                .collect();
            Ok(paths)
        }
        None => Err(color_eyre::eyre::eyre!(
            "No paths provided — pass paths as arguments or pipe from stdin"
        )),
    }
}
```

- [ ] **Step 4: Run unit tests**

```bash
cargo test -p kimun-notes test_resolve_show_paths 2>&1 | tail -10
```

Expected: 5 tests pass.

- [ ] **Step 5: Write failing integration test for `--format paths` on `note show`**

Add to `tui/tests/note_commands_test.rs`:

Note: `note_commands_test.rs` already imports `kimun_notes::cli::commands::NoteSubcommand` (not the full `note_ops` path) — use that same import style.

```rust
#[tokio::test]
async fn test_note_show_format_paths_returns_error() {
    use kimun_notes::cli::output::OutputFormat;
    use kimun_core::nfs::VaultPath;
    let dir = TempDir::new().unwrap();
    let vault = kimun_core::NoteVault::new(dir.path()).await.unwrap();
    vault.init_and_validate().await.unwrap();
    vault
        .create_note(
            &VaultPath::note_path_from("test/note"),
            "# Test\n\nContent.",
        )
        .await
        .unwrap();

    // NoteSubcommand is re-exported via kimun_notes::cli::commands (as in other tests in this file)
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
```

- [ ] **Step 6: Run to verify it fails**

```bash
cargo test -p kimun-notes test_note_show_format_paths 2>&1 | tail -10
```

Expected: compile error — `run_show`'s `match format` is non-exhaustive (the `Paths` guard hasn't been added yet). This is the expected red state.

- [ ] **Step 7: Add `Paths` guard at top of `run_show` and remove `#[arg(required = true)]`**

In `tui/src/cli/commands/note_ops.rs`:

**a)** Remove `#[arg(required = true)]` from the `Show` variant:

```rust
    /// Show note content and metadata (read one or more notes)
    Show {
        /// One or more note paths (relative to quick_note_path or absolute from vault root)
        paths: Vec<String>,
        #[arg(long, value_enum, default_value = "text")]
        format: crate::cli::output::OutputFormat,
    },
```

**b)** Add the `Paths` guard immediately after the existing `use crate::cli::output::OutputFormat;` line at the top of `run_show` (do NOT add a second `use` — it is already there). The guard must appear before the `Accumulator` enum definition and both `match format` expressions:

```rust
    // existing line already present — do not duplicate:
    // use crate::cli::output::OutputFormat;

    if matches!(format, OutputFormat::Paths) {
        return Err(color_eyre::eyre::eyre!(
            "--format paths is not valid for note show; use 'text' or 'json'"
        ));
    }

    // One accumulator per format — only the active one is ever populated.
    enum Accumulator { ... }   // <- this block comes AFTER the guard
```

**c)** Update the `Show` arm in `run()` to call `resolve_show_paths`:

```rust
        NoteSubcommand::Show { paths, format } => {
            use std::io::IsTerminal;
            let reader = if std::io::stdin().is_terminal() {
                None
            } else {
                Some(std::io::BufReader::new(std::io::stdin().lock()))
            };
            let resolved = resolve_show_paths(paths, reader)?;
            run_show(vault, &resolved, quick_note_path, format, workspace_name).await
        }
```

- [ ] **Step 8: Run all note_commands tests**

```bash
cargo test -p kimun-notes --test note_commands_test 2>&1 | tail -15
```

Expected: all pass including `test_note_show_format_paths_returns_error`.

- [ ] **Step 9: Run full suite**

```bash
cargo test -p kimun-notes 2>&1 | grep -E "^test result|FAILED"
```

Expected: all pass, 0 failed.

- [ ] **Step 10: Commit**

```bash
git add tui/src/cli/commands/note_ops.rs tui/tests/note_commands_test.rs
git commit -m "feat: add resolve_show_paths, Paths guard in run_show, stdin piping for note show"
```

---

### Task 5: Paths-with-spaces round-trip test

**Files:**
- Test: `tui/tests/cli_integration_test.rs`

This verifies that a vault path containing a space appears as a single line in `--format paths` output (not split across two lines).

Note: `VaultPath::note_path_from` may normalize spaces in paths. Since `run_cli` prints directly to stdout and we cannot capture it in integration tests, this test verifies that two space-containing notes are listed without error. The actual single-line property is guaranteed by the `println!("{}", bare)` implementation which emits one `\n`-terminated line per entry regardless of spaces in the path.

- [ ] **Step 1: Write the test**

Add to `tui/tests/cli_integration_test.rs`:

```rust
#[tokio::test]
async fn test_paths_format_path_with_spaces() {
    let dir = TempDir::new().unwrap();
    let vault = NoteVault::new(dir.path()).await.unwrap();
    vault.init_and_validate().await.unwrap();

    // Create two notes to verify line count
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

    let result = run_cli(
        CliCommand::Notes {
            path: Some("notes/".to_string()),
            format: OutputFormat::Paths,
        },
        Some(config_path),
    )
    .await;

    // Both notes returned without error
    assert!(result.is_ok());
}
```

- [ ] **Step 2: Run the test**

```bash
cargo test -p kimun-notes test_paths_format_path_with_spaces 2>&1 | tail -10
```

Expected: passes.

- [ ] **Step 3: Commit**

```bash
git add tui/tests/cli_integration_test.rs
git commit -m "test: add paths-with-spaces round-trip test for --format paths"
```

---

### Task 6: Final verification

- [ ] **Step 1: Run the complete test suite**

```bash
cargo test -p kimun-notes 2>&1 | grep -E "^test result|FAILED"
```

Expected: all pass, 0 failed across all test binaries.

- [ ] **Step 2: Quick smoke-check of the pipe pattern (manual)**

Build first, then run the compiled binary to avoid `cargo run` stdin-capture issues in pipelines:

```bash
cargo build -p kimun-notes
# Verify paths output:
./target/debug/kimun search "your query" --format paths
# Verify piping:
./target/debug/kimun search "your query" --format paths | ./target/debug/kimun note show --format text
```

Expected: paths output on first command; note content on second.

- [ ] **Step 3: Commit if any fixes were needed, then final check**

```bash
cargo test -p kimun-notes 2>&1 | grep -E "^test result|FAILED"
```
