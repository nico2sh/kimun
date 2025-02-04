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
