# Kimün CLI Phase 2: Multi-Workspace Usage Guide

Kimün CLI Phase 2 introduces comprehensive multi-workspace management, rich JSON output, and enhanced automation capabilities. This guide covers all CLI features, configuration, and best practices.

## Table of Contents

- [Installation & Setup](#installation--setup)
- [Multi-Workspace Management](#multi-workspace-management)
- [Search Operations](#search-operations)
- [Notes Listing](#notes-listing)
- [JSON Output Format](#json-output-format)
- [Configuration](#configuration)
- [Migration from Phase 1](#migration-from-phase-1)
- [Automation & Scripting](#automation--scripting)
- [Troubleshooting](#troubleshooting)
- [Performance Considerations](#performance-considerations)

## Installation & Setup

### Installation

```sh
cargo install kimun-notes
```

### Initial Setup

Kimün supports two setup approaches:

#### CLI-First Setup (Recommended)

```sh
# Initialize your first workspace directly via CLI
kimun workspace init --name default /path/to/your/notes

# Or create multiple workspaces from the start
kimun workspace init --name work /path/to/work/notes
kimun workspace init --name personal ~/personal-notes
```

#### TUI Setup

```sh
# Launch the TUI for interactive setup
kimun
```

The TUI provides a Settings screen for workspace configuration and will automatically migrate to the multi-workspace format.

## Multi-Workspace Management

### Core Concepts

- **Workspace**: An isolated notes environment with its own directory and search index
- **Current Workspace**: The active workspace for search and notes operations
- **Workspace Configuration**: Stored in `~/.config/kimun/kimun_config.toml` (Linux/macOS) or `%USERPROFILE%\kimun\kimun_config.toml` (Windows)

### Workspace Commands

#### Initialize Workspaces

```sh
# Create a new workspace
kimun workspace init --name <name> <path>

# Examples
kimun workspace init --name work ~/work-notes
kimun workspace init --name personal ~/Documents/notes
kimun workspace init --name research /mnt/external/research

# First workspace defaults to "default" name if omitted
kimun workspace init /path/to/notes
```

#### List Workspaces

```sh
kimun workspace list
```

Output shows all configured workspaces with current workspace marked:
```
* work       (/Users/user/work-notes)
  personal   (/Users/user/Documents/notes)
  research   (/mnt/external/research)
```

#### Switch Workspaces

```sh
# Switch to a different workspace
kimun workspace use <workspace-name>

# Examples
kimun workspace use personal
kimun workspace use work
```

#### Manage Workspaces

```sh
# Rename a workspace
kimun workspace rename old-name new-name

# Remove a workspace (removes from config, preserves files)
kimun workspace remove workspace-name

# Rebuild search index for a workspace
kimun workspace reindex                        # Current workspace
kimun workspace reindex --name specific-name   # Specific workspace
```

### Workspace Isolation

Each workspace maintains:
- **Separate search index** (`kimun.sqlite` in workspace directory)
- **Isolated note content** (no cross-workspace search)
- **Independent configuration** (last paths, workspace-specific settings)
- **Isolated file operations** (CLI commands only affect current workspace)

## Search Operations

### Basic Search

```sh
# Search in current workspace
kimun search "search term"

# Case-insensitive, diacritics ignored
kimun search "kimün"          # Matches "Kimün", "KIMÜN", "kimun"

# Wildcard support
kimun search "meet*"          # Matches "meeting", "meetings", "meetup"
kimun search "*report*"       # Matches "report", "reports", "quarterly-report"
```

### Advanced Search Operators

#### Filename Filtering (`@` or `at:`)

```sh
kimun search "@tasks"         # Only notes with "tasks" in filename
kimun search "at:project"     # Same as above
kimun search "@2024"          # Files with "2024" in name
```

#### Section/Title Filtering (`>` or `in:`)

```sh
kimun search ">personal"      # Content under "Personal" headings
kimun search "in:work"        # Content under "Work" headings
kimun search ">project status" # "status" under "Project" sections
```

#### Path Filtering (`/` or `pt:`)

```sh
kimun search "/journal"       # Notes in journal directory
kimun search "pt:docs"        # Notes in docs path
kimun search "/2024/reports"  # Specific path structure
```

#### Exclusion Operators (`-` prefix)

```sh
# Content exclusion
kimun search "meeting -cancelled"      # Notes with "meeting" but not "cancelled"

# Filename exclusion
kimun search "@project @-draft"        # Project files excluding drafts
kimun search "at:2024 at:-temp"       # 2024 files excluding temporary ones

# Title exclusion
kimun search ">project >-draft"       # Project sections excluding draft titles
kimun search "in:work in:-archived"   # Work sections excluding archived

# Path exclusion
kimun search "/docs /-private"        # Docs path excluding private subdirs
kimun search "pt:notes pt:-old"       # Notes path excluding old directories

# Exclusion-only searches
kimun search "-cancelled"             # All notes except those with "cancelled"
kimun search ">-draft"               # All notes except those with "draft" in title
kimun search "@-temp"                # All notes except temporary files
```

#### Combining Operators

```sh
# Complex queries combining multiple operators
kimun search "@tasks >work report -cancelled"
# Files with "tasks" in name, under "Work" sections, containing "report", excluding "cancelled"

kimun search "/journal/2024 >daily -weekend"
# Notes in 2024 journal path, under "Daily" headings, excluding weekend entries

kimun search "@project @-draft >-archived status"
# Project files (not drafts), non-archived sections, containing "status"
```

### Output Formats

```sh
# Text output (default)
kimun search "query"

# JSON output for automation
kimun search "query" --format json
```

## Notes Listing

### Basic Listing

```sh
# List all notes in current workspace
kimun notes

# Filter by path prefix
kimun notes --path "journal/"
kimun notes --path "projects/2024/"
```

### Output Formats

```sh
# Text output (default) - shows titles, paths, and modification dates
kimun notes

# JSON output with comprehensive metadata
kimun notes --format json
kimun notes --path "journal/" --format json
```

## JSON Output Format

Both `search` and `notes` commands support rich JSON output for automation and integration.

### JSON Structure

```json
{
  "metadata": {
    "workspace": "work",
    "workspace_path": "/Users/user/work-notes",
    "total_results": 15,
    "query": "meeting",                    // null for notes listing
    "is_listing": false,                   // true for notes command
    "generated_at": "2024-03-15T10:30:00Z"
  },
  "notes": [
    {
      "path": "projects/status-meeting.md",
      "title": "Weekly Status Meeting",
      "content": "# Weekly Status Meeting\n\n...",
      "size": 1024,
      "modified": 1710504600,
      "created": 1710418200,
      "hash": "a1b2c3d4e5f67890",
      "journal_date": "2024-03-15",        // null if not a journal note
      "metadata": {
        "tags": ["meeting", "status", "weekly"],
        "links": ["project-roadmap", "team-updates"],
        "headers": [
          {"level": 1, "text": "Weekly Status Meeting"},
          {"level": 2, "text": "Agenda"},
          {"level": 2, "text": "Action Items"}
        ]
      }
    }
  ]
}
```

### Metadata Extraction

JSON output includes rich metadata extracted from note content:

- **Tags**: Extracted from hashtags (`#tag`) and YAML frontmatter
- **Links**: Wiki-style links (`[[link]]`) and Markdown links
- **Headers**: All Markdown headers with levels and text
- **Journal Detection**: Automatically detects and formats journal dates
- **Timestamps**: Both modification and creation times
- **Content Hash**: For change detection and caching

### Processing JSON Output

#### Using jq for Processing

```sh
# Extract note titles and paths
kimun notes --format json | jq '.notes[] | {title, path}'

# Find notes with specific tags
kimun search "project" --format json | jq '.notes[] | select(.metadata.tags[] | contains("urgent"))'

# Get notes modified in the last week
kimun notes --format json | jq --argjson week_ago $(date -d "1 week ago" +%s) '.notes[] | select(.modified > $week_ago)'

# Count notes by tag
kimun notes --format json | jq '[.notes[].metadata.tags[]] | group_by(.) | map({tag: .[0], count: length})'

# Extract all wiki links
kimun notes --format json | jq '.notes[].metadata.links[]' | sort | uniq

# Find journal entries from specific date range
kimun notes --format json | jq '.notes[] | select(.journal_date and (.journal_date | strptime("%Y-%m-%d") | mktime) > (now - 604800))'
```

#### Integration Examples

```sh
# Export to CSV
kimun notes --format json | jq -r '.notes[] | [.title, .path, .modified] | @csv' > notes.csv

# Create backlink index
kimun notes --format json | jq '.notes[] | {path: .path, links: .metadata.links}' > backlinks.json

# Monitor for changes
current_hash=$(kimun notes --format json | jq '.notes[] | .hash' | sort | md5sum)
```

## Configuration

### Config File Location

- **Linux/macOS**: `~/.config/kimun/kimun_config.toml`
- **Windows**: `%USERPROFILE%\kimun\kimun_config.toml`

### Custom Config Path

```sh
# Use custom config for all commands
kimun --config /path/to/custom-config.toml search "query"
kimun --config /path/to/custom-config.toml workspace list
kimun --config /path/to/custom-config.toml notes --format json
```

### Phase 2 Config Structure

```toml
config_version = 2

[workspace_config.global]
current_workspace = "work"
theme = "dark"

[workspace_config.workspaces.work]
path = "/Users/user/work-notes"
last_paths = ["/journal", "/projects"]
created = "2024-01-15T10:30:00Z"

[workspace_config.workspaces.personal]
path = "/Users/user/personal-notes"
last_paths = ["/thoughts", "/learning"]
created = "2024-01-20T15:45:00Z"
```

## Migration from Phase 1

### Automatic Migration

Kimün automatically detects and migrates Phase 1 configurations:

**Phase 1 Config (Single Workspace)**:
```toml
workspace_dir = "/Users/user/notes"
theme = "dark"
last_paths = ["/journal", "/projects"]
```

**Migrated to Phase 2**:
```toml
config_version = 2

[workspace_config.global]
current_workspace = "default"
theme = "dark"

[workspace_config.workspaces.default]
path = "/Users/user/notes"
last_paths = ["/journal", "/projects"]
created = "2024-03-15T10:30:00Z"
```

### Migration Process

1. **Detection**: Kimün detects `workspace_dir` field without `workspace_config`
2. **Validation**: Ensures workspace directory still exists
3. **Conversion**: Creates new multi-workspace structure with "default" workspace
4. **Preservation**: Maintains all existing settings (theme, last_paths, etc.)
5. **Update**: Sets `config_version = 2` marker

### Migration Verification

```sh
# Check migration status
kimun workspace list

# Should show your previous workspace as "default"
# * default    (/path/to/your/old/workspace)
```

### Post-Migration Workflow

After migration, you can:

```sh
# Continue using as before (now uses "default" workspace)
kimun search "query"
kimun notes

# Add additional workspaces
kimun workspace init --name work /path/to/work/notes
kimun workspace init --name research /path/to/research

# Rename the migrated workspace
kimun workspace rename default personal
```

## Automation & Scripting

### Bash Completion

Add to your `.bashrc` or `.zshrc`:

```sh
# Generate completion script (if available)
eval "$(kimun completion bash)"  # or zsh
```

### Common Automation Patterns

#### Daily Journal Creation

```sh
#!/bin/bash
# daily-journal.sh
date_today=$(date +%Y-%m-%d)
journal_path="journal/${date_today}.md"

kimun workspace use personal
if ! kimun notes --path "journal/" --format json | jq -e ".notes[] | select(.path | contains(\"$date_today\"))" > /dev/null; then
    mkdir -p ~/personal-notes/journal
    echo "# Daily Journal - $date_today" > ~/personal-notes/journal/${date_today}.md
    echo "" >> ~/personal-notes/journal/${date_today}.md
    echo "## Tasks" >> ~/personal-notes/journal/${date_today}.md
    echo "## Notes" >> ~/personal-notes/journal/${date_today}.md
fi
```

#### Project Status Reports

```sh
#!/bin/bash
# project-report.sh
kimun workspace use work
kimun search "@project status" --format json | \
  jq '.notes[] | select(.metadata.tags[] | contains("active"))' | \
  jq -r '"\(.title): \(.path)"'
```

#### Content Analytics

```sh
#!/bin/bash
# content-analytics.sh

echo "=== Content Statistics ==="
for workspace in $(kimun workspace list | grep -v "^*" | awk '{print $1}' | grep -v "Configured"); do
    echo "Workspace: $workspace"
    kimun workspace use $workspace

    total=$(kimun notes --format json | jq '.metadata.total_results')
    echo "  Total notes: $total"

    tags=$(kimun notes --format json | jq '.notes[].metadata.tags[]' | sort | uniq -c | sort -nr | head -5)
    echo "  Top tags:"
    echo "$tags" | while read count tag; do
        echo "    $tag: $count"
    done
    echo
done
```

#### Backup and Sync

```sh
#!/bin/bash
# backup-workspaces.sh

backup_dir="$HOME/kimun-backups/$(date +%Y-%m-%d)"
mkdir -p "$backup_dir"

kimun workspace list | grep -E "^\s*\*?\s*\w+" | while read line; do
    workspace=$(echo "$line" | sed 's/^\s*\*\?\s*//' | awk '{print $1}')
    path=$(echo "$line" | grep -o '([^)]*)' | tr -d '()')

    echo "Backing up workspace: $workspace"
    rsync -av "$path/" "$backup_dir/$workspace/"
done

# Export configuration
cp ~/.config/kimun/kimun_config.toml "$backup_dir/"
```

### Integration with External Tools

#### With fzf for Interactive Selection

```sh
# Interactive note selection
note=$(kimun notes --format json | jq -r '.notes[] | "\(.title)|\(.path)"' | fzf --delimiter='|' --with-nth=1 | cut -d'|' -f2)
if [ -n "$note" ]; then
    editor "$note"  # Open in your preferred editor
fi
```

#### With Obsidian or Other Tools

```sh
# Export to Obsidian format
kimun notes --format json | jq '.notes[]' | while read -r note; do
    path=$(echo "$note" | jq -r '.path')
    title=$(echo "$note" | jq -r '.title')
    tags=$(echo "$note" | jq -r '.metadata.tags[]' | tr '\n' ' ')

    # Add Obsidian frontmatter
    obsidian_path="$OBSIDIAN_VAULT/$(basename "$path")"
    echo "---" > "$obsidian_path"
    echo "title: $title" >> "$obsidian_path"
    echo "tags: [$tags]" >> "$obsidian_path"
    echo "kimun_path: $path" >> "$obsidian_path"
    echo "---" >> "$obsidian_path"
    echo "" >> "$obsidian_path"
    cat "$(dirname "$(kimun notes --format json | jq -r '.metadata.workspace_path')")/$path" >> "$obsidian_path"
done
```

## Troubleshooting

### Common Issues

#### "No workspace configured" Error

```sh
# Problem: CLI shows "No workspace configured"
# Solution: Initialize a workspace
kimun workspace init --name default /path/to/notes

# Or use TUI for interactive setup
kimun
```

#### Workspace Directory Not Found

```sh
# Problem: "workspace directory no longer exists"
# Check current workspaces
kimun workspace list

# Remove invalid workspace
kimun workspace remove invalid-workspace

# Re-add with correct path
kimun workspace init --name corrected /correct/path
```

#### Search Returns No Results

```sh
# Check current workspace
kimun workspace list

# Verify workspace has notes
ls -la $(kimun notes --format json | jq -r '.metadata.workspace_path')

# Rebuild search index
kimun workspace reindex
```

#### JSON Parsing Issues

```sh
# Validate JSON output
kimun notes --format json | jq '.'

# Check for binary files or encoding issues
kimun notes --format json | jq '.notes[] | select(.size > 100000)'
```

### Performance Issues

#### Large Workspace Optimization

```sh
# For workspaces with many files (>10,000 notes)

# 1. Use path filters for better performance
kimun notes --path "recent/" --format json

# 2. Regular reindexing
kimun workspace reindex

# 3. Consider splitting large workspaces
kimun workspace init --name archive /path/to/archived/notes
```

#### Memory Usage

```sh
# Monitor memory usage during large operations
time kimun notes --format json > /dev/null

# Use streaming for large JSON outputs
kimun notes --format json | jq -c '.notes[]' | while read note; do
    # Process one note at a time
    echo "$note" | jq '.title'
done
```

### Configuration Issues

#### Config File Corruption

```sh
# Backup current config
cp ~/.config/kimun/kimun_config.toml ~/.config/kimun/kimun_config.toml.backup

# Reset to defaults (will prompt for workspace setup)
rm ~/.config/kimun/kimun_config.toml
kimun workspace init --name default /path/to/notes
```

#### Permission Issues

```sh
# Check config directory permissions
ls -la ~/.config/kimun/

# Fix permissions
chmod 755 ~/.config/kimun/
chmod 644 ~/.config/kimun/kimun_config.toml
```

### Debug Mode

```sh
# Enable verbose logging (if available)
RUST_LOG=debug kimun search "query"

# Or check for debug flags
kimun --help
```

## Performance Considerations

### Indexing Performance

- **Initial Indexing**: First-time indexing scales with content size
- **Incremental Updates**: Subsequent operations are fast due to incremental indexing
- **Index Size**: SQLite index is typically 5-10% of total content size
- **Memory Usage**: Scales with concurrent operations and result set size

### Best Practices

#### Workspace Organization

```sh
# Good: Moderate-sized workspaces (1,000-10,000 notes)
kimun workspace init --name current-work ~/work/2024
kimun workspace init --name archive ~/work/archive

# Avoid: Single massive workspace with 50,000+ notes
# Instead: Split by time, project, or category
```

#### Query Optimization

```sh
# Efficient: Specific path filters
kimun search "meeting" --path "journal/2024/"

# Less efficient: Broad searches on large workspaces
kimun search "*"

# Efficient: Combined filters
kimun search "@status >weekly"

# Less efficient: Multiple separate queries
kimun search "@status" && kimun search ">weekly"
```

#### JSON Output Optimization

```sh
# For large result sets, use streaming processing
kimun notes --format json | jq -c '.notes[]' | head -100

# Filter early in the pipeline
kimun notes --path "recent/" --format json | jq '.notes[] | select(.modified > 1640995200)'

# Use path filters to limit scope
kimun search "query" --path "specific/directory/"
```

### Resource Monitoring

```sh
# Monitor disk usage
du -sh ~/.config/kimun/
find ~/notes -name "kimun.sqlite" -exec du -sh {} \;

# Check index status
sqlite3 ~/notes/kimun.sqlite "SELECT COUNT(*) as note_count FROM notes;"
sqlite3 ~/notes/kimun.sqlite "SELECT COUNT(*) as search_entries FROM search_index;"
```

---

This guide covers all CLI Phase 2 features and capabilities. For additional help or feature requests, please refer to the main project documentation or submit issues through the project repository.