+++
title = "Journaling Workflow"
weight = 1
+++

# Journaling Workflow Guide

## Journal Entries

Kimün treats any note in a `journal/` directory as a journal entry. Name the file `YYYY-MM-DD.md` (for example, `journal/2024-01-15.md`). Then Kimün reads the journal date from the file name and includes the date in search results and JSON output.

## Creating a journal entry

### In the TUI

Press `Ctrl+J` to create a new journal entry. Kimün creates a file in `journal/` in the current workspace. The file name is today's date. Kimün opens the file in the editor.

### In the CLI

```sh
kimun journal               # Append to today's journal entry (creates it if it does not exist)
kimun journal "Quick note"  # Append inline content
kimun journal show          # Display today's entry
```

### Piping content

If you give no content argument and stdin is not a terminal, `kimun journal` reads from stdin. You can pipe command output into your journal:

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

A journal entry is a regular Markdown note. Use headings to organize it:

```markdown
# 2024-01-15

## Morning
Reviewed the Q1 roadmap...

## Tasks
- [ ] Follow up with Alex
- [ ] Finish the report draft
```

## Writing to a specific date

By default, `kimun journal` writes to today's entry. To write to a different entry, use `--date`:

```sh
kimun journal --date 2024-01-15 "Retroactive note for January 15th"
kimun journal --date 2025-12-31 "New Year's Eve plans"
```

Kimün creates the entry if it does not exist. The date must be in `YYYY-MM-DD` format.

## Browsing journal entries

In the FILES drawer, journal entries appear by default with the newest first. To change the order, set `journal_sort_field` and `journal_sort_order` in the configuration.

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

For thoughts that do not belong in today's journal, use quick notes (`Ctrl+W` in the TUI or `kimun note quick` in the CLI). Kimün saves each quick note in the `/inbox` directory. The file name is a timestamp.

Later, you can read the inbox and move notes into the journal. If you use the [MCP server](@/using-kimun/ai-mcp-server.md), the `triage_inbox` prompt asks the AI to suggest a place for each inbox note: the journal, a proper note, or the inbox.

### Search within sections

```sh
kimun search "/journal @tasks"      # Journal entries with a "Tasks" section
kimun search "/journal @tasks -done" # Tasks sections without "done"
```

## Tips

- Use the same heading names in all entries (for example, always `## Tasks`). Then section search finds them.
- The `*` wildcard helps with partial dates. For example, `=2024-0*` matches January to September 2024.
- In scripts, use JSON output: `kimun search "/journal" --format json | jq '.notes[] | {date: .journal_date, title: .title}'`
