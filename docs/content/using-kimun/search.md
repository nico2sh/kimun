+++
title = "Search"
weight = 12
+++

# Search

Kimün provides powerful search capabilities to find notes by content, filename, section, and path. Search indexes all Markdown files in your workspace and supports structured queries with operators and filters.

## Opening search

### In TUI
Press `Ctrl+E` to open the search box. Start typing to search across note content and filenames.

### In CLI
Use the `search` command:

```sh
kimun search "your search query"
```

## Free text

Free text search looks across both note content and filenames simultaneously. Searches are:
- **Case-insensitive**: `kimun` matches `Kimün`, `KIMÜN`, `kimun`, etc.
- **Diacritics-ignored**: `kimun` matches `Kimün`, accent marks are ignored
- **Wildcard-supported**: Use `*` to match patterns

### Wildcard patterns

- `kimu*` — matches anything starting with "kimu" (e.g., "kimun", "kimune", "kimurei")
- `*meeting*` — matches "meeting" anywhere in the note (e.g., "meeting notes", "team-meeting", "zoom-meeting-2024")
- `*report` — matches anything ending with "report"

### Multiple terms

Space between terms functions as AND — all terms must match for a note to appear:

```
kimun search     → must contain both "kimun" AND "search"
task report      → must contain both "task" AND "report"
```

## Operators

Operators allow you to filter by filename, section, or path. Each operator has both a short form (symbol) and a long form (colon-prefixed).

### `@` or `at:` — filename filter

Filter notes by their filename (basename only, not full path):

```
@tasks           → notes whose filename contains "tasks"
at:tasks         → same (long form)
@project         → notes with "project" in the filename
at:notes         → notes with "notes" in the filename
```

### `>` or `in:` — section filter

Filter notes by Markdown sections (defined by `#`, `##`, `###`, etc.). The search term must appear within that section:

```
>personal        → content under a "Personal" heading
in:personal      → same (long form)
>work            → content under a "Work" heading
in:meeting       → content under a "Meeting" heading
```

Section names are matched case-insensitively against heading text. A note matches if any of its sections contain the search term.

### `/` or `pt:` — path filter

Filter notes by their full path within the workspace:

```
/docs            → notes under a "docs/" directory
pt:docs          → same (long form)
/journal/2024    → notes under "journal/2024/" directory
/archive         → notes under an "archive/" directory
```

Paths are matched as prefixes, so `/docs` matches both `/docs/readme.md` and `/docs/guides/tutorial.md`.

## Exclusion operators

Use the `-` prefix to exclude terms from search results. Exclusion works with all operators and free text:

```
-cancelled           → exclude notes containing "cancelled"
>-draft              → exclude notes with "draft" in any section title
@-temp               → exclude notes with "temp" in the filename
/-private            → exclude notes under a "private/" directory
```

### Exclusion-only searches

You can search using only exclusions to find all notes except those matching the exclusion:

```
-cancelled           → all notes EXCEPT those containing "cancelled"
>-draft              → all notes EXCEPT those with "draft" in section titles
@-temp               → all notes EXCEPT those with "temp" in the filename
/-archive            → all notes EXCEPT those under an "archive/" directory
```

## Combining filters

All operators compose freely in a single query. Space between terms = AND.

Each term (with or without an operator prefix) must match for a note to appear:

```
@tasks >work report                → file "tasks", has "Work" section, contains "report"
>personal kimun                    → "kimun" under a "Personal" section
@thoughts kimun                    → file "thoughts" containing "kimun"
meeting -cancelled                 → "meeting" but not "cancelled"
@2024 >-draft                      → files from 2024 without "draft" in section titles
/journal >-temp report             → in journal/, not titled "temp", containing "report"
screen* @notes                     → starts with "screen", in file "notes"
>personal report -completed        → "report" under "Personal", excluding "completed"
```

## Operator precedence

There is no OR operator. All terms are ANDed together. Each term must match for a note to appear in results.

## Example queries

Given these notes:

**tasks.md**
```markdown
# Work
## TODO
* Talk with Bill
* Finish the report

# Personal
* Make the search in Kimün awesome
* Buy groceries
```

**projects.md**
```markdown
# Projects
## Personal
### Kimün
The simple but great note taking app!
```

| Search | Returns | Reason |
|---|---|---|
| `kimun` | projects.md, tasks.md | both contain "kimun" |
| `>personal kimun` | projects.md, tasks.md | "kimun" under a Personal heading in both |
| `>personal report` | tasks.md | "report" only under Personal in tasks.md |
| `@tasks >work` | tasks.md | file "tasks", has Work section |
| `screen*` | any note with "screenshot", "screens", etc. | wildcard matches "screen" prefix |
| `meeting -cancelled` | notes with "meeting" but not "cancelled" | exclusion removes matching notes |
| `@2024 >-draft` | files from 2024 without "draft" in section titles | combined exclusion |
| `-cancelled` | all notes except those with "cancelled" | exclusion-only search |
| `/journal >-temp` | notes in journal/ without "temp" in section titles | path + section exclusion |
| `@tasks >work report` | tasks.md | file "tasks", "Work" section, contains "report" |
| `@-archive >-draft` | all notes except those in archive/, excluding "draft" titles | combined exclusions |

## Edge cases

- **Exclusion-only searches** return all notes except those matching the exclusion criteria
- **Wildcards with operators**: `@task* >work` matches files starting with "task" that have a "Work" section
- **Operator prefixes are case-insensitive**: `>Personal` and `>personal` are equivalent, `@Tasks` and `@tasks` are equivalent
- **Multiple operators of same type**: `>work >personal` is AND — both sections must exist
- **Empty results**: If no notes match, the search returns an empty list
