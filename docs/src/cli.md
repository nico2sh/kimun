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

For full workspace management reference, see the [Workspaces](./workspaces.md) page.

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

- `--format json` — Output as JSON (default: text). Useful for scripting with `jq`.
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

For comprehensive search documentation, see the [Search](./search.md) page.

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
- `--format json` — Output as JSON (default: text).

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

# Pipe from find or other tools
find ./notes -name "*.md" | kimun note show

# Show as JSON
kimun note show "inbox/meeting" --format json
```

## Note Operations

### Create

Create a new note (fails if it already exists):

```sh
kimun note create "path/to/note" "Initial content"
echo "My content" | kimun note create "path/to/note"
```

### Append

Append text to a note (creates it if it doesn't exist):

```sh
kimun note append "path/to/note" "Appended text"
echo "New line" | kimun note append "path/to/note"
```

### Journal

Append to today's journal entry (creates it if needed):

```sh
kimun note journal "Today's entry"
echo "Event happened" | kimun note journal
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

For scripting guides, see [Scripting](./guides/scripting.md).
