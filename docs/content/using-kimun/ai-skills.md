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

For Claude Code, copy the skill folders to your skills folder:

```sh
cp -r skills/kimun-cli skills/kimun-ask ~/.claude/skills
```

For another tool, copy `skills/kimun-cli/SKILL.md` and `skills/kimun-ask/SKILL.md` to the place where that tool loads skills. For Codex, this is `~/.agents/skills/`. Install only the skill that you need.

## Safety

The assistant must add `--force` to overwrite or delete a note. Kimün saves a backup of the old text first. See [Backups](@/using-kimun/cli.md#backups).
