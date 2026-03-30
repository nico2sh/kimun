# Workspaces

## What is a Workspace

A workspace is a notes directory with its own isolated SQLite search index (`kimun.sqlite`). Each workspace is completely independent — your work notes don't interfere with your personal notes, and each can have its own file structure, content, and search index.

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

Rename an existing workspace without changing its path:

```sh
kimun workspace rename <old-name> <new-name>
```

**Example:**

```sh
kimun workspace rename work work-archive
```

This updates your config but does not move or rename the files on disk. The workspace continues pointing to the same directory.

### Remove a Workspace

Remove a workspace from your configuration:

```sh
kimun workspace remove <name>
```

**Example:**

```sh
kimun workspace remove archive
```

This removes the workspace entry from your config but **does not delete the notes directory or files**. Your notes remain untouched on disk — you can always re-add the workspace later or access the files manually.

### Rebuild the Search Index

Reindex a workspace to rebuild its SQLite search database:

```sh
kimun workspace reindex <name>
```

**Example:**

```sh
kimun workspace reindex work
```

This is useful if the search index becomes corrupted or if you've manually added/modified notes outside of Kimün and want to rebuild the index.

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

This rebuilds the `kimun.sqlite` index in `/Users/alice/work-notes` without changing the active workspace.

## Legacy Migration

If you're upgrading from an older version of Kimün that used a single-workspace configuration, the migration happens automatically on first run. Your existing workspace configuration is preserved and converted to the new multi-workspace format with a default workspace name.

**No manual action is required.** When you run Kimün after upgrading:
1. Your old notes directory and search index continue to work
2. A new config entry is created for the workspace
3. All subsequent operations use the multi-workspace system

If you want to rename the default workspace or add additional workspaces, use the workspace commands as described above.

## TUI vs CLI

The active workspace can be changed in two ways:

### From the CLI

Use `kimun workspace use <name>` to switch workspaces:

```sh
kimun workspace use work
```

### From the TUI

Open the Settings screen (default: `Ctrl+P`) and change the active workspace. The TUI displays a list of configured workspaces to choose from.

Both methods update the same config file (`kimun_config.toml`), so changes made in the TUI are immediately reflected in CLI commands and vice versa.
