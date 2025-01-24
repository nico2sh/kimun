# Notes App
_Damn I need a better name_

A notes app. Focus on simplicity, but powerful on searchability.

100% human written.

## Building
The app uses gxhash that uses some specific hardware acceleration features. If when compiling you get an error, make sure you add the flag `RUSTFLAGS="-C target-cpu=native"`

On Windows (PowerShell):
```powershell
$env:RUSTFLAGS="-C target-cpu=native"
```

## Short-term roadmap

Here are the items I want to fix immediately to consider this usable. Then will focus on other cool features:

* [ ] Search under titles/sections in Markdown
* [ ] Different sort search results
* [X] Add title to the note editor
* [ ] Command Palette
* [ ] Display shortcuts
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

* [ ] Display path to the note preview
* [ ] Properly resolve local paths for images
* [ ] Enable wikilinks in render
* [ ] Navigate notes with links in render
* [ ] Make tags clickable
