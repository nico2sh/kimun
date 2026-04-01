---
name: kimun-cli
description: Use when the user has kimun installed and wants to create, append, search, or read notes from the terminal, or when automating note management as part of a workflow or agent task
---

# Kimün CLI Reference

Kimün is a local-first, terminal notes app. Notes are plain Markdown files indexed for fast search. Use the CLI to write, append, search, and read notes without opening the TUI — ideal for automation, piping, and AI-driven workflows.

## Quick Reference

| Task | Command |
|------|---------|
| Create note | `kimun note create "path" "content"` |
| Append to note | `kimun note append "path" "content"` |
| Log to today's journal | `kimun note journal "text"` |
| Show note content | `kimun note show "path"` |
| Search notes | `kimun search "query"` |
| List notes | `kimun notes` |

## Writing Notes

All write commands accept content as a second argument **or from stdin** (when stdin is not a TTY).

### Create
Creates a new note. **Fails if the note already exists** — use `append` if unsure.

```sh
kimun note create "inbox/idea" "My content"
echo "My content" | kimun note create "inbox/idea"
```

### Append
Appends to a note. **Creates it automatically if it doesn't exist.** Safe to use as the default write command.

```sh
kimun note append "inbox/log" "New entry"
echo "Entry" | kimun note append "inbox/log"
```

### Journal
Appends to today's journal (`journal/YYYY-MM-DD.md`). No path argument — always targets today.

```sh
kimun note journal "Captured this finding"
echo "$(date +%H:%M) — task complete" | kimun note journal
```

## Path Rules

- `"inbox/note"` — relative to the configured `quick_note_path` (default: vault root)
- `"/inbox/note"` — absolute from vault root; use a leading `/` to be explicit
- `.md` extension is optional in all commands

## Searching

```sh
kimun search "query"                     # Full-text search
kimun search "meeting -cancelled"        # Exclude a term
kimun search "@filename-fragment"        # Filter by filename
kimun search ">section-name query"       # Search within a Markdown section
kimun search "query" --format json       # Structured JSON output
kimun search "query" --format paths      # Bare paths, one per line (pipeable)
```

Query syntax summary: free text is case-insensitive with `*` wildcard; `@` filters by filename; `>` filters by section heading; `-` negates any term.

## Reading Notes

```sh
kimun note show "path/to/note"           # Display note content and metadata
kimun notes                              # List all notes
kimun notes --path "journal/"            # Filter by path prefix
kimun notes --format json                # JSON output
kimun notes --format paths               # Bare paths for piping
```

## JSON Output

`search` and `notes` both support `--format json`. Each note object includes:
`path`, `title`, `content`, `size`, `modified`, `journal_date`, `metadata.tags`, `metadata.links`, `metadata.headers`.

```sh
# Get titles and paths
kimun notes --format json | jq '.notes[] | {title, path}'

# Find all journal entries
kimun notes --format json | jq '.notes[] | select(.journal_date != null)'

# Filter by tag
kimun search "project" --format json | jq '.notes[] | select(.metadata.tags[] == "active")'

# Pipe search results directly into note show
kimun search "todo" --format paths | kimun note show
```

## Patterns for AI Workflows

```sh
# Log a research finding to today's journal
kimun note journal "Finding: X is faster than Y for large datasets"

# Capture command output as a timestamped note
some-command | kimun note create "logs/output-$(date +%F)"

# Append a running log (safe: creates if missing)
echo "$(date): step completed" | kimun note append "logs/progress"

# Search for context before starting a task
kimun search "authentication" --format json | jq '.notes[] | {title, path}'

# Read a specific note for context
kimun note show "projects/roadmap"

# Chain: search → read all matching notes
kimun search "meeting notes" --format paths | kimun note show
```

## Common Mistakes

- **`create` on an existing note** — it fails. Use `append` when you're not sure if the note exists.
- **No stdin from a live terminal** — piping works (`echo "x" | kimun note journal`); passing no content from an interactive terminal produces an empty write.
- **Relative vs absolute paths** — if a `quick_note_path` is set in `kimun_config.toml`, relative paths are resolved against it. Prefix with `/` to always target the vault root explicitly.
