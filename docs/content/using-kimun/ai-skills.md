+++
title = "CLI Skill"
weight = 21
+++

# Skills

A skill is a short guide file that teaches an AI coding assistant how to use a tool. Kimün has two skills:

- `kimun-cli` teaches the `kimun` commands. Then the assistant can manage your notes during a session.
- `kimun-ask` teaches a method to answer questions about your notes. It works with the [MCP server](@/using-kimun/ai-mcp-server.md) or with the `kimun` commands. It includes recipes to research a topic, find connections, suggest links, and brainstorm.

With the skill, you can ask:

> "Log today's standup notes to my journal."

> "Search my notes for authentication and summarize what you find."

> "Create a note in inbox/ with the output of this command."

With the `kimun-ask` skill, you can ask:

> "What did I write about the garden last week?"

> "Which notes are close in meaning to my roadmap note, but not linked to it?"

The skill uses semantic search when a Kimün server is set up. See [Semantic search](@/using-kimun/server.md). Without a server, the assistant uses word search.

## Install

Use the standard skills tool. It installs both skills:

```sh
npx skills add nico2sh/kimun
```

To refresh the skills after a new release, run this command:

```sh
npx skills update
```

### Without Node

If you do not have Node, Kimün can copy the skills itself:

```sh
kimun skill install
```

The command writes into the skills folder of Claude Code and the skills folder of Codex. If an app is not installed, the command skips its folder. To install one skill only, add `--only ask` or `--only cli`. To write into another folder, add `--dir PATH`. Kimün then writes `PATH/kimun-ask/` and `PATH/kimun-cli/`.

The command follows these rules:

- It never overwrites a skill folder that differs from the version in your Kimün. It keeps the folder and tells you.
- Add `--force` to replace such a folder. The command replaces only a plain folder.
- It never writes through a symbolic link. A link usually comes from another tool that manages the skill. The command refuses and changes nothing.

To update the skills later, run `kimun update` for the app, then run `kimun skill install` again. A folder with the old text needs `--force`.

If you installed the skills with `npx skills`, update them with `npx skills update` and not with `kimun skill install`. The standard tool can keep the real copy in the Codex folder. `kimun skill install --force` replaces that copy with the copy built into this binary.

## Safety

The assistant must add `--force` to overwrite or delete a note. Kimün saves a backup of the old text first. See [Backups](@/using-kimun/cli.md#backups).
