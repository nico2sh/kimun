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

Stripping `.md`:

```rust
let bare = path.strip_suffix(".md").unwrap_or(&path);
println!("{}", bare);
```

### Affected commands

Both `search.rs` and `notes.rs` gain a third match arm for `OutputFormat::Paths`. The arm iterates `results` and prints bare paths. No other logic changes in those files.

`note show` does **not** gain a `Paths` arm — it is a reader, not a lister.

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

### Runtime resolution in `run_show`

At the start of `run_show`, before the main loop, resolve the effective path list:

```rust
use std::io::IsTerminal;

let path_inputs: Vec<String> = if !paths.is_empty() {
    paths
} else if !std::io::stdin().is_terminal() {
    // read one path per line from stdin
    use std::io::BufRead;
    std::io::stdin()
        .lock()
        .lines()
        .filter_map(|l| l.ok())
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect()
} else {
    return Err(color_eyre::eyre::eyre!(
        "No paths provided — pass paths as arguments or pipe from stdin"
    ));
};
```

Blank lines in stdin input are silently skipped. Invalid paths continue to stderr with `had_errors = true` (existing behavior).

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
| No args, stdin is a pipe, all lines blank | Error: "No notes found — all specified paths were missing" (existing check) |
| Some paths not found | Those paths go to stderr; rest output normally; exit non-zero |
| `--format paths` with zero results | No output, exit zero (consistent with `text` and `json` on empty results) |

---

## Testing

### `OutputFormat::Paths`

- `test_search_paths_format_returns_bare_paths` — search returns results, `--format paths` emits one line per result, no `.md` suffix
- `test_notes_paths_format_returns_bare_paths` — notes list, same check
- `test_paths_format_empty_results` — zero results produces no output

### stdin piping for `note show`

- `test_note_show_reads_paths_from_stdin` — simulate piped stdin with multiple paths, verify notes are shown
- `test_note_show_no_args_no_stdin_errors` — no args + tty stdin → error message
- Existing `test_note_show_*` tests continue to pass (argument-based invocation unchanged)

---

## Files changed

| File | Change |
|------|--------|
| `tui/src/cli/output.rs` | Add `Paths` variant to `OutputFormat` |
| `tui/src/cli/commands/search.rs` | Add `OutputFormat::Paths` arm |
| `tui/src/cli/commands/notes.rs` | Add `OutputFormat::Paths` arm |
| `tui/src/cli/commands/note_ops.rs` | Remove `#[arg(required = true)]`; add stdin resolution block in `run_show` |
| `tui/tests/note_commands_test.rs` | Add stdin and paths-format tests |
| `tui/tests/` (search/notes test files) | Add paths-format tests |
