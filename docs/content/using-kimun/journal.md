+++
title = "Journal"
weight = 13
+++

# Journal

A journal is one note for each day. Kimün keeps these notes in the `journal/` folder of your workspace. The file name is the date, for example `journal/2024-01-15.md`.

## Open today's journal

- In the apps, press `Ctrl+J`. Kimün creates the note if it does not exist.
- In a terminal, run `kimun journal` to add text to today's note, or `kimun journal show` to read it.

```sh
kimun journal "Met with Alex"
kimun journal --date 2024-01-15 "Note for another day"
```

## Write an entry

A journal entry is a normal Markdown note. Use the same headings every day, so that search can find them:

```markdown
# 2024-01-15

## Morning
Reviewed the Q1 roadmap.

## Tasks
- [ ] Follow up with Alex
- [ ] Finish the report draft
```

## Find entries

```
/journal standup           entries that contain "standup"
/journal @tasks            entries with a "Tasks" heading
=2024-01                   entries from January 2024
=2024-0*                   entries from January to September 2024
```

## Quick notes

For a thought that does not belong in today's journal, press `Ctrl+W`. Kimün saves it in the `/inbox` folder with the time in the name. Later you can move it to the right place.
