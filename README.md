# Kimün

A notes app. Focus on simplicity, but powerful on searchability.

100% human written.

## Building

The app uses gxhash that uses some specific hardware acceleration features. If when compiling you get an error, make sure you add the flag `RUSTFLAGS="-C target-cpu=native"`

On Windows (PowerShell):

```powershell
$env:RUSTFLAGS="-C target-cpu=native"
```

## Searching

One cool feature of Kimün is that has a powerful but simple search syntax using Markdown features.
Open the search box with `ctrl+s` in Windows/Linux or `cmd+s` in MacOS, in the searchbox you can put any search term and will look into the content and path of the notes.

### Search free text

Anything you put on the search box will search in both the content and the file name. You can use `*` as a wildcard and the search ignores case and special characters.
As an example, if you have three notes called `note1.md`, `note2.md` and `note3.md`, all three of them containing the text "Kimün", all the following search queries will return all three notes:

* `Kimün`
* `KIMÜN`
* `kimün`
* `kimu*`
* `*imün`
* `*imu*`
* `note*`

### Narrowing the search to specific files

You can limit the search to specific files using the `@` prefix or `at:`. Both produce the same effect and ignores the file extension. In the example above, the search term `@note1` or `at:note1` will only return the note `note1.md`.
You can then combine file names and text search terms to narrow the results.

### Narrowing to sections

Additionally you can use the Markdown's document structure to find notes within sections. Each section is defined by a markdown header, and the keyword/prefix to search within section is `>` or `in:`. Both produce the same effect.
Let's pretend you have these notes:

> tasks.md

```markdown
# Work

## TODO

* Talk with Bill
* Finish the report

# Personal

* Make the search in Kimün awesome
* Buy groceries
* Take screenshots of the app
```

> projects.md

```markdown
# Projects

Here is a list of projects I'm working on.

## Personal

Personal Projects

### Kimün

The simple but great note taking app!

#### Features
* Powerful search
* You own the note files
* Markdown!

### Semtag

A bash script to generate Semantic Version tags on git releases

#### Features
* Bash! Runs almost everywhere
* Uses git tags
* Includes the commits in the commit comment
```

> personal-thoughts.md

```markdown
# My thoughts
* I prefer to keep the notes in one app and task management on a different one, so the above example doesn't reflect my own workflow with Kimün
* I can use the journal note names with dates to limit the search to specific years by searching by filename and content
```

> general-thoughts.md

```markdown
# Random thoughts
* I think this is definitively the year of Linux Desktop
* I'm intentionally not putting the name of the app in this one for the example
* I like wide screens
```

If I do a search, it will return:

| Search term | Result | Notes |
|-------------|--------|-------|
|`kimun` | `projects.md` `tasks.md` `personal-thoughts.md`| All three notes contains Kimün, the dieresis is ignored|
|`>personal kimun` |`projects.md` `tasks.md`| Only these two notes have the search term under "Personal"|
|`@thoughts` |`personal-thougts.md` `general-thougts.md`| We look for a file whose name contains "thoughts"|
|`@thoughts kimun` |`personal-thougts.md`| We look for a file called "thoughts" containing "Kimun"|
|`screen*` |`tasks.md` `general-thougts.md`| "tasks.md" contains the word "screenshot", "general-thoughts.md" contains the word "screens"|

## Short-term roadmap

Here are the items I want to fix immediately to consider this usable. Then will focus on other cool features:

* [X] Search under titles/sections in Markdown
* [ ] Different sort search results
* [X] Add title to the note editor
* [ ] Command Palette
* [ ] Display key shortcuts
* [ ] Resolve relative paths
* [ ] Modals with progress in the settings when reindexing
* [ ] Backlink support
* [ ] Inline note Tags (like `#important`)
* [ ] Shortcuts for text format (bold, italic)
* [ ] Shortcuts for inserting links
* [ ] Paste images in note
* [ ] Calendar to browse journal
* [ ] Auto continue format lists while typing (hitting enter on a list element creates a new element)

### Rendering

* [X] Display path to the note preview
* [ ] Properly resolve local paths for images
* [ ] Enable wikilinks in render
* [ ] Navigate notes with links in render
* [ ] Make tags clickable
