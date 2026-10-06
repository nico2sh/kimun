+++
title = "Journaling Workflow"
weight = 1
+++

# Journaling Workflow Guide

## Journal Entries

Kimün treats any note under a `journal/` directory as a journal entry. Name the file `YYYY-MM-DD.md` (e.g., `journal/2024-01-15.md`) so Kimün can read the journal date from it and include it in search results and JSON output.

## Creating a journal entry

### In the TUI

Press `Ctrl+J` to create a new journal entry. Kimün creates a file named with today's date under `journal/` in the current workspace and opens it in the editor.

### In the CLI

```sh
kimun journal               # Append to today's journal entry (creates it if it doesn't exist)
kimun journal "Quick note"  # Append inline content
kimun journal show          # Display today's entry
```

### Piping content

`kimun journal` reads from stdin when no content argument is provided and stdin is not a terminal, so you can pipe command output into your journal:

```sh
# Pipe a timestamped line
echo "$(date +%H:%M) — deployed v1.2 to production" | kimun journal

# Capture the last line of a script's output
./run-tests.sh | tail -1 | kimun journal

# Log system info
echo "$(hostname): $(uptime)" | kimun journal

# Append a multi-line entry with a here-string
kimun journal <<'EOF'

## Evening review

- Finished the auth refactor
- Reviewed two PRs
- TODO: follow up on deploy schedule
EOF

# Pipe to a specific date
echo "Late addition" | kimun journal --date 2024-01-15
```

## Writing in the editor

A journal entry is a regular Markdown note. Headings help organise it:

```markdown
# 2024-01-15

## Morning
Reviewed the Q1 roadmap...

## Tasks
- [ ] Follow up with Alex
- [ ] Finish the report draft
```

## Writing to a specific date

`kimun journal` defaults to today. Use `--date` to target a different entry:

```sh
kimun journal --date 2024-01-15 "Retroactive note for January 15th"
kimun journal --date 2025-12-31 "New Year's Eve plans"
```

The entry is created if it doesn't exist. The date must be in `YYYY-MM-DD` format.

## Browsing journal entries

In the FILES drawer, journal entries appear in reverse chronological order by default (newest first). You can change this in Configuration with `journal_sort_field` and `journal_sort_order`.

## Searching journal entries

### Find entries by content

```sh
kimun search "standup"              # Notes containing "standup"
kimun search "/journal standup"     # Only in journal/, containing "standup"
```

### Find entries from a specific period

```sh
kimun search "=2024-01"             # Notes with "2024-01" in the name (January 2024)
kimun search "=2024"                # All journal entries from 2024
```

## Quick notes and the inbox

For thoughts that don't belong in today's journal, use quick notes (`Ctrl+W` in the TUI or `kimun note quick` in the CLI). Each one is saved in the `/inbox` directory with a timestamp as its filename.

Later, you can go through the inbox and move notes into the journal. If you use the [MCP server](@/using-kimun/ai-mcp-server.md), the `triage_inbox` prompt asks the AI to suggest where each inbox note should go: into the journal, into a proper note, or left in the inbox.

### Search within sections

```sh
kimun search "/journal @tasks"      # Journal entries with a "Tasks" section
kimun search "/journal @tasks -done" # Tasks sections without "done"
```

## Tips

- Use the same heading names across entries (e.g. always `## Tasks`) so section search finds them
- The `*` wildcard helps with partial dates: `=2024-0*` matches Jan–Sep 2024
- For scripts, use JSON output: `kimun search "/journal" --format json | jq '.notes[] | {date: .journal_date, title: .title}'`
