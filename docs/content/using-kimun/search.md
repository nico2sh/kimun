+++
title = "Search"
weight = 13
+++

# Search

Search is Kimün's superpower. Every Markdown file in your workspace is indexed, and a small query language lets you slice by content, name, section, path, label, and links.

The whole grammar fits in one table:

| Want | Short | Long | Example |
|---|---|---|---|
| Free text | *(just type)* | | `meeting notes` |
| By note name | `=` | `name:` | `=tasks` |
| By section heading | `@` | `in:` | `@personal` |
| By path | `/` | `pt:` | `/journal/2024` |
| By label (hashtag) | `#` | `lb:` | `#finance` |
| Notes linking **to** X | `<` | `lk:` | `<projects` |
| Notes X links **to** | `>` | `fwd:` | `>projects` |
| Exclude anything | `-` prefix | | `-#draft`, `-@temp` |

Space between terms = AND. There is no OR. That's the whole precedence story.

## Opening search

- **TUI:** `Ctrl+K` opens the query search modal; `Ctrl+E` opens the [FIND drawer view](@/using-kimun/tui.md#find). Both take the same queries.
- **CLI:**

```sh
kimun search "your search query"
```

## Free text

```
kimun search     → must contain both "kimun" AND "search"
task report      → must contain both "task" AND "report"
```

Free text looks across note content and filenames at once. Searches are:

- **Case-insensitive:** `kimun` matches `Kimün`, `KIMÜN`, `kimun`
- **Diacritics-ignored:** `kimun` matches `Kimün`
- **Wildcard-friendly:** `*` matches patterns

### Wildcard patterns

```
kimu*            → anything starting with "kimu" (kimun, kimune, kimurei)
*meeting*        → "meeting" anywhere (meeting notes, team-meeting)
*report          → anything ending with "report"
```

## Operators

Each operator has a short form (symbol) and a long form (colon-prefixed). Pick whichever your fingers prefer.

### `=` or `name:` — note name

```
=tasks           → notes whose name contains "tasks"
name:tasks       → same (long form)
=project         → notes with "project" in the name
```

Matches the basename only, not the full path.

### `@` or `in:` — section

```
@personal        → content under a "Personal" heading
in:personal      → same (long form)
@meeting         → content under a "Meeting" heading
```

Filters by Markdown sections (`#`, `##`, `###`, …). The search term must appear within that section. Section names match heading text case-insensitively; a note matches if any of its sections contain the term.

> **Wildcards on `@` are prefix-only.** The section filter is full-text indexed, so `*` works only at the **end** of a term (`@meet*` matches "meeting", "meetup") and matches whole words. Unlike `=`, `<`, `>`, and `/` — which support `*` anywhere (`*report`, `ta*sk`) — the section filter does **not** support leading or mid-term `*`.

### `/` or `pt:` — path

```
/docs            → notes under a "docs/" directory
pt:docs          → same (long form)
/journal/2024    → notes under "journal/2024/"
```

Paths match as prefixes: `/docs` matches both `/docs/readme.md` and `/docs/guides/tutorial.md`.

### `<` or `lk:` — backlinks

```
<projects        → notes that link to the note "projects"
lk:projects      → same (long form)
<projects.md     → same (the .md extension is optional)
```

Finds the notes that **link to** a given note, via `[[wikilink]]` or Markdown link. Matching rules:

- **By note identity, not substring:** `<projects` matches links to `projects`, but not to `projects-archive`
- **Case-insensitive,** matched by note name; a bare name matches a note in any folder, so add a path to disambiguate (`<work/projects`) and use `*` wildcards freely (`<proj*`)
- **Only note links count:** attachments, images, and external URLs are ignored

### `>` or `fwd:` — forward links

```
>projects        → notes that the note "projects" links to
fwd:projects     → same (long form)
>projects.md     → same (the .md extension is optional)
```

The mirror image of `<`: the notes a given note **links to**. Same matching rules as backlinks.

## Labels

Labels are `#name` tokens written directly in your note body:

```markdown
Reviewed the quarterly numbers today. #finance #q2 #review
```

Search them with `#<label>` (short) or `lb:<label>` (long):

```
#finance             → notes labelled "finance"
lb:finance           → same (long form)
-#draft              → exclude notes labelled "draft"
#finance #q2         → both labels required (AND)
#finance report =2024 → mixes freely with text and other operators
```

An unknown label returns zero results, not an error.

### Label rules

- **Allowed characters:** letters, digits, underscores (`[A-Za-z0-9_]+`). A hashtag ends at the first character outside that set, so `#tag-with-dash` yields the label `tag`.
- **Case-insensitive:** stored lowercase; `#Finance` and `#finance` are the same label.
- **Not indexed as labels:** hashtags inside inline code or fenced code blocks, YAML/TOML frontmatter, HTML, Markdown link spans `[text](url#fragment)`, or wikilinks `[[#section]]`.

## Excluding things

The `-` prefix excludes. It always leads; any operator follows:

```
-cancelled           → exclude notes containing "cancelled"
-@draft              → exclude notes with "draft" in any section title
-=temp               → exclude notes with "temp" in the name
-/private            → exclude notes under "private/"
-#draft              → exclude notes labelled "draft"
-<draft              → exclude notes that link to "draft"
->draft              → exclude notes that "draft" links to
```

Long forms work the same: `-in:draft`, `-name:temp`, `-pt:private`, `-lb:draft`, `-lk:draft`, `-fwd:draft`.

Exclusion-only searches are fine too — `-cancelled` alone returns every note *except* those containing "cancelled".

## Combining filters

Everything composes. Space = AND, each term must match:

```
=tasks @work report                → name "tasks", has "Work" section, contains "report"
meeting -cancelled                 → "meeting" but not "cancelled"
=2024 -@draft                      → names from 2024 without "draft" in section titles
/journal -@temp report             → in journal/, no "temp" section, containing "report"
screen* =notes                     → starts with "screen", in name "notes"
#project -#archived @work          → labelled "project", not "archived", under "Work"
```

## Query variables

Some queries contain a `{name}` placeholder that the TUI fills in at run time, before the query reaches the search engine. The first (and currently only) variable is `{note}`:

- `{note}` resolves to the **clean name** of the note open in the editor (its filename without the extension).
- A bare note operator — `<`, `>` or `=` with no target, including the long forms `lk:` / `fwd:` / `name:` and the `-` exclusion variants — is shorthand for `<{note}`, `>{note}` or `={note}`: the backlinks of the current note, its forward links, or the note itself by name. Operators inside quoted terms are not expanded.

With `spec.md` open, `<{note}` runs as `<spec` (the notes that link to `spec`). When no note is open, `{note}` resolves to an empty string.

Variables are resolved wherever the query runs — both the FIND drawer view and the `Ctrl+K` search modal substitute `{note}` against the open note. Because [saved searches](#saved-searches) store the *template* (the unresolved `{note}`), a saved `<{note}` re-targets to whatever note is open each time you run it.

## Saved searches

A saved search stores a query under a name so you can re-run it without retyping — common filters, project views, or backlink queries. Saved searches live with the workspace and are managed from the TUI:

- **Save** the current query with `Ctrl+D` — from the [FIND view](@/using-kimun/tui.md#find) or the `Ctrl+K` search modal — then give it a name.
- **Open** the Saved Searches picker with `F3` to run a saved search (`Enter`), quick-select with `1`–`9`, or remove one with `Delete`.

Running a saved search loads its results in the FIND view. See [Saved Searches](@/using-kimun/tui.md#saved-searches) in the TUI guide for the full workflow.

### Running by name

You can also run a saved search straight from the search field, without the picker. In the [FIND view](@/using-kimun/tui.md#find) or the `Ctrl+K` search modal, type `?` as the first character to autocomplete saved-search names:

- Type `?` followed by part of a name (e.g. `?todo`) to filter the list; pick one with `Enter` or `Tab`. An empty `?` lists every saved search.
- Accepting **expands the stored query into the field**, so you can tweak it before running like any other query.
- The search-box border then shows the search's name as a breadcrumb (`‹ todo ›`). Edit the query and it gains an `‹ todo • edited ›` marker; clear the field to drop the breadcrumb. Changing only the [sort order](@/using-kimun/tui.md#find) does *not* count as edited.

Because the field holds the query *template*, any `{note}` variable stays intact and re-resolves each time you run it.

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

- **Wildcards with operators:** `=task* @work` matches notes named starting with "task" that have a "Work" section
- **Operator prefixes are case-insensitive:** `@Personal` ≡ `@personal`, `=Tasks` ≡ `=tasks`
- **Multiple operators of the same type:** `@work @personal` is AND — both sections must exist
- **Empty results:** if nothing matches, you get an empty list, never an error
- **Unknown labels:** `#nonexistent` returns zero results, not an error
- **Hashtags in code:** `` `#tag` `` and hashtags inside fenced code blocks are not treated as labels
