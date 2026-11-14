+++
title = "Workspaces"
weight = 4
+++

# Workspaces

A workspace is a notes folder with its own search. You can keep several workspaces. For example, one for `work` and one for `personal`. A search in one workspace never shows notes from another workspace.

## Switch workspace

- Press `F5`, or click the workspace name in the title bar.
- Pick a workspace from the list.

## Add, rename, or remove a workspace

Open Preferences (`Ctrl+,` or `F4`) and go to Workspaces. You can add a folder, rename a workspace, or remove it. Removing a workspace never deletes your notes.

## From the command line

```sh
kimun workspace init --name work ~/work-notes   # add a workspace
kimun workspace list                            # show all workspaces
kimun workspace use personal                    # switch workspace
kimun workspace rename old-name new-name
kimun workspace remove work
kimun workspace reindex work                    # rebuild the search index
```

A workspace name can use letters, digits, and dashes. Kimün changes it to lowercase. It rejects names with characters that some systems do not allow in file names, such as `/`, `:`, or `*`.

## Search shows wrong results

If search misses a note, rebuild the index:

```sh
kimun workspace reindex <name>
```

Do this also after you edit notes with another app.
