+++
title = "Workspaces"
weight = 4
+++

# Workspaces

## What is a Workspace

A workspace is a notes directory with its own isolated SQLite search index. The index lives next to your `config.toml` as `<workspace>.kimuncache` (regenerable; safe to delete) and is paired with a `<workspace>.txt` history file under `<config_dir>/history/`. Both locations are configurable — see [Configuration](@/getting-started/configuration.md#files-kimun-stores-on-disk).

Each workspace is completely independent — your work notes don't interfere with your personal notes, and each can have its own file structure, content, and search index.

Workspaces let you organize notes into separate contexts. For example:
- **work** — Professional projects, meeting notes, documentation
- **personal** — Journal entries, ideas, todo lists
- **archive** — Older notes you want to preserve but not actively search

You can switch between workspaces instantly using the CLI or from the Settings screen in the TUI. The active workspace determines which notes you see and search.

## Workspace Subcommands

All workspace operations are accessed via the `kimun workspace` command. Here are the available subcommands:

### Initialize a Workspace

Create a new workspace with a given name and path:

```sh
kimun workspace init --name <name> <path>
```

**Example:**

```sh
kimun workspace init --name work /Users/alice/work-notes
kimun workspace init --name personal /Users/alice/personal-notes
```

This creates a new entry in your config file and prepares the workspace for use. If the directory doesn't exist, Kimün will create it.

The name is lowercased and validated — see the [Workspace Name Rules](@/getting-started/configuration.md#workspace-name-rules) for what characters are allowed. Invalid names (e.g. containing `/`) are rejected with an error before anything is written.

### List All Workspaces

Display all configured workspaces and mark the currently active one:

```sh
kimun workspace list
```

**Example output:**

```
work         /Users/alice/work-notes       (active)
personal     /Users/alice/personal-notes
archive      /Users/alice/archive-notes
```

The workspace marked with `(active)` is the one used when you run other Kimün commands or open the TUI.

### Switch Active Workspace

Change which workspace is currently active:

```sh
kimun workspace use <name>
```

**Example:**

```sh
kimun workspace use personal
```

After running this command, all subsequent Kimün operations (search, notes listing, TUI) will use the `personal` workspace. You can verify the change by running `kimun workspace list`.

### Rename a Workspace

Rename an existing workspace without changing its notes directory:

```sh
kimun workspace rename <old-name> <new-name>
```

**Example:**

```sh
kimun workspace rename work work-archive
```

The new name is lowercased and validated against the [Workspace Name Rules](@/getting-started/configuration.md#workspace-name-rules). Kimün renames the workspace key in `config.toml` and moves the cache (`<old>.kimuncache` → `<new>.kimuncache`) and history (`<old>.txt` → `<new>.txt`) files alongside it. Your notes directory is not touched. If a cache or history file already exists at the new name, the rename aborts before any change so nothing is overwritten.

### Remove a Workspace

Remove a workspace from your configuration:

```sh
kimun workspace remove <name>
```

**Example:**

```sh
kimun workspace remove archive
```

This removes the workspace entry from your config and deletes the workspace's cache (`<name>.kimuncache`) and history (`<name>.txt`) files. **Your notes directory is not touched** — you can always re-add the workspace later or access the files manually. If you do re-add it, the cache will be rebuilt from scratch on first use.

### Rebuild the Search Index

Reindex a workspace to rebuild its SQLite search database:

```sh
kimun workspace reindex <name>
```

**Example:**

```sh
kimun workspace reindex work
```

This is useful if the search index becomes corrupted or if you've manually added/modified notes outside of Kimün and want to rebuild the index. The cache is rewritten at the configured location (`<cache_dir>/<workspace>.kimuncache`).

## Walkthrough: Setting Up Multiple Workspaces

Let's walk through setting up two workspaces — `work` and `personal` — and switching between them:

**Step 1: Create the work workspace**

```sh
kimun workspace init --name work ~/work-notes
```

**Step 2: Create the personal workspace**

```sh
kimun workspace init --name personal ~/personal-notes
```

**Step 3: List all workspaces**

```sh
kimun workspace list
```

Output:
```
work      /Users/alice/work-notes       (active)
personal  /Users/alice/personal-notes
```

The `work` workspace is now active (created first).

**Step 4: Switch to personal workspace**

```sh
kimun workspace use personal
```

**Step 5: Verify the switch**

```sh
kimun workspace list
```

Output:
```
work      /Users/alice/work-notes
personal  /Users/alice/personal-notes   (active)
```

Now `personal` is marked as active.

**Step 6: Search in the active workspace**

When you run `kimun search`, it searches only the active workspace:

```sh
kimun search "meeting"
```

This searches only notes in `~/personal-notes`.

**Step 7: Reindex the work workspace**

If you want to rebuild the index for the `work` workspace (perhaps you added files directly):

```sh
kimun workspace reindex work
```

This rebuilds the cache file (`<config_dir>/work.kimuncache` by default) without changing the active workspace.

## Legacy Migration

If you're upgrading from an older version of Kimün, the migration happens automatically on first run. The exact sequence depends on which version you're coming from:

- **Single-workspace (pre-`config_version = 2`):** your `workspace_dir` and `last_paths` are converted into a `default` workspace block.
- **Multi-workspace `config_version = 2`:** your existing `<workspace>/kimun.sqlite` files are moved into the configured `cache_dir` as `<workspace>.kimuncache`, and per-workspace `last_paths` are extracted into `<history_dir>/<workspace>.txt`. A backup of the original config is written to `config.toml.bak.v2`. See [Configuration → Upgrading from `config_version = 2`](@/getting-started/configuration.md#upgrading-from-config-version-2) for details.

**No manual action is required** unless one of your existing workspace names violates the new [Workspace Name Rules](@/getting-started/configuration.md#workspace-name-rules) — in that case Kimün aborts the migration with an error listing every offending name and leaves the config at version 2 so you can rename them and relaunch.

## TUI vs CLI

The active workspace can be changed in two ways:

### From the CLI

Use `kimun workspace use <name>` to switch workspaces:

```sh
kimun workspace use work
```

### From the TUI

Open the Settings screen (default: `Ctrl+P`) and change the active workspace. The TUI displays a list of configured workspaces to choose from.

Both methods update the same config file (`config.toml`), so changes made in the TUI are immediately reflected in CLI commands and vice versa.
