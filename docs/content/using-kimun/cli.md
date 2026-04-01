+++
title = "CLI"
weight = 11
+++

# CLI

Kimün provides a powerful command-line interface for quick operations, multi-workspace management, and scripting. All CLI commands support both the TUI and background use.

## Global Configuration

Pass a custom config file to any command:

```sh
kimun --config /path/to/config.toml <subcommand>
```

Without `--config`, Kimün uses the default location:
- **Linux / macOS:** `~/.config/kimun/kimun_config.toml`
- **Windows:** `%USERPROFILE%\kimun\kimun_config.toml`

## Initial Setup

On first use, choose one of these approaches:

**Option A — CLI First:**
```sh
kimun workspace init --name default /path/to/notes
```

**Option B — TUI First:**
```sh
kimun
```
Then configure your workspace through the Settings screen.

Legacy single-workspace configurations are automatically migrated to multi-workspace format.

## Workspaces

Multi-workspace support allows you to manage separate note directories.

For full workspace management reference, see the [Workspaces](@/getting-started/workspaces.md) page.

**Quick reference:**
```sh
kimun workspace init --name work /path/to/work/notes
kimun workspace list
kimun workspace use work
kimun workspace rename old-name new-name
kimun workspace remove work
kimun workspace reindex work
```

## Search

Search notes in the current workspace:

```sh
kimun search "your search query"
kimun search "meeting -cancelled"                # Exclude terms
kimun search "@project >-draft"                  # Combine filters
kimun search "rust" --format json                # JSON output
```

### Flags

- `--format json` — Output as JSON. Useful for scripting with `jq`.
- `--format paths` — Output bare paths only (one per line). Ideal for piping into `kimun note show` or `fzf`.
- `--workspace <name>` — Search a specific workspace (if applicable).

### Query Syntax

Searches support free text, filters, and operators:

- **Free text:** Case-insensitive, diacritics ignored, `*` wildcard supported
- **Filter by filename:** `@tasks` or `at:tasks`
- **Filter by section:** `>personal` or `in:personal` (Markdown headers)
- **Exclusion:** `-term` to exclude from results
  - Content: `meeting -cancelled`
  - Title: `>-draft` or `in:-draft`
  - Filename: `@-temp` or `at:-temp`

For comprehensive search documentation, see the [Search](@/using-kimun/search.md) page.

### Examples

```sh
# Find notes about "rust"
kimun search "rust"

# Find in files matching "2024" but exclude "draft" in title
kimun search "@2024 >-draft"

# Find content under "Personal" section containing "kimun"
kimun search ">personal kimun"

# Combine with jq for advanced filtering
kimun search "rust" --format json | jq '.notes[] | select(.metadata.tags[] == "rust")'
```

## Notes

List all notes in the current workspace:

```sh
kimun notes
kimun notes --path "journal/"                    # Filter by path prefix
kimun notes --format json                        # JSON output
```

### Flags

- `--path <prefix>` — Filter notes by path prefix (e.g., `journal/`, `projects/`).
- `--format json` — Output as JSON. Useful for scripting with `jq`.
- `--format paths` — Output bare paths only (one per line). Ideal for piping into `kimun note show` or `fzf`.

### Examples

```sh
# List all notes
kimun notes

# List only journal entries
kimun notes --path "journal/"

# Get titles and paths as JSON
kimun notes --format json | jq '.notes[] | {title, path}'

# Count notes by workspace
kimun notes --format json | jq '.metadata | {workspace, total_results}'
```

## Show

Display note content and metadata in the terminal:

```sh
kimun note show "path/to/note"
kimun note show "path/to/note" "another/note"   # Multiple notes
```

### Flags

- `--format json` — Output as JSON (default: text).

### Features

- Accepts note paths relative to workspace root
- Paths work with or without `.md` extension
- Reads from stdin for batch processing
- Displays content, title, tags, links, and backlinks

### Examples

```sh
# Show a single note
kimun note show "inbox/meeting-notes"

# Show multiple notes
kimun note show "projects/foo" "inbox/bar"

# Read paths from stdin (one per line)
echo "journal/2024-01-01" | kimun note show

# Pipe paths from search results
kimun search "rust" --format paths | kimun note show

# Show as JSON
kimun note show "inbox/meeting" --format json
```

## Note Operations

### Create

Create a new note. Fails with an error if the note already exists.

```sh
kimun note create "path/to/note" "Initial content"
echo "My content" | kimun note create "path/to/note"
```

### Features

- Accepts content as a second argument or from stdin (when stdin is not a TTY)
- Paths are relative to the configured `quick_note_path`, or absolute from the vault root when prefixed with `/`
- Prints `Note saved: <path>` on success

### Examples

```sh
# Create a note with inline content
kimun note create "inbox/idea" "Use kimun for daily notes"

# Create a note at an absolute vault path
kimun note create "/projects/roadmap" "Q3 goals"

# Pipe content from a command
date | kimun note create "inbox/timestamp"

# Capture command output into a new note
curl -s https://example.com/api/status | kimun note create "inbox/status-check"

# Create from a here-string
kimun note create "inbox/snippet" <<'EOF'
## Snippet

Some important code or text to save.
EOF
```

### Append

Append text to an existing note. Creates the note if it does not exist.

```sh
kimun note append "path/to/note" "Appended text"
echo "New line" | kimun note append "path/to/note"
```

### Features

- Accepts content as a second argument or from stdin (when stdin is not a TTY)
- If the note does not exist, it is created automatically
- New content is joined with a newline after the existing content
- Prints `Note saved: <path>` on success

### Examples

```sh
# Append a quick thought to an existing note
kimun note append "inbox/ideas" "Another idea just came to me"

# Log the output of a command to a running log note
date >> /dev/null; echo "$(date): build succeeded" | kimun note append "logs/build-log"

# Accumulate cron job output
0 * * * * kimun note append "logs/hourly" "$(date): checked in"

# Append multiline content
kimun note append "inbox/research" <<'EOF'

## New finding

Something worth noting from today's reading.
EOF

# Use with search results: append a summary of found notes to a log
kimun search "rust" --format paths | kimun note append "inbox/rust-refs"
```

### Journal

Append text to today's journal entry (`journal/YYYY-MM-DD.md`). Creates the entry and the `journal/` directory if they do not exist.

```sh
kimun note journal "Today's entry"
echo "Event happened" | kimun note journal
```

### Features

- No path argument — always targets today's date automatically
- Creates `journal/YYYY-MM-DD.md` in the vault if it does not exist
- Accepts content as an argument or from stdin (when stdin is not a TTY)
- New content is joined with a newline after any existing content
- Prints `Note saved: <path>` on success

### Examples

```sh
# Capture a quick thought
kimun note journal "Had a good retro today"

# Pipe in a timestamped log line
echo "$(date +%H:%M) — finished the auth refactor" | kimun note journal

# Record the result of a script
./run-tests.sh | tail -1 | kimun note journal

# Append a longer entry with a here-string
kimun note journal <<'EOF'

## Evening review

- Completed the CLI documentation
- Reviewed two PRs
- TODO: follow up with team on deploy schedule
EOF

# Use in a cron job to log system info daily
@daily kimun note journal "$(hostname): $(uptime)"

# Chain with other commands — log search activity
kimun search "todo" --format paths | xargs -I{} echo "open: {}" | kimun note journal
```

## JSON Output

Both `search` and `notes` support JSON output for scripting and automation.

### Output Structure

```json
{
  "metadata": {
    "workspace": "default",
    "workspace_path": "/home/user/notes",
    "total_results": 5,
    "query": "rust",
    "is_listing": false,
    "generated_at": "2024-03-27T10:30:00Z"
  },
  "notes": [
    {
      "path": "projects/rust-cli.md",
      "title": "Rust CLI Project",
      "content": "...",
      "size": 1024,
      "modified": 1711525800,
      "created": 1711525800,
      "hash": "abc123def",
      "journal_date": null,
      "metadata": {
        "tags": ["rust", "cli"],
        "links": ["projects/parser", "projects/lexer"],
        "headers": ["Overview", "Architecture", "TODO"]
      },
      "backlinks": ["blog/rust-post.md"]
    }
  ]
}
```

### Processing with jq

```sh
# Extract all tags from search results
kimun search "project" --format json | jq '.notes[].metadata.tags[]'

# Find notes modified in the last 7 days
kimun notes --format json | jq '.notes[] | select(.modified > now - 604800)'

# Get path and title only
kimun notes --format json | jq '.notes[] | {path, title}'

# Count total notes
kimun notes --format json | jq '.metadata.total_results'
```

For scripting guides, see [Scripting](@/guides/scripting.md).
