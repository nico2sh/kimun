+++
title = "Workspaces"
weight = 4
+++

# Workspaces

A workspace is a notes directory with its own search index. Each workspace has its own files and its own index, so a search in one workspace never returns notes from another workspace. For example:

- `work`: projects, meeting notes, documentation
- `personal`: journal, ideas, todo lists
- `archive`: old notes that you want to keep out of the way

The active workspace determines which notes you see and search. You can switch the workspace from the CLI or from the Preferences screen of the TUI.

Kimün stores the index of each workspace next to your `config.toml` as `<workspace>.kimuncache`. Kimün can rebuild this file, so you can delete it. The history file is `<workspace>.txt` in `<config_dir>/history/`. You can change both locations. See [Configuration](@/getting-started/configuration.md#files-kimun-stores-on-disk).

## Quick Tour

This example sets up two workspaces in five commands:

```sh
kimun workspace init --name work ~/work-notes        # create
kimun workspace init --name personal ~/personal-notes
kimun workspace list                                 # see them ("work" is active, because you created it first)
kimun workspace use personal                         # switch
kimun search "meeting"                               # searches ~/personal-notes only
```

`kimun workspace list` marks the active one:

```
work      /Users/alice/work-notes
personal  /Users/alice/personal-notes   (active)
```

## Subcommands

### Create a workspace (`init`)

```sh
kimun workspace init --name <name> <path>
```

This command creates the configuration entry. It also creates the directory if the directory does not exist. Kimün changes the name to lowercase and checks it against the [Workspace Name Rules](@/getting-started/configuration.md#workspace-name-rules). Kimün rejects an invalid name (for example, a name that contains `/`) before it writes anything.

### List workspaces (`list`)

```sh
kimun workspace list
```

This command lists every configured workspace and marks the `(active)` workspace. All other commands and the TUI use the active workspace.

### Switch the active workspace (`use`)

```sh
kimun workspace use <name>
```

After this command, search, note listing, and the TUI use that workspace.

### Rename a workspace (`rename`)

```sh
kimun workspace rename <old-name> <new-name>
```

This command renames the key in `config.toml`. It also moves the cache file (`<old>.kimuncache` → `<new>.kimuncache`) and the history file (`<old>.txt` → `<new>.txt`). Kimün does not change your notes directory. Kimün checks the new name like any other name. If a cache file or a history file already exists with the new name, the command stops before it changes anything, so nothing is overwritten.

### Remove a workspace (`remove`)

```sh
kimun workspace remove <name>
```

This command removes the configuration entry and deletes the cache file and the history file of the workspace. Kimün does not change your notes directory. If you add the workspace again, Kimün rebuilds the index from the beginning.

### Rebuild the search index (`reindex`)

```sh
kimun workspace reindex <name>
```

This command rebuilds the SQLite search database at the configured location (`<cache_dir>/<workspace>.kimuncache`). Use it if the index is corrupted, or after you edit notes outside Kimün.

## Legacy Migration

When you upgrade from an older version of Kimün, Kimün migrates the configuration automatically on the first run:

- Single workspace (before `config_version = 2`): your `workspace_dir` and `last_paths` become a `default` workspace block.
- Multiple workspaces (`config_version = 2`): Kimün moves the cache files to `cache_dir`, extracts the history, and saves a backup of the original configuration at `config.toml.bak.v2`. For details, see [Configuration → Upgrading](@/getting-started/configuration.md#upgrading-from-config-version-2).

You do not need to do anything, unless an existing workspace name breaks the [name rules](@/getting-started/configuration.md#workspace-name-rules). In that case, Kimün stops with an error that lists every invalid name. Rename the workspaces and start Kimün again.

## TUI vs CLI

You can switch the active workspace in two ways:

- CLI: `kimun workspace use <name>`
- TUI: open the Preferences screen (`Ctrl+,`) and pick a workspace from the list

Both ways write the same `config.toml`, so a change in one way also appears in the other.
