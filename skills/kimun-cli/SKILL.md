---
name: kimun-cli
description: Use when the user has kimun installed and wants to create, append, search, or read notes from the terminal, or when automating note management as part of a workflow or agent task
---

# Kimün CLI Reference

Kimün is a local-first, terminal notes app. Notes are plain Markdown files indexed for fast search. Use the CLI to write, append, search, and read notes without opening the TUI — ideal for automation, piping, and AI-driven workflows.

## Quick Reference

| Task | Command |
|------|---------|
| Create note | `kimun note create "path" "content"` |
| Append to note | `kimun note append "path" "content"` |
| Overwrite a note | `kimun note overwrite "path" "content" --force` |
| Replace text in a note | `kimun note replace "path" "old" "new" [--all] [--regex] [--preview]` |
| Delete a note | `kimun note delete "path" --force` |
| Log to today's journal | `kimun journal "text"` |
| Log to a specific date | `kimun journal --date YYYY-MM-DD "text"` |
| Show today's journal | `kimun journal show` |
| Show a specific journal entry | `kimun journal show --date YYYY-MM-DD` |
| Show note content | `kimun note show "path"` |
| Search notes | `kimun search "query"` |
| List notes | `kimun notes` |
| List all labels | `kimun labels` |

## Writing Notes

All write commands accept content as a second argument **or from stdin** (when stdin is not a TTY).

### Create
Creates a new note. **Fails if the note already exists** — use `append` if unsure.

```sh
kimun note create "inbox/idea" "My content"
echo "My content" | kimun note create "inbox/idea"
```

### Append
Appends to a note. **Creates it automatically if it doesn't exist.** Safe to use as the default write command.

```sh
kimun note append "inbox/log" "New entry"
echo "Entry" | kimun note append "inbox/log"
```

## Modifying Notes

Three destructive operations let you change or remove existing content. Each one
**backs up the note first** (see below), so an edit is recoverable.

### Overwrite
Replaces a note's **entire** body. Requires `--force` (it discards the old body).
Content comes from an argument or stdin.

```sh
kimun note overwrite "projects/roadmap" "Brand new body" --force
echo "New body" | kimun note overwrite "projects/roadmap" --force
```

### Replace
Swaps text for new text, leaving the rest of the note intact. The find text must
match **exactly once** — the command errors if it is missing or appears more than
once, so it never edits the wrong spot. Use `--all` to replace every occurrence.
No `--force` needed (it's a targeted, scriptable edit).

By default the find text is a **literal substring**. Add `--regex` to treat it as
a regular expression; the replacement may then use capture references (`$1`,
`${name}`; `$$` for a literal `$`, and `${1}`/`${name}` when the next character
is alphanumeric, e.g. `${1}_`). Inline flags `(?m)`, `(?s)`, `(?i)` control
line/case behaviour. An invalid pattern errors without touching the note.

`--preview` is a dry run: it prints the resulting note content to stdout (match
count to stderr) and writes nothing — useful to inspect before committing.

```sh
kimun note replace "projects/roadmap" "Q2" "Q3"
kimun note replace "projects/roadmap" "TODO" "DONE" --all
kimun note replace "notes/log" "v\d+\.\d+" "v2.0" --regex
kimun note replace "notes/log" "(\w+)@(\w+)" "$2.$1" --regex --all
kimun note replace "projects/roadmap" "TODO" "DONE" --all --preview
```

### Delete
Removes a note. Requires `--force`.

```sh
kimun note delete "inbox/stale-idea" --force
```

### Automatic backups
Every CLI/MCP edit that overwrites or deletes a note's content first copies the
old content into a hidden, dated backup inside the vault (excluded from search).
Backups are kept for 30 days, then purged automatically. This covers `overwrite`,
`replace`, `delete`, and the backlink rewrites done by rename/move. `create` and
`append` of a new note have nothing to back up. Interactive TUI editing does not
back up (the editor has its own history). If a backup can't be written, the edit
is aborted (fail-closed) — the note is left untouched.

## Journal

`kimun journal` appends to a journal entry (today by default). `kimun journal show` displays one.

```sh
kimun journal "Captured this finding"             # append to today
kimun journal --date 2024-01-15 "Backdated note"  # append to specific date
echo "$(date +%H:%M) — task complete" | kimun journal

kimun journal show                                # show today's entry
kimun journal show --date 2024-01-15             # show specific date
kimun journal show --format json                 # JSON output
```

- Defaults to today; `--date YYYY-MM-DD` targets any date
- Creates the entry if it doesn't exist
- `show` supports `--format text` (default) and `--format json`

## Path Rules

- `"inbox/note"` — relative to the configured `quick_note_path` (default: vault root)
- `"/inbox/note"` — absolute from vault root; use a leading `/` to be explicit
- `.md` extension is optional in all commands

## Searching

```sh
kimun search "query"               # Full-text search
kimun search "query" --format json # JSON output
kimun search "query" --format paths # Bare paths, one per line (pipeable)
```

### Query syntax

Free text is case-insensitive, diacritics-ignored. Multiple terms = AND.

**Wildcards** — `*` matches any sequence of characters, works in free text and with operators:

```
kimu*          # starts with "kimu"
*report        # ends with "report"
*meeting*      # contains "meeting" anywhere
@task*         # filename starts with "task"
@*2024*        # filename contains "2024"
```

| Operator | Long form | Matches |
|----------|-----------|---------|
| `@term` | `at:term` | filename contains term |
| `<term` | `in:term` | Markdown section heading contains term |
| `/term` | `pt:term` | note path (directory) contains term |
| `#label` | `lb:label` | note carries that hashtag label (from `#label` in body) |
| `>note` | `lk:note` | notes that link to `note` (its backlinks) |
| `-term` | | exclude notes containing term |

**Hashtag labels** — any `#name` token in a note body (letters/digits/underscore) is indexed as a label and is case-insensitive. Hashtags inside frontmatter, code spans, fenced blocks, HTML, link bodies, and wikilinks are NOT indexed. Multiple `#` filters AND together.

**Link filter** — `>note` (or `lk:note`) matches notes that link to `note`. Matched by note name (`.md` optional, case-insensitive); a bare name matches a linked note in any folder, `>dir/note` disambiguates, and `*` wildcards work (`>proj*`). Only links to other notes count, not attachments or URLs.

**Exclusion composes with all operators — `-` leads, then the operator:**

```
-cancelled           # exclude notes containing "cancelled"
-@temp               # exclude notes with "temp" in filename
-<draft              # exclude notes with "draft" in any section title
-/private            # exclude notes under a "private/" path
-#archived           # exclude notes carrying #archived label
->draft              # exclude notes that link to "draft"
```

**Combining filters** (all terms are ANDed):

```
meeting -cancelled           # "meeting" but not "cancelled"
@tasks <work report          # file "tasks", has "Work" section, contains "report"
@2024 -<draft                # files from 2024, no "draft" section title
/journal <tasks -done        # in journal/, "tasks" section, excluding "done"
<personal kimun              # "kimun" under a "Personal" section
>spec #urgent                # links to "spec" and labelled "urgent"
>kimun ->draft               # links to "kimun" but not to "draft"
#important -#archived        # notes carrying #important but not #archived
meeting #important           # "meeting" in notes also carrying #important
```

## Listing Labels

`kimun labels` enumerates every distinct hashtag label in the vault with note counts. Three formats:

```sh
kimun labels                  # text: `name (N notes)` per line, alphabetical
kimun labels --format paths   # bare label names, one per line (pipeable)
kimun labels --format json    # JSON: { workspace, total, labels: [{name, note_count}] }
```

### When to reach for it

- **Map the vault's topical landscape** before answering "what does the user write about?" — `kimun labels` is the cheapest taxonomy survey available.
- **Discover orphan/typo labels** — single-use labels often reveal a typo (`#imporant` vs `#important`) or a rarely-touched topic worth pruning.
- **Drive label-based agentic browsing** — pick a label, then `kimun search "#label" --format paths | kimun note show` to read every note under it.
- **Build per-label digests / per-topic dashboards** programmatically.

### JSON schema

```json
{
  "workspace": "personal",
  "total": 12,
  "labels": [
    { "name": "idea",    "note_count": 5 },
    { "name": "reading", "note_count": 4 }
  ]
}
```

### Effective patterns

```sh
# Top 10 most-used labels (most signal-rich topics first)
kimun labels --format json | jq -r '.labels | sort_by(-.note_count) | .[:10][] | "\(.note_count)\t\(.name)"'

# Single-use labels (candidate orphans / typos)
kimun labels --format json | jq -r '.labels[] | select(.note_count == 1) | .name'

# Read every note carrying a label
kimun search "#systems" --format paths | kimun note show

# Cross-tabulate two labels (AND)
kimun search "#api #perf" --format paths

# Label minus another label (e.g. open ideas)
kimun search "#idea -#done" --format paths

# Per-label index file
kimun labels --format paths | while read l; do
  echo "## $l"
  kimun search "#$l" --format paths | sed 's/^/- /'
  echo
done > vault-by-label.md

# Quick "what do I work on?" summary
kimun labels --format json | jq -r '.labels[] | "\(.note_count)\t\(.name)"' | sort -rn | head
```

### Notes

- Label names are stored lowercase. `#Foo` and `#foo` collapse into one entry.
- A label is only counted once per note even if the hashtag appears many times.
- Counts reflect what's INDEXED — hashtags inside frontmatter, code, HTML, link bodies, and wikilinks are excluded.

## Reading Notes

```sh
kimun note show "path/to/note"           # Display note content and metadata
kimun notes                              # List all notes
kimun notes --path "journal/"            # Filter by path prefix
kimun notes --format json                # JSON output
kimun notes --format paths               # Bare paths for piping
```

## JSON Output

`search`, `notes`, `note show`, and `journal show` all support `--format json`. Each note object includes:
`path`, `title`, `content`, `size`, `modified`, `journal_date`, `metadata.tags`, `metadata.links`, `metadata.headers`.

```sh
# Get titles and paths
kimun notes --format json | jq '.notes[] | {title, path}'

# Find all journal entries
kimun notes --format json | jq '.notes[] | select(.journal_date != null)'

# Filter by tag
kimun search "project" --format json | jq '.notes[] | select(.metadata.tags[] == "active")'

# Get today's journal headings
kimun journal show --format json | jq '.notes[0].metadata.headers[].text'

# Pipe search results directly into note show
kimun search "todo" --format paths | kimun note show
```

## Patterns for AI Workflows

```sh
# Log a research finding to today's journal
kimun journal "Finding: X is faster than Y for large datasets"

# Capture command output as a timestamped note
some-command | kimun note create "logs/output-$(date +%F)"

# Append a running log (safe: creates if missing)
echo "$(date): step completed" | kimun note append "logs/progress"

# Search for context before starting a task
kimun search "authentication" --format json | jq '.notes[] | {title, path}'

# Read a specific note for context
kimun note show "projects/roadmap"

# Read today's journal for context
kimun journal show

# Chain: search → read all matching notes
kimun search "meeting notes" --format paths | kimun note show

# Survey topics before working: list labels, pick the relevant one, read all notes
kimun labels
kimun search "#api" --format paths | kimun note show

# Pre-tag a captured note in one shot (labels live in body, not frontmatter)
echo "Quick thought about caching. #idea #perf" | kimun note append "inbox/cache-thoughts"
```

## Common Mistakes

- **`create` on an existing note** — it fails. Use `append` when you're not sure if the note exists.
- **`overwrite`/`delete` without `--force`** — they refuse to run. The flag is the confirmation; there is no interactive prompt (the CLI is built for automation).
- **`replace` with a non-unique `old` string** — it errors rather than guess. Make the match unique, or pass `--all` to replace every occurrence on purpose.
- **No stdin from a live terminal** — piping works (`echo "x" | kimun journal`); passing no content from an interactive terminal produces an empty write.
- **Relative vs absolute paths** — if a `quick_note_path` is set in `kimun_config.toml`, relative paths are resolved against it. Prefix with `/` to always target the vault root explicitly.
- **`kimun journal show` — `--format paths` is not supported**; use `text` or `json`.
