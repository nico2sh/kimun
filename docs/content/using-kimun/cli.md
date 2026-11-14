+++
title = "CLI"
weight = 17
+++

# Command line

The `kimun` command lets you work with your notes from a terminal. Run `kimun --help` to see every command. This page shows the common ones.

## Find notes

```sh
kimun search "rust"                       # notes that contain "rust"
kimun search "#important -#archived"      # tagged important, not archived
kimun search "<projects"                  # notes that link to "projects"
kimun notes                               # list all notes
kimun notes --path "journal/"             # list the notes in a folder
kimun labels                              # list all tags
```

The query is the same as in the apps. See [Search](@/using-kimun/search.md).

## Read a note

```sh
kimun note show "projects/garden"
```

You can leave out the `.md` ending.

## Write notes

```sh
kimun note create "projects/garden" "First idea"     # new note (fails if it exists)
kimun note append "projects/garden" "Another idea"   # add to the end (creates the note if needed)
kimun note quick "Look into async patterns"          # save to the inbox
kimun journal "Met with Alex"                        # add to today's journal
```

You can also send text to a command from another program:

```sh
echo "Piped idea" | kimun note quick
```

To change text in a note, use `replace`. To replace the whole note, use `overwrite`. To remove a note, use `delete`. The last two need `--force`.

```sh
kimun note replace "projects/garden" "TODO" "DONE" --all
kimun note delete "inbox/old-idea" --force
```

### Properties

```sh
kimun note prop list "projects/garden"
kimun note prop set "projects/garden" status active
kimun note prop remove "projects/garden" status
```

## Backups

Before a command overwrites, replaces, or deletes text, Kimün saves the old text in a hidden folder in your notes folder. Kimün keeps these backups for 30 days. Search does not show them. Editing in the apps does not make backups, because the editor has undo.

## Workspaces

```sh
kimun workspace init --name work ~/work-notes
kimun workspace list
kimun workspace use work
```

See [Workspaces](@/getting-started/workspaces.md).

## Check your keys

```sh
kimun doctor
```

This shows which keys your terminal can send. See [Troubleshooting](@/getting-started/troubleshooting.md).

## Use another settings file

```sh
kimun --config /path/to/config.toml <command>
```
