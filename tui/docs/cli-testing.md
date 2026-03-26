# Kimun CLI Testing Documentation

This document provides comprehensive guidance for testing the Kimun CLI functionality, including both automated integration tests and manual testing procedures.

## Overview

Kimun provides two CLI commands for interacting with your notes from the command line:
- `search` - Search notes by query
- `notes` - List all notes with optional path filtering

Both commands support text output format and require a configured workspace.

### Current Limitations

- **Database Initialization:** CLI commands require the workspace database to be initialized first via the TUI
- **Output Format:** Only text format is currently supported (JSON planned for future)
- **Read-Only:** CLI commands are read-only; note creation/editing requires TUI
- **No Streaming:** Large result sets are loaded entirely into memory

## Running Integration Tests

### Prerequisites

- Rust toolchain installed
- All dependencies resolved (`cargo build`)

### Test Execution

Run all CLI integration tests:
```bash
cd tui
cargo test cli_integration_test
```

Run specific integration tests:
```bash
# Test search command functionality
cargo test test_cli_search_command

# Test notes command functionality
cargo test test_cli_notes_command

# Test error handling when no workspace configured
cargo test test_cli_no_workspace_error

# Test custom config file support
cargo test test_cli_custom_config
```

### Test Coverage

The integration tests cover four main scenarios:

1. **Search Command (`test_cli_search_command`)**
   - Creates temporary vault with indexed test notes
   - Executes search command with query "hello"
   - Verifies command succeeds and returns results

2. **Notes Command (`test_cli_notes_command`)**
   - Tests listing all notes without path filter
   - Tests listing notes with path prefix filter ("sub/")
   - Verifies both scenarios succeed

3. **No Workspace Error (`test_cli_no_workspace_error`)**
   - Tests behavior when config has no workspace_dir set
   - Verifies settings layer correctly returns None
   - Ensures CLI would hit error branch for missing workspace

4. **Custom Config (`test_cli_custom_config`)**
   - Tests loading settings from custom config file path
   - Verifies end-to-end CLI execution with custom config
   - Ensures --config flag works correctly

## Manual Testing Procedures

### Setup Test Environment

1. **Create Test Workspace**
   ```bash
   mkdir -p /tmp/kimun-test-workspace
   cd /tmp/kimun-test-workspace
   ```

2. **Create Test Notes**
   ```bash
   mkdir -p sub
   echo "# Hello World\n\nThis is a hello note." > hello.md
   echo "# Nested Note\n\nThis note lives in a subdirectory." > sub/nested.md
   echo "# Another Note\n\nSome other content." > another.md
   ```

3. **Create Test Config**
   ```bash
   mkdir -p ~/.config/kimun
   cat > ~/.config/kimun/config.toml << EOF
   workspace_dir = "/tmp/kimun-test-workspace"
   EOF
   ```

4. **Initialize Workspace Database**

   **Important:** The CLI commands require the workspace database to be initialized. This happens automatically when the TUI starts, but must be done manually for CLI-only usage.

   **Option A - Use TUI to initialize (Recommended):**
   ```bash
   # Run TUI once to initialize database and create index
   kimun --config ~/.config/kimun/config.toml
   # Exit after startup completes (Ctrl+Q)
   ```

   **Option B - Manual database setup (Advanced):**
   ```bash
   # The database file will be created at workspace/.kimun.db
   # Currently there's no CLI command to initialize without TUI
   # This is a limitation of the current implementation
   ```

### Testing Search Command

#### Basic Search
```bash
# Search for notes containing "hello"
kimun search hello

# Expected output format:
# hello.md	"Hello World"	<size>	<timestamp>
```

#### Search with Different Queries
```bash
# Search for "nested"
kimun search nested

# Search for "note"
kimun search note

# Search for non-existent term
kimun search nonexistent
```

#### Expected Output Format
Each result line contains tab-separated fields:
- File path relative to workspace
- Note title in quotes
- File size in bytes
- Modified timestamp (Unix seconds)
- Journal date (if applicable): `journal:YYYY-MM-DD`

### Testing Notes Command

#### List All Notes
```bash
# List all notes in workspace
kimun notes

# Expected output: all notes with same format as search
```

#### Path Filtering
```bash
# List notes in subdirectory
kimun notes --path sub/

# List notes with specific prefix
kimun notes --path hello
```

#### Output Format Options
```bash
# Explicit text format (default)
kimun notes --format text
kimun search hello --format text
```

#### Exclusion Operator Testing

Test exclusion operators with various combinations:

```bash
# Basic content exclusion
kimun search "meeting -cancelled"

# Title exclusion
kimun search ">project >-draft"

# Filename exclusion
kimun search "@2024 @-temp"

# Path exclusion
kimun search "/docs /-private"

# Exclusion-only queries
kimun search "-cancelled"
kimun search ">-draft"

# Complex combinations
kimun search "meeting @2024 -cancelled >-draft /docs"
```

**Expected Behavior:**
- Content exclusions filter out notes containing excluded terms
- Title exclusions work on note titles/breadcrumbs only
- Filename exclusions work on note filenames only
- Path exclusions work on note path prefixes
- Exclusion-only queries return all notes except those matching excluded terms

### Testing Configuration

#### Custom Config File
```bash
# Create custom config
cat > /tmp/custom-config.toml << EOF
workspace_dir = "/tmp/kimun-test-workspace"
EOF

# Test with custom config
kimun --config /tmp/custom-config.toml notes
kimun --config /tmp/custom-config.toml search hello
```

#### No Workspace Configuration
```bash
# Create config without workspace
cat > /tmp/no-workspace.toml << EOF
# empty config
EOF

# Test error handling
kimun --config /tmp/no-workspace.toml notes
# Expected: Error message and exit code 1
```

### Error Scenarios

#### Missing Workspace Directory
```bash
# Point to non-existent directory
cat > /tmp/bad-config.toml << EOF
workspace_dir = "/tmp/does-not-exist"
EOF

kimun --config /tmp/bad-config.toml notes
# Expected: Error about unable to access workspace
```

#### Invalid Config File
```bash
# Test with malformed TOML
echo "invalid toml content [[[" > /tmp/bad.toml
kimun --config /tmp/bad.toml notes
# Expected: Config parsing error
```

## Test Scenarios and Expected Outputs

### Scenario 1: Fresh Workspace with Notes

**Setup:**
- Empty workspace with 3 notes: hello.md, sub/nested.md, another.md
- All notes properly indexed

**Test Cases:**

| Command | Expected Output | Notes |
|---------|----------------|-------|
| `kimun search hello` | `hello.md	"Hello World"	<size>	<timestamp>` | Single match |
| `kimun search note` | Multiple entries for notes containing "note" | Multiple matches |
| `kimun notes` | All 3 notes listed | Full listing |
| `kimun notes --path sub/` | Only `sub/nested.md` | Path filtering |

### Scenario 2: Journal Notes

**Setup:**
- Workspace with journal entries following YYYY-MM-DD.md pattern

**Test Cases:**

| Command | Expected Output | Notes |
|---------|----------------|-------|
| `kimun notes` | Entries include `journal:YYYY-MM-DD` suffix | Journal date detection |
| `kimun search today` | Journal entries with "today" in content | Search in journal |

### Scenario 3: Empty Workspace

**Setup:**
- Valid workspace directory but no notes

**Test Cases:**

| Command | Expected Output | Notes |
|---------|----------------|-------|
| `kimun search anything` | No output | Empty results |
| `kimun notes` | No output | Empty listing |

## Troubleshooting Guide

### Test Failures

#### Integration Test Failures

**Symptom:** `test_cli_search_command` fails
- **Check:** Verify search indexing is working in test vault
- **Solution:** Ensure `vault.recreate_index()` completes successfully
- **Debug:** Add logging to see if notes are being created and indexed

**Symptom:** `test_cli_no_workspace_error` fails
- **Check:** Verify AppSettings correctly handles missing workspace_dir
- **Solution:** Ensure config parsing returns None for missing workspace
- **Debug:** Print settings.workspace_dir value in test

**Symptom:** Tests hang or timeout
- **Check:** Async test execution and vault operations
- **Solution:** Verify tokio runtime is properly configured
- **Debug:** Add timeouts and logging to async operations

#### Manual Test Issues

**Symptom:** "No workspace configured" error
- **Check:** Config file location and format
- **Solution:** Verify config.toml exists and has valid workspace_dir
- **Debug:** Use `--config` flag with absolute path to config file

**Symptom:** CLI exits with code 1
- **Check:** Workspace directory exists and is accessible
- **Solution:** Create workspace directory or fix permissions
- **Debug:** Run with verbose logging if available

**Symptom:** No search results when notes exist
- **Check:** Notes are properly indexed
- **Solution:** Run TUI once to ensure indexing, or check if vault.recreate_index() needed
- **Debug:** Verify note content matches search query

**Symptom:** "no such table: notes" error
- **Check:** Database has been initialized
- **Solution:** Run TUI once to initialize SQLite database and create tables
- **Debug:** Verify .kimun.db file exists in workspace directory

**Symptom:** Notes command shows no output
- **Check:** Notes exist in workspace directory
- **Solution:** Create test notes or verify workspace path
- **Debug:** List directory contents manually

### Environment Issues

#### Path Problems
- Ensure all paths in config use absolute paths
- Check workspace directory permissions
- Verify config file is readable

#### Index Problems
- Notes may need initial indexing through TUI
- Search index might be corrupt or missing
- Try recreating workspace or running TUI once

#### Config Issues
- TOML syntax must be valid
- workspace_dir must point to accessible directory
- Custom config paths must be absolute

### Debugging Commands

```bash
# Check config loading
kimun --config /path/to/config.toml notes 2>&1 | head

# Verify workspace access
ls -la "$(grep workspace_dir ~/.config/kimun/config.toml | cut -d'"' -f2)"

# Test with minimal config
echo 'workspace_dir = "/tmp/test"' | kimun --config - notes

# Check file permissions
stat ~/.config/kimun/config.toml
```

## Performance Considerations

### Large Workspaces
- Search performance depends on index size
- Notes listing loads all entries into memory
- Consider pagination for very large note collections

### Concurrent Access
- CLI and TUI can safely access same workspace simultaneously
- Index updates are atomic
- File system locking handles concurrent note access

## Integration with CI/CD

### Automated Testing
```bash
# Run in CI pipeline
cargo test --package kimun-notes cli_integration_test
```

### Test Coverage
```bash
# Generate coverage report including CLI tests
cargo tarpaulin --include-tests --out Html
```

### Performance Benchmarks
```bash
# Benchmark CLI performance with large datasets
hyperfine 'kimun search common-term' --warmup 3
```

## Future Test Considerations

### Current Limitation Improvements
- CLI-only database initialization command
- Streaming output for large result sets
- Write operations via CLI (create, edit, delete notes)
- Live indexing without requiring TUI startup

### Additional Test Scenarios
- Unicode/non-ASCII content in notes
- Very large notes (>1MB)
- Malformed note files
- Symlinks in workspace
- Network-mounted workspace directories
- Concurrent CLI operations

### Output Format Extensions
- JSON output format testing
- Machine-readable output validation
- CSV/TSV output formats
- Structured metadata output

### Configuration Extensions
- Multiple workspace support
- Environment variable configuration
- XDG base directory compliance
- Profile-based configurations