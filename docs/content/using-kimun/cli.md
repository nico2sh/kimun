+++
title = "CLI"
weight = 12
+++

# CLI

The `kimun` command gives you the features of Kimün from the shell. Commands read content from stdin and write plain text or JSON to stdout. You can use them in scripts, pipelines, and cron jobs.

## Global Configuration

To use a custom configuration file, pass it to any command:

```sh
kimun --config /path/to/config.toml <subcommand>
```

Without `--config`, Kimün uses the default location:

- Linux and macOS: `~/.config/kimun/config.toml`
- Windows: `%USERPROFILE%\kimun\config.toml`

## Initial Setup

On first use, choose one of these two ways to start.

Option A: start from the CLI.
```sh
kimun workspace init --name default /path/to/notes
```

Option B: start from the TUI.
```sh
kimun
```
Then set up your workspace in the Preferences screen.

Kimün migrates a legacy single-workspace configuration to the multi-workspace format automatically.

## Workspaces

You can keep several workspaces. Each workspace has its own notes directory.

For the full workspace reference, see the [Workspaces](@/getting-started/workspaces.md) page.

Quick reference:
```sh
kimun workspace init --name work /path/to/work/notes
kimun workspace list
kimun workspace use work
kimun workspace rename old-name new-name
kimun workspace remove work
kimun workspace reindex work
```

## Search

To search the notes in the current workspace, run `kimun search`:

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

A space means AND. The `*` character is a wildcard. Case and accents are ignored. The [Search](@/using-kimun/search.md) page has the full grammar, which includes the wildcards for each operator, the link matching rules, and the query variables.

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

To list every hashtag label in your vault with the note count of each label, run `kimun labels`:

```sh
kimun labels                  # alphabetical list: `name (N notes)`
kimun labels --format paths   # bare labels, one per line (pipeable)
kimun labels --format json    # JSON with total + per-label note_count
```

Labels come from in-text `#hashtag` tokens and from a frontmatter `tags` property. Kimün does not index hashtags inside code, HTML, link bodies, and wikilinks. For the full label rules, see [Search](@/using-kimun/search.md#labels).

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

To list all notes in the current workspace, run `kimun notes`:

```sh
kimun notes
kimun notes --path "journal/"                    # Filter by path prefix
kimun notes --format json                        # JSON output
```

### Flags

- `--path <prefix>`: filter notes by path prefix (for example `journal/` or `projects/`).
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

To display the content and metadata of a note in the terminal, run `kimun note show`:

```sh
kimun note show "path/to/note"
kimun note show "path/to/note" "another/note"   # Multiple notes
```

### Flags

- `--format json`: output as JSON (the default is text).

### Features

- The command accepts note paths relative to the workspace root.
- Paths work with or without the `.md` extension.
- The command reads paths from stdin for batch processing.
- The command displays the content, title, tags, links, and backlinks.

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

To create a new note, run `kimun note create`. If the note already exists, the command fails with an error.

```sh
kimun note create "path/to/note" "Initial content"
echo "My content" | kimun note create "path/to/note"
```

### Features

- The command accepts content as a second argument. If stdin is not a TTY, the command also reads content from stdin.
- A path is relative to the configured `quick_note_path`. A path that starts with `/` is absolute from the vault root.
- On success, the command prints `Note saved: <path>`.

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

To add text to the end of an existing note, run `kimun note append`. If the note does not exist, the command creates it.

```sh
kimun note append "path/to/note" "Appended text"
echo "New line" | kimun note append "path/to/note"
```

### Features

- The command accepts content as a second argument. If stdin is not a TTY, the command also reads content from stdin.
- If the note does not exist, the command creates it.
- The command joins the new content to the existing content with a newline.
- On success, the command prints `Note saved: <path>`.

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

To replace the entire body of a note with new content, run `kimun note overwrite`. The command discards the old body, so it requires `--force`.

```sh
kimun note overwrite "projects/roadmap" "Brand new body" --force
echo "New body" | kimun note overwrite "projects/roadmap" --force
```

#### Features

- The command accepts content as a second argument. If stdin is not a TTY, the command also reads content from stdin.
- The command requires `--force`. Without it, the command refuses to run. The command has no interactive prompt because the CLI is built for automation.
- The command first makes a backup of the previous content (see [Backups](#backups)).

### Replace

To replace one piece of text in a note and leave the rest unchanged, run `kimun note replace`. The find text is a literal substring by default. With `--regex`, it is a regular expression.

```sh
kimun note replace "projects/roadmap" "Q2" "Q3"
kimun note replace "projects/roadmap" "TODO" "DONE" --all
kimun note replace "notes/log" "v\d+\.\d+" "v2.0" --regex
kimun note replace "notes/log" "(\w+)@(\w+)" "$2.$1" --regex --all
kimun note replace "projects/roadmap" "TODO" "DONE" --all --preview
```

#### Features

- The find text must match exactly once. If the text is missing or appears more than once, the command fails with an error. This way the command never edits the wrong place.
- `--all` replaces every occurrence on purpose.
- `--regex` treats the find text as a regular expression. The replacement can then use capture groups: `$1` and `${name}`. Use `$$` for a literal `$`. If the next character is alphanumeric, use `${1}` or `${name}`, for example `${1}_`.
- With `--regex`, use inline flags to control line and case behavior: `(?m)`, `(?s)`, `(?i)`. An invalid pattern causes an error and the command does not change the note.
- `--preview` is a dry run. The command prints the resulting note content to stdout and the match count to stderr. The command writes nothing. To compare the result with the current note, pipe the output to `diff`: `kimun note replace … --preview | diff <(kimun note show "…") -`
- The command does not require `--force` because it changes only the matched text.
- The command first makes a backup of the previous content (see [Backups](#backups)).

### Delete

To remove a note, run `kimun note delete`. The command requires `--force`.

```sh
kimun note delete "inbox/stale-idea" --force
```

#### Features

- The command requires `--force`. Without it, the command refuses to run.
- The command removes the note from the index and from the disk.
- The command first makes a backup of the deleted content (see [Backups](#backups)).

### Properties

To read and edit the frontmatter [properties](@/using-kimun/search.md#properties) of a note without changing the rest of the file, run `kimun note prop`.

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

- `tags` is always a list. Comma-separated values become separate items (`set n tags "work, q1"`). The command `set n tags` with no value clears the list. `aliases` and `cssclasses` are also always lists. For these two keys, the command keeps the values as you give them, because an alias can contain a comma.
- If other notes already use the key, the value must exactly match a value of the type that most of those notes give it. Example: other notes give `priority` a number. The command stores `2` as a number and refuses `high`. The command also refuses `02134`, because a number type will rewrite it. A single value for a list key becomes a list with one item.
- If the key is new, the command picks the type from the form of the value. `5` and `4.5` become numbers. `true` and `false` become true/false values. `2026-05-01` becomes a date. `2026-05-01T14:30` becomes a date and time. Anything else becomes text. Text that only looks like a number (`02134`, `1.10`) stays text, so the command never rewrites it.
- Date and time values keep their form. `2026-05-01T14:30` stays a local time. The command writes it without an offset, as Obsidian does. `2026-05-01T14:30:00+02:00` keeps its offset. Searches and sorting compare these values as instants in time, and they read a local time as UTC.
- `--type text|number|bool|date|datetime|list` forces the type. Use it to store a value that does not fit the usual type of the key. Only the one note changes. `--type list` with no value sets an empty list.

#### Features

- The command keeps the rest of the frontmatter (format, comments, key order) as it was.
- A note without frontmatter gets a TOML (`+++`) block. With `--yaml`, the block is YAML (`---`). An existing block keeps its format.
- Keys are case-insensitive, but the command keeps the spelling that you give. For example, `set n dueDate …` writes `dueDate`, and `dueDate`, `duedate`, and `DUEDATE` all reach the same property. An existing key keeps the spelling that is already in the note.
- Negative numbers work as values (`set n delta -5`). A text value that starts with `-` goes after `--` (`set n mood -- -meh`).
- `get` fails for a property that the note does not have. `remove` does not fail.
- If the existing frontmatter of a note does not parse, the command refuses to edit the note and leaves it unchanged.
- The command first makes a backup of the previous content (see [Backups](#backups)).

### Backups

A CLI or MCP edit can overwrite or delete the content of a note. Before it does, Kimün copies the old content into a hidden, dated directory inside the vault. Kimün excludes these backups from indexing and search. Kimün keeps them for 30 days and then purges them automatically.

- Backups cover `overwrite`, `replace`, `delete`, `prop set`, `prop remove`, and the backlink rewrites that rename and move operations perform. `create` and a first-time `append` have nothing to back up.
- Interactive TUI editing does not create backups, because the editor has its own history.
- If Kimün cannot write a backup, it stops the operation and leaves the note unchanged (fail-closed).

## Quick Note

To capture a thought as a new note, run `kimun note quick`. Kimün saves the note in the inbox directory with a filename that is based on the time.

```sh
kimun note quick "My quick thought"
echo "Piped idea" | kimun note quick
```

### Features

- The command saves to the configured inbox directory (the default is `/inbox`).
- The command generates the filename from the current UTC time (`YYYY-MM-DDTHH-MM-SS.md`).
- If two notes get the same timestamp, the command adds `-2`, `-3`, and so on to the filename.
- The command accepts content as an argument or from stdin.
- The command ignores empty content without a message and creates no note.

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

To list the notes in the inbox for review, run `kimun note triage`:

```sh
kimun note triage
```

The command prints the path and title of each inbox note. Use it to see what the inbox holds before you organize the notes with the [MCP triage prompt](@/using-kimun/ai-mcp-server.md#prompts).

## Journal

To add to or show journal entries, run `kimun journal`. Kimün stores journal entries as `YYYY-MM-DD.md` files in the journal directory of the vault. You set this directory in the configuration.

```sh
kimun journal "Today's entry"
kimun journal --date 2024-01-15 "Retroactive entry"
kimun journal show
kimun journal show --date 2024-01-15
```

### Append

The command adds text to a journal entry. If the entry does not exist, the command creates it.

```sh
kimun journal [--date YYYY-MM-DD] [content]
```

### Features

- The command uses today's date by default. Use `--date` to target a specific entry.
- The command accepts content as an argument. If stdin is not a TTY, the command also reads content from stdin.
- The command joins the new content to any existing content with a newline.
- On success, the command prints `Note saved: <path>`.

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

The command displays the content and metadata of a journal entry.

```sh
kimun journal show [--date YYYY-MM-DD] [--format text|json]
```

### Flags

- `--date <YYYY-MM-DD>`: show the entry of a specific date (the default is today).
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

The command reports what your terminal can deliver and what happens to every key binding. If a key does nothing or does the wrong thing, run the command and paste the output into a bug report.

A terminal sends `Ctrl` chords as single bytes. Some of those bytes already belong to real keys: `Ctrl+I` is `Tab`, `Ctrl+M` is `Enter`, and `Ctrl+[` is `Esc`. The `doctor` command names every binding that cannot arrive on this terminal. It also names the key that the binding arrives as instead:

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

If the action has another key, an unreachable chord is not a problem. In the example above, `Ctrl+,` cannot arrive, but `F4` still opens Preferences. Read the last line. It names any action that has no usable key.

The terminal itself answers two of the queries, so run `doctor` in the terminal where you use Kimün. If you redirect the output to a file or a pipe, nothing answers the query. In that case `doctor` reports that fact and does not guess.

The default keymap always keeps a reachable key for every action. A complaint from `doctor` is therefore almost always about a `[key_bindings]` override. See [Key Bindings](@/getting-started/configuration.md#key-bindings). One exception exists: `ctrl_h = "backspace"` gives up the `Ctrl+H` chord by design. If an action is bound only to `Ctrl+H`, `doctor` lists the action and says that this setting is the reason.

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

In `properties`, the value is `null` in two cases. In the first case, the note has the key with no value (YAML `due:`). In the second case, the value is one that Kimün cannot read (a TOML time of day such as `10:30:00`, or `nan`).

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
