+++
title = "Scripting with JSON"
weight = 3
+++

# Scripting with JSON

Kimün can print search results and note listings as JSON. In scripts, you can process the JSON with `jq` or other tools. This guide describes the JSON structure and gives recipes for common tasks.

## Enabling JSON output

To get JSON output, use the `--format json` flag with a command that prints results:

```sh
kimun search "query" --format json
kimun notes --format json
```

## JSON structure

The output is one JSON object with two top-level keys: `metadata` and `notes`.

### `metadata` object

- `workspace`: active workspace name
- `workspace_path`: absolute path to the workspace directory
- `total_results`: number of notes returned
- `query`: the search query (null for `notes` listings)
- `is_listing`: true for `notes`, false for `search`
- `generated_at`: ISO 8601 timestamp

### `notes` array

Each note object contains:

- `path`: note path relative to workspace root (includes `.md`)
- `title`: note title (from the first heading or the file name)
- `content`: full note content
- `size`: file size in bytes
- `modified`: last modified timestamp (Unix seconds)
- `created`: creation timestamp (Unix seconds, currently the same as `modified`)
- `hash`: content hash (hex string)
- `journal_date`: date string `YYYY-MM-DD` if the note is a journal entry. Otherwise this key is absent.
- `metadata.tags`: array of `#tag` strings from the content
- `metadata.links`: array of wikilink targets from the content
- `metadata.headers`: array of `{level, text}` objects, one for each Markdown heading

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

To export all notes for a backup or for processing, run:

```sh
kimun notes --format json > notes-backup.json
```

### Count journal entries

Count the notes that are journal entries:

```sh
kimun notes --format json | jq '[.notes[] | select(.journal_date != null) | .journal_date] | length'
```

### Build custom reports

To calculate other values from the JSON output, run:

```sh
# Get average note size
kimun notes --format json | jq '[.notes[].size] | add / length'

# Find notes created today
kimun notes --format json | jq '.notes[] | select(.created > (now | floor - 86400))'
```

## Quick note automation

`kimun note quick` saves text as a note in your inbox. The file name is a timestamp. The command is useful in scripts:

```sh
# Log a command's output as a quick note
some-command 2>&1 | kimun note quick

# Capture clipboard contents (macOS)
pbpaste | kimun note quick

# Batch-capture from a file (one note per line)
while IFS= read -r line; do kimun note quick "$line"; done < ideas.txt
```

For shell aliases such as `alias q='kimun note quick'`, see [Piping Output](@/guides/piping-output.md#shell-aliases-and-functions).

To list the inbox contents, run `kimun note triage`. Or use the MCP `triage_inbox` prompt to let an AI organize the notes.

## Tips

Test a query on a small result set before you run it on a large workspace.
