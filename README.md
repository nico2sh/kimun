# Kimün

A terminal-based notes app focused on simplicity and powerful search.

Notes are plain Markdown files stored in a directory you own. Kimün indexes them into a local SQLite database for fast full-text and structured search.

> Small disclaimer: Although by no means this has been vibe coded — the core has been written manually — there is a good chunk of AI-assisted code (using Claude) with manual reviews. Mostly for tedious refactors, data structures I'm too lazy to code myself, but also to help me building the foundations of more complex stuff, especially on the UI side. Use AI as a tool, not as a replacement.

## Installation

```sh
cargo install kimun-notes
```

On first launch, Kimün will open the Settings screen so you can set your notes directory. It will create a `kimun.sqlite` file in the root of your notes directory.

## Configuration

The config file is created automatically on first run:

- **Linux / macOS:** `~/.config/kimun/kimun_config.toml`
- **Windows:** `%USERPROFILE%\kimun\kimun_config.toml`

You can also pass a custom config path:

```sh
kimun --config /path/to/my-config.toml
```

## How it works

Kimün opens in a terminal UI with a few screens:

- **Browse** — file tree navigator for a directory
- **Editor** — Markdown editor with a file browser pane
- **Settings** — configure your notes directory, theme, key bindings, and more

Your notes directory is called the **workspace**. Kimün creates a `kimun.sqlite` file at the workspace root to store the search index. Everything else is plain `.md` files — no lock-in.

## Command Line Interface

Kimün provides a powerful CLI for quick operations and multi-workspace management:

### Multi-Workspace Management

Kimün supports multiple workspaces, each with its own notes directory and isolated content:

```sh
# Initialize a new workspace
kimun workspace init --name work /path/to/work/notes
kimun workspace init --name personal /path/to/personal/notes

# List all workspaces
kimun workspace list

# Switch between workspaces
kimun workspace use work
kimun workspace use personal

# Manage workspaces
kimun workspace rename old-name new-name
kimun workspace remove old-workspace
kimun workspace reindex work                    # Rebuild search index
```

### Search Notes

```sh
kimun search "your search query"                # Search in current workspace
kimun search "meeting -cancelled"               # Use exclusion operators
kimun search "@project >-draft"                 # Combine filename and title filters
```

Search works the same as in the TUI and supports all search features described below.

### List Notes

```sh
kimun notes                                     # List all notes in current workspace
kimun notes --path "journal/"                   # Filter by path prefix
```

### JSON Output

Both search and notes commands support JSON output for automation and scripting:

```sh
kimun search "query" --format json             # Rich JSON with metadata
kimun notes --format json                       # Structured note listing
```

JSON output includes comprehensive metadata:
- Note content, title, size, timestamps
- Extracted tags, links, and headers
- Journal date detection
- Workspace context

Example with jq for processing:
```sh
# Find all notes with "rust" tag
kimun search "rust" --format json | jq '.notes[] | select(.metadata.tags[] == "rust")'

# Get note titles and paths
kimun notes --format json | jq '.notes[] | {title, path}'

# Count notes by workspace
kimun notes --format json | jq '.metadata | {workspace, total_results}'
```

### Custom Config

```sh
kimun --config /path/to/config.toml search "query"
kimun --config /path/to/config.toml workspace list
```

### Initial Setup

The CLI automatically creates workspace configuration on first use. For new installations:

1. **Option A - CLI First:** `kimun workspace init --name default /path/to/notes`
2. **Option B - TUI First:** Run `kimun` (TUI) to configure through the Settings screen

Legacy single-workspace configurations are automatically migrated to the new multi-workspace format.

## Search

Open search with `Ctrl+E`. The search box looks across note content and file paths.

### Free text

Searches both content and filenames. Case-insensitive, diacritics ignored, `*` wildcard supported.

```
kimun          → matches "Kimün", "KIMÜN", "kimun", etc.
kimu*          → matches anything starting with "kimu"
*meeting*      → matches any note containing "meeting" anywhere
```

### Filter by filename — `@` or `at:`

```
@tasks         → only notes whose filename contains "tasks"
at:tasks       → same
```

### Filter by section — `>` or `in:`

Sections are defined by Markdown headers (`#`, `##`, etc.). The search term must appear within that section.

```
>personal      → only content under a "Personal" heading
in:personal    → same
```

### Exclusion Operators — `-` prefix

Use `-` prefix to exclude terms from search results:

```sh
# Exclude specific content
kimun search "meeting -cancelled"

# Exclude from titles
kimun search ">project >-draft"

# Exclude from filenames
kimun search "@2024 @-temp"

# Exclude from paths
kimun search "/docs /-private"

# Exclusion-only searches
kimun search "-cancelled"        # All notes except those containing "cancelled"
kimun search ">-draft"          # All notes except those with "draft" in title
```

**Exclusion operators work with:**
- Content search: `-term` excludes from note content
- Title search: `>-term` or `in:-term` excludes from note titles
- Filename search: `@-term` or `at:-term` excludes from filenames
- Path search: `/-term` or `pt:-term` excludes from paths
- All operators can be combined in a single query

### Combine filters

Filters compose freely:

```
@tasks >work report       → in a file called "tasks", under a "Work" section, containing "report"
>personal kimun           → any note with "kimun" under a "Personal" section
@thoughts kimun           → a file called "thoughts" containing "kimun"
screen*                   → matches "screenshot", "screens", etc.
meeting -cancelled        → notes with "meeting" but not "cancelled"
@2024 >-draft             → files from 2024 that don't have "draft" in title
```

### Example

Given these notes:

**tasks.md**

```markdown
# Work
## TODO
* Talk with Bill
* Finish the report

# Personal
* Make the search in Kimün awesome
* Buy groceries
```

**projects.md**

```markdown
# Projects
## Personal
### Kimün
The simple but great note taking app!
```

| Search               | Returns                                     |
| -------------------- | ------------------------------------------- |
| `kimun`            | `projects.md`, `tasks.md`               |
| `>personal kimun`  | `projects.md`, `tasks.md`               |
| `>personal report` | `tasks.md`                                |
| `@tasks >work`     | `tasks.md`                                |
| `screen*`          | any note with "screenshot", "screens", etc. |

## Key bindings

Default bindings (all configurable in the config file):

| Action                               | Default    |
| ------------------------------------ | ---------- |
| Quit                                 | `Ctrl+Q` |
| Settings                             | `Ctrl+P` |
| Search notes                         | `Ctrl+E` |
| Open note (fuzzy finder)             | `Ctrl+O` |
| Toggle note browser                  | `Ctrl+F` |
| Toggle preview                       | `Ctrl+Y` |
| New journal entry                    | `Ctrl+J` |
| Toggle sidebar                       | `Ctrl+B` |
| Focus editor                         | `Ctrl+L` |
| Focus sidebar                        | `Ctrl+H` |
| Sort by name                         | `Ctrl+N` |
| Sort by title                        | `Ctrl+G` |
| Reverse sort order                   | `Ctrl+R` |
| File operations (rename/move/delete) | `F2`     |
| Bold                                 | `Ctrl+B` |
| Italic                               | `Ctrl+I` |
| Strikethrough                        | `Ctrl+S` |
| Toggle header                        | `Ctrl+T` |

## Roadmap

- [ ] Command palette
- [ ] Display key shortcuts in command palette and help modal
- [ ] Backlinks panel
- [ ] Inline tags and search by tag (`#important`)
- [ ] Resolve relative paths on links and images
- [ ] Paste images into notes
- [ ] Calendar view for journal browsing
- [ ] Auto-continue list formatting on Enter
- [X] Multiple workspaces
- [X] Search under Markdown sections
- [X] File management (create, rename, move, delete notes and directories)
- [X] Autosave
- [X] Wikilinks in preview
- [X] Navigate notes via links in preview
- [ ] Embed neoVim as an option
