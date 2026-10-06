+++
title = "CLI"
weight = 12
+++

# CLI

The `kimun` command gives you Kimün's features from the shell. Commands read content from stdin and write plain text or JSON to stdout, so you can use them in scripts, pipelines, and cron jobs.

## Global Configuration

Pass a custom config file to any command:

```sh
kimun --config /path/to/config.toml <subcommand>
```

Without `--config`, Kimün uses the default location:

- **Linux / macOS:** `~/.config/kimun/config.toml`
- **Windows:** `%USERPROFILE%\kimun\config.toml`

## Initial Setup

On first use, choose one of these approaches:

**Option A: start from the CLI**
```sh
kimun workspace init --name default /path/to/notes
```

**Option B: start from the TUI**
```sh
kimun
```
Then configure your workspace through the Preferences screen.

Legacy single-workspace configurations are automatically migrated to multi-workspace format.

## Workspaces

You can keep several workspaces, each with its own notes directory.

For the full workspace reference, see the [Workspaces](@/getting-started/workspaces.md) page.

**Quick reference:**
```sh
kimun workspace init --name work /path/to/work/notes
kimun workspace list
kimun workspace use work
kimun workspace rename old-name new-name
kimun workspace remove work
kimun workspace reindex work
```

## Search

Search notes in the current workspace:

```sh
kimun search "your search query"
kimun search "meeting -cancelled"                # Exclude terms
kimun search "=project -@draft"                  # Combine filters
kimun search "#important"                        # Filter by hashtag label
kimun search "meeting #important -#draft"        # Mix labels with other filters
kimun search "rust" --format json                # JSON output
```

### Flags

- `--format json`: output as JSON, for use with `jq` in scripts.
- `--format paths`: output bare paths only, one per line. Use it to pipe results into `kimun note show` or `fzf`.
- `--workspace <name>`: search a specific workspace instead of the current one.

### Query Syntax

The CLI uses the same query language as the TUI:

| Want | Operator | Example |
|---|---|---|
| Free text | *(just type)* | `meeting notes` |
| By note name | `=` / `name:` | `=tasks` |
| By section | `@` / `in:` | `@personal` |
| By path | `/` / `pt:` | `/journal` |
| By label | `#` / `lb:` | `#important` |
| Backlinks | `<` / `lk:` | `<projects` |
| Forward links | `>` / `fwd:` | `>projects` |
| Exclude | `-` prefix | `-#draft`, `-@temp` |

Space means AND, `*` is a wildcard, and case and accents are ignored. The full grammar (wildcards per operator, link matching rules, query variables) is on the [Search](@/using-kimun/search.md) page.

### Examples

```sh
# Find notes about "rust"
kimun search "rust"

# Find notes named "2024" but exclude "draft" in section titles
kimun search "=2024 -@draft"

# Find content under "Personal" section containing "kimun"
kimun search "@personal kimun"

# Find notes labelled #important but not #archived
kimun search "#important -#archived"

# Find notes that link to "projects" (its backlinks)
kimun search "<projects"

# Find notes that "projects" links to (its forward links)
kimun search ">projects"

# Combine with jq for advanced filtering
kimun search "rust" --format json | jq '.notes[] | select(.metadata.tags[] == "rust")'
```

## Labels

List every hashtag label in your vault with note counts:

```sh
kimun labels                  # alphabetical list: `name (N notes)`
kimun labels --format paths   # bare labels, one per line (pipeable)
kimun labels --format json    # JSON with total + per-label note_count
```

Labels come from in-text `#hashtag` tokens and from a frontmatter `tags` property. Hashtags inside code, HTML, link bodies, and wikilinks are not indexed. See [Search](@/using-kimun/search.md#labels) for the full label rules.

### JSON schema

```json
{
  "workspace": "personal",
  "total": 12,
  "labels": [
    { "name": "idea",     "note_count": 5 },
    { "name": "reading",  "note_count": 4 },
    { "name": "systems",  "note_count": 5 }
  ]
}
```

### Patterns

```sh
# Top 10 most-used labels
kimun labels --format json | jq -r '.labels | sort_by(-.note_count) | .[:10][] | "\(.note_count)\t\(.name)"'

# Labels that appear in only one note (orphans worth reviewing)
kimun labels --format json | jq -r '.labels[] | select(.note_count == 1) | .name'

# Open every note carrying a given label in the TUI editor of your choice
kimun search "#systems" --format paths | xargs -r $EDITOR

# Build a per-label index file
kimun labels --format paths | while read l; do
  echo "## $l"
  kimun search "#$l" --format paths | sed 's/^/- /'
  echo
done > vault-by-label.md

# Cross-tabulate two labels (notes carrying BOTH)
kimun search "#api #perf" --format paths

# Notes labelled #idea but not yet #done
kimun search "#idea -#done" --format paths
```

## Notes

List all notes in the current workspace:

```sh
kimun notes
kimun notes --path "journal/"                    # Filter by path prefix
kimun notes --format json                        # JSON output
```

### Flags

- `--path <prefix>`: filter notes by path prefix (e.g., `journal/`, `projects/`).
- `--format json`: output as JSON, for use with `jq` in scripts.
- `--format paths`: output bare paths only, one per line. Use it to pipe results into `kimun note show` or `fzf`.

### Examples

```sh
# List all notes
kimun notes

# List only journal entries
kimun notes --path "journal/"

# Get titles and paths as JSON
kimun notes --format json | jq '.notes[] | {title, path}'

# Count notes by workspace
kimun notes --format json | jq '.metadata | {workspace, total_results}'
```

## Show

Display note content and metadata in the terminal:

```sh
kimun note show "path/to/note"
kimun note show "path/to/note" "another/note"   # Multiple notes
```

### Flags

- `--format json`: output as JSON (default: text).

### Features

- Accepts note paths relative to workspace root
- Paths work with or without `.md` extension
- Reads from stdin for batch processing
- Displays content, title, tags, links, and backlinks

### Examples

```sh
# Show a single note
kimun note show "inbox/meeting-notes"

# Show multiple notes
kimun note show "projects/foo" "inbox/bar"

# Read paths from stdin (one per line)
echo "journal/2024-01-01" | kimun note show

# Pipe paths from search results
kimun search "rust" --format paths | kimun note show

# Show as JSON
kimun note show "inbox/meeting" --format json
```

## Note Operations

### Create

Create a new note. Fails with an error if the note already exists.

```sh
kimun note create "path/to/note" "Initial content"
echo "My content" | kimun note create "path/to/note"
```

### Features

- Accepts content as a second argument or from stdin (when stdin is not a TTY)
- Paths are relative to the configured `quick_note_path`, or absolute from the vault root when prefixed with `/`
- Prints `Note saved: <path>` on success

### Examples

```sh
# Create a note with inline content
kimun note create "inbox/idea" "Use kimun for daily notes"

# Create a note at an absolute vault path
kimun note create "/projects/roadmap" "Q3 goals"

# Pipe content from a command
date | kimun note create "inbox/timestamp"

# Capture command output into a new note
curl -s https://example.com/api/status | kimun note create "inbox/status-check"

# Create from a here-string
kimun note create "inbox/snippet" <<'EOF'
## Snippet

Some important code or text to save.
EOF
```

### Append

Append text to an existing note. Creates the note if it does not exist.

```sh
kimun note append "path/to/note" "Appended text"
echo "New line" | kimun note append "path/to/note"
```

### Features

- Accepts content as a second argument or from stdin (when stdin is not a TTY)
- If the note does not exist, it is created automatically
- New content is joined with a newline after the existing content
- Prints `Note saved: <path>` on success

### Examples

```sh
# Append a quick thought to an existing note
kimun note append "inbox/ideas" "Another idea just came to me"

# Log the output of a command to a running log note
echo "$(date): build succeeded" | kimun note append "logs/build-log"

# Accumulate cron job output
0 * * * * kimun note append "logs/hourly" "$(date): checked in"

# Append multiline content
kimun note append "inbox/research" <<'EOF'

## New finding

Something worth noting from today's reading.
EOF

# Use with search results: append a summary of found notes to a log
kimun search "rust" --format paths | kimun note append "inbox/rust-refs"
```

### Overwrite

Replace a note's entire body with new content. Because it discards the old
body, it requires `--force`.

```sh
kimun note overwrite "projects/roadmap" "Brand new body" --force
echo "New body" | kimun note overwrite "projects/roadmap" --force
```

#### Features

- Accepts content as a second argument or from stdin (when stdin is not a TTY)
- Requires `--force`; without it the command refuses to run (there is no
  interactive prompt, because the CLI is built for automation)
- Backs up the previous content first (see [Backups](#backups))

### Replace

Replace one piece of text in a note and leave the rest of it unchanged. The find text is a
literal substring by default, or a regular expression with `--regex`.

```sh
kimun note replace "projects/roadmap" "Q2" "Q3"
kimun note replace "projects/roadmap" "TODO" "DONE" --all
kimun note replace "notes/log" "v\d+\.\d+" "v2.0" --regex
kimun note replace "notes/log" "(\w+)@(\w+)" "$2.$1" --regex --all
kimun note replace "projects/roadmap" "TODO" "DONE" --all --preview
```

#### Features

- The find text must match exactly once; the command errors if it is missing
  or appears more than once, so it never edits the wrong place
- `--all` replaces every occurrence on purpose
- `--regex` treats the find text as a regular expression; the replacement may
  then reference capture groups (`$1`, `${name}`; use `$$` for a literal `$`,
  and `${1}`/`${name}` when the next character is alphanumeric, e.g. `${1}_`).
  Use inline flags for line/case behaviour: `(?m)`, `(?s)`, `(?i)`. An invalid
  pattern errors without touching the note.
- `--preview` is a dry run: it prints the resulting note content to stdout (the
  match count goes to stderr) and writes nothing. Pipe it to compare, e.g.
  `kimun note replace … --preview | diff <(kimun note show "…") -`
- Does not require `--force`, because it only changes the matched text
- Backs up the previous content first (see [Backups](#backups))

### Delete

Remove a note. Requires `--force`.

```sh
kimun note delete "inbox/stale-idea" --force
```

#### Features

- Requires `--force`; without it the command refuses to run
- Removes the note from the index as well as from disk
- Backs up the deleted content first (see [Backups](#backups))

### Properties

Read and edit a note's frontmatter [properties](@/using-kimun/search.md#properties) without touching the rest of the file.

```sh
kimun note prop list "projects/garden"               # key: value lines ("key:" when it has no readable value)
kimun note prop list "projects/garden" --format json # one JSON object (null when a key has no readable value)
kimun note prop get "projects/garden" status         # just the value (empty, or null in JSON, when it has none)
kimun note prop set "projects/garden" status active
kimun note prop set "projects/garden" tags garden spring   # several values → a list
kimun note prop set "projects/garden" due 2026-05-01
kimun note prop set "projects/garden" priority high --type text
kimun note prop remove "projects/garden" status
```

#### How `set` picks a type

- **`tags`** is always a list: comma-separated values become separate items (`set n tags "work, q1"`), and `set n tags` with no value clears it. **`aliases`** and **`cssclasses`** are always lists too, with values kept as given (an alias may contain a comma).
- **A key other notes already use:** the value must be exactly a value of the type most of them give it. If `priority` is a number elsewhere, `2` is stored as a number, while `high` is refused. So is `02134`, which storing as a number would rewrite. A single value for a list key becomes a one-item list.
- **A new key:** typed by its look. `5` and `4.5` → number, `true`/`false` → true/false, `2026-05-01` → date, `2026-05-01T14:30` → date & time, anything else → text. Text that only looks numeric (`02134`, `1.10`) stays text, so it is never rewritten.
- **Date & time values keep their form:** `2026-05-01T14:30` stays a local time (written without an offset, as Obsidian does), `2026-05-01T14:30:00+02:00` keeps its offset. Searches and sorting compare them as instants, a local time read as UTC.
- **`--type text|number|bool|date|datetime|list`** forces the type. Use it to store a value that doesn't fit the key's usual type; only this note changes. `--type list` with no value sets an empty list.

#### Features

- Keeps the rest of the frontmatter (format, comments, key order) as it was
- A note without frontmatter gets a TOML (`+++`) block; `--yaml` makes it YAML (`---`). An existing block keeps its format
- Keys are case-insensitive but keep the spelling you give them: `set n dueDate …` writes `dueDate`, and `dueDate`, `duedate` and `DUEDATE` all reach the same property. An existing key keeps the spelling already in the note
- Negative numbers work as values (`set n delta -5`); a text value starting with `-` goes after `--` (`set n mood -- -meh`)
- `get` fails for a property the note doesn't have; `remove` doesn't
- Refuses to edit a note whose existing frontmatter doesn't parse, leaving it untouched
- Backs up the previous content first (see [Backups](#backups))

### Backups

Every CLI (and MCP) edit that overwrites or deletes a note's content copies the
old content into a hidden, dated directory inside the vault before changing it.
These backups are excluded from indexing and search, kept for 30 days, then
purged automatically.

- Covers `overwrite`, `replace`, `delete`, `prop set`/`prop remove`, and the
  backlink rewrites performed by rename/move. `create` and a first-time `append` have nothing to back up.
- Interactive TUI editing does not create backups (the editor has its own
  history).
- If a backup cannot be written, the operation is aborted and the note is left
  untouched (fail-closed).

## Quick Note

Capture a thought as a new note. It is saved in the inbox directory with a timestamp-based filename.

```sh
kimun note quick "My quick thought"
echo "Piped idea" | kimun note quick
```

### Features

- Saves to the configured inbox directory (default: `/inbox`)
- Filename is generated from the current UTC time (`YYYY-MM-DDTHH-MM-SS.md`)
- Handles timestamp collisions by appending `-2`, `-3`, etc.
- Accepts content as an argument or from stdin
- Empty content is silently ignored (no note created)

### Examples

```sh
# Capture a quick thought
kimun note quick "Look into async trait patterns"

# Pipe in command output
echo "$(date +%H:%M) — deploy completed" | kimun note quick

# Capture a snippet from clipboard (macOS)
pbpaste | kimun note quick
```

## Inbox Triage

List notes in the inbox for review:

```sh
kimun note triage
```

Prints each inbox note's path and title. Use this to see what has accumulated before organizing with the [MCP triage prompt](@/using-kimun/ai-mcp-server.md#prompts).

## Journal

Append to or show journal entries. Journal entries are stored as `YYYY-MM-DD.md` files in the vault's configured journal directory.

```sh
kimun journal "Today's entry"
kimun journal --date 2024-01-15 "Retroactive entry"
kimun journal show
kimun journal show --date 2024-01-15
```

### Append

Appends text to a journal entry. Creates the entry if it does not exist.

```sh
kimun journal [--date YYYY-MM-DD] [content]
```

### Features

- Defaults to today's date; use `--date` to target a specific entry
- Accepts content as an argument or from stdin (when stdin is not a TTY)
- New content is joined with a newline after any existing content
- Prints `Note saved: <path>` on success

### Examples

```sh
# Capture a quick thought
kimun journal "Had a good retro today"

# Pipe in a timestamped log line
echo "$(date +%H:%M) — finished the auth refactor" | kimun journal

# Record the result of a script
./run-tests.sh | tail -1 | kimun journal

# Append to a specific date's entry
kimun journal --date 2024-01-15 "Retroactive note"

# Append a longer entry with a here-string
kimun journal <<'EOF'

## Evening review

- Completed the CLI documentation
- Reviewed two PRs
- TODO: follow up with team on deploy schedule
EOF

# Use in a cron job to log system info daily
@daily kimun journal "$(hostname): $(uptime)"

# Chain with other commands — log search activity
kimun search "todo" --format paths | xargs -I{} echo "open: {}" | kimun journal
```

### Show

Displays a journal entry's content and metadata.

```sh
kimun journal show [--date YYYY-MM-DD] [--format text|json]
```

### Flags

- `--date <YYYY-MM-DD>`: show a specific date's entry (defaults to today).
- `--format json`: output as JSON, for use with `jq` in scripts.

### Examples

```sh
# Show today's journal entry
kimun journal show

# Show a specific date
kimun journal show --date 2024-01-15

# Output as JSON for scripting
kimun journal show --format json | jq '.notes[0].metadata.headers'

# Get today's headings
kimun journal show --format json | jq '.notes[0].metadata.headers[].text'
```

## Doctor

```sh
kimun doctor
```

Reports what your terminal can deliver and what becomes of every key binding.
Run it when a key seems to do nothing, or the wrong thing, and paste the output
into a bug report.

Terminals send `Ctrl` chords as single bytes, and some of those bytes already
belong to real keys: `Ctrl+I` is `Tab`, `Ctrl+M` is `Enter`, `Ctrl+[` is
`Esc`. `doctor` names every binding that cannot arrive on *this* terminal, and
which key it arrives as instead:

```
Terminal
  TERM                 xterm-256color
  TERM_PROGRAM         (unset)
  kitty keyboard       no reply — Ctrl chords share bytes with Tab, Enter and Esc
  tty erase character  ^? (0x7f) — the usual setting
  ctrl_h = auto         Ctrl+H is a chord (the tty's erase character is not 0x08)

Key bindings
  OpenSettings           F4 · ctrl&, (not sent by this terminal)
  Quit                   ctrl&Q
  ...

Every bound action has a key this terminal can send.
```

An unreachable chord is fine as long as the action has another key. In the
example above, `Ctrl+,` cannot arrive, but `F4` still opens Preferences. Check
the last line: it names any action left with no usable key at all.

Two of these answers come from the terminal itself, so run `doctor` in the
terminal you use Kimün in. If you redirect its output to a file or a pipe,
nothing answers the query, and `doctor` reports that instead of guessing.

The default keymap always keeps a reachable key for every action, so a
complaint here is almost always about a `[key_bindings]` override. See [Key
Bindings](@/getting-started/configuration.md#key-bindings). The one exception
is `ctrl_h = "backspace"`, which gives up the `Ctrl+H` chord by design; any
action bound only to it is listed here, and `doctor` says so when that is why.

## JSON Output

Both `search` and `notes` support JSON output for scripting.

### Output Structure

```json
{
  "metadata": {
    "workspace": "default",
    "workspace_path": "/home/user/notes",
    "total_results": 5,
    "query": "rust",
    "is_listing": false,
    "generated_at": "2024-03-27T10:30:00Z"
  },
  "notes": [
    {
      "path": "projects/rust-cli.md",
      "title": "Rust CLI Project",
      "content": "...",
      "size": 1024,
      "modified": 1711525800,
      "created": 1711525800,
      "hash": "abc123def",
      "journal_date": null,
      "metadata": {
        "tags": ["rust", "cli"],
        "links": ["projects/parser", "projects/lexer"],
        "headers": ["Overview", "Architecture", "TODO"],
        "properties": {"status": "active", "priority": 2, "tags": ["rust", "cli"]}
      },
      "backlinks": ["blog/rust-post.md"]
    }
  ]
}
```

In `properties`, a key the note has with no value (YAML `due:`), or with one Kimün can't read (a TOML time of day such as `10:30:00`, `nan`), is `null`.

### Processing with jq

```sh
# Extract all tags from search results
kimun search "project" --format json | jq '.notes[].metadata.tags[]'

# Find notes modified in the last 7 days
kimun notes --format json | jq '.notes[] | select(.modified > now - 604800)'

# Get path and title only
kimun notes --format json | jq '.notes[] | {path, title}'

# Count total notes
kimun notes --format json | jq '.metadata.total_results'
```

For scripting guides, see [Scripting](@/guides/scripting.md).
