+++
title = "Search"
weight = 12
+++

# Search

Kimün provides powerful search capabilities to find notes by content, note name, section, path, and links. Search indexes all Markdown files in your workspace and supports structured queries with operators and filters.

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

Operators allow you to filter by note name, section, path, or links. Each operator has both a short form (symbol) and a long form (colon-prefixed).

### `=` or `name:` — note name filter

Filter notes by their name (basename only, not full path):

```
=tasks           → notes whose name contains "tasks"
name:tasks       → same (long form)
=project         → notes with "project" in the name
name:notes       → notes with "notes" in the name
```

### `@` or `in:` — section filter

Filter notes by Markdown sections (defined by `#`, `##`, `###`, etc.). The search term must appear within that section:

```
@personal        → content under a "Personal" heading
in:personal      → same (long form)
@work            → content under a "Work" heading
in:meeting       → content under a "Meeting" heading
```

Section names are matched case-insensitively against heading text. A note matches if any of its sections contain the search term.

> **Wildcards on `@` are prefix-only.** The section filter is full-text indexed, so `*` works only at the **end** of a term (`@meet*` matches "meeting", "meetup") and matches whole words. Unlike `=`, `<`, `>`, and `/` — which support `*` anywhere (`*report`, `ta*sk`) — the section filter does **not** support leading or mid-term `*`.

### `/` or `pt:` — path filter

Filter notes by their full path within the workspace:

```
/docs            → notes under a "docs/" directory
pt:docs          → same (long form)
/journal/2024    → notes under "journal/2024/" directory
/archive         → notes under an "archive/" directory
```

Paths are matched as prefixes, so `/docs` matches both `/docs/readme.md` and `/docs/guides/tutorial.md`.

### `<` or `lk:` — backlink filter

Filter to notes that **link to** a given note (its backlinks). A note matches when its body contains a note link — a `[[wikilink]]` or a Markdown link to a vault note — pointing at the target:

```
<projects        → notes that link to the note "projects"
lk:projects      → same (long form)
<projects.md     → same (the .md extension is optional)
```

The target is matched by note name, case-insensitively. The match is by **note identity**, not substring: `<projects` matches links to `projects` but not to `projects-archive`.

A bare name matches a linked note in **any** folder. Add a path to disambiguate notes that share a name:

```
<projects        → links to any note named "projects" (e.g. work/projects, personal/projects)
<work/projects   → links to work/projects only
```

Wildcards work here too:

```
<proj*           → notes linking to any note whose name starts with "proj"
```

Only links to other notes count — links to attachments, images, and external URLs are ignored.

### `>` or `fwd:` — forward link filter

Filter to the notes that a given note **links to** (its forward links). A note matches when the target note's body contains a link pointing at it:

```
>projects        → notes that the note "projects" links to
fwd:projects     → same (long form)
>projects.md     → same (the .md extension is optional)
```

Like the backlink filter, the target is matched by note name (case-insensitive), a bare name matches any folder, a path disambiguates, and `*` wildcards are allowed.

## Labels

Labels are `#name` tokens written directly in your note body. When Kimün indexes a note, any word starting with `#` followed by one or more `[A-Za-z0-9_]` characters is recorded as a label for that note.

```markdown
Reviewed the quarterly numbers today. #finance #q2 #review
```

### Label rules

- **Allowed characters:** letters, digits, and underscores (`[A-Za-z0-9_]+`). A hashtag ends at the first character outside that set, so `#tag-with-dash` yields the label `tag`.
- **Case-insensitive:** labels are stored in lowercase. `#Finance` and `#finance` are the same label.
- **Code is excluded:** hashtags inside inline code spans (`` `#tag` ``) and fenced code blocks are not indexed as labels.
- **Frontmatter is excluded:** hashtags inside YAML (`---`) or TOML (`+++`) frontmatter blocks are not indexed as labels.
- **HTML is excluded:** hashtags inside HTML blocks or inline HTML (e.g. `<span data-tag="#foo">`) are not indexed as labels.
- **Link bodies are excluded:** hashtags inside markdown link spans `[text](url#fragment)` — including URL fragments — are not indexed as labels.
- **Wikilinks are excluded:** hashtags inside wikilink spans `[[...]]` (e.g. `[[#section]]`) are not indexed as labels.

### Searching by label

Use `#<label>` (short form) or `lb:<label>` (long form) to filter to notes carrying that label:

```
#finance             → notes labelled "finance"
lb:finance           → same (long form)
#q2                  → notes labelled "q2"
lb:review            → notes labelled "review"
```

### Excluding by label

Prefix the label with `-` to exclude notes that carry it:

```
-#draft              → exclude notes labelled "draft"
-lb:draft            → same (long form)
```

### Combining label filters

Multiple label filters are ANDed together — every label must be present:

```
#finance #q2         → notes with both "finance" AND "q2" labels
lb:finance lb:review → notes with both "finance" AND "review" labels
```

Label filters mix freely with free-text search and other operators:

```
#finance report =2024           → labelled "finance", contains "report", name has "2024"
#project -#archived @work        → labelled "project", not "archived", under a "Work" section
```

An unknown label (one that has never appeared in any note) returns zero results, not an error.

## Exclusion operators

Use the `-` prefix to exclude terms from search results. The `-` always leads, then the operator prefix follows:

```
-cancelled           → exclude notes containing "cancelled"
-@draft              → exclude notes with "draft" in any section title
-=temp               → exclude notes with "temp" in the name
-/private            → exclude notes under a "private/" directory
-#draft              → exclude notes labelled "draft"
-<draft              → exclude notes that link to "draft"
->draft              → exclude notes that "draft" links to
```

Long forms work the same way: `-in:draft`, `-name:temp`, `-pt:private`, `-lb:draft`, `-lk:draft`, `-fwd:draft`.

### Exclusion-only searches

You can search using only exclusions to find all notes except those matching the exclusion:

```
-cancelled           → all notes EXCEPT those containing "cancelled"
-@draft              → all notes EXCEPT those with "draft" in section titles
-=temp               → all notes EXCEPT those with "temp" in the name
-/archive            → all notes EXCEPT those under an "archive/" directory
```

## Combining filters

All operators compose freely in a single query. Space between terms = AND.

Each term (with or without an operator prefix) must match for a note to appear:

```
=tasks @work report                → name "tasks", has "Work" section, contains "report"
@personal kimun                    → "kimun" under a "Personal" section
=thoughts kimun                    → name "thoughts" containing "kimun"
meeting -cancelled                 → "meeting" but not "cancelled"
=2024 -@draft                      → names from 2024 without "draft" in section titles
/journal -@temp report             → in journal/, not titled "temp", containing "report"
screen* =notes                     → starts with "screen", in name "notes"
@personal report -completed        → "report" under "Personal", excluding "completed"
```

## Query variables

Some queries contain a `{name}` placeholder that the TUI fills in at run time, before the query reaches the search engine. The first (and currently only) variable is `{note}`:

- `{note}` resolves to the **clean name** of the note open in the editor (its filename without the extension).
- In the [query panel](@/using-kimun/tui.md#query-panel), a bare `<` is shorthand for `<{note}` — the backlinks of the current note.

With `spec.md` open, `<{note}` runs as `<spec` (the notes that link to `spec`). When no note is open, `{note}` resolves to an empty string.

Variables are resolved wherever the query runs — both the query panel and the `Ctrl+K` search modal substitute `{note}` against the open note. Because [saved searches](#saved-searches) store the *template* (the unresolved `{note}`), a saved `<{note}` re-targets to whatever note is open each time you run it.

## Saved searches

A saved search stores a query under a name so you can re-run it without retyping — common filters, project views, or backlink queries. Saved searches live with the workspace and are managed from the TUI:

- **Save** the current query with `Ctrl+D` — from the [query panel](@/using-kimun/tui.md#query-panel) or the `Ctrl+K` search modal — then give it a name.
- **Open** the Saved Searches picker with `F3` to run a saved search (`Enter`), quick-select with `1`–`9`, or remove one with `Delete`.

Running a saved search loads its results in the query panel. See [Saved Searches](@/using-kimun/tui.md#saved-searches) in the TUI guide for the full workflow.

### Running by name

You can also run a saved search straight from the search field, without the picker. In the [query panel](@/using-kimun/tui.md#query-panel) or the `Ctrl+K` search modal, type `?` as the first character to autocomplete saved-search names:

- Type `?` followed by part of a name (e.g. `?todo`) to filter the list; pick one with `Enter` or `Tab`. An empty `?` lists every saved search.
- Accepting **expands the stored query into the field**, so you can tweak it before running like any other query.
- The search-box border then shows the search's name as a breadcrumb (`‹ todo ›`). Edit the query and it gains an `‹ todo • edited ›` marker; clear the field to drop the breadcrumb. Changing only the [sort order](@/using-kimun/tui.md#query-panel) does *not* count as edited.

Because the field holds the query *template*, any `{note}` variable stays intact and re-resolves each time you run it.

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
| `@personal kimun` | projects.md, tasks.md | "kimun" under a Personal heading in both |
| `@personal report` | tasks.md | "report" only under Personal in tasks.md |
| `=tasks @work` | tasks.md | name "tasks", has Work section |
| `screen*` | any note with "screenshot", "screens", etc. | wildcard matches "screen" prefix |
| `meeting -cancelled` | notes with "meeting" but not "cancelled" | exclusion removes matching notes |
| `=2024 -@draft` | names from 2024 without "draft" in section titles | combined exclusion |
| `-cancelled` | all notes except those with "cancelled" | exclusion-only search |
| `/journal -@temp` | notes in journal/ without "temp" in section titles | path + section exclusion |
| `=tasks @work report` | tasks.md | name "tasks", "Work" section, contains "report" |
| `-=archive -@draft` | all notes except those named archive/, excluding "draft" titles | combined exclusions |
| `#finance` | notes labelled "finance" | label filter |
| `lb:review` | notes labelled "review" | label filter (long form) |
| `#finance #q2` | notes with both "finance" and "q2" labels | combined label filters |
| `#project -#draft` | notes labelled "project" but not "draft" | label inclusion + exclusion |
| `<kimun` | notes that link to the note "kimun" | backlink filter |
| `lk:kimun #project` | notes linking to "kimun" and labelled "project" | backlink + label |
| `<spec -<draft` | notes linking to "spec" but not to "draft" | backlink inclusion + exclusion |
| `>kimun` | notes that the note "kimun" links to | forward link filter |
| `fwd:spec #project` | notes that "spec" links to and labelled "project" | forward link + label |

## Edge cases

- **Exclusion-only searches** return all notes except those matching the exclusion criteria
- **Wildcards with operators**: `=task* @work` matches notes named starting with "task" that have a "Work" section
- **Operator prefixes are case-insensitive**: `@Personal` and `@personal` are equivalent, `=Tasks` and `=tasks` are equivalent
- **Multiple operators of same type**: `@work @personal` is AND — both sections must exist
- **Empty results**: If no notes match, the search returns an empty list
- **Unknown labels**: `#nonexistent` returns zero results, not an error
- **Hashtags in code**: `` `#tag` `` and hashtags inside fenced code blocks are not treated as labels
