# Spec: `--format paths` + stdin piping for `note show`

**Date:** 2026-03-27
**Status:** Approved

---

## Overview

Two related changes:

1. Add a `Paths` variant to `OutputFormat` so `kimun search` and `kimun notes` can emit one bare vault path per line.
2. Allow `kimun note show` to read paths from stdin when none are provided as arguments, enabling direct piping without `xargs`.

Together these enable:

```sh
kimun search "rust" --format paths | kimun note show --format json
kimun notes --format paths | kimun note show
```

---

## 1. `OutputFormat::Paths`

### Change

Add `Paths` to the `OutputFormat` enum in `tui/src/cli/output.rs`:

```rust
#[derive(ValueEnum, Clone, Debug)]
pub enum OutputFormat {
    Text,
    Json,
    Paths,
}
```

Clap exposes this as `--format paths` automatically via `ValueEnum`.

### Output format

One bare vault path per line, no `.md` suffix, no header, no trailing blank line:

```
projects/rust-notes
journal/2026-03-01
inbox/todo
```

Paths that contain spaces are safe — they appear as a single line and are read back correctly by `lines()`. A test with a space-containing path verifies this.

Stripping `.md` (note: `entry_data.path` is `VaultPath`, requires `.to_string()` first):

```rust
let s = entry_data.path.to_string();
let bare = s.strip_suffix(".md").unwrap_or(&s);
println!("{}", bare);
```

### Affected commands

Both `search.rs` and `notes.rs` gain a third match arm for `OutputFormat::Paths`. The arm iterates `results` and prints bare paths. No other logic changes in those files.

`note show` does **not** gain a `Paths` arm — it is a reader, not a lister. However, adding `Paths` to the enum makes **two** existing matches in `run_show` non-exhaustive:

1. `let mut acc = match format { Text => ..., Json => ... }` — the accumulator initializer
2. `match &mut acc { ... }` — the per-note formatting loop

The `Paths` guard is placed at the **top of `run_show`**, before either match, so both remain two-armed:

```rust
if matches!(format, OutputFormat::Paths) {
    return Err(color_eyre::eyre::eyre!(
        "--format paths is not valid for note show; use 'text' or 'json'"
    ));
}
```

This fires before stdin is consumed, giving a clean error regardless of whether paths were passed as args or piped.

---

## 2. stdin piping for `note show`

### Change

Remove `#[arg(required = true)]` from the `paths` field of `NoteSubcommand::Show`. The field becomes an optional list (defaults to empty `Vec`):

```rust
Show {
    #[arg()]
    paths: Vec<String>,
    #[arg(long, value_enum, default_value = "text")]
    format: OutputFormat,
}
```

### Runtime resolution

The stdin-resolution logic is extracted into a free function `resolve_show_paths` so it can be tested with an injected reader:

```rust
/// Resolves the effective path list for `note show`.
/// - If `args` is non-empty, returns it directly.
/// - If `args` is empty and `reader` yields non-blank lines, returns those.
/// - If `args` is empty and `reader` is None (stdin is a TTY), returns an error.
fn resolve_show_paths<R: BufRead>(
    args: Vec<String>,
    reader: Option<R>,
) -> Result<Vec<String>> {
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

The call site in `run()` passes `Some(stdin().lock())` when stdin is not a terminal, `None` when it is:

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

Tests call `resolve_show_paths` directly with a `std::io::Cursor<&[u8]>` — no process-stdin involvement. `run_show` continues to receive `&[String]` and is unchanged in signature.

Blank lines in stdin input are silently skipped. Invalid paths continue to stderr with `had_errors = true` (existing behavior). The `.unwrap_or` in the bare-path stripping is deliberate — if a vault path has no `.md` suffix, the original string is used as-is.

### Pipe usage

```sh
# text output (default)
kimun search "rust" --format paths | kimun note show

# json output
kimun search "rust" --format paths | kimun note show --format json

# listing all notes, showing as json
kimun notes --format paths | kimun note show --format json
```

---

## Error handling

| Situation | Behavior |
|-----------|----------|
| No args, stdin is a tty | Error: "No paths provided — pass paths as arguments or pipe from stdin" |
| No args, stdin is a pipe, all lines blank | The stdin block produces an empty `Vec`; the existing empty-accumulator check in `run_show` fires: "No notes found — all specified paths were missing" |
| Some paths not found | Those paths go to stderr; rest output normally; exit non-zero |
| `--format paths` with zero results | No output, exit zero (consistent with `text` and `json` on empty results) |
| `kimun note show --format paths` | Error: "--format paths is not valid for note show; use 'text' or 'json'" |

---

## Testing

### `OutputFormat::Paths`

Tests live in `tui/tests/cli_integration_test.rs` (alongside existing search/notes integration tests).

- `test_search_paths_format_returns_bare_paths` — search returns results; `--format paths` emits one line per result, no `.md` suffix
- `test_notes_paths_format_returns_bare_paths` — notes list, same check
- `test_paths_format_path_with_spaces` — a note whose vault path contains a space appears as a single line and round-trips correctly
- `test_paths_format_empty_results` — zero results produces no output

### stdin piping for `note show`

`resolve_show_paths` unit tests (in a `#[cfg(test)]` module in `note_ops.rs`, using `std::io::Cursor`):

- `test_resolve_show_paths_uses_args_when_given` — non-empty `args` → returns args, ignores reader
- `test_resolve_show_paths_reads_from_reader` — empty args + `Cursor` with paths → returns those paths
- `test_resolve_show_paths_skips_blank_lines` — reader with blank lines → blank lines filtered
- `test_resolve_show_paths_no_args_no_reader_errors` — empty args + `None` reader → "No paths provided" error

Integration tests in `tui/tests/note_commands_test.rs` (calling `run_show` directly with pre-resolved paths):

- `test_note_show_format_paths_returns_error` — `--format paths` passed to `run_show` → usage error
- Existing `test_note_show_*` tests continue to pass (argument-based invocation unchanged)

---

## Files changed

| File | Change |
|------|--------|
| `tui/src/cli/output.rs` | Add `Paths` variant to `OutputFormat` |
| `tui/src/cli/commands/search.rs` | Add `OutputFormat::Paths` arm |
| `tui/src/cli/commands/notes.rs` | Add `OutputFormat::Paths` arm |
| `tui/src/cli/commands/note_ops.rs` | Remove `#[arg(required = true)]`; add `resolve_show_paths` free fn; add `Paths` guard at top of `run_show`; update `Show` arm in `run()` to call `resolve_show_paths` |
| `tui/tests/note_commands_test.rs` | Add stdin and paths-format tests |
| `tui/tests/cli_integration_test.rs` | Add paths-format tests for search and notes |
