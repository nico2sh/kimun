+++
title = "Search"
weight = 13
+++

# Search

Kimün indexes every Markdown file in your workspace. A small query language lets you filter notes by content, name, section, path, label, property, and links.

This page covers exact-match search over the local index. To find notes by meaning, or to ask questions that Kimün answers from your notes, see [Semantic Search & Ask](@/using-kimun/server.md).

The full query syntax:

| Want | Short | Long | Example |
|---|---|---|---|
| Free text | *(just type)* | | `meeting notes` |
| By note name | `=` | `name:` | `=tasks` |
| By section heading | `@` | `in:` | `@personal` |
| By path | `/` | `pt:` | `/journal/2024` |
| By label (hashtag) | `#` | `lb:` | `#finance` |
| By property | `%` | `prop:` | `%priority>=2` |
| Notes linking to X | `<` | `lk:` | `<projects` |
| Notes X links to | `>` | `fwd:` | `>projects` |
| Sort results | `^` | `or:` | `^title`, `-^title` |
| Exclude anything | `-` prefix | | `-#draft`, `-@temp` |

Kimün combines terms that you separate with spaces with AND. There is no OR operator.

## Opening search

- TUI: `Ctrl+K` opens the query search modal. `Ctrl+E` opens the [FIND drawer view](@/using-kimun/tui.md#find). Both take the same queries.
- CLI: run this command.

```sh
kimun search "your search query"
```

## Free text

```
kimun search     → must contain both "kimun" AND "search"
task report      → must contain both "task" AND "report"
```

Free text looks across the note content and the filenames at the same time. Searches have these properties:

- Case-insensitive: `kimun` matches `Kimün`, `KIMÜN`, `kimun`.
- Diacritics-ignored: `kimun` matches `Kimün`.
- Wildcards: `*` matches any run of characters.

### Wildcard patterns

```
kimu*            → anything starting with "kimu" (kimun, kimune, kimurei)
*meeting*        → "meeting" anywhere (meeting notes, team-meeting)
*report          → anything ending with "report"
```

## Operators

Each operator has a short form and a long form. The short form is a symbol. The long form is a word followed by a colon. You can use either form.

### `=` or `name:`: note name

```
=tasks           → notes whose name contains "tasks"
name:tasks       → same (long form)
=project         → notes with "project" in the name
```

The operator matches the basename only, not the full path.

### `@` or `in:`: section

```
@personal        → content under a "Personal" heading
in:personal      → same (long form)
@meeting         → content under a "Meeting" heading
```

The operator filters by Markdown sections (`#`, `##`, `###`, and so on). The search term must appear inside that section. Section names match the heading text without regard to case. If any section of a note contains the term, the note matches.

> Wildcards on `@` work on the prefix only. Kimün indexes the section filter for full-text search, so `*` works only at the end of a term. It matches whole words: `@meet*` matches "meeting" and "meetup". The `=`, `<`, `>`, and `/` filters accept `*` anywhere (`*report`, `ta*sk`). The section filter does not accept a leading `*` or a `*` in the middle of a term.

### `/` or `pt:`: path

```
/docs            → notes under a "docs/" directory
pt:docs          → same (long form)
/journal/2024    → notes under "journal/2024/"
```

Paths match as prefixes. For example, `/docs` matches both `/docs/readme.md` and `/docs/guides/tutorial.md`.

### `<` or `lk:`: backlinks

```
<projects        → notes that link to the note "projects"
lk:projects      → same (long form)
<projects.md     → same (the .md extension is optional)
```

The operator finds the notes that link to a given note, with a `[[wikilink]]` or a Markdown link. Kimün reads the target the way it reads a wikilink (see [How links point at notes](#how-links-point-at-notes)):

- Whole note names: `<projects` matches links to `projects` but not to `projects-archive`.
- Case-insensitive, any folder: a bare name matches links to a note of that name in any folder.
- Folders narrow the match from the end: `<work/projects` matches links that can point at a note whose path ends with `work/projects`. These links are `[[work/projects]]`, `[[a/work/projects]]`, a Markdown link to `/a/work/projects.md`, and also a bare `[[projects]]`, which can mean that note. The operator never matches `notwork/projects`.
- A leading `/` is exact: `</work/projects` matches only links to `/work/projects.md`.
- Wildcards: `*` works anywhere (`<proj*`, `<work/proj*`). A wildcard with a folder matches only links that are written with at least that much path.
- Only note links count: Kimün ignores attachments, images, and external URLs. A link to a note that does not exist yet still matches by name.

### `>` or `fwd:`: forward links

```
>projects        → notes that the note "projects" links to
fwd:projects     → same (long form)
>projects.md     → same (the .md extension is optional)
```

This operator is the reverse of `<`. It finds the notes that a given note links to. You name the note the same way as for backlinks (`>projects`, `>work/projects`, `>/work/projects`). Kimün resolves each link of the note the way that following the link resolves it. If several notes share a name, the result lists every one of them.

## How links point at notes

Kimün reads a link by the way that you write it. A wikilink names a note. A Markdown link is a path.

| Written in `/journal/today.md` | Points at |
|---|---|
| `[[plan]]` | every note named `plan`, in any folder |
| `[[work/plan]]` | the notes whose path ends with `work/plan`: `/work/plan.md`, `/a/work/plan.md`, never `/notwork/plan.md` |
| `[[/work/plan]]` | exactly `/work/plan.md` |
| `[[./plan]]`, `[[../work/plan]]` | exactly that path, from the note's folder (`/journal/plan.md`, `/work/plan.md`) |
| `[text](plan.md)` | exactly `/journal/plan.md`, the note beside it |
| `[text](../work/plan.md)` | exactly `/work/plan.md` |
| `[text](/work/plan.md)` | exactly `/work/plan.md` |
| `[text](files/report.pdf)` | not a note: the file opens with your default program |

- Names and paths ignore case. The `.md` extension is optional in wikilinks.
- A `#section` or `^block` after the target never changes which note the link points at.
- Several matches: if several notes share a wikilink name, the link is ambiguous. When you follow it, you choose a note. The link counts as a backlink of each of those notes. To narrow the link down, add folders (`[[work/plan]]`).
- No match: when you follow the link, Kimün offers to create the note. Kimün creates a note for a wikilink name from the vault root (`[[work/new]]` makes `/work/new.md`). For a path, Kimün creates the note exactly where the path points.
- `![[image.png]]` and `[[report.pdf]]` embed or name a file, not a note, so they are not note links.

When you rename or move a note, Kimün rewrites the links that pointed at it:

- If the note was the only match, Kimün rewrites the wikilink. Kimün leaves an ambiguous `[[plan]]` alone, because it still points at the other notes. A move that keeps the name needs no rewrite.
- A rewritten wikilink keeps as many folders as you wrote. If the new name is shared, the link gets more folders (`[[plan]]` becomes `[[work/roadmap]]` if another `roadmap` exists).
- Kimün rewrites Markdown links and `/`, `./`, `../` wikilinks to the new path.
- Kimün rewrites the relative links of a moved note (`[text](sibling.md)`, `[[../x/note]]`) so that they still reach the same notes.
- When you rename or move a folder, Kimün does not yet rewrite links.

> Upgrading (kimun_core 0.7.0). Earlier versions read a bare Markdown link such as `[text](plan.md)` as a name, and matched it in any folder. The link now means the `plan.md` beside the linking note, as in other Markdown tools. A link like that to a note in another folder now offers to create a new note. Use a path (`[text](../work/plan.md)`) or a wikilink (`[[plan]]`) instead. `[[work/plan]]` now finds `/work/plan.md` from any folder, and counts as its backlink. The first time you open each workspace after the upgrade, Kimün rebuilds the search index once, automatically. On a large vault, the rebuild can take some time.

## Labels

Labels are `#name` tokens that you write directly in the body of a note:

```markdown
Reviewed the quarterly numbers today. #finance #q2 #review
```

To search labels, use `#<label>` (short) or `lb:<label>` (long):

```
#finance             → notes labelled "finance"
lb:finance           → same (long form)
-#draft              → exclude notes labelled "draft"
#finance #q2         → both labels required (AND)
#finance report =2024 → mixes freely with text and other operators
```

An unknown label returns zero results, not an error.

### Label rules

- Allowed characters: letters, digits, and the `_` character (`[A-Za-z0-9_]+`). A hashtag ends at the first character outside that set, so `#tag-with-dash` gives the label `tag`.
- Case-insensitive: Kimün stores labels in lowercase, so `#Finance` and `#finance` are the same label.
- Not indexed as labels: hashtags inside inline code or fenced code blocks, HTML, Markdown link spans `[text](url#fragment)`, or wikilinks `[[#section]]`. A `#word` inside frontmatter is not a label either.
- Frontmatter tags count too. A `tags` property (`tags = ["a", "b"]` in TOML, `tags: [a, b]` or a `- a` list in YAML) adds its items as labels, so `#a` finds the note. The singular `tag` key that older Obsidian notes use works the same way. Kimün splits a plain string on commas (`tags: project, urgent` is two labels). A part with spaces stays one label (`tags: big project`). Frontmatter tags can contain characters that inline hashtags cannot contain (spaces, dashes). Quote them in queries: `#"big project"`.

## Properties

A note can carry typed properties in a frontmatter block at the very top. The block is TOML between `+++` lines (the default of Kimün) or YAML between `---` lines (what Obsidian writes).

```toml
+++
status = "in progress"
priority = 2
due = 2024-03-01
tags = ["work", "q1"]
+++
```

To query properties, use `prop:` or its short form `%`:

```
prop:status=done      → status equals "done" (ignores case and accents: Status=DONE, état=etat)
%priority>=2          → numeric comparison
%due<2024-04-01       → dates compare chronologically
%tags=work            → a list property contains "work"
%tags!=work           → has tags, but not "work"
%due                  → has a due property, whatever its value
-%due                 → has no due property
-%status=done         → excludes notes whose status is "done" (notes with no status are kept)
%status="in progress" → quote a value that has spaces
%"due date"<2025-01-01 → quote a key that has spaces (the whole term works too: prop:"due date<2025-01-01")
%status=d*            → `*` is a wildcard with = and != (also in list items and dates: %at=2024-01*; a date & time with an offset matches by its UTC form). A quoted value is literal: %rating="***"
%due<2024-02          → a partial date (2024-02, or a year: 2024) is the whole period: = is within it, < is before it, > is after it, <= and >= include it
%flag="!important"    → quote a value that starts with = ! < or >
^%"due date"          → sort by a key that has spaces
```

A key on its own (`%due`, `prop:"due date"`) finds the notes that have the property at all, even with no value. An empty list (`tags = []`) counts. A blank YAML entry (`due:`, which Obsidian writes for an unset property) also counts. `%due!=x` includes these notes too. Kimün ignores a key followed by an operator but no value (`%due=`).

The operators are `=`, `!=`, `<`, `<=`, `>`, and `>=`. The value that you compare with decides what the query can match:

- A number matches numbers.
- A date (`2024-04-01`) or a date and time matches dates and dates and times. Kimün compares them as moments in time.
- Anything else matches text.

A property of another type does not match. For example, `%due<today` never matches a date, and `<` on a list matches nothing. A date that you write as text (`due = "2024-01-31"`, or the `date = '2023-08-24T11:49:46-07:00'` of Hugo) compares as a date. `=` also matches a value that is written exactly as you typed it. A four-digit value such as `2024` matches both numbers and dates.

To sort by a property, use `or:prop:key` or `^%key`. See [Sorting](#sorting). Notes without the property always come last.

The supported types are:

- text
- number
- true/false
- date (`2024-03-01`)
- date and time (`2024-03-01T14:30`, or with an offset such as `2024-03-01T14:30:00+02:00`). Kimün compares a time without an offset as UTC.
- lists of text

Kimün ignores nested tables. If a block fails to parse, you can still search the block as plain text, but it gives no properties.

To view or edit the properties of a note in the TUI, see [Properties](@/using-kimun/tui.md#properties) in the TUI guide.

## Excluding things

The `-` prefix excludes. It always comes first. Any operator follows it:

```
-cancelled           → exclude notes containing "cancelled"
-@draft              → exclude notes with "draft" in any section title
-=temp               → exclude notes with "temp" in the name
-/private            → exclude notes under "private/"
-#draft              → exclude notes labelled "draft"
-<draft              → exclude notes that link to "draft"
->draft              → exclude notes that "draft" links to
```

Long forms work the same way: `-in:draft`, `-name:temp`, `-pt:private`, `-lb:draft`, `-lk:draft`, `-fwd:draft`.

Exclusion-only searches also work. For example, `-cancelled` on its own returns every note except those that contain "cancelled".

## Combining filters

You can combine any filters. Every term must match:

```
=tasks @work report                → name "tasks", has "Work" section, contains "report"
meeting -cancelled                 → "meeting" but not "cancelled"
=2024 -@draft                      → names from 2024 without "draft" in section titles
/journal -@temp report             → in journal/, no "temp" section, containing "report"
screen* =notes                     → starts with "screen", in name "notes"
#project -#archived @work          → labelled "project", not "archived", under "Work"
```

## Sorting

Results come back in the default order unless you add a sort directive. The directive is `or:` (long) or `^` (short), followed by a key. To sort in descending order, put `-` before the directive.

```
^title               → by note title, ascending (same as or:title)
-^title              → by note title, descending (same as -or:title)
or:file              → by file name
^%due                → by the "due" property, ascending
-or:prop:priority    → by the "priority" property, descending
```

The sort keys are:

| Key | Aliases | Sorts by |
|---|---|---|
| `title` | `t` | note title |
| `file` | `filename`, `f` | file name |
| `prop:<key>` | `%<key>` | a [property](#properties) value. Notes without it come last in either direction. |

The directive combines with any filter (`#project -#draft ^title`).

The TUI sort dialog writes this directive into the query for you. To open the dialog, press `Ctrl+R` in the FIND view or in the `Ctrl+K` search modal. For a query, the Sort by row cycles Name, Title, and Property. Property adds a Key field that suggests the keys in your vault. Nothing applies until you choose a key. To toggle a row, click it. To pick a suggestion, click it.

The FILES panel and the `Ctrl+O` file finder also offer Property. They re-sort their rows by the value of the key, which they read from the index. A note that the index has never seen sorts last. An example is a note that you created outside Kimün. A note that you edited outside Kimün sorts by its old value until the next reindex.

## Query variables

Some queries contain a `{name}` placeholder. The TUI fills it in at run time, before the query reaches the search engine. Currently the only variable is `{note}`:

- `{note}` resolves to the clean name of the note that is open in the editor (its filename without the extension).
- A bare note operator is shorthand for `<{note}`, `>{note}`, or `={note}`. A bare note operator is `<`, `>`, or `=` with no target. It includes the long forms `lk:`, `fwd:`, and `name:`, and the `-` exclusion variants. The shorthand means the backlinks of the current note, its forward links, or the note itself by name. Kimün does not expand operators inside quoted terms.

With `spec.md` open, `<{note}` runs as `<spec` (the notes that link to `spec`). When no note is open, `{note}` resolves to an empty string.

Both the FIND drawer view and the `Ctrl+K` search modal substitute `{note}` with the open note. [Saved searches](#saved-searches) store the template (the unresolved `{note}`). A saved `<{note}` therefore targets the note that is open each time you run it.

## Saved searches

A saved search stores a query under a name, so you can run it again without typing it. Saved searches belong to the workspace. You manage them from the TUI:

- To save the current query, press `Ctrl+D` in the [FIND view](@/using-kimun/tui.md#find) or in the `Ctrl+K` search modal. Then give the query a name.
- To open the Saved Searches picker, press `F3`. Press `Enter` to run the selected search, `1` to `9` to select one quickly, or `Delete` to remove it.

When you run a saved search, Kimün loads its results in the FIND view. For the full workflow, see [Saved Searches](@/using-kimun/tui.md#saved-searches) in the TUI guide.

### Running by name

You can also run a saved search from the search field, without the picker. In the [FIND view](@/using-kimun/tui.md#find) or in the `Ctrl+K` search modal, type `?` as the first character. Kimün then autocompletes the names of saved searches:

- To filter the list, type `?` and part of a name (for example `?todo`). Pick a name with `Enter` or `Tab`. An empty `?` lists every saved search.
- When you accept a name, Kimün expands the stored query into the field. You can edit the query before you run it.
- The border of the search box then shows the name of the search as a breadcrumb (`‹ todo ›`). If you edit the query, the border shows the marker `‹ todo • edited ›`. To drop the breadcrumb, clear the field. A change to the [sort order](@/using-kimun/tui.md#find) also counts as an edit, because it rewrites the sort directive of the query.

The field holds the query template, so any `{note}` variable stays intact. It resolves again each time you run the query.

## Example queries

These examples use the following notes.

tasks.md
```markdown
# Work
## TODO
* Talk with Bill
* Finish the report

# Personal
* Make the search in Kimün awesome
* Buy groceries
```

projects.md
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
| `%status=done` | notes whose status is "done" | property filter |
| `%priority>=2 #work ^%due` | work notes with priority 2 or more, soonest due first | property + label + sort |
| `#project -^title` | project notes, titles Z to A | sort descending |

## Edge cases

- Wildcards with operators: `=task* @work` matches notes whose names start with "task" and that have a "Work" section.
- Operator prefixes are case-insensitive: `@Personal` is the same as `@personal`, and `=Tasks` is the same as `=tasks`.
- Multiple operators of the same type: `@work @personal` is AND, so both sections must exist.
- Empty results: if nothing matches, you get an empty list, not an error.
- Hashtags in code: Kimün does not treat `` `#tag` `` and hashtags inside fenced code blocks as labels.
