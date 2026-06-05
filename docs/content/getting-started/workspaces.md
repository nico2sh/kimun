+++
title = "Workspaces"
weight = 4
+++

# Workspaces

A workspace is a notes directory with its own isolated search index. Work notes don't bleed into personal notes; each workspace has its own file structure, content, and index. For example:

- **work** — projects, meeting notes, documentation
- **personal** — journal, ideas, todo lists
- **archive** — old notes you want to keep but not trip over

The active workspace determines what you see and search. Switch instantly from the CLI or the TUI's Settings screen.

Under the hood, each workspace's index lives next to your `config.toml` as `<workspace>.kimuncache` (regenerable; safe to delete), paired with a `<workspace>.txt` history file under `<config_dir>/history/`. Both locations are configurable — see [Configuration](@/getting-started/configuration.md#files-kimun-stores-on-disk).

## Quick Tour

A whole multi-workspace setup in five commands:

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

Details on each subcommand below.

## Subcommands

### `init` — create a workspace

```sh
kimun workspace init --name <name> <path>
```

Creates the config entry and the directory itself if it doesn't exist. The name is lowercased and validated against the [Workspace Name Rules](@/getting-started/configuration.md#workspace-name-rules) — invalid names (e.g. containing `/`) are rejected before anything is written.

### `list` — show all workspaces

```sh
kimun workspace list
```

Lists every configured workspace and marks the `(active)` one — the workspace used by all other commands and the TUI.

### `use` — switch the active workspace

```sh
kimun workspace use <name>
```

From then on, search, note listing, and the TUI all use that workspace.

### `rename` — rename a workspace

```sh
kimun workspace rename <old-name> <new-name>
```

Renames the key in `config.toml` and moves the cache (`<old>.kimuncache` → `<new>.kimuncache`) and history (`<old>.txt` → `<new>.txt`) files with it. Your notes directory is not touched. The new name is validated like any other; if a cache or history file already exists at the new name, the rename aborts before any change so nothing is overwritten.

### `remove` — remove a workspace

```sh
kimun workspace remove <name>
```

Removes the config entry and deletes the workspace's cache and history files. **Your notes directory is not touched** — re-add the workspace anytime and the index rebuilds from scratch.

### `reindex` — rebuild the search index

```sh
kimun workspace reindex <name>
```

Rebuilds the SQLite search database at the configured location (`<cache_dir>/<workspace>.kimuncache`). Useful if the index gets corrupted, or you've been editing notes behind Kimün's back and want it to catch up.

## Legacy Migration

Upgrading from an older Kimün? Migration happens automatically on first run:

- **Single-workspace (pre-`config_version = 2`):** your `workspace_dir` and `last_paths` become a `default` workspace block.
- **Multi-workspace `config_version = 2`:** cache files move to `cache_dir`, history is extracted, and a backup of the original config lands at `config.toml.bak.v2`. Full details in [Configuration → Upgrading](@/getting-started/configuration.md#upgrading-from-config-version-2).

No manual action needed — unless an existing workspace name violates the [name rules](@/getting-started/configuration.md#workspace-name-rules), in which case Kimün aborts with an error listing every offending name so you can rename and relaunch.

## TUI vs CLI

Two ways to switch the active workspace, same result:

- **CLI:** `kimun workspace use <name>`
- **TUI:** Settings screen (`Ctrl+,`) → pick from the workspace list

Both write the same `config.toml`, so changes in one are immediately visible in the other.
