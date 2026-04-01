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
| Log to today's journal | `kimun journal "text"` |
| Log to a specific date | `kimun journal --date YYYY-MM-DD "text"` |
| Show today's journal | `kimun journal show` |
| Show a specific journal entry | `kimun journal show --date YYYY-MM-DD` |
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

## Journal

`kimun journal` appends to a journal entry (today by default). `kimun journal show` displays one.

```sh
kimun journal "Captured this finding"             # append to today
kimun journal --date 2024-01-15 "Backdated note"  # append to specific date
echo "$(date +%H:%M) — task complete" | kimun journal

kimun journal show                                # show today's entry
kimun journal show --date 2024-01-15             # show specific date
kimun journal show --format json                 # JSON output
```

- Defaults to today; `--date YYYY-MM-DD` targets any date
- Creates the entry if it doesn't exist
- `show` supports `--format text` (default) and `--format json`

## Path Rules

- `"inbox/note"` — relative to the configured `quick_note_path` (default: vault root)
- `"/inbox/note"` — absolute from vault root; use a leading `/` to be explicit
- `.md` extension is optional in all commands

## Searching

```sh
kimun search "query"               # Full-text search
kimun search "query" --format json # JSON output
kimun search "query" --format paths # Bare paths, one per line (pipeable)
```

### Query syntax

Free text is case-insensitive, diacritics-ignored. Multiple terms = AND.

**Wildcards** — `*` matches any sequence of characters, works in free text and with operators:

```
kimu*          # starts with "kimu"
*report        # ends with "report"
*meeting*      # contains "meeting" anywhere
@task*         # filename starts with "task"
@*2024*        # filename contains "2024"
```

| Operator | Long form | Matches |
|----------|-----------|---------|
| `@term` | `at:term` | filename contains term |
| `>term` | `in:term` | Markdown section heading contains term |
| `/term` | `pt:term` | note path (directory) contains term |
| `-term` | | exclude notes containing term |

**Exclusion composes with all operators:**

```
-cancelled           # exclude notes containing "cancelled"
@-temp               # exclude notes with "temp" in filename
>-draft              # exclude notes with "draft" in any section title
/-private            # exclude notes under a "private/" path
```

**Combining filters** (all terms are ANDed):

```
meeting -cancelled           # "meeting" but not "cancelled"
@tasks >work report          # file "tasks", has "Work" section, contains "report"
@2024 >-draft                # files from 2024, no "draft" section title
/journal >tasks -done        # in journal/, "tasks" section, excluding "done"
>personal kimun              # "kimun" under a "Personal" section
```

## Reading Notes

```sh
kimun note show "path/to/note"           # Display note content and metadata
kimun notes                              # List all notes
kimun notes --path "journal/"            # Filter by path prefix
kimun notes --format json                # JSON output
kimun notes --format paths               # Bare paths for piping
```

## JSON Output

`search`, `notes`, `note show`, and `journal show` all support `--format json`. Each note object includes:
`path`, `title`, `content`, `size`, `modified`, `journal_date`, `metadata.tags`, `metadata.links`, `metadata.headers`.

```sh
# Get titles and paths
kimun notes --format json | jq '.notes[] | {title, path}'

# Find all journal entries
kimun notes --format json | jq '.notes[] | select(.journal_date != null)'

# Filter by tag
kimun search "project" --format json | jq '.notes[] | select(.metadata.tags[] == "active")'

# Get today's journal headings
kimun journal show --format json | jq '.notes[0].metadata.headers[].text'

# Pipe search results directly into note show
kimun search "todo" --format paths | kimun note show
```

## Patterns for AI Workflows

```sh
# Log a research finding to today's journal
kimun journal "Finding: X is faster than Y for large datasets"

# Capture command output as a timestamped note
some-command | kimun note create "logs/output-$(date +%F)"

# Append a running log (safe: creates if missing)
echo "$(date): step completed" | kimun note append "logs/progress"

# Search for context before starting a task
kimun search "authentication" --format json | jq '.notes[] | {title, path}'

# Read a specific note for context
kimun note show "projects/roadmap"

# Read today's journal for context
kimun journal show

# Chain: search → read all matching notes
kimun search "meeting notes" --format paths | kimun note show
```

## Common Mistakes

- **`create` on an existing note** — it fails. Use `append` when you're not sure if the note exists.
- **No stdin from a live terminal** — piping works (`echo "x" | kimun journal`); passing no content from an interactive terminal produces an empty write.
- **Relative vs absolute paths** — if a `quick_note_path` is set in `kimun_config.toml`, relative paths are resolved against it. Prefix with `/` to always target the vault root explicitly.
- **`kimun journal show` — `--format paths` is not supported**; use `text` or `json`.
