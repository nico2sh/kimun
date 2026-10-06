+++
title = "Scripting with JSON"
weight = 3
+++

# Scripting with JSON

Kimün can print search results and note listings as JSON, which you can process with `jq` or other tools in scripts. This guide describes the JSON structure and gives recipes for common tasks.

## Enabling JSON output

Use the `--format json` flag with commands that produce output:

```sh
kimun search "query" --format json
kimun notes --format json
```

## JSON structure

The output is a single JSON object with two top-level keys:

### `metadata` object

- `workspace`: active workspace name
- `workspace_path`: absolute path to workspace directory
- `total_results`: number of notes returned
- `query`: the search query (null for `notes` listings)
- `is_listing`: true for `notes`, false for `search`
- `generated_at`: ISO 8601 timestamp

### `notes` array

Each note object contains:

- `path`: note path relative to workspace root (includes `.md`)
- `title`: note title (extracted from first heading or filename)
- `content`: full note content
- `size`: file size in bytes
- `modified`: last modified timestamp (Unix seconds)
- `created`: creation timestamp (Unix seconds, currently same as modified)
- `hash`: content hash (hex string)
- `journal_date`: date string `YYYY-MM-DD` if the note is a journal entry, otherwise absent
- `metadata.tags`: array of `#tag` strings extracted from content
- `metadata.links`: array of wikilink targets extracted from content
- `metadata.headers`: array of `{level, text}` objects for each Markdown heading

## Common recipes

### Find notes with a specific tag

```sh
kimun search "rust" --format json | jq '.notes[] | select(.metadata.tags[] == "rust")'
```

### Get note titles and paths

```sh
kimun notes --format json | jq '.notes[] | {title, path}'
```

### Count notes in workspace

```sh
kimun notes --format json | jq '.metadata.total_results'
```

### Sort notes by last modified (newest first)

```sh
kimun notes --format json | jq '[.notes[] | {title, path, modified}] | sort_by(.modified) | reverse'
```

### List all unique tags across all notes

```sh
kimun notes --format json | jq '[.notes[].metadata.tags[]] | unique | sort'
```

### Extract all wikilinks

```sh
kimun notes --format json | jq '[.notes[].metadata.links[]] | unique | sort'
```

### Find all journal entries

```sh
kimun notes --format json | jq '.notes[] | select(.journal_date != null) | {journal_date, title, path}'
```

### Find notes larger than 10KB

```sh
kimun notes --format json | jq '.notes[] | select(.size > 10240) | {path, size}'
```

### Get all headings from a search result

```sh
kimun search "project" --format json | jq '.notes[] | {path, headers: .metadata.headers}'
```

## Practical workflows

### Save to file

Export all notes for backup or processing:

```sh
kimun notes --format json > notes-backup.json
```

### Count journal entries

Count the notes that are journal entries:

```sh
kimun notes --format json | jq '[.notes[] | select(.journal_date != null) | .journal_date] | length'
```

### Build custom reports

Compute other figures from the JSON output:

```sh
# Get average note size
kimun notes --format json | jq '[.notes[].size] | add / length'

# Find notes created today
kimun notes --format json | jq '.notes[] | select(.created > (now | floor - 86400))'
```

## Quick note automation

`kimun note quick` saves text as a timestamped note in your inbox, which is useful in scripts:

```sh
# Log a command's output as a quick note
some-command 2>&1 | kimun note quick

# Capture clipboard contents (macOS)
pbpaste | kimun note quick

# Batch-capture from a file (one note per line)
while IFS= read -r line; do kimun note quick "$line"; done < ideas.txt
```

For shell aliases like `alias q='kimun note quick'`, see [Piping Output](@/guides/piping-output.md#shell-aliases-and-functions).

List inbox contents with `kimun note triage`, or use the MCP `triage_inbox` prompt to let an AI organize them.

## Tips

Test a query on a small result set before running it against a large workspace.
