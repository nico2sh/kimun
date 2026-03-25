# Kimün

A terminal-based notes app focused on simplicity and powerful search.

Notes are plain Markdown files stored in a directory you own. Kimün indexes them into a local SQLite database for fast full-text and structured search.

> Small disclaimer: Although by no means this has been vibe coded — the core has been written manually — there is a good chunk of AI-assisted code (using Claude) with manual reviews. Mostly for tedious refactors, data structures I'm too lazy to code myself, but also to help me building the foundations of more complex stuff, especially on the UI side. Use AI as a tool, not as a replacement.

## Installation

```sh
cargo install kimun
```

On first launch, Kimün will open the Settings screen so you can set your notes directory. It will create a `kimun.sqlite` file in the root of your notes directory.

## Configuration

The config file is created automatically on first run:

- **Linux / macOS:** `~/.config/kimun/kimun_config.toml`
- **Windows:** `%USERPROFILE%\kimun\kimun_config.toml`

You can also pass a custom config path:

```sh
kimun --config /path/to/my-config.toml
```

## How it works

Kimün opens in a terminal UI with a few screens:

- **Browse** — file tree navigator for a directory
- **Editor** — Markdown editor with a file browser pane
- **Settings** — configure your notes directory, theme, key bindings, and more

Your notes directory is called the **workspace**. Kimün creates a `kimun.sqlite` file at the workspace root to store the search index. Everything else is plain `.md` files — no lock-in.

## Search

Open search with `Ctrl+E`. The search box looks across note content and file paths.

### Free text

Searches both content and filenames. Case-insensitive, diacritics ignored, `*` wildcard supported.

```
kimun          → matches "Kimün", "KIMÜN", "kimun", etc.
kimu*          → matches anything starting with "kimu"
*meeting*      → matches any note containing "meeting" anywhere
```

### Filter by filename — `@` or `at:`

```
@tasks         → only notes whose filename contains "tasks"
at:tasks       → same
```

### Filter by section — `>` or `in:`

Sections are defined by Markdown headers (`#`, `##`, etc.). The search term must appear within that section.

```
>personal      → only content under a "Personal" heading
in:personal    → same
```

### Combine filters

Filters compose freely:

```
@tasks >work report       → in a file called "tasks", under a "Work" section, containing "report"
>personal kimun           → any note with "kimun" under a "Personal" section
@thoughts kimun           → a file called "thoughts" containing "kimun"
screen*                   → matches "screenshot", "screens", etc.
```

### Example

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

| Search               | Returns                                     |
| -------------------- | ------------------------------------------- |
| `kimun`            | `projects.md`, `tasks.md`               |
| `>personal kimun`  | `projects.md`, `tasks.md`               |
| `>personal report` | `tasks.md`                                |
| `@tasks >work`     | `tasks.md`                                |
| `screen*`          | any note with "screenshot", "screens", etc. |

## Key bindings

Default bindings (all configurable in the config file):

| Action                               | Default    |
| ------------------------------------ | ---------- |
| Quit                                 | `Ctrl+Q` |
| Settings                             | `Ctrl+P` |
| Search notes                         | `Ctrl+E` |
| Open note (fuzzy finder)             | `Ctrl+O` |
| Toggle note browser                  | `Ctrl+F` |
| Toggle preview                       | `Ctrl+Y` |
| New journal entry                    | `Ctrl+J` |
| Toggle sidebar                       | `Ctrl+B` |
| Focus editor                         | `Ctrl+L` |
| Focus sidebar                        | `Ctrl+H` |
| Sort by name                         | `Ctrl+N` |
| Sort by title                        | `Ctrl+G` |
| Reverse sort order                   | `Ctrl+R` |
| File operations (rename/move/delete) | `F2`     |
| Bold                                 | `Ctrl+B` |
| Italic                               | `Ctrl+I` |
| Strikethrough                        | `Ctrl+S` |
| Toggle header                        | `Ctrl+T` |

## Roadmap

- [ ] Command palette
- [ ] Display key shortcuts in command palette and help modal
- [ ] Backlinks panel
- [ ] Inline tags and search by tag (`#important`)
- [ ] Resolve relative paths on links and images
- [ ] Paste images into notes
- [ ] Calendar view for journal browsing
- [ ] Auto-continue list formatting on Enter
- [ ] Multiple workspaces
- [X] Search under Markdown sections
- [X] File management (create, rename, move, delete notes and directories)
- [X] Autosave
- [X] Wikilinks in preview
- [X] Navigate notes via links in preview
- [ ] Embed neoVim as an option
