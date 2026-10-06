+++
title = "Workspaces"
weight = 4
+++

# Workspaces

A workspace is a notes directory with its own search index. Each workspace has its own files and index, so searches in one never return notes from another. For example:

- **work**: projects, meeting notes, documentation
- **personal**: journal, ideas, todo lists
- **archive**: old notes you want to keep out of the way

The active workspace determines what you see and search. You can switch from the CLI or from the TUI's Preferences screen.

Each workspace's index is stored next to your `config.toml` as `<workspace>.kimuncache` (regenerable, safe to delete), with a `<workspace>.txt` history file under `<config_dir>/history/`. Both locations are configurable. See [Configuration](@/getting-started/configuration.md#files-kimun-stores-on-disk).

## Quick Tour

A multi-workspace setup in five commands:

```sh
kimun workspace init --name work ~/work-notes        # create
kimun workspace init --name personal ~/personal-notes
kimun workspace list                                 # see them ("work" is active — created first)
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

Creates the config entry and the directory itself if it doesn't exist. The name is lowercased and validated against the [Workspace Name Rules](@/getting-started/configuration.md#workspace-name-rules). Invalid names (for example, ones containing `/`) are rejected before anything is written.

### List workspaces (`list`)

```sh
kimun workspace list
```

Lists every configured workspace and marks the `(active)` one, which all other commands and the TUI use.

### Switch the active workspace (`use`)

```sh
kimun workspace use <name>
```

From then on, search, note listing, and the TUI all use that workspace.

### Rename a workspace (`rename`)

```sh
kimun workspace rename <old-name> <new-name>
```

Renames the key in `config.toml` and moves the cache (`<old>.kimuncache` → `<new>.kimuncache`) and history (`<old>.txt` → `<new>.txt`) files with it. Your notes directory is not touched. The new name is validated like any other; if a cache or history file already exists at the new name, the rename aborts before any change so nothing is overwritten.

### Remove a workspace (`remove`)

```sh
kimun workspace remove <name>
```

Removes the config entry and deletes the workspace's cache and history files. Your notes directory is not touched. If you add the workspace again, the index is rebuilt from scratch.

### Rebuild the search index (`reindex`)

```sh
kimun workspace reindex <name>
```

Rebuilds the SQLite search database at the configured location (`<cache_dir>/<workspace>.kimuncache`). Use it if the index gets corrupted, or after editing notes outside Kimün.

## Legacy Migration

When you upgrade from an older version of Kimün, the config is migrated automatically on first run:

- **Single-workspace (pre-`config_version = 2`):** your `workspace_dir` and `last_paths` become a `default` workspace block.
- **Multi-workspace `config_version = 2`:** cache files move to `cache_dir`, history is extracted, and a backup of the original config lands at `config.toml.bak.v2`. Full details in [Configuration → Upgrading](@/getting-started/configuration.md#upgrading-from-config-version-2).

You don't need to do anything, unless an existing workspace name breaks the [name rules](@/getting-started/configuration.md#workspace-name-rules). In that case Kimün stops with an error listing every invalid name, so you can rename them and relaunch.

## TUI vs CLI

You can switch the active workspace in two ways:

- **CLI:** `kimun workspace use <name>`
- **TUI:** Preferences screen (`Ctrl+,`) → pick from the workspace list

Both write the same `config.toml`, so a change made in one shows up in the other.
