# CLI Phase 2: JSON Output & Workspace Management Design

**Date:** March 26, 2026
**Status:** Design Approved
**Phase:** CLI Phase 2 - Enhanced Commands & Multi-Workspace Support

## Overview

This specification defines CLI Phase 2 enhancements for Kimun, focusing on two core areas:

1. **Rich JSON Output Format** - Comprehensive structured output for automation, scripting, and AI context processing
2. **Multi-Workspace Management** - Full CLI-based workspace lifecycle management with per-workspace isolation

These enhancements eliminate the current TUI dependency for workspace setup and provide machine-readable output for advanced use cases including `jq` filtering and AI-driven note traversal.

## Command Structure Redesign

### New Command Hierarchy

The CLI will be restructured with logical command groups for better organization and extensibility:

```bash
# Workspace Management
kimun workspace init <path> [--name <name>]    # Create new workspace
kimun workspace list                            # List all workspaces
kimun workspace use <name>                      # Switch active workspace
kimun workspace rename <old-name> <new-name>   # Rename workspace
kimun workspace remove <name>                   # Remove workspace

# Content Operations (enhanced from Phase 1)
kimun search <query> [--format json|text] [--workspace <name>]
kimun notes [--path <prefix>] [--format json|text] [--workspace <name>]

# Configuration Management
kimun config get [<key>]                       # Get config value(s)
kimun config set <key> <value>                 # Set config value
kimun config list                              # List all configuration
```

### Global Flags

- `--workspace <name>` - Override active workspace for any command
- `--config <path>` - Use custom config file (existing behavior)
- `--format json|text` - Output format for applicable commands

### Breaking Changes from Phase 1

- **Command reorganization:** Workspace operations now grouped under `workspace` subcommand
- **Configuration changes:** New multi-workspace config format (migration handled automatically)
- **Output behavior:** JSON format provides significantly richer data structure

## Multi-Workspace Configuration

### Configuration File Structure

The new `config.toml` format supports multiple named workspaces with per-workspace settings:

```toml
[global]
current_workspace = "default"
theme = "dark"
# Future: other global settings

[workspaces.default]
path = "/Users/user/notes"
last_paths = ["/journal", "/projects"]
created = "2024-01-15T10:30:00Z"

[workspaces.work]
path = "/Users/user/work-notes"
last_paths = ["/meetings", "/tasks"]
created = "2024-01-20T14:15:00Z"

[workspaces.personal]
path = "/Users/user/personal-notes"
last_paths = ["/diary", "/ideas"]
created = "2024-01-22T09:45:00Z"
```

### Workspace Isolation

Each workspace maintains complete isolation:

- **Separate database:** Each workspace has its own `kimun.sqlite` file in the workspace root
- **Independent navigation:** `last_paths` tracked per workspace for TUI integration
- **Isolated search index:** Content indexing scoped to workspace boundaries
- **Independent metadata:** Creation timestamps, usage patterns tracked separately

### Workspace Operations

**Creation:**
```bash
kimun workspace init ~/my-notes --name personal
kimun workspace init ~/work                      # Creates "default" if none exists
```

**Behavior:**
- Validates target path exists or creates directory structure
- Initializes SQLite database (`kimun.sqlite`)
- Creates workspace entry in config with timestamp
- Sets as current workspace if it's the first one created
- **Error handling:** Duplicate name detection with helpful error message:
  ```
  Error: Workspace 'work' already exists at /Users/user/existing-work-notes
  Use 'kimun workspace rename work new-name' to rename the existing workspace.
  ```

**Switching:**
```bash
kimun workspace use work
kimun workspace list              # Shows current workspace marked with *
```

**Validation:**
- Verifies workspace path still exists before switching
- Validates workspace database integrity
- Graceful error handling for missing or corrupted workspaces

## Rich JSON Output Format

### JSON Structure Design

Both `search` and `notes` commands support `--format json` with comprehensive data:

```json
{
  "metadata": {
    "workspace": "work",
    "workspace_path": "/Users/user/work-notes",
    "query": "meeting -cancelled",
    "total_results": 15,
    "execution_time_ms": 45,
    "generated_at": "2024-03-26T14:30:00Z"
  },
  "notes": [
    {
      "path": "meetings/weekly-standup.md",
      "title": "Weekly Standup - March 26",
      "content": "# Weekly Standup - March 26\n\n## Agenda\n- Sprint review...",
      "size": 2048,
      "modified": 1711454400,
      "created": 1711368000,
      "hash": "abc123def456",
      "journal_date": "2024-03-26",
      "metadata": {
        "tags": ["meeting", "standup", "sprint"],
        "links": ["sprint-goals.md", "https://jira.company.com"],
        "backlinks": ["team-notes.md", "retrospective.md"],
        "headers": [
          {"text": "Weekly Standup - March 26", "level": 1},
          {"text": "Agenda", "level": 2},
          {"text": "Action Items", "level": 2}
        ]
      }
    }
  ]
}
```

### JSON Field Specifications

**Top-level metadata:**
- `workspace` - Active workspace name
- `workspace_path` - Full path to workspace directory
- `query` - Search query used (for search command)
- `total_results` - Number of notes returned
- `execution_time_ms` - Query performance timing
- `generated_at` - ISO 8601 timestamp of output generation

**Per-note fields:**
- `path` - Relative path within workspace
- `title` - Extracted note title
- `content` - Full markdown content
- `size` - File size in bytes
- `modified` - Last modified timestamp (Unix epoch)
- `created` - File creation timestamp (Unix epoch)
- `hash` - Content hash for change detection
- `journal_date` - ISO date if note is detected as journal entry (optional)

**Rich metadata extraction:**
- `tags` - Extracted hashtags and YAML frontmatter tags
- `links` - All markdown links and references found in content
- `backlinks` - Other notes that reference this note
- `headers` - Structured list of markdown headers with levels

### Text Format Preservation

The existing text output format remains unchanged for backward compatibility:
```
path\ttitle\tsize\tmodified\tjournal:date
```

This ensures existing shell scripts and tooling continue to work.

### Usage Examples for Automation

**AI Context Generation:**
```bash
kimun search "architecture" --format json | jq '.notes[] | {title, content, links: .metadata.links}'
```

**Tag-based Filtering:**
```bash
kimun notes --format json | jq '.notes[] | select(.metadata.tags[] == "urgent")'
```

**Journal Entry Processing:**
```bash
kimun notes --format json | jq '.notes[] | select(.journal_date) | {date: .journal_date, title, path}'
```

**Content Analysis Pipeline:**
```bash
kimun search "project" --format json | jq -r '.notes[].content' | wc -w
```

## Technical Architecture

### Configuration Management

**Config Migration Strategy:**
- Automatic detection of Phase 1 vs Phase 2 config format
- Seamless migration of existing single workspace to "default" named workspace
- Preserve all existing settings during migration
- Migration happens on first Phase 2 CLI invocation

**Config Access Pattern:**
- Centralized config loading with validation
- Per-command workspace resolution (CLI flag > active workspace > error)
- Atomic config updates with backup/rollback capability

### Output System Design

**Dual Format Support:**
- Abstract output trait supporting both text and JSON formats
- Consistent data gathering for both formats
- JSON serialization with proper error handling
- Performance optimization for large result sets

**Content Extraction Pipeline:**
- Reuse existing note parsing logic from core
- Enhanced metadata extraction (tags, links, headers)
- Backlink computation via existing vault index
- Lazy evaluation for expensive operations (backlinks, content parsing)

### Error Handling Strategy

**Workspace Errors:**
- Clear error messages with suggested remediation
- Graceful degradation when workspace paths are missing
- Validation of workspace integrity before operations

**JSON Output Errors:**
- Partial success handling (some notes fail to parse)
- Error information included in JSON metadata
- Fallback to text format on catastrophic JSON errors

## Implementation Priorities

### Phase 2.1: Core Infrastructure
1. Multi-workspace config format and migration logic
2. Workspace management commands (`init`, `list`, `use`)
3. Enhanced output system with JSON support for existing commands

### Phase 2.2: Advanced Features
1. Workspace maintenance commands (`rename`, `remove`)
2. Rich metadata extraction (tags, links, backlinks, headers)
3. Configuration management commands
4. Performance optimization for large workspaces

## Success Criteria

**Functional Requirements:**
- ✅ Complete CLI workspace lifecycle without TUI dependency
- ✅ Rich JSON output suitable for `jq` processing and AI context
- ✅ Seamless migration from Phase 1 configuration
- ✅ Per-workspace isolation of all data and settings

**Non-Functional Requirements:**
- ✅ JSON output performance suitable for 1000+ note workspaces
- ✅ Intuitive command structure and helpful error messages
- ✅ Comprehensive test coverage for all workspace operations
- ✅ Documentation and examples for automation use cases

**User Experience Goals:**
- New users can set up Kimun entirely via CLI
- Power users can manage multiple workspaces efficiently
- Automation and AI tools can easily consume note data
- Existing workflows remain uninterrupted during migration

## Future Extensibility

This Phase 2 design establishes patterns for future enhancements:

- **Workspace-specific themes and settings** - Config structure ready for expansion
- **Remote workspace synchronization** - Workspace metadata supports future sync features
- **Plugin system integration** - Command structure accommodates future plugin commands
- **Advanced JSON filtering** - Rich metadata enables sophisticated query capabilities

The multi-workspace foundation and rich JSON output create a platform for advanced note management and automation scenarios in future CLI phases.