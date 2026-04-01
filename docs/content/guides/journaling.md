+++
title = "Journaling Workflow"
weight = 1
+++

# Journaling Workflow Guide

## What is a journal entry in kimun?

Kimun treats any note under a `journal/` directory as a journal entry. The filename should follow the format `YYYY-MM-DD.md` for date detection (e.g., `journal/2024-01-15.md`). This allows kimun to extract the journal date and include it in search results and JSON output.

## Creating a journal entry

### In the TUI

Press `Ctrl+J` to create a new journal entry. Kimun creates a file named with today's date under `journal/` in the current workspace and opens it in the editor.

### In the CLI

```sh
kimun journal               # Append to today's journal entry (creates it if it doesn't exist)
kimun journal "Quick note"  # Append inline content
kimun journal show          # Display today's entry
```

### Piping content

`kimun journal` reads from stdin when no content argument is provided and stdin is not a terminal. This makes it easy to capture command output directly into your journal:

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

Once a journal entry is open, write freely in Markdown. Use headers to organise your entry:

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

The entry will be created if it doesn't exist. The date must be in `YYYY-MM-DD` format.

## Browsing journal entries

In the TUI sidebar, journal entries appear in reverse chronological order by default (newest first). You can change this in Configuration with `journal_sort_field` and `journal_sort_order`.

## Searching journal entries

### Find entries by content

```sh
kimun search "standup"              # Notes containing "standup"
kimun search "/journal standup"     # Only in journal/, containing "standup"
```

### Find entries from a specific period

```sh
kimun search "@2024-01"             # Files with "2024-01" in filename (January 2024)
kimun search "@2024"                # All journal entries from 2024
```

### Search within sections

```sh
kimun search "/journal >tasks"      # Journal entries with a "Tasks" section
kimun search "/journal >tasks -done" # Tasks sections without "done"
```

## Tips

- Use consistent heading names across entries (e.g. always `## Tasks`) to make section search effective
- The `*` wildcard helps with partial dates: `@2024-0*` matches Jan–Sep 2024
- Combine with JSON output for automation: `kimun search "/journal" --format json | jq '.notes[] | {date: .journal_date, title: .title}'`
