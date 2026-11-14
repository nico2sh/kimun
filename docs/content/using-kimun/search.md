+++
title = "Search"
weight = 12
+++

# Search

Kimün searches every note in your workspace. You can type words, or add filters for the name, the tags, the links, and the properties of a note.

To search, press `Ctrl+K` (or `Ctrl+E` for the FIND panel). In a terminal, you can also run `kimun search "your query"`.

## The basics

![The Find view with the query "<carlos or:title"](/img/gui-find.png)


Type words to find notes that contain all of them. Case and accents do not matter. Use `*` as a wildcard: `kimu*` finds `kimun`.

You can add filters. Separate the words and filters with spaces. A note must match all of them.

| To find | Type | Example |
|---|---|---|
| Notes with a tag | `#` | `#finance` |
| Notes with a name | `=` | `=tasks` |
| Text under a heading | `@` | `@meeting` |
| Notes in a folder | `/` | `/journal/2024` |
| Notes with a property | `%` | `%priority>=2` |
| Notes that link to a note | `<` | `<projects` |
| Notes that a note links to | `>` | `>projects` |
| Sorted results | `^` | `^title` |

To leave something out, put `-` before it: `-#draft`, `-@temp`, `-cancelled`.

## Tags

A tag is a word with `#` in front, written in a note: `#finance`. Tags use letters, digits, and `_`. `#Finance` and `#finance` are the same tag. Tags in the `tags` property of a note count too.

```
#finance             notes with the tag finance
#finance #q2         notes with both tags
#project -#archived  tagged project, but not archived
```

## Properties

Properties are facts at the top of a note:

```
+++
status = "in progress"
priority = 2
due = 2024-03-01
tags = ["work", "q1"]
+++
```

Kimün also reads the YAML style (`---` lines) that other note apps use. To search them:

```
%status=done            status is done
%priority>=2            a number is at least 2
%due<2024-04-01         a date is before April 2024
%tags=work              the list has work
%due                    the note has a due property
-%due                   the note has no due property
%status="in progress"   quote a value that has spaces
```

You can use `=`, `!=`, `<`, `<=`, `>`, and `>=`.

## Links

Use `<` to find the notes that link to a note, and `>` to find the notes that it links to. Both `[[wikilinks]]` and Markdown links count.

```
<projects        notes that link to "projects"
>projects        notes that "projects" links to
<work/projects   links to a "projects" note in a "work" folder
```

A bare `<` or `>` uses the note that you have open. For example, `<` alone shows the backlinks of the open note.

### How a link finds its note

- `[[plan]]` points to any note named `plan`, in any folder. If two notes share the name, Kimün asks which one to open. Add a folder to be exact: `[[work/plan]]`.
- `[[/work/plan]]` points to exactly `/work/plan.md`.
- `[text](plan.md)` points to the `plan.md` that is in the same folder as the current note.
- If a link points to no note, Kimün offers to create it when you follow it.

When you rename or move a note, Kimün updates the links that point to it.

## Sort

Add `^title` to sort by title, `^file` to sort by file name, or `^%due` to sort by a property. Put `-` before it to reverse the order: `-^title`. In the apps, press `Ctrl+R` to pick a sort from a list.

## Saved searches

Press `Ctrl+D` to save a query under a name. Press `F3` to see your saved searches. In any search box, type `?` and a name to use one.

## Examples

```
#project -#archived @work       tagged project, not archived, under a "Work" heading
/journal @tasks -done           journal entries with a "Tasks" heading and no "done"
=2024 -@draft                   notes with 2024 in the name and no "draft" heading
%status=open ^%due              open notes, sorted by due date
```

## Search by meaning

This page covers search by exact words. To find notes by meaning, or to ask questions about your notes, see [Semantic search and Ask](@/using-kimun/server.md).
